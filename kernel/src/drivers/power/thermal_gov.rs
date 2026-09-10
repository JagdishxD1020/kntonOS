//! Handheld PC Multi-Zone Thermal Governor and Dynamic Fan Controller.

/// Thermal sensor zones monitored across handheld gaming hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandheldThermalZone {
    CpuPackage,
    IgpuHotspot,
    BatteryThermistor,
    AmbientChassis,
}

/// Instantaneous temperature reading in millidegrees Celsius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThermalReading {
    pub zone: HandheldThermalZone,
    pub raw_millicelsius: i32,
    pub filtered_millicelsius: i32,
    pub timestamp_ms: u64,
}

impl ThermalReading {
    pub const fn from_celsius(zone: HandheldThermalZone, celsius: f32, timestamp_ms: u64) -> Self {
        let mc = (celsius * 1000.0) as i32;
        Self {
            zone,
            raw_millicelsius: mc,
            filtered_millicelsius: mc,
            timestamp_ms,
        }
    }

    #[inline]
    pub fn celsius(&self) -> f32 {
        self.filtered_millicelsius as f32 / 1000.0
    }
}

/// Moving average filter state for a single thermal sensor zone.
#[derive(Clone, Copy, Debug)]
pub struct ZoneFilter {
    samples: [i32; 4],
    index: usize,
    count: usize,
    last_filtered: i32,
}

impl ZoneFilter {
    pub const fn new() -> Self {
        Self {
            samples: [0; 4],
            index: 0,
            count: 0,
            last_filtered: 0,
        }
    }

    pub fn push(&mut self, sample_mc: i32) -> i32 {
        self.samples[self.index] = sample_mc;
        self.index = (self.index + 1) % 4;
        if self.count < 4 {
            self.count += 1;
        }

        let mut sum: i64 = 0;
        for i in 0..self.count {
            sum += self.samples[i] as i64;
        }
        let filtered = (sum / (self.count as i64)) as i32;
        self.last_filtered = filtered;
        filtered
    }
}

/// Multi-zone thermal sensor monitor managing telemetry across all zones.
pub struct MultiZoneThermalMonitor {
    filters: [ZoneFilter; 4],
    readings: [ThermalReading; 4],
}

impl MultiZoneThermalMonitor {
    pub const fn new() -> Self {
        Self {
            filters: [ZoneFilter::new(); 4],
            readings: [
                ThermalReading::from_celsius(HandheldThermalZone::CpuPackage, 35.0, 0),
                ThermalReading::from_celsius(HandheldThermalZone::IgpuHotspot, 35.0, 0),
                ThermalReading::from_celsius(HandheldThermalZone::BatteryThermistor, 25.0, 0),
                ThermalReading::from_celsius(HandheldThermalZone::AmbientChassis, 25.0, 0),
            ],
        }
    }

    #[inline]
    fn zone_to_index(zone: HandheldThermalZone) -> usize {
        match zone {
            HandheldThermalZone::CpuPackage => 0,
            HandheldThermalZone::IgpuHotspot => 1,
            HandheldThermalZone::BatteryThermistor => 2,
            HandheldThermalZone::AmbientChassis => 3,
        }
    }

    pub fn update_reading(&mut self, zone: HandheldThermalZone, raw_mc: i32, timestamp_ms: u64) -> f32 {
        let idx = Self::zone_to_index(zone);
        let filtered = self.filters[idx].push(raw_mc);
        self.readings[idx] = ThermalReading {
            zone,
            raw_millicelsius: raw_mc,
            filtered_millicelsius: filtered,
            timestamp_ms,
        };
        filtered as f32 / 1000.0
    }

    pub fn set_zone_immediate(&mut self, zone: HandheldThermalZone, celsius: f32) {
        let raw_mc = (celsius * 1000.0) as i32;
        let idx = Self::zone_to_index(zone);
        self.filters[idx] = ZoneFilter {
            samples: [raw_mc; 4],
            index: 0,
            count: 4,
            last_filtered: raw_mc,
        };
        self.readings[idx] = ThermalReading {
            zone,
            raw_millicelsius: raw_mc,
            filtered_millicelsius: raw_mc,
            timestamp_ms: 0,
        };
    }

    pub fn get_reading(&self, zone: HandheldThermalZone) -> ThermalReading {
        let idx = Self::zone_to_index(zone);
        self.readings[idx]
    }

    pub fn max_silicon_temperature(&self) -> f32 {
        let cpu_t = self.get_reading(HandheldThermalZone::CpuPackage).celsius();
        let gpu_t = self.get_reading(HandheldThermalZone::IgpuHotspot).celsius();
        if cpu_t > gpu_t { cpu_t } else { gpu_t }
    }
}

