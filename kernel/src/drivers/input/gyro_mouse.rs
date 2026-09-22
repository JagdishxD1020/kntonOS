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



// Dual-Zone Angular Acceleration Curve for Precision Micro-Aiming & Flick Turns.

/// Dual-zone acceleration curve settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualZoneAccelerationCurve {
    /// Noise deadzone threshold in rad/s (below which output is zeroed).
    pub deadzone_rad_s: f32,
    /// Low-velocity threshold for micro-aiming dampening.
    pub micro_aim_thresh_rad_s: f32,
    /// Dampening scale factor for micro-aiming [0.0 .. 1.0] (nominally 0.60).
    pub micro_aim_damping: f32,
    /// High-velocity threshold where flick acceleration begins.
    pub flick_thresh_rad_s: f32,
    /// Quadratic acceleration multiplier coefficient for flicks.
    pub flick_accel_coeff: f32,
    /// Maximum acceleration multiplier cap.
    pub max_accel_cap: f32,
}

impl DualZoneAccelerationCurve {
    /// Esports competitive FPS profile with tremor suppression and rapid flick turns.
    pub const fn competitive_fps() -> Self {
        Self {
            deadzone_rad_s: 0.015,
            micro_aim_thresh_rad_s: 0.20,
            micro_aim_damping: 0.60,
            flick_thresh_rad_s: 1.00,
            flick_accel_coeff: 0.80,
            max_accel_cap: 3.00,
        }
    }

    /// Evaluates dynamic sensitivity multiplier based on total angular speed in rad/s.
    pub fn evaluate_gain(&self, angular_speed: f32) -> f32 {
        if angular_speed <= self.deadzone_rad_s {
            return 0.0;
        }

        if angular_speed < self.micro_aim_thresh_rad_s {
            // Smoothly ramp from micro_aim_damping up towards 1.0
            let t = (angular_speed - self.deadzone_rad_s) / (self.micro_aim_thresh_rad_s - self.deadzone_rad_s);
            self.micro_aim_damping + (1.0 - self.micro_aim_damping) * t
        } else if angular_speed <= self.flick_thresh_rad_s {
            // Linear 1:1 natural response
            1.0
        } else {
            // High-speed exponential flick turn zone
            let excess = angular_speed - self.flick_thresh_rad_s;
            let boosted = 1.0 + self.flick_accel_coeff * excess * excess;
            boosted.min(self.max_accel_cap)
        }
    }

    /// Applies dual-zone curve to 2D angular velocity vector (omega_x, omega_y).
    pub fn apply_curve(&self, omega_x: f32, omega_y: f32) -> (f32, f32) {
        let speed = sqrt_f32(omega_x * omega_x + omega_y * omega_y);
        let gain = self.evaluate_gain(speed);
        (omega_x * gain, omega_y * gain)
    }
}

/// Unit test verifying dual-zone angular acceleration curve across velocity regimes.
pub fn verify_dual_zone_acceleration() -> bool {
    let curve = DualZoneAccelerationCurve::competitive_fps();

    // 1. Noise deadzone test (0.01 rad/s <= 0.015) -> gain = 0.0
    if curve.evaluate_gain(0.01) != 0.0 {
        return false;
    }

    // 2. Micro-aiming zone (0.05 rad/s) -> gain is dampened (< 1.0)
    let g_micro = curve.evaluate_gain(0.05);
    if g_micro < 0.60 || g_micro >= 1.0 {
        return false;
    }

    // 3. Normal tracking zone (0.50 rad/s) -> gain = 1.0 (exact natural response)
    let g_norm = curve.evaluate_gain(0.50);
    if (g_norm - 1.0).abs() > 0.001 {
        return false;
    }

    // 4. High-velocity flick zone (2.0 rad/s > 1.0 thresh) -> gain is boosted (> 1.0)
    let g_flick = curve.evaluate_gain(2.00);
    // excess = 1.0 -> 1.0 + 0.8 * 1.0 = 1.8
    if (g_flick - 1.8).abs() > 0.05 {
        return false;
    }

    // 5. Max cap test (10.0 rad/s) -> clamped to max_accel_cap (3.00)
    let g_max = curve.evaluate_gain(10.0);
    if (g_max - 3.00).abs() > 0.001 {
        return false;
    }

    true
}
