//! SteamDeck Thermal & Fan Acoustic Profile Governor (TASK-KNT-330).
//!
//! Provides multi-zone thermal matrix monitoring, low-noise acoustic fan curve
//! smoothing with hysteresis, handheld skin-temperature comfort governance,
//! and fan tachometer fault detection with emergency thermal relief.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

// ============================================================================
// Feature 1: Multi-Zone Thermal Sensor Matrix Monitoring
// ============================================================================

/// Thermal monitoring zone across SteamDeck handheld chassis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalZone {
    /// AMD Aerith/Van Gogh APU core & RDNA2 GPU silicon junction.
    ApuCore,
    /// Chassis internal intake and motherboard ambient air.
    Ambient,
    /// Dual-cell Li-ion battery pack monitoring sensor.
    BatteryCell,
    /// PCIe M.2 2230 NVMe solid state drive controller.
    NvmeDrive,
}

impl ThermalZone {
    pub const COUNT: usize = 4;

    pub const fn index(&self) -> usize {
        match self {
            Self::ApuCore => 0,
            Self::Ambient => 1,
            Self::BatteryCell => 2,
            Self::NvmeDrive => 3,
        }
    }

    pub const fn from_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(Self::ApuCore),
            1 => Some(Self::Ambient),
            2 => Some(Self::BatteryCell),
            3 => Some(Self::NvmeDrive),
            _ => None,
        }
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Self::ApuCore => "APU Core/GPU Silicon",
            Self::Ambient => "Chassis Ambient",
            Self::BatteryCell => "Battery Pack",
            Self::NvmeDrive => "NVMe M.2 SSD",
        }
    }

    pub const fn default_warning_mdeg(&self) -> i32 {
        match self {
            Self::ApuCore => 85_000,     // 85.0°C
            Self::Ambient => 45_000,     // 45.0°C
            Self::BatteryCell => 50_000, // 50.0°C
            Self::NvmeDrive => 70_000,   // 70.0°C
        }
    }

    pub const fn default_critical_mdeg(&self) -> i32 {
        match self {
            Self::ApuCore => 100_000,    // 100.0°C
            Self::Ambient => 55_000,     // 55.0°C
            Self::BatteryCell => 60_000, // 60.0°C
            Self::NvmeDrive => 85_000,   // 85.0°C
        }
    }
}

/// Alert severity level for a thermal monitoring zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ZoneAlertLevel {
    /// Safe operating range.
    Normal = 0,
    /// Elevated temperature approaching acoustic boost threshold.
    Warm = 1,
    /// High thermal load; requires maximum active dissipation.
    Hot = 2,
    /// Critical limit breached; immediate thermal throttling or relief required.
    Critical = 3,
}

/// Per-zone atomic telemetry reading.
pub struct ZoneTelemetry {
    temp_mdeg_c: AtomicI32,
    raw_sample_mdeg: AtomicI32,
    warning_mdeg_c: AtomicI32,
    critical_mdeg_c: AtomicI32,
    update_count: AtomicU32,
    last_update_ts: AtomicU64,
}

impl ZoneTelemetry {
    pub const fn new(zone: ThermalZone) -> Self {
        Self {
            temp_mdeg_c: AtomicI32::new(35_000), // Default 35.0°C
            raw_sample_mdeg: AtomicI32::new(35_000),
            warning_mdeg_c: AtomicI32::new(zone.default_warning_mdeg()),
            critical_mdeg_c: AtomicI32::new(zone.default_critical_mdeg()),
            update_count: AtomicU32::new(0),
            last_update_ts: AtomicU64::new(0),
        }
    }

    /// Ingests a new raw sample and applies exponential moving average (alpha = 0.25).
    pub fn update(&self, raw_sample: i32, timestamp: u64) {
        self.raw_sample_mdeg.store(raw_sample, Ordering::Release);
        let prev = self.temp_mdeg_c.load(Ordering::Acquire);
        // EMA: (3 * prev + raw) / 4
        let filtered = (prev * 3 + raw_sample) / 4;
        self.temp_mdeg_c.store(filtered, Ordering::Release);
        self.update_count.fetch_add(1, Ordering::Relaxed);
        self.last_update_ts.store(timestamp, Ordering::Release);
    }

