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


// ============================================================================
// Feature 2: Low-Noise Acoustic Fan Curve Smoothing & Hysteresis Governor
// ============================================================================

/// Fan curve calibration point mapping temperature (m°C) to target fan RPM.
#[derive(Debug, Clone, Copy)]
pub struct FanCurvePoint {
    pub temp_mdeg_c: i32,
    pub target_rpm: u32,
}

/// Acoustic fan profile curve with piecewise linear interpolation.
pub struct AcousticFanCurve {
    points: [FanCurvePoint; 6],
}

impl AcousticFanCurve {
    pub const DEFAULT: Self = Self {
        points: [
            FanCurvePoint { temp_mdeg_c: 40_000, target_rpm: 0 },      // Below 40°C: Silent stop
            FanCurvePoint { temp_mdeg_c: 55_000, target_rpm: 2_000 },  // 55°C: Whisper intake
            FanCurvePoint { temp_mdeg_c: 68_000, target_rpm: 3_200 },  // 68°C: Low-resonance sweet spot
            FanCurvePoint { temp_mdeg_c: 78_000, target_rpm: 4_400 },  // 78°C: Sustained gaming
            FanCurvePoint { temp_mdeg_c: 88_000, target_rpm: 5_600 },  // 88°C: High-cooling profile
            FanCurvePoint { temp_mdeg_c: 100_000, target_rpm: 6_800 }, // 100°C: Max emergency cooling
        ],
    };

    /// Interpolates target RPM for a given temperature in millidegrees Celsius.
    pub fn evaluate_rpm(&self, temp_mdeg_c: i32) -> u32 {
        if temp_mdeg_c <= self.points[0].temp_mdeg_c {
            return self.points[0].target_rpm;
        }
        let last_idx = self.points.len() - 1;
        if temp_mdeg_c >= self.points[last_idx].temp_mdeg_c {
            return self.points[last_idx].target_rpm;
        }

        let mut i = 0;
        while i < last_idx {
            let p0 = self.points[i];
            let p1 = self.points[i + 1];
            if temp_mdeg_c >= p0.temp_mdeg_c && temp_mdeg_c <= p1.temp_mdeg_c {
                let temp_delta = (p1.temp_mdeg_c - p0.temp_mdeg_c) as i64;
                let rpm_delta = (p1.target_rpm as i64) - (p0.target_rpm as i64);
                let offset = (temp_mdeg_c - p0.temp_mdeg_c) as i64;
                let interpolated = (p0.target_rpm as i64) + (offset * rpm_delta) / temp_delta;
                return interpolated as u32;
            }
            i += 1;
        }

        self.points[last_idx].target_rpm
    }
}

/// Slew rate and hysteresis governor preventing acoustic RPM hunting and flutter.
pub struct AcousticFanGovernor {
    curve: AcousticFanCurve,
    current_commanded_rpm: AtomicU32,
    target_rpm: AtomicU32,
    effective_temp_mdeg: AtomicI32,
    hysteresis_deadband_mdeg: i32,
    max_ramp_up_step_rpm: u32,
    max_ramp_down_step_rpm: u32,
}

impl AcousticFanGovernor {
    pub const fn new() -> Self {
        Self {
            curve: AcousticFanCurve::DEFAULT,
            current_commanded_rpm: AtomicU32::new(0),
            target_rpm: AtomicU32::new(0),
            effective_temp_mdeg: AtomicI32::new(35_000),
            hysteresis_deadband_mdeg: 3_000, // 3.0°C deadband to avoid downshift flutter
            max_ramp_up_step_rpm: 150,       // Smooth upward transition (+150 RPM/tick)
            max_ramp_down_step_rpm: 50,      // Very gentle downward transition (-50 RPM/tick)
        }
    }

    /// Evaluates current APU temperature and applies hysteresis deadband and slew-rate smoothing.
    pub fn step(&self, measured_temp_mdeg: i32) -> u32 {
        let prev_effective = self.effective_temp_mdeg.load(Ordering::Acquire);
        let mut new_effective = prev_effective;

        // Apply hysteresis deadband:
        // Temperature must fall by more than hysteresis_deadband_mdeg below effective to downshift.
        if measured_temp_mdeg > prev_effective {
            new_effective = measured_temp_mdeg;
        } else if measured_temp_mdeg < prev_effective - self.hysteresis_deadband_mdeg {
            new_effective = measured_temp_mdeg + self.hysteresis_deadband_mdeg;
        }
        self.effective_temp_mdeg.store(new_effective, Ordering::Release);

        // Determine raw target RPM from acoustic curve
        let target = self.curve.evaluate_rpm(new_effective);
        self.target_rpm.store(target, Ordering::Release);

        // Apply asymmetric slew-rate limiter
        let current = self.current_commanded_rpm.load(Ordering::Acquire);
        let next_rpm = if target > current {
            let delta = target - current;
            if delta > self.max_ramp_up_step_rpm {
                current + self.max_ramp_up_step_rpm
            } else {
                target
            }
        } else if target < current {
            let delta = current - target;
            if delta > self.max_ramp_down_step_rpm {
                current - self.max_ramp_down_step_rpm
            } else {
                target
            }
        } else {
            current
        };

        self.current_commanded_rpm.store(next_rpm, Ordering::Release);
        next_rpm
    }

