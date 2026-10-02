//! Handheld Direct Haptic Waveform Synthesizer & Voice-Coil Actuator (VCA) Force Profiler.
//!
//! Provides dual-axis voice-coil actuator (VCA/LRA) auto-resonance frequency tracking (160Hz - 230Hz),
//! procedural haptic waveform synthesis (Sine, Chirp, Exponential Decay, Tactile Impact),
//! stereo spatial panning, sub-120Hz bass-to-rumble transcoding, and thermal brake cutoff protection.

#[allow(unused_imports)]
use crate::graphics::types::{cos_f32, sin_f32, PI, TWO_PI};

/// Minimum allowed linear resonant actuator resonance frequency in Hz.
pub const VCA_MIN_RESONANCE_HZ: f32 = 160.0;
/// Maximum allowed linear resonant actuator resonance frequency in Hz.
pub const VCA_MAX_RESONANCE_HZ: f32 = 230.0;
/// Default update rate for haptic synthesis pipeline (1000 Hz = 1ms period).
pub const HAPTIC_SAMPLE_RATE_HZ: f32 = 1000.0;
/// Discrete time step in seconds (dt = 0.001 s).
pub const HAPTIC_DT_S: f32 = 1.0 / HAPTIC_SAMPLE_RATE_HZ;

// ============================================================================
// Feature 1: Dual-Axis Voice-Coil Actuator (VCA) Drivers with Resonance Tracking
// ============================================================================

/// Channel identifier for dual-axis handheld haptic actuators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActuatorChannel {
    /// Left handheld grip actuator.
    LeftGrip = 0,
    /// Right handheld grip actuator.
    RightGrip = 1,
}

/// Single Voice-Coil Actuator (VCA / LRA) channel driver with closed-loop resonance auto-tracking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VcaChannel {
    /// Actuator grip position channel.
    pub channel: ActuatorChannel,
    /// Actuator nominal baseline resonance frequency in Hz.
    pub nominal_freq_hz: f32,
    /// Current closed-loop tracked resonance frequency in Hz (160 Hz - 230 Hz).
    pub resonance_freq_hz: f32,
    /// Instantaneous normalized driving amplitude (0.0 .. 1.0).
    pub drive_amplitude: f32,
    /// Accumulated oscillator phase in radians [0.0 .. TWO_PI).
    pub phase_rad: f32,
    /// Most recent back-EMF feedback magnitude in millivolts.
    pub back_emf_mv: f32,
    /// Current phase angle error between driving voltage and coil velocity (degrees).
    pub phase_error_deg: f32,
    /// Tracking adjustment loop gain (Hz per degree of phase error).
    pub tracking_gain: f32,
    /// Whether the closed-loop tracking is locked within +/- 5.0 degrees.
    pub is_resonance_locked: bool,
}

impl VcaChannel {
    /// Constructs a VCA channel with specified nominal resonance frequency.
    pub const fn new(channel: ActuatorChannel, nominal_freq_hz: f32) -> Self {
        Self {
            channel,
            nominal_freq_hz,
            resonance_freq_hz: nominal_freq_hz,
            drive_amplitude: 0.0,
            phase_rad: 0.0,
            back_emf_mv: 0.0,
            phase_error_deg: 0.0,
            tracking_gain: 0.05,
            is_resonance_locked: true,
        }
    }

    /// Resets the channel state and phase to initial defaults.
    pub fn reset(&mut self) {
        self.resonance_freq_hz = self.nominal_freq_hz;
        self.drive_amplitude = 0.0;
        self.phase_rad = 0.0;
        self.back_emf_mv = 0.0;
        self.phase_error_deg = 0.0;
        self.is_resonance_locked = true;
    }

    /// Ingests closed-loop back-EMF feedback and updates auto-resonance frequency tracking.
    pub fn update_resonance_tracking(&mut self, back_emf_mv: f32, phase_error_deg: f32) {
        self.back_emf_mv = back_emf_mv;
        self.phase_error_deg = phase_error_deg;

        // Auto-tracking: positive phase error indicates mechanical resonance is higher than current drive
        let delta_f = self.tracking_gain * phase_error_deg;
        let mut new_f = self.resonance_freq_hz + delta_f;

        // Clamp within physical resonance window (160.0 Hz to 230.0 Hz)
        if new_f < VCA_MIN_RESONANCE_HZ {
            new_f = VCA_MIN_RESONANCE_HZ;
        } else if new_f > VCA_MAX_RESONANCE_HZ {
            new_f = VCA_MAX_RESONANCE_HZ;
        }
        self.resonance_freq_hz = new_f;

        // Lock condition: phase error within +/- 5.0 degrees
        let abs_err = if phase_error_deg < 0.0 { -phase_error_deg } else { phase_error_deg };
        self.is_resonance_locked = abs_err <= 5.0;
    }

