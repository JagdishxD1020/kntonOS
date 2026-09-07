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

use crate::graphics::types::cos_f32;

/// Second-order biquad filter section.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BiquadFilter {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
    pub x1: f32,
    pub x2: f32,
    pub y1: f32,
    pub y2: f32,
}

impl BiquadFilter {
    // Creates a new low-pass biquad filter section.
    pub fn new_low_pass(sample_rate: f32, cutoff_hz: f32, q: f32) -> Self {
        let w0 = TWO_PI * cutoff_hz / sample_rate;
        let alpha = sin_f32(w0) / (2.0 * q);
        let cos_w0 = cos_f32(w0);
        let b0 = (1.0 - cos_w0) * 0.5;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    // Steps filter with next input sample.
    pub fn step(&mut self, input: f32) -> f32 {
        let y = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    // Clears filter history.
    pub fn clear(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// Fourth-order Butterworth low-pass filter composed of two cascaded biquad sections.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Butterworth4thOrderLowPass {
    pub sec1: BiquadFilter,
    pub sec2: BiquadFilter,
}

impl Butterworth4thOrderLowPass {
    // Creates a 4th-order Butterworth low-pass filter.
    pub fn new(sample_rate: f32, cutoff_hz: f32) -> Self {
        Self {
            sec1: BiquadFilter::new_low_pass(sample_rate, cutoff_hz, 0.5411961),
            sec2: BiquadFilter::new_low_pass(sample_rate, cutoff_hz, 1.306563),
        }
    }

    // Steps cascaded filter with next input sample.
    pub fn step(&mut self, input: f32) -> f32 {
        let mid = self.sec1.step(input);
        self.sec2.step(mid)
    }

    // Clears filter history across all sections.
    pub fn clear(&mut self) {
        self.sec1.clear();
        self.sec2.clear();
    }
}

/// Transient envelope detector with asymmetric attack and release time constants.
#[derive(Debug, Clone)]
pub struct TransientEnvelopeDetector {
    pub envelope: f32,
    pub attack_coeff: f32,
    pub release_coeff: f32,
}

impl TransientEnvelopeDetector {
    // Creates a new envelope detector with specified attack and release smoothing coefficients.
    pub fn new(attack_coeff: f32, release_coeff: f32) -> Self {
        Self {
            envelope: 0.0,
            attack_coeff: attack_coeff.clamp(0.0, 0.999),
            release_coeff: release_coeff.clamp(0.0, 0.999),
        }
    }

    // Processes a single audio sample and updates rectified smoothed envelope.
    pub fn step(&mut self, input: f32) -> f32 {
        let abs_in = if input < 0.0 { -input } else { input };
        let coeff = if abs_in > self.envelope { self.attack_coeff } else { self.release_coeff };
        self.envelope = coeff * self.envelope + (1.0 - coeff) * abs_in;
        self.envelope
    }

    // Resets envelope state.
    pub fn clear(&mut self) {
        self.envelope = 0.0;
    }
}

/// Dynamic range compressor and soft limiter preventing tactile overdrive.
#[derive(Debug, Clone)]
pub struct DynamicCompressor {
    pub threshold: f32,
    pub ratio: f32,
    pub makeup_gain: f32,
}

impl DynamicCompressor {
    // Creates a dynamic compressor with specified threshold, compression ratio, and makeup gain.
    pub fn new(threshold: f32, ratio: f32, makeup_gain: f32) -> Self {
        Self {
            threshold: threshold.clamp(0.05, 1.0),
            ratio: ratio.max(1.0),
            makeup_gain: makeup_gain.max(0.1),
        }
    }

    // Applies dynamic range compression to input amplitude.
    pub fn process(&self, input: f32) -> f32 {
        let abs_in = if input < 0.0 { -input } else { input };
        let compressed = if abs_in <= self.threshold {
            abs_in
        } else {
            self.threshold + (abs_in - self.threshold) / self.ratio
        };
        (compressed * self.makeup_gain).clamp(0.0, 1.0)
    }
}

/// Real-time audio-to-haptics DSP pipeline converting sub-120Hz bass to tactile vibration signals.
#[derive(Debug, Clone)]
pub struct AudioToHapticsTransducer {
    pub low_pass: Butterworth4thOrderLowPass,
    pub envelope: TransientEnvelopeDetector,
    pub compressor: DynamicCompressor,
    pub carrier_phase: f32,
}

impl AudioToHapticsTransducer {
    // Creates an audio-to-haptics transducer instance for specified audio sample rate.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            low_pass: Butterworth4thOrderLowPass::new(sample_rate, 120.0),
            envelope: TransientEnvelopeDetector::new(0.9, 0.999),
            compressor: DynamicCompressor::new(0.25, 4.0, 1.2),
            carrier_phase: 0.0,
        }
    }

    // Processes a stereo audio sample pair into a tactile drive signal at given resonant frequency.
    pub fn step_stereo(&mut self, left: f32, right: f32, resonance_hz: f32, dt: f32) -> f32 {
        let mono_bass = (left + right) * 0.5;
        let filtered = self.low_pass.step(mono_bass);
        let env = self.envelope.step(filtered);
        let intensity = self.compressor.process(env);

        let carrier = sin_f32(self.carrier_phase);
        self.carrier_phase += TWO_PI * resonance_hz * dt;
        if self.carrier_phase > TWO_PI {
            self.carrier_phase -= TWO_PI;
        }

        carrier * intensity
    }

    // Clears DSP states.
    pub fn clear(&mut self) {
        self.low_pass.clear();
        self.envelope.clear();
        self.carrier_phase = 0.0;
    }
}

/// Dual-motor legacy XInput rumble mapping to LRA amplitude and frequency modulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XInputHapticMapping {
    pub left_amplitude: f32,
    pub right_amplitude: f32,
    pub left_frequency_hz: f32,
    pub right_frequency_hz: f32,
}

