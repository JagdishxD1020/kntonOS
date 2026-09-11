//! Handheld Controller Profile Configurator & Remapping Daemon.

use crate::graphics::types::sqrt_f32;

/// Stick sensitivity curve characteristics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickCurveType {
    Linear,
    Exponential,
    SCurve,
}

/// Analog stick calibration, deadzones, and response curve settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickProfile {
    pub inner_deadzone: f32,
    pub outer_deadzone: f32,
    pub anti_deadzone: f32,
    pub curve_type: StickCurveType,
    pub sensitivity: f32,
}

impl StickProfile {
    pub const fn default_left() -> Self {
        Self {
            inner_deadzone: 0.08,
            outer_deadzone: 0.95,
            anti_deadzone: 0.0,
            curve_type: StickCurveType::Linear,
            sensitivity: 1.0,
        }
    }

    pub const fn default_right() -> Self {
        Self {
            inner_deadzone: 0.05,
            outer_deadzone: 0.98,
            anti_deadzone: 0.12,
            curve_type: StickCurveType::Exponential,
            sensitivity: 1.0,
        }
    }

    /// Evaluates raw normalized stick input (-1.0 to 1.0) through deadzones and curves.
    pub fn transform_input(&self, x: f32, y: f32) -> (f32, f32) {
        let mag = sqrt_f32(x * x + y * y);
        if mag <= self.inner_deadzone || mag < 1e-6 {
            return (0.0, 0.0);
        }

        let clamped_outer = if mag > self.outer_deadzone { self.outer_deadzone } else { mag };
        let active_span = self.outer_deadzone - self.inner_deadzone;
        if active_span <= 0.001 {
            return (0.0, 0.0);
        }

        let norm_mag = ((clamped_outer - self.inner_deadzone) / active_span).clamp(0.0, 1.0);

        let curved_mag = match self.curve_type {
            StickCurveType::Linear => norm_mag,
            StickCurveType::Exponential => norm_mag * norm_mag,
            StickCurveType::SCurve => norm_mag * norm_mag * (3.0 - 2.0 * norm_mag),
        };

        let scaled_mag = self.anti_deadzone + (1.0 - self.anti_deadzone) * curved_mag;
        let final_mag = (scaled_mag * self.sensitivity).clamp(0.0, 1.0);

        let dir_x = x / mag;
        let dir_y = y / mag;
        (dir_x * final_mag, dir_y * final_mag)
    }
}

/// Per-game controller profile holding stick configurations and identifier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerGameProfile {
    pub game_id_hash: u64,
    pub left_stick: StickProfile,
    pub right_stick: StickProfile,
}

impl ControllerGameProfile {
    pub const fn new(game_id_hash: u64) -> Self {
        Self {
            game_id_hash,
            left_stick: StickProfile::default_left(),
            right_stick: StickProfile::default_right(),
        }
    }
}

/// Verifies controller profile data models, stick deadzones, anti-deadzones, and sensitivity curves.
pub fn verify_profile_data_model_and_stick_curves() -> bool {
    let mut profile = StickProfile::default_right();
    profile.inner_deadzone = 0.10;
    profile.outer_deadzone = 0.90;
    profile.anti_deadzone = 0.15;
    profile.curve_type = StickCurveType::Linear;

    // 1. Within inner deadzone (0.05 < 0.10) -> output must be zero
    let (zx, zy) = profile.transform_input(0.05, 0.0);
    if zx != 0.0 || zy != 0.0 {
        return false;
    }

    // 2. Just outside inner deadzone -> output jumps to anti-deadzone floor (0.15)
    let (ax, _ay) = profile.transform_input(0.1001, 0.0);
    if ax < 0.149 || ax > 0.16 {
        return false;
    }

    // 3. Beyond outer deadzone -> capped to 1.0
    let (cx, _cy) = profile.transform_input(0.95, 0.0);
    if (cx - 1.0).abs() > 0.001 {
        return false;
    }

    // 4. Exponential curve test: midpoint 0.5 becomes 0.25 before anti-deadzone
    profile.anti_deadzone = 0.0;
    profile.curve_type = StickCurveType::Exponential;
    profile.inner_deadzone = 0.0;
    profile.outer_deadzone = 1.0;
    let (ex, _ey) = profile.transform_input(0.5, 0.0);
    if (ex - 0.25).abs() > 0.01 {
        return false;
    }

    true
}

