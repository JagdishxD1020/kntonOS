//! Handheld Battery Gauge Fuel Coulomb Counter & Dynamic Charge Limiter Service.

#![allow(dead_code)]

/// Battery fuel gauge hardware registers and instant telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryGaugeRegisters {
    pub cell_voltage_mv: u16,
    pub current_ma: i16,
    pub state_of_charge_pct: u8,
    pub temperature_c: i8,
    pub internal_impedance_mohm: u16,
}

impl BatteryGaugeRegisters {
    /// Creates a new default battery gauge reading (4.0V, idle, 75% SoC, 28°C, 35 mΩ).
    pub const fn new() -> Self {
        Self {
            cell_voltage_mv: 4000,
            current_ma: -500, // 500mA discharge
            state_of_charge_pct: 75,
            temperature_c: 28,
            internal_impedance_mohm: 35,
        }
    }

    /// Validates electrical sanity checks for handheld lithium-ion / LiPo cells.
    pub fn is_electrically_valid(&self) -> bool {
        self.cell_voltage_mv >= 2800
            && self.cell_voltage_mv <= 4450
            && self.state_of_charge_pct <= 100
            && self.temperature_c >= -20
            && self.temperature_c <= 85
            && self.internal_impedance_mohm > 0
            && self.internal_impedance_mohm < 1000
    }
}

/// Unit verification for battery gauge hardware registers.
pub fn verify_battery_gauge_registers() -> bool {
    let gauge = BatteryGaugeRegisters::new();
    if !gauge.is_electrically_valid() {
        return false;
    }

    // Invalid high voltage check (overcharged)
    let invalid = BatteryGaugeRegisters {
        cell_voltage_mv: 5000,
        current_ma: 0,
        state_of_charge_pct: 100,
        temperature_c: 25,
        internal_impedance_mohm: 30,
    };
    if invalid.is_electrically_valid() {
        return false;
    }

    true
}