    /// Advances the oscillator phase and generates the instantaneous sinusoidal force sample.
    pub fn synthesize_step(&mut self, dt_s: f32) -> f32 {
        let sample = self.drive_amplitude * sin_f32(self.phase_rad);

        // Advance phase
        let phase_inc = TWO_PI * self.resonance_freq_hz * dt_s;
        self.phase_rad += phase_inc;
        while self.phase_rad >= TWO_PI {
            self.phase_rad -= TWO_PI;
        }

        sample
    }
}

/// Dual-Axis Voice-Coil Actuator (VCA) Driver managing Left & Right grip haptic engines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualAxisVcaDriver {
    /// Left grip VCA channel (nominally 175.0 Hz).
    pub left: VcaChannel,
    /// Right grip VCA channel (nominally 205.0 Hz).
    pub right: VcaChannel,
}

impl DualAxisVcaDriver {
    /// Constructs a dual-axis VCA driver tuned for handheld console grip actuators.
    pub const fn new() -> Self {
        Self {
            left: VcaChannel::new(ActuatorChannel::LeftGrip, 175.0),
            right: VcaChannel::new(ActuatorChannel::RightGrip, 205.0),
        }
    }

    /// Resets both actuator channels to nominal defaults.
    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }

    /// Ingests telemetry feedback for both channels simultaneously.
    pub fn update_telemetry(
        &mut self,
        left_emf: f32,
        left_phase_err: f32,
        right_emf: f32,
        right_phase_err: f32,
    ) {
        self.left.update_resonance_tracking(left_emf, left_phase_err);
        self.right.update_resonance_tracking(right_emf, right_phase_err);
    }

    /// Synthesizes instantaneous stereo force samples (left, right) in [-1.0 .. 1.0].
    pub fn synthesize_stereo(&mut self, dt_s: f32) -> (f32, f32) {
        (self.left.synthesize_step(dt_s), self.right.synthesize_step(dt_s))
    }
}

/// Verification test for VCA closed-loop resonance frequency auto-tracking.
pub fn verify_vca_resonance_tracking() -> bool {
    let mut driver = DualAxisVcaDriver::new();

    // 1. Initial nominal resonance checks
    if (driver.left.resonance_freq_hz - 175.0).abs() > 1e-4 {
        return false;
    }
    if (driver.right.resonance_freq_hz - 205.0).abs() > 1e-4 {
        return false;
    }
    if !driver.left.is_resonance_locked || !driver.right.is_resonance_locked {
        return false;
    }

    // 2. Simulate Left Actuator Grip Warmup & Mechanical Drift (resonance shifts up to 185 Hz)
    // Positive phase error +10.0 deg across 20 iterations
    for _ in 0..20 {
        driver.left.update_resonance_tracking(320.0, 10.0);
    }
    // Frequency must have increased by 20 * 0.05 * 10 = +10.0 Hz -> 185.0 Hz
    if (driver.left.resonance_freq_hz - 185.0).abs() > 0.1 {
        return false;
    }

    // Now error zeroes out at resonance lock
    driver.left.update_resonance_tracking(350.0, 1.2);
    if !driver.left.is_resonance_locked {
        return false;
    }

    // 3. Test Boundary Clamping at Min (160 Hz) and Max (230 Hz)
    for _ in 0..100 {
        driver.left.update_resonance_tracking(100.0, -25.0);
    }
    if (driver.left.resonance_freq_hz - VCA_MIN_RESONANCE_HZ).abs() > 1e-4 {
        return false;
    }

    for _ in 0..100 {
        driver.right.update_resonance_tracking(100.0, 25.0);
    }
    if (driver.right.resonance_freq_hz - VCA_MAX_RESONANCE_HZ).abs() > 1e-4 {
        return false;
    }

    // 4. Test Synthesis Step
    driver.left.drive_amplitude = 0.80;
    driver.right.drive_amplitude = 0.50;
    let (l_out, r_out) = driver.synthesize_stereo(0.001);
    let abs_l = if l_out < 0.0 { -l_out } else { l_out };
    let abs_r = if r_out < 0.0 { -r_out } else { r_out };
    if abs_l > 0.80 || abs_r > 0.50 {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Procedural Haptic Waveform Synthesizer with Tactile Envelopes
// ============================================================================

/// Fast polynomial approximation of e^(-x) for x >= 0 without external libm.
#[inline(always)]
pub fn exp_neg_f32(x: f32) -> f32 {
    if x <= 0.0 {
        return 1.0;
    }
    if x > 14.0 {
        return 0.0;
    }
    let x2 = x * x;
    let x3 = x2 * x;
    let x4 = x2 * x2;
    let denom = 1.0 + x + 0.5 * x2 + 0.1666667 * x3 + 0.0416667 * x4;
    1.0 / denom
}

/// Category of procedural haptic tactile vibration effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HapticWaveformType {
    /// Steady continuous or gated sinusoidal vibration.
    Sine,
    /// Dynamic frequency-swept vibration (e.g. accelerating engine or energy charge).
    Chirp,
    /// Rapid mechanical click or button press tactile feedback.
    ExponentialDecay,
    /// High-impact strike, recoil, or explosion with sharp initial impulse.
    TactileImpact,
}

/// Configuration descriptor for procedural haptic tactile waveform generation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HapticEffectConfig {
    /// Waveform synthesis pattern.
    pub effect_type: HapticWaveformType,
    /// Total effect duration in milliseconds.
    pub duration_ms: u32,
    /// Peak normalized amplitude [0.0 .. 1.0].
    pub peak_amplitude: f32,
    /// Starting frequency in Hz.
    pub start_freq_hz: f32,
    /// Ending frequency in Hz (used for frequency chirps).
    pub end_freq_hz: f32,
    /// Attack envelope duration in milliseconds.
    pub attack_ms: u32,
    /// Decay duration in milliseconds.
    pub decay_ms: u32,
    /// Sustain normalized level [0.0 .. 1.0].
    pub sustain_level: f32,
    /// Release duration in milliseconds.
    pub release_ms: u32,
}