// Input Action Remapping Matrix, Trigger Thresholds, and Grip Paddles

/// Physical and virtual handheld gamepad buttons and back grip paddles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandheldButton {
    A,
    B,
    X,
    Y,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    LeftBumper,
    RightBumper,
    LeftStickClick,
    RightStickClick,
    Start,
    Back,
    Guide,
    PaddleL4,
    PaddleL5,
    PaddleR4,
    PaddleR5,
}

pub const BUTTON_COUNT: usize = 19;

impl HandheldButton {
    pub const fn to_index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
            Self::X => 2,
            Self::Y => 3,
            Self::DpadUp => 4,
            Self::DpadDown => 5,
            Self::DpadLeft => 6,
            Self::DpadRight => 7,
            Self::LeftBumper => 8,
            Self::RightBumper => 9,
            Self::LeftStickClick => 10,
            Self::RightStickClick => 11,
            Self::Start => 12,
            Self::Back => 13,
            Self::Guide => 14,
            Self::PaddleL4 => 15,
            Self::PaddleL5 => 16,
            Self::PaddleR4 => 17,
            Self::PaddleR5 => 18,
        }
    }

    pub const fn from_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(Self::A),
            1 => Some(Self::B),
            2 => Some(Self::X),
            3 => Some(Self::Y),
            4 => Some(Self::DpadUp),
            5 => Some(Self::DpadDown),
            6 => Some(Self::DpadLeft),
            7 => Some(Self::DpadRight),
            8 => Some(Self::LeftBumper),
            9 => Some(Self::RightBumper),
            10 => Some(Self::LeftStickClick),
            11 => Some(Self::RightStickClick),
            12 => Some(Self::Start),
            13 => Some(Self::Back),
            14 => Some(Self::Guide),
            15 => Some(Self::PaddleL4),
            16 => Some(Self::PaddleL5),
            17 => Some(Self::PaddleR4),
            18 => Some(Self::PaddleR5),
            _ => None,
        }
    }
}

/// Analog trigger digital activation threshold and hair-trigger configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriggerThresholdConfig {
    pub activation_threshold: f32,
    pub hair_trigger_mode: bool,
}

impl TriggerThresholdConfig {
    pub const fn normal() -> Self {
        Self {
            activation_threshold: 0.50,
            hair_trigger_mode: false,
        }
    }

    pub const fn hair_trigger() -> Self {
        Self {
            activation_threshold: 0.20,
            hair_trigger_mode: true,
        }
    }

    pub fn is_activated(&self, raw_0_to_1: f32) -> bool {
        raw_0_to_1 >= self.activation_threshold
    }
}

/// Physical-to-virtual button remapping lookup table.
#[derive(Clone, Copy, Debug)]
pub struct ButtonRemappingTable {
    pub mappings: [Option<HandheldButton>; BUTTON_COUNT],
}

impl ButtonRemappingTable {
    pub const fn identity() -> Self {
        Self {
            mappings: [
                Some(HandheldButton::A),
                Some(HandheldButton::B),
                Some(HandheldButton::X),
                Some(HandheldButton::Y),
                Some(HandheldButton::DpadUp),
                Some(HandheldButton::DpadDown),
                Some(HandheldButton::DpadLeft),
                Some(HandheldButton::DpadRight),
                Some(HandheldButton::LeftBumper),
                Some(HandheldButton::RightBumper),
                Some(HandheldButton::LeftStickClick),
                Some(HandheldButton::RightStickClick),
                Some(HandheldButton::Start),
                Some(HandheldButton::Back),
                Some(HandheldButton::Guide),
                // Default back paddles assigned to face buttons
                Some(HandheldButton::A), // L4 -> A (jump)
                Some(HandheldButton::X), // L5 -> X (reload)
                Some(HandheldButton::B), // R4 -> B (crouch)
                Some(HandheldButton::Y), // R5 -> Y (switch weapon)
            ],
        }
    }

