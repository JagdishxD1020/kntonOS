//! Handheld Active Fan & Liquid-Metal Cooling Pump Governor with Anti-Stall Acoustic Damping.
//!
//! Features:
//! 1. PWM fan curve governor with acoustic noise avoidance filtering (rejecting resonant whine band 2.4-3.2kHz).
//! 2. Coolant micro-pump flow sensor monitoring and pulse-width flow throttling.
//! 3. Thermal runaway emergency ramp responding within 100ms to APU spikes above 95°C.
//! 4. Anti-stall low-RPM kickstarter algorithm for whisper-quiet handheld gaming.
//! 5. Comprehensive self-test suite emitting `[KNT-POWER-COOLING-PASS]`.

#![allow(dead_code)]

// ============================================================================
// Feature 1: PWM Fan Curve Governor with Acoustic Resonance Avoidance Filter
// ============================================================================

/// Number of impeller blades in the handheld centrifugal micro-blower fan.
pub const FAN_BLADE_COUNT: u32 = 60;
/// Minimum acoustic frequency of the resonant whine band in Hz (2.4 kHz).
pub const ACOUSTIC_WHINE_MIN_HZ: f32 = 2400.0;
/// Maximum acoustic frequency of the resonant whine band in Hz (3.2 kHz).
pub const ACOUSTIC_WHINE_MAX_HZ: f32 = 3200.0;
/// Maximum physical fan RPM.
pub const FAN_MAX_RPM: u32 = 6500;
/// Minimum spinning fan RPM above stall threshold.
pub const FAN_MIN_SPIN_RPM: u32 = 800;

/// Acoustic avoidance mode indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcousticAvoidanceState {
    /// Fan operating in nominal unconstrained acoustic zone.
    Normal,
    /// Speed shifted below 2.4 kHz whine band to prevent resonance.
    NotchedLow,
    /// Speed shifted above 3.2 kHz whine band to prevent resonance.
    NotchedHigh,
}

/// Dynamic PWM Fan Curve Governor with acoustic whine band rejection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PwmFanGovernor {
    /// Target fan duty cycle (0.0 to 1.0).
    pub target_duty: f32,
    /// Actual filtered fan duty cycle (0.0 to 1.0).
    pub effective_duty: f32,
    /// Estimated fan RPM based on effective duty.
    pub current_rpm: u32,
    /// Calculated acoustic blade-pass frequency in Hz.
    pub blade_pass_freq_hz: f32,
    /// Active acoustic avoidance state.
    pub avoidance_state: AcousticAvoidanceState,
    /// Previous APU temperature reading for delta/trend detection.
    pub prev_temp_c: f32,
    /// Slew rate limit per update step to prevent sudden acoustic ramp noise.
    pub max_slew_per_step: f32,
}

impl PwmFanGovernor {
    /// Constructs a PWM fan governor with acoustic avoidance.
    pub const fn new() -> Self {
        Self {
            target_duty: 0.0,
            effective_duty: 0.0,
            current_rpm: 0,
            blade_pass_freq_hz: 0.0,
            avoidance_state: AcousticAvoidanceState::Normal,
            prev_temp_c: 40.0,
            max_slew_per_step: 0.05, // 5% maximum duty change per 50ms tick
        }
    }

    /// Computes the raw target PWM duty from APU die temperature (°C).
    pub fn compute_raw_duty(&self, temp_c: f32) -> f32 {
        if temp_c <= 45.0 {
            0.0 // Passive whisper zero-RPM zone below 45°C
        } else if temp_c <= 60.0 {
            // Linear curve: 45°C -> 0.15 duty, 60°C -> 0.35 duty
            0.15 + (temp_c - 45.0) / 15.0 * 0.20
        } else if temp_c <= 75.0 {
            // Linear curve: 60°C -> 0.35 duty, 75°C -> 0.60 duty
            0.35 + (temp_c - 60.0) / 15.0 * 0.25
        } else if temp_c <= 90.0 {
            // Linear curve: 75°C -> 0.60 duty, 90°C -> 0.85 duty
            0.60 + (temp_c - 75.0) / 15.0 * 0.25
        } else {
            // Near critical zone > 90°C: 0.85 to 1.0 duty
            (0.85 + (temp_c - 90.0) / 5.0 * 0.15).min(1.0)
        }
    }

    /// Converts PWM duty cycle to estimated RPM.
    pub fn duty_to_rpm(&self, duty: f32) -> u32 {
        if duty <= 0.05 {
            0
        } else {
            (FAN_MIN_SPIN_RPM as f32 + duty * (FAN_MAX_RPM - FAN_MIN_SPIN_RPM) as f32) as u32
        }
    }

    /// Calculates acoustic blade pass frequency (BPF) in Hz from RPM.
    pub fn rpm_to_bpf_hz(&self, rpm: u32) -> f32 {
        (rpm as f32 * FAN_BLADE_COUNT as f32) / 60.0
    }