/// Verifies multi-zone thermal sensor reading and moving average smoothing pipeline.
pub fn verify_multizone_thermal_sensors() -> bool {
    let mut monitor = MultiZoneThermalMonitor::new();

    if monitor.get_reading(HandheldThermalZone::CpuPackage).celsius() < 30.0 {
        return false;
    }

    monitor.update_reading(HandheldThermalZone::CpuPackage, 60_000, 100);
    monitor.update_reading(HandheldThermalZone::CpuPackage, 62_000, 200);
    monitor.update_reading(HandheldThermalZone::CpuPackage, 64_000, 300);
    let filtered_c = monitor.update_reading(HandheldThermalZone::CpuPackage, 66_000, 400);

    if (filtered_c - 63.0).abs() > 0.05 {
        return false;
    }

    monitor.update_reading(HandheldThermalZone::IgpuHotspot, 72_000, 400);
    if (monitor.max_silicon_temperature() - 72.0).abs() > 0.1 {
        return false;
    }

    true
}

// Piecewise Linear Dynamic Fan Curve Governor

/// Control point along a piecewise linear fan curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FanCurvePoint {
    pub temp_c: f32,
    pub pwm_pct: u8,
}

impl FanCurvePoint {
    pub const fn new(temp_c: f32, pwm_pct: u8) -> Self {
        Self { temp_c, pwm_pct }
    }
}

/// Piecewise linear fan curve with up to 6 control points.
#[derive(Clone, Copy, Debug)]
pub struct FanCurve {
    pub points: [FanCurvePoint; 6],
    pub count: usize,
}

impl FanCurve {
    pub const fn default_balanced() -> Self {
        Self {
            points: [
                FanCurvePoint::new(35.0, 0),
                FanCurvePoint::new(50.0, 20),
                FanCurvePoint::new(65.0, 45),
                FanCurvePoint::new(75.0, 65),
                FanCurvePoint::new(85.0, 85),
                FanCurvePoint::new(95.0, 100),
            ],
            count: 6,
        }
    }

    /// Evaluates PWM duty percentage using piecewise linear interpolation.
    pub fn evaluate(&self, temp_c: f32) -> u8 {
        if self.count == 0 {
            return 0;
        }
        if temp_c <= self.points[0].temp_c {
            return self.points[0].pwm_pct;
        }
        if temp_c >= self.points[self.count - 1].temp_c {
            return self.points[self.count - 1].pwm_pct;
        }

        for i in 0..(self.count - 1) {
            let p0 = self.points[i];
            let p1 = self.points[i + 1];
            if temp_c >= p0.temp_c && temp_c <= p1.temp_c {
                let range = p1.temp_c - p0.temp_c;
                if range <= 0.001 {
                    return p1.pwm_pct;
                }
                let t = (temp_c - p0.temp_c) / range;
                let pwm_f = (p0.pwm_pct as f32) + t * ((p1.pwm_pct as f32) - (p0.pwm_pct as f32));
                return (pwm_f + 0.5) as u8;
            }
        }

        self.points[self.count - 1].pwm_pct
    }
}

/// Dynamic fan governor applying piecewise linear curves with acoustic hysteresis and slew limiting.
pub struct DynamicFanGovernor {
    pub curve: FanCurve,
    pub current_pwm: u8,
    pub last_temp_c: f32,
    pub hysteresis_c: f32,
    pub max_rpm: u16,
}

impl DynamicFanGovernor {
    pub const fn new(curve: FanCurve) -> Self {
        Self {
            curve,
            current_pwm: 0,
            last_temp_c: 25.0,
            hysteresis_c: 2.0,
            max_rpm: 6500,
        }
    }

    /// Evaluates current temperature and updates PWM with hysteresis.
    pub fn step(&mut self, temp_c: f32) -> u8 {
        let target_pwm = self.curve.evaluate(temp_c);

        if target_pwm > self.current_pwm {
            // Instant ramp up when silicon temperature rises
            self.current_pwm = target_pwm;
            self.last_temp_c = temp_c;
        } else if target_pwm < self.current_pwm {
            // Apply acoustic hysteresis on cooldown to prevent fan hunting
            if temp_c < (self.last_temp_c - self.hysteresis_c) {
                self.current_pwm = target_pwm;
                self.last_temp_c = temp_c;
            }
        }

        self.current_pwm
    }

