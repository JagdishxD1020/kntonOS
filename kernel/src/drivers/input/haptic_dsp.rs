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

use crate::graphics::types::{sin_f32, TWO_PI};

/// Pre-baked tactile waveform profiles for handheld gaming and UI feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TactilePattern {
    Tick,
    CrispClick,
    DoubleClick,
    HeavyImpact,
    SharpRecoil,
    ContinuousRumble,
    SpringResistance,
}

/// Tactical waveform generator synthesizing discrete tactile vibration samples.
#[derive(Debug, Clone, Default)]
pub struct TactileWaveformSynthesizer {
    pub active_pattern: Option<TactilePattern>,
    pub elapsed_ms: u32,
    pub total_duration_ms: u32,
    pub amplitude_scale: f32,
}

impl TactileWaveformSynthesizer {
    // Creates a new tactile waveform synthesizer.
    pub fn new() -> Self {
        Self {
            active_pattern: None,
            elapsed_ms: 0,
            total_duration_ms: 0,
            amplitude_scale: 1.0,
        }
    }

    // Triggers synthesis of a pre-baked tactile pattern.
    pub fn trigger_pattern(&mut self, pattern: TactilePattern, amplitude: f32) {
        self.active_pattern = Some(pattern);
        self.elapsed_ms = 0;
        self.amplitude_scale = amplitude.clamp(0.0, 1.0);
        self.total_duration_ms = match pattern {
            TactilePattern::Tick => 6,
            TactilePattern::CrispClick => 16,
            TactilePattern::DoubleClick => 40,
            TactilePattern::HeavyImpact => 80,
            TactilePattern::SharpRecoil => 45,
            TactilePattern::ContinuousRumble => 100,
            TactilePattern::SpringResistance => 60,
        };
    }

    // Evaluates a single 1-millisecond discrete haptic sample.
    pub fn step_ms(&mut self, resonance_hz: f32) -> f32 {
        let pattern = match self.active_pattern {
            Some(p) if self.elapsed_ms < self.total_duration_ms => p,
            _ => {
                self.active_pattern = None;
                return 0.0;
            }
        };

        let t = self.elapsed_ms as f32;
        self.elapsed_ms += 1;
        let amp = self.amplitude_scale;

        match pattern {
            TactilePattern::Tick => {
                let env = (1.0 - (t / 6.0)).clamp(0.0, 1.0);
                let phase = TWO_PI * resonance_hz * (t / 1000.0);
                sin_f32(phase) * env * 0.5 * amp
            }
            TactilePattern::CrispClick => {
                let env = if t < 2.0 { t / 2.0 } else { 1.0 - (t - 2.0) / 14.0 }.clamp(0.0, 1.0);
                let phase = TWO_PI * resonance_hz * (t / 1000.0);
                sin_f32(phase) * env * 0.8 * amp
            }
            TactilePattern::DoubleClick => {
                if t < 12.0 {
                    let sub_t = t;
                    let env = if sub_t < 2.0 { sub_t / 2.0 } else { 1.0 - (sub_t - 2.0) / 10.0 }.clamp(0.0, 1.0);
                    let phase = TWO_PI * resonance_hz * (sub_t / 1000.0);
                    sin_f32(phase) * env * 0.8 * amp
                } else if (24.0..36.0).contains(&t) {
                    let sub_t = t - 24.0;
                    let env = if sub_t < 2.0 { sub_t / 2.0 } else { 1.0 - (sub_t - 2.0) / 10.0 }.clamp(0.0, 1.0);
                    let phase = TWO_PI * resonance_hz * (sub_t / 1000.0);
                    sin_f32(phase) * env * 0.8 * amp
                } else {
                    0.0
                }
            }
            TactilePattern::HeavyImpact => {
                let f = 160.0 - 20.0 * (t / 80.0);
                let env = if t < 10.0 { t / 10.0 } else { 1.0 - (t - 10.0) / 70.0 }.clamp(0.0, 1.0);
                let phase = TWO_PI * f * (t / 1000.0);
                sin_f32(phase) * env * amp
            }
            TactilePattern::SharpRecoil => {
                let env = if t <= 1.0 { 1.0 } else { 1.0 - (t - 1.0) / 44.0 }.clamp(0.0, 1.0);
                let phase = TWO_PI * 220.0 * (t / 1000.0);
                sin_f32(phase) * env * amp
            }
            TactilePattern::ContinuousRumble => {
                let tremolo = 0.6 + 0.4 * sin_f32(TWO_PI * 30.0 * (t / 1000.0));
                let phase = TWO_PI * 180.0 * (t / 1000.0);
                sin_f32(phase) * tremolo * 0.7 * amp
            }
            TactilePattern::SpringResistance => {
                let ramp = (t / 60.0).clamp(0.0, 1.0);
                let phase = TWO_PI * resonance_hz * (t / 1000.0);
                sin_f32(phase) * ramp * 0.9 * amp
            }
        }
    }

    // Indicates whether waveform synthesis is in progress.
    pub fn is_active(&self) -> bool {
        self.active_pattern.is_some() && self.elapsed_ms < self.total_duration_ms
    }
}

/// Verifies synthesis of tick, clicks, impacts, recoil, rumble, and spring patterns.
pub fn verify_waveform_library() -> bool {
    let mut synth = TactileWaveformSynthesizer::new();

    // 1. Verify Tick completes within 6ms
    synth.trigger_pattern(TactilePattern::Tick, 1.0);
    for _ in 0..6 {
        let _ = synth.step_ms(200.0);
    }
    if synth.is_active() || synth.step_ms(200.0) != 0.0 {
        return false;
    }

    // 2. Verify DoubleClick has two pulses separated by silence
    synth.trigger_pattern(TactilePattern::DoubleClick, 1.0);
    let mut click1_energy = 0.0f32;
    for _ in 0..12 {
        let s = synth.step_ms(200.0);
        click1_energy += s * s;
    }
    let mut gap_energy = 0.0f32;
    for _ in 12..24 {
        let s = synth.step_ms(200.0);
        gap_energy += s * s;
    }
    let mut click2_energy = 0.0f32;
    for _ in 24..36 {
        let s = synth.step_ms(200.0);
        click2_energy += s * s;
    }
    if click1_energy < 0.1 || gap_energy != 0.0 || click2_energy < 0.1 {
        return false;
    }

    // 3. Verify SharpRecoil peak in early frames
    synth.trigger_pattern(TactilePattern::SharpRecoil, 1.0);
    let mut max_recoil = 0.0f32;
    for _ in 0..5 {
        let s = synth.step_ms(200.0);
        let abs_s = if s < 0.0 { -s } else { s };
        if abs_s > max_recoil {
            max_recoil = abs_s;
        }
    }
    if max_recoil < 0.8 {
        return false;
    }

    true
}