    pub fn remap(&self, physical: HandheldButton) -> Option<HandheldButton> {
        self.mappings[physical.to_index()]
    }

    pub fn set_mapping(&mut self, physical: HandheldButton, target: Option<HandheldButton>) {
        self.mappings[physical.to_index()] = target;
    }
}

/// Controller remapping manager evaluating button tables and analog triggers.
pub struct ControllerRemapManager {
    pub table: ButtonRemappingTable,
    pub left_trigger: TriggerThresholdConfig,
    pub right_trigger: TriggerThresholdConfig,
}

impl ControllerRemapManager {
    pub const fn new() -> Self {
        Self {
            table: ButtonRemappingTable::identity(),
            left_trigger: TriggerThresholdConfig::normal(),
            right_trigger: TriggerThresholdConfig::hair_trigger(),
        }
    }

    /// Remaps a 32-bit physical button bitmask to virtual output bitmask.
    pub fn remap_button_mask(&self, physical_mask: u32) -> u32 {
        let mut out_mask: u32 = 0;
        for i in 0..BUTTON_COUNT {
            if (physical_mask & (1 << i)) != 0 {
                if let Some(phys_btn) = HandheldButton::from_index(i) {
                    if let Some(virt_btn) = self.table.remap(phys_btn) {
                        out_mask |= 1 << virt_btn.to_index();
                    }
                }
            }
        }
        out_mask
    }
}

/// Verifies input button remapping matrix, trigger thresholds, and grip paddles.
pub fn verify_button_remapping_and_paddles() -> bool {
    let mut manager = ControllerRemapManager::new();

    // 1. Verify default paddle L4 mapping -> outputs virtual A (bit 0)
    let l4_bit = 1 << HandheldButton::PaddleL4.to_index();
    let remapped_l4 = manager.remap_button_mask(l4_bit);
    if remapped_l4 != (1 << HandheldButton::A.to_index()) {
        return false;
    }

    // 2. Custom remapping: remap B to LeftStickClick
    manager.table.set_mapping(HandheldButton::B, Some(HandheldButton::LeftStickClick));
    let b_bit = 1 << HandheldButton::B.to_index();
    let remapped_b = manager.remap_button_mask(b_bit);
    if remapped_b != (1 << HandheldButton::LeftStickClick.to_index()) {
        return false;
    }

    // 3. Hair trigger threshold verification (activation at 0.20)
    if manager.right_trigger.is_activated(0.15) {
        return false;
    }
    if !manager.right_trigger.is_activated(0.25) {
        return false;
    }

    true
}

// Gyro-to-Stick & Gyro-to-Mouse Redirection Engine

/// Target mode for gyro motion vector translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionRedirectionMode {
    Disabled,
    GyroToMouse,
    GyroToRightStick,
}

/// Motion translation parameters and sensitivities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionRedirectionConfig {
    pub mode: MotionRedirectionMode,
    pub yaw_sensitivity: f32,
    pub pitch_sensitivity: f32,
    pub roll_factor: f32,
    pub invert_yaw: bool,
    pub invert_pitch: bool,
    pub deadzone_dps: f32,
    pub stick_deflection_scale: f32,
}

impl MotionRedirectionConfig {
    pub const fn default_mouse() -> Self {
        Self {
            mode: MotionRedirectionMode::GyroToMouse,
            yaw_sensitivity: 1.5,
            pitch_sensitivity: 1.5,
            roll_factor: 0.5,
            invert_yaw: false,
            invert_pitch: false,
            deadzone_dps: 1.5,
            stick_deflection_scale: 400.0,
        }
    }

    pub const fn default_right_stick() -> Self {
        Self {
            mode: MotionRedirectionMode::GyroToRightStick,
            yaw_sensitivity: 1.0,
            pitch_sensitivity: 1.0,
            roll_factor: 0.5,
            invert_yaw: false,
            invert_pitch: false,
            deadzone_dps: 1.5,
            stick_deflection_scale: 500.0,
        }
    }
}

/// Motion translation engine transforming angular rates to mouse deltas or stick deflections.
pub struct MotionRedirectionEngine {
    pub config: MotionRedirectionConfig,
}