    pub fn get_commanded_rpm(&self) -> u32 {
        self.current_commanded_rpm.load(Ordering::Acquire)
    }

    pub fn get_target_rpm(&self) -> u32 {
        self.target_rpm.load(Ordering::Acquire)
    }

    pub fn get_effective_temp(&self) -> i32 {
        self.effective_temp_mdeg.load(Ordering::Acquire)
    }

    pub fn force_rpm(&self, rpm: u32) {
        self.target_rpm.store(rpm, Ordering::Release);
        self.current_commanded_rpm.store(rpm, Ordering::Release);
    }
}

pub static FAN_GOVERNOR: AcousticFanGovernor = AcousticFanGovernor::new();


// ============================================================================
// Feature 3: Handheld Skin-Temperature Comfort Governor & Throttling Prevention
// ============================================================================

/// Handheld skin comfort status for palm grips and rear chassis touch zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinComfortStatus {
    /// Ergonomically comfortable (< 39.5°C).
    Comfortable,
    /// Warm to touch (39.5°C - 41.5°C); fan acoustic offset applied.
    WarmGrip,
    /// Uncomfortable heat soak (> 41.5°C); active chassis cooling required.
    HotGrip,
    /// Threshold exceeded (> 44.0°C); thermal safety TDP throttling recommended.
    ExceededLimit,
}

/// Throttling risk assessment based on predictive thermal gradient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThrottlingRiskLevel {
    /// Safe thermal headroom (> 15°C margin to PROCHOT).
    Low,
    /// Moderate temperature rise; monitor gradient.
    Moderate,
    /// Rapid thermal escalation; anticipatory fan boost active.
    High,
    /// APU silicon within 3°C of PROCHOT 100°C; throttling imminent.
    Imminent,
}

/// Handheld skin comfort governor and thermal dissipation estimator.
pub struct SkinComfortGovernor {
    comfort_threshold_mdeg: i32,
    warning_threshold_mdeg: i32,
    limit_threshold_mdeg: i32,
    estimated_grip_mdeg: AtomicI32,
    status: AtomicU32, // SkinComfortStatus as u32
    recommended_tdp_clamp_watts: AtomicU32,
}

impl SkinComfortGovernor {
    pub const fn new() -> Self {
        Self {
            comfort_threshold_mdeg: 39_500, // 39.5°C
            warning_threshold_mdeg: 41_500, // 41.5°C
            limit_threshold_mdeg: 44_000,   // 44.0°C max touch limit
            estimated_grip_mdeg: AtomicI32::new(32_000),
            status: AtomicU32::new(0),
            recommended_tdp_clamp_watts: AtomicU32::new(15), // Standard 15W TDP
        }
    }

    /// Estimates palm grip temperature from multi-zone thermal flux:
    /// Grip ≈ Ambient + 0.18*(APU - Ambient) + 0.35*(Battery - Ambient)
    pub fn update(&self, apu_mdeg: i32, ambient_mdeg: i32, battery_mdeg: i32) -> SkinComfortStatus {
        let apu_flux = if apu_mdeg > ambient_mdeg { apu_mdeg - ambient_mdeg } else { 0 };
        let battery_flux = if battery_mdeg > ambient_mdeg { battery_mdeg - ambient_mdeg } else { 0 };

        let est_grip = ambient_mdeg + (apu_flux * 18) / 100 + (battery_flux * 35) / 100;
        self.estimated_grip_mdeg.store(est_grip, Ordering::Release);

        let (stat, tdp_clamp) = if est_grip >= self.limit_threshold_mdeg {
            (SkinComfortStatus::ExceededLimit, 9) // Clamp to 9W to cool handheld
        } else if est_grip >= self.warning_threshold_mdeg {
            (SkinComfortStatus::HotGrip, 12)     // Clamp to 12W
        } else if est_grip >= self.comfort_threshold_mdeg {
            (SkinComfortStatus::WarmGrip, 15)    // Maintain 15W with fan offset
        } else {
            (SkinComfortStatus::Comfortable, 15) // Full 15W unrestricted
        };

        self.status.store(stat as u32, Ordering::Release);
        self.recommended_tdp_clamp_watts.store(tdp_clamp, Ordering::Release);
        stat
    }