    pub fn get_temperature(&self) -> i32 {
        self.temp_mdeg_c.load(Ordering::Acquire)
    }

    pub fn get_raw_sample(&self) -> i32 {
        self.raw_sample_mdeg.load(Ordering::Acquire)
    }

    pub fn get_alert_level(&self) -> ZoneAlertLevel {
        let temp = self.get_temperature();
        let crit = self.critical_mdeg_c.load(Ordering::Relaxed);
        let warn = self.warning_mdeg_c.load(Ordering::Relaxed);

        if temp >= crit {
            ZoneAlertLevel::Critical
        } else if temp >= warn {
            ZoneAlertLevel::Hot
        } else if temp >= warn - 10_000 {
            ZoneAlertLevel::Warm
        } else {
            ZoneAlertLevel::Normal
        }
    }
}

/// Multi-zone sensor matrix tracking all handheld zones concurrently.
pub struct MultiZoneSensorMatrix {
    zones: [ZoneTelemetry; ThermalZone::COUNT],
    has_alarm: AtomicBool,
}

impl MultiZoneSensorMatrix {
    pub const fn new() -> Self {
        Self {
            zones: [
                ZoneTelemetry::new(ThermalZone::ApuCore),
                ZoneTelemetry::new(ThermalZone::Ambient),
                ZoneTelemetry::new(ThermalZone::BatteryCell),
                ZoneTelemetry::new(ThermalZone::NvmeDrive),
            ],
            has_alarm: AtomicBool::new(false),
        }
    }

    /// Ingests a sensor update for a specified thermal zone.
    pub fn update_sensor(&self, zone: ThermalZone, sample_mdeg_c: i32, timestamp: u64) {
        self.zones[zone.index()].update(sample_mdeg_c, timestamp);
        if self.highest_alert_level() >= ZoneAlertLevel::Hot {
            self.has_alarm.store(true, Ordering::Release);
        } else {
            self.has_alarm.store(false, Ordering::Release);
        }
    }

    /// Queries the filtered temperature for a zone in millidegrees Celsius.
    pub fn get_temperature(&self, zone: ThermalZone) -> i32 {
        self.zones[zone.index()].get_temperature()
    }

    /// Queries the alert severity for a zone.
    pub fn get_alert_level(&self, zone: ThermalZone) -> ZoneAlertLevel {
        self.zones[zone.index()].get_alert_level()
    }

    /// Determines the maximum alert level across all monitoring zones.
    pub fn highest_alert_level(&self) -> ZoneAlertLevel {
        let mut max_alert = ZoneAlertLevel::Normal;
        let mut i = 0;
        while i < ThermalZone::COUNT {
            let alert = self.zones[i].get_alert_level();
            if alert > max_alert {
                max_alert = alert;
            }
            i += 1;
        }
        max_alert
    }

    /// Returns whether any zone is in a critical over-temperature condition.
    pub fn has_critical_overtemp(&self) -> bool {
        self.highest_alert_level() == ZoneAlertLevel::Critical
    }

    /// Resets all zone sensors to baseline ambient.
    pub fn reset(&self) {
        let mut i = 0;
        while i < ThermalZone::COUNT {
            self.zones[i].temp_mdeg_c.store(35_000, Ordering::Release);
            self.zones[i].raw_sample_mdeg.store(35_000, Ordering::Release);
            self.zones[i].update_count.store(0, Ordering::Release);
            self.zones[i].last_update_ts.store(0, Ordering::Release);
            i += 1;
        }
        self.has_alarm.store(false, Ordering::Release);
    }
}

pub static SENSOR_MATRIX: MultiZoneSensorMatrix = MultiZoneSensorMatrix::new();
