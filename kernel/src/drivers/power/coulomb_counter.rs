//! Handheld Real-Time Battery Discharge Curve Profiler & Micro-Watt Coulomb Counter.

#![allow(dead_code)]

/// Full design battery pack capacity in milliampere-hours (5000 mAh).
pub const PACK_DESIGN_CAPACITY_MAH: f32 = 5000.0;
/// Full design battery pack energy in micro-watt-hours (38.5 Wh = 38,500,000 μWh).
pub const PACK_DESIGN_ENERGY_UWH: u64 = 38_500_000;
/// Nominal 2S battery pack terminal voltage in millivolts (7700 mV).
pub const PACK_NOMINAL_VOLTAGE_MV: f32 = 7700.0;

/// Micro-Watt Coulomb Counter tracking charge and energy displacement.
#[derive(Debug, Clone)]
pub struct MicroWattCoulombCounter {
    /// Instantaneous battery pack terminal voltage in millivolts (mV).
    pub voltage_mv: f32,
    /// Instantaneous battery current in milliamperes (mA, negative = discharge, positive = charge).
    pub current_ma: f32,
    /// Instantaneous power in micro-watts (μW).
    pub power_uw: u64,
    /// Remaining usable charge in milliampere-hours (mAh).
    pub remaining_charge_mah: f32,
    /// Remaining usable energy in micro-watt-hours (μWh).
    pub remaining_energy_uwh: u64,
    /// Total lifetime discharged charge in milliampere-hours (mAh).
    pub total_discharged_mah: f64,
    /// Total lifetime discharged energy in micro-watt-hours (μWh).
    pub total_discharged_uwh: u64,
    /// Relative State-of-Charge percentage (0.0 to 100.0%).
    pub state_of_charge_pct: f32,
}

impl MicroWattCoulombCounter {
    pub const fn new() -> Self {
        Self {
            voltage_mv: 8400.0,
            current_ma: 0.0,
            power_uw: 0,
            remaining_charge_mah: PACK_DESIGN_CAPACITY_MAH,
            remaining_energy_uwh: PACK_DESIGN_ENERGY_UWH,
            total_discharged_mah: 0.0,
            total_discharged_uwh: 0,
            state_of_charge_pct: 100.0,
        }
    }

    /// Integrates instantaneous current and voltage over time delta (in seconds).
    pub fn integrate(&mut self, voltage_mv: f32, current_ma: f32, delta_sec: f32) {
        self.voltage_mv = voltage_mv;
        self.current_ma = current_ma;

        // Instantaneous power: P (uW) = |I (mA)| * V (mV) * 1000
        let abs_i_ma = current_ma.abs();
        self.power_uw = (abs_i_ma * voltage_mv) as u64;

        // Charge delta: dQ (mAh) = I (mA) * dt (s) / 3600
        let delta_mah = (current_ma * delta_sec) / 3600.0;
        self.remaining_charge_mah = (self.remaining_charge_mah + delta_mah)
            .clamp(0.0, PACK_DESIGN_CAPACITY_MAH);

        // Energy delta: dE (uWh) = P (uW) * dt (s) / 3600
        let delta_uwh = ((self.power_uw as f64 * delta_sec as f64) / 3600.0) as u64;

        if current_ma < 0.0 {
            // Discharging: subtract energy from pack
            self.remaining_energy_uwh = self.remaining_energy_uwh.saturating_sub(delta_uwh);
            self.total_discharged_mah += (-delta_mah) as f64;
            self.total_discharged_uwh += delta_uwh;
        } else {
            // Charging: accumulate energy into pack
            self.remaining_energy_uwh = (self.remaining_energy_uwh + delta_uwh)
                .min(PACK_DESIGN_ENERGY_UWH);
        }

        self.state_of_charge_pct = (self.remaining_charge_mah / PACK_DESIGN_CAPACITY_MAH) * 100.0;
    }
}

/// Unit test verifying micro-watt coulomb counter and energy integration.
pub fn verify_coulomb_integration() -> bool {
    let mut counter = MicroWattCoulombCounter::new();
    if counter.state_of_charge_pct != 100.0 || counter.remaining_charge_mah != 5000.0 {
        return false;
    }

    // 1. Simulate 1 hour (3600s) of steady gaming discharge at 2500mA (2.5A) and 7600mV (~19W)
    // Run 360 steps of 10s each
    for _ in 0..360 {
        counter.integrate(7600.0, -2500.0, 10.0);
    }

    // Must have discharged exactly 2500 mAh (leaving 2500 mAh = 50% SoC)
    if (counter.remaining_charge_mah - 2500.0).abs() > 1.0 {
        return false;
    }
    if (counter.state_of_charge_pct - 50.0).abs() > 0.1 {
        return false;
    }
    // Discharged energy: 19W * 1h = 19 Wh = 19,000,000 uWh
    if counter.total_discharged_uwh < 18_900_000 || counter.total_discharged_uwh > 19_100_000 {
        return false;
    }

    // 2. Simulate 30 minutes (1800s) of fast charging at +3000mA (3A) and 8200mV
    for _ in 0..180 {
        counter.integrate(8200.0, 3000.0, 10.0);
    }

    // Charged 3000mA * 0.5h = 1500 mAh (total now 4000 mAh = 80% SoC)
    if (counter.remaining_charge_mah - 4000.0).abs() > 1.5 {
        return false;
    }
    if (counter.state_of_charge_pct - 80.0).abs() > 0.1 {
        return false;
    }

    true
}