    pub fn get_estimated_grip_temp(&self) -> i32 {
        self.estimated_grip_mdeg.load(Ordering::Acquire)
    }

    pub fn get_status(&self) -> SkinComfortStatus {
        match self.status.load(Ordering::Acquire) {
            0 => SkinComfortStatus::Comfortable,
            1 => SkinComfortStatus::WarmGrip,
            2 => SkinComfortStatus::HotGrip,
            _ => SkinComfortStatus::ExceededLimit,
        }
    }

    pub fn get_recommended_tdp(&self) -> u32 {
        self.recommended_tdp_clamp_watts.load(Ordering::Acquire)
    }

    /// Additional fan RPM boost required to mitigate skin warmth.
    pub fn get_acoustic_rpm_boost(&self) -> u32 {
        match self.get_status() {
            SkinComfortStatus::Comfortable => 0,
            SkinComfortStatus::WarmGrip => 300,
            SkinComfortStatus::HotGrip => 750,
            SkinComfortStatus::ExceededLimit => 1400,
        }
    }
}

/// Throttling prevention model tracking thermal rise gradients (dT/dt).
pub struct ThrottlingPreventionModel {
    last_apu_temp_mdeg: AtomicI32,
    last_timestamp_ms: AtomicU64,
    gradient_mdeg_per_sec: AtomicI32,
    risk_level: AtomicU32,
}

impl ThrottlingPreventionModel {
    pub const fn new() -> Self {
        Self {
            last_apu_temp_mdeg: AtomicI32::new(35_000),
            last_timestamp_ms: AtomicU64::new(0),
            gradient_mdeg_per_sec: AtomicI32::new(0),
            risk_level: AtomicU32::new(0),
        }
    }

    /// Evaluates current APU temperature against historical gradient to preempt PROCHOT.
    pub fn evaluate(&self, current_apu_mdeg: i32, timestamp_ms: u64) -> ThrottlingRiskLevel {
        let prev_temp = self.last_apu_temp_mdeg.swap(current_apu_mdeg, Ordering::AcqRel);
        let prev_time = self.last_timestamp_ms.swap(timestamp_ms, Ordering::AcqRel);

        let gradient = if timestamp_ms > prev_time && prev_time > 0 {
            let dt_ms = (timestamp_ms - prev_time) as i64;
            let d_temp = (current_apu_mdeg - prev_temp) as i64;
            ((d_temp * 1000) / dt_ms) as i32
        } else {
            0
        };
        self.gradient_mdeg_per_sec.store(gradient, Ordering::Release);

        // PROCHOT limit is 100°C (100_000 m°C)
        let risk = if current_apu_mdeg >= 97_000 {
            ThrottlingRiskLevel::Imminent
        } else if current_apu_mdeg >= 90_000 || (current_apu_mdeg >= 84_000 && gradient > 3_000) {
            ThrottlingRiskLevel::High
        } else if current_apu_mdeg >= 80_000 || gradient > 4_000 {
            ThrottlingRiskLevel::Moderate
        } else {
            ThrottlingRiskLevel::Low
        };

        self.risk_level.store(risk as u32, Ordering::Release);
        risk
    }

    pub fn get_gradient(&self) -> i32 {
        self.gradient_mdeg_per_sec.load(Ordering::Acquire)
    }

    pub fn get_risk_level(&self) -> ThrottlingRiskLevel {
        match self.risk_level.load(Ordering::Acquire) {
            0 => ThrottlingRiskLevel::Low,
            1 => ThrottlingRiskLevel::Moderate,
            2 => ThrottlingRiskLevel::High,
            _ => ThrottlingRiskLevel::Imminent,
        }
    }

    /// Proactive RPM anticipation boost based on gradient.
    pub fn get_anticipatory_fan_boost(&self) -> u32 {
        match self.get_risk_level() {
            ThrottlingRiskLevel::Low => 0,
            ThrottlingRiskLevel::Moderate => 350,
            ThrottlingRiskLevel::High => 900,
            ThrottlingRiskLevel::Imminent => 2000,
        }
    }
}

pub static SKIN_COMFORT: SkinComfortGovernor = SkinComfortGovernor::new();
pub static THROTTLING_PREVENTION: ThrottlingPreventionModel = ThrottlingPreventionModel::new();
