//! SteamDeck Battery Health & Charging Management Subsystem (TASK-KNT-336).
//!
//! Provides fuel gauge coulomb counting, battery degradation cycle aging estimation,
//! NVDC power-path passthrough charging governance, and JEITA low-temperature
//! lithium charging protection with dynamic current throttling.

#![allow(dead_code)]

use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};

// ============================================================================
// Feature 1: Fuel Gauge Coulomb Counter & Cycle Aging Estimator
// ============================================================================

/// Design capacity of standard SteamDeck 40.04 Wh Li-ion battery pack.
pub const STEAMDECK_DESIGN_CAPACITY_MWH: u32 = 40_040;
pub const STEAMDECK_DESIGN_VOLTAGE_MV: u32 = 7_700; // 2S nominal 7.7V (3.85V/cell)
pub const STEAMDECK_FULL_VOLTAGE_MV: u32 = 8_700;   // 2S full 8.7V (4.35V/cell)
pub const STEAMDECK_CUTOFF_VOLTAGE_MV: u32 = 6_000; // 2S empty 6.0V (3.00V/cell)

/// Battery operating state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryState {
    Discharging,
    Charging,
    Full,
    NotChargingPassthrough,
    CriticalLow,
}

/// Fuel gauge telemetry snapshot.
#[derive(Debug, Clone, Copy)]
pub struct FuelGaugeTelemetry {
    pub pack_voltage_mv: u32,
    pub current_ma: i32, // Positive = into battery (charging), negative = discharging
    pub power_mw: i32,
    pub state_of_charge_pct: u8,
    pub state_of_health_pct: u8,
    pub remaining_capacity_mwh: u32,
    pub full_charge_capacity_mwh: u32,
    pub cycle_count: u32,
    pub cell_temp_mdeg_c: i32,
}

/// High-precision fuel gauge coulomb counter and battery degradation monitor.
pub struct PrecisionCoulombCounter {
    pack_voltage_mv: AtomicU32,
    current_ma: AtomicI32,
    coulomb_accum_uamp_sec: AtomicI32,
    remaining_capacity_mwh: AtomicU32,
    full_charge_capacity_mwh: AtomicU32,
    design_capacity_mwh: u32,
    accumulated_discharge_mah: AtomicU32,
    cycle_count: AtomicU32,
    internal_resistance_mohm: AtomicU32,
    last_update_ts: AtomicU64,
}

impl PrecisionCoulombCounter {
    pub const fn new() -> Self {
        Self {
            pack_voltage_mv: AtomicU32::new(7_700),
            current_ma: AtomicI32::new(0),
            coulomb_accum_uamp_sec: AtomicI32::new(0),
            remaining_capacity_mwh: AtomicU32::new(STEAMDECK_DESIGN_CAPACITY_MWH),
            full_charge_capacity_mwh: AtomicU32::new(STEAMDECK_DESIGN_CAPACITY_MWH),
            design_capacity_mwh: STEAMDECK_DESIGN_CAPACITY_MWH,
            accumulated_discharge_mah: AtomicU32::new(0),
            cycle_count: AtomicU32::new(0),
            internal_resistance_mohm: AtomicU32::new(65), // Nominal ~65 mΩ pack impedance
            last_update_ts: AtomicU64::new(0),
        }
    }

    /// Integrates current over time (coulomb counting) and updates remaining capacity.
    pub fn update_coulomb(&self, voltage_mv: u32, current_ma: i32, dt_ms: u32, timestamp: u64) {
        self.pack_voltage_mv.store(voltage_mv, Ordering::Release);
        self.current_ma.store(current_ma, Ordering::Release);
        self.last_update_ts.store(timestamp, Ordering::Release);

        // Microamp-seconds = current_ma * 1000 * dt_ms / 1000 = current_ma * dt_ms
        let delta_uamp_sec = current_ma as i64 * dt_ms as i64;
        let prev_accum = self.coulomb_accum_uamp_sec.load(Ordering::Acquire) as i64;
        let new_accum = prev_accum + delta_uamp_sec;

        // 1 mAh = 3,600,000 uA*sec
        // 1 mWh = (1 mAh * voltage_mv) / 1000
        if new_accum.abs() >= 3_600_000 {
            let delta_mah = (new_accum / 3_600_000) as i32;
            let remainder = (new_accum % 3_600_000) as i32;
            self.coulomb_accum_uamp_sec.store(remainder, Ordering::Release);

            let delta_mwh = (delta_mah as i64 * voltage_mv as i64 / 1000) as i32;
            let current_rem = self.remaining_capacity_mwh.load(Ordering::Acquire) as i32;
            let fcc = self.full_charge_capacity_mwh.load(Ordering::Acquire) as i32;

            let updated_rem = (current_rem + delta_mwh).max(0).min(fcc) as u32;
            self.remaining_capacity_mwh.store(updated_rem, Ordering::Release);

            // Track discharge cycles: 1 cycle = full design capacity discharged (~5200 mAh)
            if delta_mah < 0 {
                let discharge = self.accumulated_discharge_mah.fetch_add((-delta_mah) as u32, Ordering::AcqRel) + (-delta_mah) as u32;
                let design_mah = (self.design_capacity_mwh * 1000) / STEAMDECK_DESIGN_VOLTAGE_MV;
                let cycles = discharge / design_mah;
                self.cycle_count.store(cycles, Ordering::Release);

                // Age full charge capacity slightly with cycles (empirical Li-ion model: ~0.035% per cycle)
                if cycles > 0 {
                    let degraded_fcc = (self.design_capacity_mwh as u64 * (10_000 - (cycles as u64 * 35).min(4000))) / 10_000;
                    self.full_charge_capacity_mwh.store(degraded_fcc as u32, Ordering::Release);
                }
            }
        }
    }

    /// Calculates State of Charge (SoC) percentage (0..100%).
    pub fn get_state_of_charge_pct(&self) -> u8 {
        let rem = self.remaining_capacity_mwh.load(Ordering::Acquire);
        let fcc = self.full_charge_capacity_mwh.load(Ordering::Acquire);
        if fcc == 0 {
            return 0;
        }
        ((rem as u64 * 100) / fcc as u64).min(100) as u8
    }

    /// Calculates State of Health (SoH) percentage based on capacity retention.
    pub fn get_state_of_health_pct(&self) -> u8 {
        let fcc = self.full_charge_capacity_mwh.load(Ordering::Acquire);
        ((fcc as u64 * 100) / self.design_capacity_mwh as u64).min(100) as u8
    }

    pub fn get_voltage_mv(&self) -> u32 {
        self.pack_voltage_mv.load(Ordering::Acquire)
    }

    pub fn get_current_ma(&self) -> i32 {
        self.current_ma.load(Ordering::Acquire)
    }

    pub fn get_remaining_capacity_mwh(&self) -> u32 {
        self.remaining_capacity_mwh.load(Ordering::Acquire)
    }

    pub fn get_full_charge_capacity_mwh(&self) -> u32 {
        self.full_charge_capacity_mwh.load(Ordering::Acquire)
    }

    pub fn get_cycle_count(&self) -> u32 {
        self.cycle_count.load(Ordering::Acquire)
    }

    pub fn set_telemetry(&self, rem_mwh: u32, fcc_mwh: u32, cycles: u32) {
        self.remaining_capacity_mwh.store(rem_mwh, Ordering::Release);
        self.full_charge_capacity_mwh.store(fcc_mwh, Ordering::Release);
        self.cycle_count.store(cycles, Ordering::Release);
    }
}

pub static COULOMB_COUNTER: PrecisionCoulombCounter = PrecisionCoulombCounter::new();
