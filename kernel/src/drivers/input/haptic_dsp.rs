//! Handheld PC Haptic Waveform Library & Audio-to-Haptics Transducer Service.

#![allow(dead_code)]

// Actuator hardware model constants.
pub const LRA_MIN_RESONANCE_HZ: f32 = 170.0;
pub const LRA_MAX_RESONANCE_HZ: f32 = 230.0;
pub const LRA_NOMINAL_RESONANCE_HZ: f32 = 200.0;
pub const MAX_DISPLACEMENT_MM: f32 = 1.2;
pub const THERMAL_MAX_TEMP_C: f32 = 85.0;
pub const AMBIENT_TEMP_C: f32 = 25.0;

/// Actuator transducer hardware category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActuatorType {
    LinearResonantActuator,
    VoiceCoilActuator,
}

/// Dynamic physical actuator model with resonant frequency tracking and thermal protection.
#[derive(Debug, Clone)]
pub struct ActuatorDriver {
    pub actuator_type: ActuatorType,
    pub resonance_hz: f32,
    pub target_freq_hz: f32,
    pub displacement_mm: f32,
    pub temperature_c: f32,
    pub resistance_ohms: f32,
    pub thermal_resistance: f32,
    pub thermal_time_constant: f32,
    pub is_throttled: bool,
}

impl ActuatorDriver {
    // Creates a new actuator driver instance.
    pub fn new(actuator_type: ActuatorType, nominal_hz: f32) -> Self {
        let res = nominal_hz.clamp(LRA_MIN_RESONANCE_HZ, LRA_MAX_RESONANCE_HZ);
        Self {
            actuator_type,
            resonance_hz: res,
            target_freq_hz: res,
            displacement_mm: 0.0,
            temperature_c: AMBIENT_TEMP_C,
            resistance_ohms: 8.0,
            thermal_resistance: 12.0,
            thermal_time_constant: 2.0,
            is_throttled: false,
        }
    }

    // Updates auto-resonance tracking frequency clamped to hardware limits.
    pub fn update_resonance(&mut self, tracked_freq_hz: f32) {
        self.resonance_hz = tracked_freq_hz.clamp(LRA_MIN_RESONANCE_HZ, LRA_MAX_RESONANCE_HZ);
    }

    // Evaluates actuator physics, limits coil displacement, and applies thermal throttling.
    pub fn step_drive(&mut self, voltage: f32, freq_hz: f32, dt: f32) -> f32 {
        self.target_freq_hz = freq_hz;
        let p_watts = (voltage * voltage) / self.resistance_ohms;
        let delta_t = ((p_watts * self.thermal_resistance) - (self.temperature_c - AMBIENT_TEMP_C)) * (dt / self.thermal_time_constant);
        self.temperature_c = (self.temperature_c + delta_t).max(AMBIENT_TEMP_C);

        self.is_throttled = self.temperature_c >= THERMAL_MAX_TEMP_C;
        let thermal_scale = if self.is_throttled { 0.4 } else { 1.0 };
        let mut drive_v = voltage * thermal_scale;

        // Peak displacement calculation near resonant frequency
        let freq_ratio = (freq_hz / self.resonance_hz.max(1.0)).clamp(0.5, 2.0);
        let q_factor = if self.actuator_type == ActuatorType::LinearResonantActuator { 4.0 } else { 1.5 };
        let resonance_gain = q_factor / (1.0 + q_factor * (freq_ratio - 1.0).abs());
        self.displacement_mm = (drive_v * 0.15 * resonance_gain).min(MAX_DISPLACEMENT_MM * 1.5);

        if self.displacement_mm > MAX_DISPLACEMENT_MM {
            let limit_scale = MAX_DISPLACEMENT_MM / self.displacement_mm;
            drive_v *= limit_scale;
            self.displacement_mm = MAX_DISPLACEMENT_MM;
        }

        drive_v
    }
}

/// Dual-channel handheld actuator controller managing left and right motor transducers.
#[derive(Debug, Clone)]
pub struct DualChannelActuatorController {
    pub left: ActuatorDriver,
    pub right: ActuatorDriver,
}

impl DualChannelActuatorController {
    // Creates a new dual-channel handheld haptic actuator controller.
    pub fn new() -> Self {
        Self {
            left: ActuatorDriver::new(ActuatorType::LinearResonantActuator, LRA_NOMINAL_RESONANCE_HZ),
            right: ActuatorDriver::new(ActuatorType::LinearResonantActuator, LRA_NOMINAL_RESONANCE_HZ),
        }
    }
}

/// Verifies actuator resonance frequency clamping, displacement limits, and thermal throttling.
pub fn verify_actuator_model() -> bool {
    let mut driver = ActuatorDriver::new(ActuatorType::LinearResonantActuator, 200.0);

    // 1. Verify resonance frequency clamping
    driver.update_resonance(120.0);
    if driver.resonance_hz != LRA_MIN_RESONANCE_HZ {
        return false;
    }
    driver.update_resonance(280.0);
    if driver.resonance_hz != LRA_MAX_RESONANCE_HZ {
        return false;
    }

    // 2. Verify displacement limiting prevents mechanical overshoot
    let limited_v = driver.step_drive(12.0, driver.resonance_hz, 0.01);
    if driver.displacement_mm > MAX_DISPLACEMENT_MM || limited_v >= 12.0 {
        return false;
    }

    // 3. Verify thermal protection under continuous heavy load
    for _ in 0..1000 {
        driver.step_drive(10.0, driver.resonance_hz, 0.01);
    }
    if !driver.is_throttled || driver.temperature_c < THERMAL_MAX_TEMP_C {
        return false;
    }

    true
}
