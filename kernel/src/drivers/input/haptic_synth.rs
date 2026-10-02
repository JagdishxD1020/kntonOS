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