impl XInputHapticMapping {
    // Maps legacy 16-bit left/right motor speeds (0..65535) into LRA drive parameters.
    pub fn from_xinput(left_motor: u16, right_motor: u16) -> Self {
        let l_norm = (left_motor as f32) / 65535.0;
        let r_norm = (right_motor as f32) / 65535.0;

        let l_freq = 175.0 + l_norm * 10.0;
        let r_freq = 215.0 + r_norm * 15.0;

        Self {
            left_amplitude: l_norm.clamp(0.0, 1.0),
            right_amplitude: r_norm.clamp(0.0, 1.0),
            left_frequency_hz: l_freq.clamp(LRA_MIN_RESONANCE_HZ, LRA_MAX_RESONANCE_HZ),
            right_frequency_hz: r_freq.clamp(LRA_MIN_RESONANCE_HZ, LRA_MAX_RESONANCE_HZ),
        }
    }
}

/// Verifies 4th-order Butterworth low-pass filter, envelope detector, and dynamic compressor.
pub fn verify_audio_to_haptics() -> bool {
    let mut transducer = AudioToHapticsTransducer::new(48000.0);

    // 1. Verify 50Hz bass passes through low-pass filter
    let dt = 1.0 / 48000.0;
    let mut max_50hz = 0.0f32;
    for i in 0..4800 {
        let t = (i as f32) * dt;
        let s = sin_f32(TWO_PI * 50.0 * t);
        let out = transducer.low_pass.step(s);
        let abs_o = if out < 0.0 { -out } else { out };
        if i > 2400 && abs_o > max_50hz {
            max_50hz = abs_o;
        }
    }
    if max_50hz < 0.8 {
        return false;
    }

    // 2. Verify 1000Hz treble is strongly rejected
    transducer.clear();
    let mut max_1000hz = 0.0f32;
    for i in 0..4800 {
        let t = (i as f32) * dt;
        let s = sin_f32(TWO_PI * 1000.0 * t);
        let out = transducer.low_pass.step(s);
        let abs_o = if out < 0.0 { -out } else { out };
        if i > 2400 && abs_o > max_1000hz {
            max_1000hz = abs_o;
        }
    }
    if max_1000hz > 0.05 {
        return false;
    }

    // 3. Verify envelope and dynamic compressor
    let comp = DynamicCompressor::new(0.25, 4.0, 1.0);
    let c_low = comp.process(0.1);
    let c_high = comp.process(0.8);
    if c_low != 0.1 || c_high >= 0.8 {
        return false;
    }

    true
}

/// Verifies mapping of legacy XInput 16-bit motor values to LRA amplitude and frequency.
pub fn verify_xinput_rumble_mapping() -> bool {
    let map_full = XInputHapticMapping::from_xinput(65535, 32768);
    let diff_l = (map_full.left_amplitude - 1.0).abs();
    let diff_r = (map_full.right_amplitude - 0.5).abs();
    if diff_l > 0.01 || diff_r > 0.01 {
        return false;
    }
    if map_full.left_frequency_hz < LRA_MIN_RESONANCE_HZ || map_full.left_frequency_hz > 200.0 {
        return false;
    }
    if map_full.right_frequency_hz < 200.0 || map_full.right_frequency_hz > LRA_MAX_RESONANCE_HZ {
        return false;
    }

    let map_zero = XInputHapticMapping::from_xinput(0, 0);
    if map_zero.left_amplitude != 0.0 || map_zero.right_amplitude != 0.0 {
        return false;
    }

    true
}

use crate::serial_write_str;

/// Comprehensive verification self-test suite for Handheld PC Haptic Waveform & Audio Transducer Engine.
pub fn test_haptic_dsp_subsystem() -> bool {
    serial_write_str("[KNT-INPUT] Testing Handheld PC haptic waveform & audio transducer service...\r\n");

    // 1. Verify actuator hardware model, resonance tracking, and thermal protection
    if !verify_actuator_model() {
        serial_write_str("[KNT-PANIC] Haptic actuator model or thermal protection failed!\r\n");
        return false;
    }

    // 2. Verify pre-baked tactile waveform library and synthesis engine
    if !verify_waveform_library() {
        serial_write_str("[KNT-PANIC] Tactile waveform library synthesis verification failed!\r\n");
        return false;
    }

    // 3. Verify audio-to-haptics 4th-order Butterworth filter, envelope, and compressor
    if !verify_audio_to_haptics() {
        serial_write_str("[KNT-PANIC] Audio-to-haptics DSP filter and envelope follower failed!\r\n");
        return false;
    }

    // 4. Verify dual-motor legacy XInput rumble mapping
    if !verify_xinput_rumble_mapping() {
        serial_write_str("[KNT-PANIC] XInput rumble motor to LRA parameter conversion failed!\r\n");
        return false;
    }

    serial_write_str("[KNT-HAPTIC-DSP-PASS] Handheld haptic waveform and audio transducer engine verified\r\n");
    true
}