    /// Calculates estimated fan RPM corresponding to current PWM duty.
    pub fn current_rpm(&self) -> u16 {
        if self.current_pwm == 0 {
            0
        } else {
            // Handheld blower fans spin between 1200 RPM and max_rpm
            let min_spin_rpm: u16 = 1200;
            let span = self.max_rpm.saturating_sub(min_spin_rpm);
            min_spin_rpm + ((span as u32 * self.current_pwm as u32) / 100) as u16
        }
    }
}

/// Verifies piecewise linear interpolation and acoustic hysteresis on dynamic fan curves.
pub fn verify_dynamic_fan_curve() -> bool {
    let curve = FanCurve::default_balanced();

    // 1. Below minimum threshold -> 0% PWM
    if curve.evaluate(30.0) != 0 {
        return false;
    }

    // 2. Exact point matches
    if curve.evaluate(50.0) != 20 || curve.evaluate(95.0) != 100 {
        return false;
    }

    // 3. Piecewise interpolation between 50C (20%) and 65C (45%): midpoint 57.5C = 32.5% -> 33%
    let mid_pwm = curve.evaluate(57.5);
    if mid_pwm != 32 && mid_pwm != 33 {
        return false;
    }

    // 4. Governor step with acoustic hysteresis
    let mut gov = DynamicFanGovernor::new(curve);
    let ramp_up = gov.step(75.0);
    if ramp_up != 65 {
        return false;
    }

    // Small drop by 1C (74C): within 2C hysteresis -> fan holds 65% PWM
    let hold = gov.step(74.0);
    if hold != 65 {
        return false;
    }

    // Drop by 3C (71C): exceeds 2C hysteresis -> fan cools down smoothly
    let cooldown = gov.step(71.0);
    if cooldown >= 65 {
        return false;
    }

    true
}

// Gaming Thermal Profile Governor & TDP Envelopes

/// Handheld gaming thermal profile modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamingThermalProfile {
    Quiet,
    Balanced,
    Performance,
    Custom,
}

/// Power and thermal envelope for a gaming profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileTdpEnvelope {
    pub sustained_tdp_w: u16,
    pub boost_tdp_w: u16,
    pub boost_duration_s: u8,
    pub max_fan_pwm_pct: u8,
}

impl ProfileTdpEnvelope {
    pub const fn quiet() -> Self {
        Self {
            sustained_tdp_w: 12,
            boost_tdp_w: 15,
            boost_duration_s: 5,
            max_fan_pwm_pct: 50,
        }
    }

    pub const fn balanced() -> Self {
        Self {
            sustained_tdp_w: 18,
            boost_tdp_w: 22,
            boost_duration_s: 10,
            max_fan_pwm_pct: 80,
        }
    }

    pub const fn performance() -> Self {
        Self {
            sustained_tdp_w: 28,
            boost_tdp_w: 32,
            boost_duration_s: 15,
            max_fan_pwm_pct: 100,
        }
    }
}

impl FanCurve {
    pub const fn quiet() -> Self {
        Self {
            points: [
                FanCurvePoint::new(40.0, 0),
                FanCurvePoint::new(55.0, 15),
                FanCurvePoint::new(65.0, 25),
                FanCurvePoint::new(75.0, 35),
                FanCurvePoint::new(85.0, 45),
                FanCurvePoint::new(95.0, 50),
            ],
            count: 6,
        }
    }

    pub const fn performance() -> Self {
        Self {
            points: [
                FanCurvePoint::new(35.0, 20),
                FanCurvePoint::new(50.0, 40),
                FanCurvePoint::new(65.0, 65),
                FanCurvePoint::new(75.0, 80),
                FanCurvePoint::new(85.0, 95),
                FanCurvePoint::new(90.0, 100),
            ],
            count: 6,
        }
    }
}

/// Governor coordinating active gaming profiles, TDP ceilings, and fan response.
pub struct ThermalProfileGovernor {
    pub active_profile: GamingThermalProfile,
    pub envelope: ProfileTdpEnvelope,
    pub fan_governor: DynamicFanGovernor,
}

impl ThermalProfileGovernor {
    pub fn new(profile: GamingThermalProfile) -> Self {
        let (env, curve) = match profile {
            GamingThermalProfile::Quiet => (ProfileTdpEnvelope::quiet(), FanCurve::quiet()),
            GamingThermalProfile::Balanced => (ProfileTdpEnvelope::balanced(), FanCurve::default_balanced()),
            GamingThermalProfile::Performance => (ProfileTdpEnvelope::performance(), FanCurve::performance()),
            GamingThermalProfile::Custom => (ProfileTdpEnvelope::balanced(), FanCurve::default_balanced()),
        };
        Self {
            active_profile: profile,
            envelope: env,
            fan_governor: DynamicFanGovernor::new(curve),
        }
    }