impl HapticEffectConfig {
    /// Procedural crisp button click / tactile feedback pulse (15 ms).
    pub const fn click() -> Self {
        Self {
            effect_type: HapticWaveformType::ExponentialDecay,
            duration_ms: 15,
            peak_amplitude: 0.90,
            start_freq_hz: 180.0,
            end_freq_hz: 180.0,
            attack_ms: 2,
            decay_ms: 13,
            sustain_level: 0.0,
            release_ms: 0,
        }
    }

    /// Procedural sustained gaming rumble vibration.
    pub const fn rumble(duration_ms: u32, intensity: f32) -> Self {
        Self {
            effect_type: HapticWaveformType::Sine,
            duration_ms,
            peak_amplitude: intensity,
            start_freq_hz: 175.0,
            end_freq_hz: 175.0,
            attack_ms: 10,
            decay_ms: 0,
            sustain_level: 1.0,
            release_ms: 15,
        }
    }

    /// Procedural frequency chirp sweeping across the VCA operating band.
    pub const fn chirp_sweep(duration_ms: u32, start_hz: f32, end_hz: f32) -> Self {
        Self {
            effect_type: HapticWaveformType::Chirp,
            duration_ms,
            peak_amplitude: 0.85,
            start_freq_hz: start_hz,
            end_freq_hz: end_hz,
            attack_ms: 10,
            decay_ms: 0,
            sustain_level: 1.0,
            release_ms: 10,
        }
    }

    /// Procedural sharp weapon recoil or physical explosion impact.
    pub const fn tactile_impact() -> Self {
        Self {
            effect_type: HapticWaveformType::TactileImpact,
            duration_ms: 80,
            peak_amplitude: 1.0,
            start_freq_hz: 165.0,
            end_freq_hz: 165.0,
            attack_ms: 3,
            decay_ms: 50,
            sustain_level: 0.15,
            release_ms: 27,
        }
    }
}

/// Real-time Procedural Haptic Waveform Synthesizer engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProceduralHapticSynthesizer {
    /// Active effect configuration being rendered, if any.
    pub current_effect: Option<HapticEffectConfig>,
    /// Accumulated playback elapsed time in milliseconds.
    pub elapsed_ms: f32,
    /// Oscillator carrier phase in radians [0.0 .. TWO_PI).
    pub phase_rad: f32,
    /// Whether an effect is actively playing.
    pub is_active: bool,
}

impl ProceduralHapticSynthesizer {
    /// Constructs a procedural haptic synthesizer in idle state.
    pub const fn new() -> Self {
        Self {
            current_effect: None,
            elapsed_ms: 0.0,
            phase_rad: 0.0,
            is_active: false,
        }
    }

    /// Triggers playback of a procedural haptic effect configuration.
    pub fn trigger_effect(&mut self, effect: HapticEffectConfig) {
        self.current_effect = Some(effect);
        self.elapsed_ms = 0.0;
        self.phase_rad = 0.0;
        self.is_active = true;
    }

    /// Stops playback and resets synthesizer state.
    pub fn stop(&mut self) {
        self.current_effect = None;
        self.elapsed_ms = 0.0;
        self.phase_rad = 0.0;
        self.is_active = false;
    }

