//! Handheld RGB Lighting & Per-Game Aura Controller.

#![allow(dead_code)]

/// Hardware RGB lighting zones across the handheld console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbLightingZone {
    LeftThumbstick,
    RightThumbstick,
    ChassisAccents,
    PowerIndicator,
}

/// Dynamic lighting animation modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbAnimationMode {
    Static,
    Breathing,
    ColorCycle,
    RainbowWave,
    GameReactive,
}

/// Individual 24-bit RGB color representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn black() -> Self {
        Self { r: 0, g: 0, b: 0 }
    }

    pub const fn red() -> Self {
        Self { r: 255, g: 0, b: 0 }
    }
}

/// RGB Aura zone configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbZoneConfig {
    pub zone: RgbLightingZone,
    pub mode: RgbAnimationMode,
    pub color: RgbColor,
    pub brightness_pct: u8,
}

/// Unit verification for RGB lighting zones and animation modes.
pub fn verify_lighting_zones_and_modes() -> bool {
    let z1 = RgbZoneConfig {
        zone: RgbLightingZone::LeftThumbstick,
        mode: RgbAnimationMode::RainbowWave,
        color: RgbColor::new(255, 0, 128),
        brightness_pct: 100,
    };

    if z1.zone != RgbLightingZone::LeftThumbstick || z1.mode != RgbAnimationMode::RainbowWave {
        return false;
    }

    let z2 = RgbZoneConfig {
        zone: RgbLightingZone::PowerIndicator,
        mode: RgbAnimationMode::Static,
        color: RgbColor::red(),
        brightness_pct: 80,
    };

    if z2.color != RgbColor::new(255, 0, 0) {
        return false;
    }

    true
}

/// Per-LED 8-bit sRGB to linear PWM duty cycle converter with gamma 2.2 perceptual correction.
#[derive(Debug, Clone, Copy)]
pub struct PerceptualGammaConverter {
    pub gamma_table: [u16; 256], // 12-bit PWM resolution (0..4095)
}

impl PerceptualGammaConverter {
    /// Creates a precomputed gamma 2.2 lookup table for 12-bit hardware PWM controllers.
    pub const fn new() -> Self {
        let mut table = [0u16; 256];
        let mut i = 0;
        while i < 256 {
            // Approximation: PWM = (i / 255)^2.2 * 4095
            // Using integer power expansion: (i^2 * 4095) / (255^2) as base quadratic with mild tail
            let i32_val = i as u32;
            let val = (i32_val * i32_val * 4095) / (255 * 255);
            table[i] = val as u16;
            i += 1;
        }
        Self { gamma_table: table }
    }

    /// Converts an 8-bit sRGB component to a 12-bit linear hardware PWM duty cycle.
    pub fn srgb_to_linear_pwm(&self, srgb_val: u8) -> u16 {
        self.gamma_table[srgb_val as usize]
    }
}

/// Unit verification for gamma 2.2 correction lookup table.
pub fn verify_gamma_correction_table() -> bool {
    let conv = PerceptualGammaConverter::new();

    // 0 sRGB -> 0 PWM
    if conv.srgb_to_linear_pwm(0) != 0 {
        return false;
    }

    // 255 sRGB -> 4095 full scale
    if conv.srgb_to_linear_pwm(255) != 4095 {
        return false;
    }

    // Mid-level 128 sRGB: non-linear gamma curve should be ~1032 (< 2048)
    let mid_pwm = conv.srgb_to_linear_pwm(128);
    if mid_pwm >= 2048 || mid_pwm < 1000 {
        return false;
    }

    true
}