    pub fn set_profile(&mut self, profile: GamingThermalProfile) {
        self.active_profile = profile;
        match profile {
            GamingThermalProfile::Quiet => {
                self.envelope = ProfileTdpEnvelope::quiet();
                self.fan_governor.curve = FanCurve::quiet();
            }
            GamingThermalProfile::Balanced => {
                self.envelope = ProfileTdpEnvelope::balanced();
                self.fan_governor.curve = FanCurve::default_balanced();
            }
            GamingThermalProfile::Performance => {
                self.envelope = ProfileTdpEnvelope::performance();
                self.fan_governor.curve = FanCurve::performance();
            }
            GamingThermalProfile::Custom => {}
        }
    }

    /// Evaluates current temperature, updating fan PWM clamped by profile envelope.
    pub fn step_temperature(&mut self, temp_c: f32) -> (u8, u16) {
        let raw_pwm = self.fan_governor.step(temp_c);
        let clamped_pwm = raw_pwm.min(self.envelope.max_fan_pwm_pct);
        (clamped_pwm, self.envelope.sustained_tdp_w)
    }
}

/// Verifies gaming thermal profiles (Quiet acoustic ceiling, Balanced 15W-20W, and Performance 28W-30W).
pub fn verify_thermal_gaming_profiles() -> bool {
    let mut gov = ThermalProfileGovernor::new(GamingThermalProfile::Balanced);

    // 1. Balanced mode default TDP and fan behavior
    let (pwm_bal, tdp_bal) = gov.step_temperature(65.0);
    if tdp_bal != 18 || pwm_bal != 45 {
        return false;
    }

    // 2. Switch to Quiet mode: fan capped at 50%, TDP target 12W
    gov.set_profile(GamingThermalProfile::Quiet);
    let (pwm_quiet, tdp_quiet) = gov.step_temperature(90.0);
    if tdp_quiet != 12 || pwm_quiet > 50 {
        return false;
    }

    // 3. Switch to Performance mode: TDP 28W, fan up to 100%
    gov.set_profile(GamingThermalProfile::Performance);
    let (pwm_perf, tdp_perf) = gov.step_temperature(90.0);
    if tdp_perf != 28 || pwm_perf != 100 {
        return false;
    }

    true
}

// Thermal Tripwire and Proactive Throttling Governor

/// Safety thresholds for handheld silicon, battery, and chassis skin comfort.
pub const THROTTLE_START_TEMP_C: f32 = 85.0;
pub const CRITICAL_SHUTDOWN_TEMP_C: f32 = 100.0;
pub const BATTERY_HOT_TEMP_C: f32 = 45.0;
pub const BATTERY_CRITICAL_TEMP_C: f32 = 60.0;
pub const CHASSIS_COMFORT_TEMP_C: f32 = 48.0;

/// Real-time status of thermal throttling and safety tripwires.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThermalThrottleStatus {
    pub throttling_active: bool,
    pub prochot_asserted: bool,
    pub emergency_shutdown: bool,
    pub effective_tdp_w: u16,
    pub battery_charge_limited: bool,
}

impl ThermalThrottleStatus {
    pub const fn normal(tdp_w: u16) -> Self {
        Self {
            throttling_active: false,
            prochot_asserted: false,
            emergency_shutdown: false,
            effective_tdp_w: tdp_w,
            battery_charge_limited: false,
        }
    }
}

/// Proactive thermal governor evaluating tripwires across multi-zone telemetry.
pub struct ThermalTripwireGovernor {
    pub monitor: MultiZoneThermalMonitor,
    pub profile_gov: ThermalProfileGovernor,
    pub status: ThermalThrottleStatus,
}

impl ThermalTripwireGovernor {
    pub fn new(profile: GamingThermalProfile) -> Self {
        let p_gov = ThermalProfileGovernor::new(profile);
        let base_tdp = p_gov.envelope.sustained_tdp_w;
        Self {
            monitor: MultiZoneThermalMonitor::new(),
            profile_gov: p_gov,
            status: ThermalThrottleStatus::normal(base_tdp),
        }
    }

