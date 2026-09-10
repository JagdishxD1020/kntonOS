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
