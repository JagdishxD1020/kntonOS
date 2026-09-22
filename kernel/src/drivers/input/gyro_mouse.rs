//! Handheld Gyro-to-Mouse & Dynamic Flick Aiming Translator.

#![allow(dead_code)]

#[allow(unused_imports)]
use crate::graphics::{cos_f32, sin_f32, sqrt_f32};

/// Gyro aiming activation mode trigger categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GyroMouseActivation {
    /// Activated while thumb touches the capacitive thumbstick cap.
    CapacitiveThumbTouch,
    /// Activated while Left Trigger (LT/L2) is pulled beyond threshold (ADS aiming).
    TriggerSoftPull,
    /// Gyro aiming is permanently active.
    AlwaysOn,
    /// Activated while a designated paddle or grip button is held.
    ButtonHold,
}

/// Hardware trigger input state for evaluating gyro activation gating.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroTriggerInput {
    pub cap_touch: bool,
    pub trigger_pull: f32,
    pub button_held: bool,
}

impl GyroTriggerInput {
    pub const fn empty() -> Self {
        Self {
            cap_touch: false,
            trigger_pull: 0.0,
            button_held: false,
        }
    }
}

/// Evaluates gyro activation state based on trigger mode and hardware input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroActivationGating {
    pub mode: GyroMouseActivation,
    pub trigger_threshold: f32,
}

impl GyroActivationGating {
    pub const fn new(mode: GyroMouseActivation, trigger_threshold: f32) -> Self {
        Self {
            mode,
            trigger_threshold,
        }
    }

    /// Evaluates if gyro tracking should be active.
    pub fn is_active(&self, input: &GyroTriggerInput) -> bool {
        match self.mode {
            GyroMouseActivation::AlwaysOn => true,
            GyroMouseActivation::CapacitiveThumbTouch => input.cap_touch,
            GyroMouseActivation::TriggerSoftPull => input.trigger_pull >= self.trigger_threshold,
            GyroMouseActivation::ButtonHold => input.button_held,
        }
    }
}

/// Unit test verifying gyro activation modes and gating logic.
pub fn verify_activation_gating() -> bool {
    let mut input = GyroTriggerInput::empty();

    // 1. AlwaysOn
    let gate_always = GyroActivationGating::new(GyroMouseActivation::AlwaysOn, 0.25);
    if !gate_always.is_active(&input) {
        return false;
    }

    // 2. CapacitiveThumbTouch
    let gate_cap = GyroActivationGating::new(GyroMouseActivation::CapacitiveThumbTouch, 0.25);
    if gate_cap.is_active(&input) {
        return false;
    }
    input.cap_touch = true;
    if !gate_cap.is_active(&input) {
        return false;
    }
    input.cap_touch = false;

    // 3. TriggerSoftPull (threshold 0.25)
    let gate_trig = GyroActivationGating::new(GyroMouseActivation::TriggerSoftPull, 0.25);
    input.trigger_pull = 0.20;
    if gate_trig.is_active(&input) {
        return false;
    }
    input.trigger_pull = 0.26;
    if !gate_trig.is_active(&input) {
        return false;
    }
    input.trigger_pull = 0.0;

    // 4. ButtonHold
    let gate_btn = GyroActivationGating::new(GyroMouseActivation::ButtonHold, 0.25);
    if gate_btn.is_active(&input) {
        return false;
    }
    input.button_held = true;
    if !gate_btn.is_active(&input) {
        return false;
    }

    true
}



// Natural Yaw-Roll Sensor Combiner for Intuitive Handheld Tilt Aiming.

/// Natural yaw-roll combiner blending angular velocities based on device tilt pitch angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NaturalYawRollCombiner {
    /// Invert horizontal steering axis.
    pub invert_x: bool,
    /// Invert vertical pitch axis.
    pub invert_y: bool,
}

impl NaturalYawRollCombiner {
    pub const fn new(invert_x: bool, invert_y: bool) -> Self {
        Self { invert_x, invert_y }
    }

    /// Blends yaw and roll angular rates into 2D screen aiming angular velocity [omega_x, omega_y].
    /// pitch_angle_rad is the current handheld tilt pitch angle in radians (0.0 = flat, PI/2 = upright).
    pub fn combine(
        &self,
        gyro_pitch: f32,
        gyro_yaw: f32,
        gyro_roll: f32,
        pitch_angle_rad: f32,
    ) -> (f32, f32) {
        // Clamp tilt angle to [-PI/2 .. +PI/2]
        let half_pi = 1.5707963;
        let clamped_pitch = pitch_angle_rad.clamp(-half_pi, half_pi);

        let cos_p = cos_f32(clamped_pitch);
        let sin_p = sin_f32(clamped_pitch);

        // Horizontal aim combines yaw when flat and roll when upright
        let mut omega_x = gyro_yaw * cos_p + gyro_roll * sin_p;
        let mut omega_y = gyro_pitch;

        if self.invert_x {
            omega_x = -omega_x;
        }
        if self.invert_y {
            omega_y = -omega_y;
        }

        (omega_x, omega_y)
    }
}

/// Unit test verifying natural yaw-roll sensor combiner across tilt angles.
pub fn verify_yaw_roll_tilt_combiner() -> bool {
    let combiner = NaturalYawRollCombiner::new(false, false);

    // 1. Flat orientation (pitch = 0.0 rad): pure yaw maps to horizontal aim
    // cos(0) = 1.0, sin(0) = 0.0
    let (ox_flat, oy_flat) = combiner.combine(0.5, 2.0, 1.0, 0.0);
    if (ox_flat - 2.0).abs() > 0.01 || (oy_flat - 0.5).abs() > 0.01 {
        return false;
    }

    // 2. Upright orientation (pitch = PI/2 ~ 1.5708 rad): pure roll maps to horizontal aim
    // cos(PI/2) ~ 0.0, sin(PI/2) ~ 1.0
    let (ox_upright, oy_upright) = combiner.combine(0.5, 2.0, 1.0, 1.5707963);
    if (ox_upright - 1.0).abs() > 0.02 || (oy_upright - 0.5).abs() > 0.01 {
        return false;
    }

    // 3. 45-degree angle (pitch = PI/4 ~ 0.7854 rad): 50/50 blend of yaw and roll
    // cos(PI/4) = sin(PI/4) ~ 0.7071
    let (ox_45, _) = combiner.combine(0.0, 1.0, 1.0, 0.7853982);
    let expected = 1.0 * 0.7071068 + 1.0 * 0.7071068; // ~1.4142
    if (ox_45 - expected).abs() > 0.02 {
        return false;
    }

    // 4. Inversion check
    let combiner_inv = NaturalYawRollCombiner::new(true, true);
    let (ox_inv, oy_inv) = combiner_inv.combine(0.5, 2.0, 0.0, 0.0);
    if (ox_inv - (-2.0)).abs() > 0.01 || (oy_inv - (-0.5)).abs() > 0.01 {
        return false;
    }

    true
}
