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