    /// Evaluates multi-zone temperatures, updating fan curves and safety tripwires.
    pub fn step_thermal_safety(&mut self) -> ThermalThrottleStatus {
        let max_silicon = self.monitor.max_silicon_temperature();
        let battery_t = self.monitor.get_reading(HandheldThermalZone::BatteryThermistor).celsius();
        let (_pwm, base_tdp) = self.profile_gov.step_temperature(max_silicon);

        let mut st = ThermalThrottleStatus::normal(base_tdp);

        // 1. Critical silicon emergency shutdown tripwire (PROCHOT)
        if max_silicon >= CRITICAL_SHUTDOWN_TEMP_C {
            st.emergency_shutdown = true;
            st.prochot_asserted = true;
            st.throttling_active = true;
            st.effective_tdp_w = 5; // Absolute hardware minimum floor
        } else if max_silicon >= THROTTLE_START_TEMP_C {
            // 2. Proactive soft throttling: reduce 2W per degree Celsius above 85C
            let over_c = max_silicon - THROTTLE_START_TEMP_C;
            let reduction = (over_c * 2.0) as u16;
            st.throttling_active = true;
            st.effective_tdp_w = base_tdp.saturating_sub(reduction).max(7);
        }

        // 3. Battery thermal protection tripwires
        if battery_t >= BATTERY_CRITICAL_TEMP_C {
            st.emergency_shutdown = true;
            st.battery_charge_limited = true;
        } else if battery_t >= BATTERY_HOT_TEMP_C {
            st.battery_charge_limited = true;
        }

        self.status = st;
        st
    }
}

/// Verifies proactive thermal throttling, dynamic TDP scaling, and critical tripwire shutdown.
pub fn verify_thermal_throttling_and_tripwire() -> bool {
    let mut gov = ThermalTripwireGovernor::new(GamingThermalProfile::Balanced);

    // 1. Normal operating temperature (65C CPU & GPU)
    gov.monitor.set_zone_immediate(HandheldThermalZone::CpuPackage, 65.0);
    gov.monitor.set_zone_immediate(HandheldThermalZone::IgpuHotspot, 65.0);
    let st_normal = gov.step_thermal_safety();
    if st_normal.throttling_active || st_normal.emergency_shutdown || st_normal.effective_tdp_w != 18 {
        return false;
    }

    // 2. Soft throttling at 88C: 3C over 85C threshold -> 6W reduction from 18W = 12W
    gov.monitor.set_zone_immediate(HandheldThermalZone::CpuPackage, 88.0);
    gov.monitor.set_zone_immediate(HandheldThermalZone::IgpuHotspot, 88.0);
    let st_throttle = gov.step_thermal_safety();
    if !st_throttle.throttling_active || st_throttle.emergency_shutdown || st_throttle.effective_tdp_w != 12 {
        return false;
    }

    // 3. Critical tripwire at 101C: emergency shutdown triggered, PROCHOT asserted
    gov.monitor.set_zone_immediate(HandheldThermalZone::CpuPackage, 101.0);
    let st_crit = gov.step_thermal_safety();
    if !st_crit.emergency_shutdown || !st_crit.prochot_asserted || st_crit.effective_tdp_w > 5 {
        return false;
    }

    // 4. Battery thermistor safety at 50C: charge limited
    gov.monitor.set_zone_immediate(HandheldThermalZone::CpuPackage, 60.0);
    gov.monitor.set_zone_immediate(HandheldThermalZone::IgpuHotspot, 60.0);
    gov.monitor.set_zone_immediate(HandheldThermalZone::BatteryThermistor, 50.0);
    let st_bat = gov.step_thermal_safety();
    if !st_bat.battery_charge_limited || st_bat.emergency_shutdown {
        return false;
    }

    true
}


/// Comprehensive verification self-test suite for Handheld PC Fan & Thermal Governor Service.
pub fn test_thermal_governor_subsystem() -> bool {
    crate::serial_write_str("[KNT-POWER] Testing Handheld PC Fan & Thermal Governor service...\r\n");

    // 1. Verify multi-zone thermal sensor reading and moving average filtering
    if !verify_multizone_thermal_sensors() {
        crate::serial_write_str("[KNT-PANIC] Multi-zone thermal sensor verification failed!\r\n");
        return false;
    }

    // 2. Verify piecewise linear dynamic fan curve and acoustic hysteresis
    if !verify_dynamic_fan_curve() {
        crate::serial_write_str("[KNT-PANIC] Dynamic fan curve governor verification failed!\r\n");
        return false;
    }

    // 3. Verify handheld gaming thermal profiles and TDP envelopes
    if !verify_thermal_gaming_profiles() {
        crate::serial_write_str("[KNT-PANIC] Gaming thermal profiles verification failed!\r\n");
        return false;
    }

    // 4. Verify proactive thermal throttling and critical tripwire emergency shutdown
    if !verify_thermal_throttling_and_tripwire() {
        crate::serial_write_str("[KNT-PANIC] Thermal throttling and tripwire verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-THERMAL-GOV-PASS] Handheld thermal governor and dynamic fan curve engine verified\r\n");
    true
}