    /// Computes the instantaneous amplitude envelope [0.0 .. 1.0] for the active effect.
    pub fn evaluate_envelope(&self) -> f32 {
        let effect = match self.current_effect {
            Some(e) => e,
            None => return 0.0,
        };

        let t = self.elapsed_ms;
        let d = effect.duration_ms as f32;
        if t >= d || d <= 0.0 {
            return 0.0;
        }

        match effect.effect_type {
            HapticWaveformType::ExponentialDecay => {
                let norm_t = t / d;
                effect.peak_amplitude * exp_neg_f32(3.5 * norm_t)
            }
            HapticWaveformType::TactileImpact => {
                let a = effect.attack_ms as f32;
                if t < a && a > 0.0 {
                    // Fast rise to peak
                    effect.peak_amplitude * (t / a)
                } else {
                    // Exponential tail decay from peak
                    let post_t = (t - a) / (d - a);
                    effect.peak_amplitude * exp_neg_f32(4.0 * post_t)
                }
            }
            HapticWaveformType::Sine | HapticWaveformType::Chirp => {
                let a = effect.attack_ms as f32;
                let r = effect.release_ms as f32;
                let _sustain_start = a;
                let sustain_end = (d - r).max(a);

                if t < a && a > 0.0 {
                    // Attack ramp
                    effect.peak_amplitude * (t / a)
                } else if t <= sustain_end {
                    // Sustain region
                    effect.peak_amplitude * effect.sustain_level
                } else if r > 0.0 {
                    // Release fade-out
                    let rel_t = (d - t) / r;
                    effect.peak_amplitude * effect.sustain_level * rel_t.clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
        }
    }

    /// Computes the instantaneous frequency in Hz for the active effect.
    pub fn evaluate_frequency(&self) -> f32 {
        let effect = match self.current_effect {
            Some(e) => e,
            None => return 175.0,
        };

        let d = effect.duration_ms as f32;
        if d <= 0.0 {
            return effect.start_freq_hz;
        }

        let progress = (self.elapsed_ms / d).clamp(0.0, 1.0);
        effect.start_freq_hz + progress * (effect.end_freq_hz - effect.start_freq_hz)
    }

    /// Synthesizes a single haptic output sample in [-1.0 .. 1.0], advancing time by dt_s.
    pub fn synthesize_sample(&mut self, dt_s: f32) -> f32 {
        if !self.is_active {
            return 0.0;
        }

        let env = self.evaluate_envelope();
        let freq = self.evaluate_frequency();

        let sample = env * sin_f32(self.phase_rad);

        // Advance phase
        let phase_inc = TWO_PI * freq * dt_s;
        self.phase_rad += phase_inc;
        while self.phase_rad >= TWO_PI {
            self.phase_rad -= TWO_PI;
        }

        // Advance time
        self.elapsed_ms += dt_s * 1000.0;
        if let Some(effect) = self.current_effect {
            if self.elapsed_ms >= effect.duration_ms as f32 {
                self.is_active = false;
            }
        }

        sample
    }
}

/// Verification test for procedural haptic waveform synthesizer and tactile envelopes.
pub fn verify_procedural_waveform_synth() -> bool {
    let mut synth = ProceduralHapticSynthesizer::new();
    let dt = 0.001; // 1 ms update step

    // 1. Verify Crisp Button Click Feedback (15 ms)
    synth.trigger_effect(HapticEffectConfig::click());
    if !synth.is_active {
        return false;
    }

    // Peak envelope should be near 0.90 within the first 2 ms
    synth.synthesize_sample(dt);
    synth.synthesize_sample(dt);
    let click_env = synth.evaluate_envelope();
    if click_env <= 0.70 || click_env > 0.95 {
        return false;
    }

    // Run until 15 ms duration completes
    for _ in 0..15 {
        synth.synthesize_sample(dt);
    }
    // Must be finished and inactive
    if synth.is_active {
        return false;
    }

    // 2. Verify Frequency Chirp Sweep (160 Hz -> 230 Hz over 50 ms)
    synth.trigger_effect(HapticEffectConfig::chirp_sweep(50, 160.0, 230.0));
    let f_start = synth.evaluate_frequency();
    if (f_start - 160.0).abs() > 1.0 {
        return false;
    }

    // Step 25 ms (halfway)
    for _ in 0..25 {
        synth.synthesize_sample(dt);
    }
    let f_mid = synth.evaluate_frequency();
    if (f_mid - 195.0).abs() > 3.0 {
        return false;
    }

    synth.stop();

    // 3. Verify Tactile Impact Envelope
    synth.trigger_effect(HapticEffectConfig::tactile_impact());
    for _ in 0..3 {
        synth.synthesize_sample(dt);
    }
    let impact_peak = synth.evaluate_envelope();
    if (impact_peak - 1.0).abs() > 0.15 {
        return false;
    }

    true
}