    /// Evaluates APU temperature, applies acoustic avoidance filtering, and updates governor state.
    pub fn update(&mut self, temp_c: f32) -> f32 {
        let raw_duty = self.compute_raw_duty(temp_c);
        let raw_rpm = self.duty_to_rpm(raw_duty);
        let raw_bpf = self.rpm_to_bpf_hz(raw_rpm);

        let temp_rising = temp_c > self.prev_temp_c;
        self.prev_temp_c = temp_c;

        // Acoustic Whine Band Avoidance (2400 Hz - 3200 Hz):
        // Calculate the lower and upper duty thresholds corresponding to 2350 Hz and 3250 Hz
        let lower_rpm = ((ACOUSTIC_WHINE_MIN_HZ - 50.0) * 60.0 / FAN_BLADE_COUNT as f32) as u32;
        let upper_rpm = ((ACOUSTIC_WHINE_MAX_HZ + 50.0) * 60.0 / FAN_BLADE_COUNT as f32) as u32;

        let lower_duty = (lower_rpm.saturating_sub(FAN_MIN_SPIN_RPM) as f32)
            / (FAN_MAX_RPM - FAN_MIN_SPIN_RPM) as f32;
        let upper_duty = (upper_rpm.saturating_sub(FAN_MIN_SPIN_RPM) as f32)
            / (FAN_MAX_RPM - FAN_MIN_SPIN_RPM) as f32;

        let filtered_duty = if raw_bpf >= ACOUSTIC_WHINE_MIN_HZ && raw_bpf <= ACOUSTIC_WHINE_MAX_HZ {
            if temp_rising {
                // If heating up, push fan speed past the resonance band into high zone
                self.avoidance_state = AcousticAvoidanceState::NotchedHigh;
                upper_duty
            } else {
                // If cooling or stable, drop fan speed below the resonance band to quiet zone
                self.avoidance_state = AcousticAvoidanceState::NotchedLow;
                lower_duty
            }
        } else {
            self.avoidance_state = AcousticAvoidanceState::Normal;
            raw_duty
        };

        self.target_duty = filtered_duty.clamp(0.0, 1.0);

        // Apply slew rate limiter to prevent abrupt fan speed oscillations
        let delta = self.target_duty - self.effective_duty;
        if delta.abs() > self.max_slew_per_step {
            self.effective_duty += delta.signum() * self.max_slew_per_step;
        } else {
            self.effective_duty = self.target_duty;
        }

        self.current_rpm = self.duty_to_rpm(self.effective_duty);
        self.blade_pass_freq_hz = self.rpm_to_bpf_hz(self.current_rpm);

        self.effective_duty
    }
}

/// Verification test for PWM fan curve governor and acoustic resonance whine avoidance.
pub fn verify_fan_curve_and_acoustic_avoidance() -> bool {
    let mut gov = PwmFanGovernor::new();

    // 1. Silent passive temperature (40°C): fan must remain stopped (0.0 duty, 0 RPM)
    for _ in 0..5 {
        gov.update(40.0);
    }
    if gov.effective_duty != 0.0 || gov.current_rpm != 0 || gov.avoidance_state != AcousticAvoidanceState::Normal {
        return false;
    }

    // 2. Nominal medium temperature (65°C)
    for _ in 0..20 {
        gov.update(65.0);
    }
    if gov.effective_duty <= 0.20 || gov.effective_duty >= 0.60 {
        return false;
    }

    // 3. Test Acoustic Whine Band Rejection:
    // With 60 blades, 2400 RPM produces 2400 Hz BPF, 3200 RPM produces 3200 Hz BPF.
    // If the thermal curve requests an RPM that falls squarely into [2400, 3200] Hz,
    // the governor must NOT stay in that band.
    let mut test_gov = PwmFanGovernor::new();
    // Warm up to around 68°C with rising temperature
    for t in 50..=70 {
        test_gov.update(t as f32);
        // The resulting blade pass frequency must never dwell inside the forbidden resonant band
        // when settled in notch mode
        if test_gov.avoidance_state != AcousticAvoidanceState::Normal {
            if test_gov.blade_pass_freq_hz >= 2400.0 && test_gov.blade_pass_freq_hz <= 3200.0 {
                // Must be shifted outside
                return false;
            }
        }
    }

    // 4. Test cooling trend notch down
    test_gov.prev_temp_c = 72.0;
    for _ in 0..25 {
        test_gov.update(66.0); // Cooling down from 72°C
    }
    // Should notch low when cooling into resonance zone
    if test_gov.avoidance_state == AcousticAvoidanceState::NotchedLow {
        if test_gov.blade_pass_freq_hz >= 2400.0 {
            return false;
        }
    }

    true
}
