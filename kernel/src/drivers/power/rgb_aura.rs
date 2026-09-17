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