impl MotionRedirectionEngine {
    pub const fn new(config: MotionRedirectionConfig) -> Self {
        Self { config }
    }

    /// Processes gyro angular rates in degrees/second, producing (mouse_dx, mouse_dy) and (stick_x, stick_y).
    pub fn process_motion(
        &self,
        yaw_dps: f32,
        pitch_dps: f32,
        roll_dps: f32,
    ) -> ((i16, i16), (i16, i16)) {
        if self.config.mode == MotionRedirectionMode::Disabled {
            return ((0, 0), (0, 0));
        }

        // Combine horizontal yaw and device roll
        let eff_yaw = yaw_dps + self.config.roll_factor * roll_dps;
        let eff_pitch = pitch_dps;

        // Apply deadzone threshold
        let filtered_yaw = if eff_yaw.abs() <= self.config.deadzone_dps {
            0.0
        } else {
            eff_yaw - eff_yaw.signum() * self.config.deadzone_dps
        };

        let filtered_pitch = if eff_pitch.abs() <= self.config.deadzone_dps {
            0.0
        } else {
            eff_pitch - eff_pitch.signum() * self.config.deadzone_dps
        };

        let inv_yaw_mult = if self.config.invert_yaw { -1.0 } else { 1.0 };
        let inv_pitch_mult = if self.config.invert_pitch { -1.0 } else { 1.0 };

        let scaled_yaw = filtered_yaw * self.config.yaw_sensitivity * inv_yaw_mult;
        let scaled_pitch = filtered_pitch * self.config.pitch_sensitivity * inv_pitch_mult;

        match self.config.mode {
            MotionRedirectionMode::GyroToMouse => {
                let dx = scaled_yaw.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                let dy = scaled_pitch.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                ((dx, dy), (0, 0))
            }
            MotionRedirectionMode::GyroToRightStick => {
                let sx = (scaled_yaw * self.config.stick_deflection_scale)
                    .clamp(-32768.0, 32767.0) as i16;
                let sy = (scaled_pitch * self.config.stick_deflection_scale)
                    .clamp(-32768.0, 32767.0) as i16;
                ((0, 0), (sx, sy))
            }
            MotionRedirectionMode::Disabled => ((0, 0), (0, 0)),
        }
    }
}

/// Verifies motion vector redirection to analog right stick deflections and relative mouse deltas.
pub fn verify_motion_redirection_engine() -> bool {
    let mut config = MotionRedirectionConfig::default_mouse();
    config.deadzone_dps = 2.0;
    config.yaw_sensitivity = 2.0;

    let engine_mouse = MotionRedirectionEngine::new(config);

    // 1. Angular rate within noise deadzone (1.0 dps < 2.0 dps) -> zero output
    let ((m_dx_dead, _), _) = engine_mouse.process_motion(1.0, 0.0, 0.0);
    if m_dx_dead != 0 {
        return false;
    }

    // 2. Active motion in GyroToMouse mode (12.0 dps -> (12 - 2) * 2 = 20 delta)
    let ((m_dx, m_dy), (s_x, s_y)) = engine_mouse.process_motion(12.0, 0.0, 0.0);
    if m_dx != 20 || m_dy != 0 || s_x != 0 || s_y != 0 {
        return false;
    }

    // 3. Switch to GyroToRightStick mode
    let mut config_stick = MotionRedirectionConfig::default_right_stick();
    config_stick.deadzone_dps = 2.0;
    config_stick.stick_deflection_scale = 500.0;
    let engine_stick = MotionRedirectionEngine::new(config_stick);

    let ((m2_dx, _), (s2_x, s2_y)) = engine_stick.process_motion(12.0, 0.0, 0.0);
    // (12 - 2) * 1.0 * 500 = 5000 stick units
    if m2_dx != 0 || s2_x != 5000 || s2_y != 0 {
        return false;
    }

    // 4. Inversion toggle
    let mut config_inv = config_stick;
    config_inv.invert_yaw = true;
    let engine_inv = MotionRedirectionEngine::new(config_inv);
    let (_, (s3_x, _)) = engine_inv.process_motion(12.0, 0.0, 0.0);
    if s3_x != -5000 {
        return false;
    }

    true
}
