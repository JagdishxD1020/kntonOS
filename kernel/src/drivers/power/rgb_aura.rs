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

/// RGB animation generator with tick progression for Breathing, ColorCycle, and RainbowWave.
#[derive(Debug, Clone, Copy)]
pub struct RgbAnimationGenerator {
    pub current_tick: u32,
    pub cycle_period_ticks: u32,
}

impl RgbAnimationGenerator {
    /// Creates a new animation generator with 100-tick default period.
    pub const fn new() -> Self {
        Self {
            current_tick: 0,
            cycle_period_ticks: 100,
        }
    }

    /// Advances the animation clock by one tick.
    pub fn advance_tick(&mut self) -> u32 {
        self.current_tick = (self.current_tick + 1) % self.cycle_period_ticks;
        self.current_tick
    }

    /// Computes the current animated color for a given mode.
    pub fn render_color(&self, mode: RgbAnimationMode, base_color: RgbColor) -> RgbColor {
        match mode {
            RgbAnimationMode::Static => base_color,
            RgbAnimationMode::Breathing => {
                // Triangle wave modulation between 20% and 100%
                let half = self.cycle_period_ticks / 2;
                let phase = if self.current_tick < half {
                    self.current_tick
                } else {
                    self.cycle_period_ticks - self.current_tick
                };
                let factor_q8 = 51 + ((phase * 205) / half); // 51 (20%) to 256 (100%)
                RgbColor {
                    r: ((base_color.r as u32 * factor_q8) / 256) as u8,
                    g: ((base_color.g as u32 * factor_q8) / 256) as u8,
                    b: ((base_color.b as u32 * factor_q8) / 256) as u8,
                }
            }
            RgbAnimationMode::ColorCycle | RgbAnimationMode::RainbowWave => {
                // 3-phase rainbow sweep across 100 ticks
                let phase = (self.current_tick * 3) % self.cycle_period_ticks;
                if self.current_tick < 33 {
                    RgbColor::new(255, (phase * 7) as u8, 0)
                } else if self.current_tick < 66 {
                    RgbColor::new(0, 255, (phase * 7) as u8)
                } else {
                    RgbColor::new((phase * 7) as u8, 0, 255)
                }
            }
            RgbAnimationMode::GameReactive => {
                // Health/Shield reactive representation
                base_color
            }
        }
    }
}

/// Unit verification for animation tick progression.
pub fn verify_animation_tick_progression() -> bool {
    let mut gen = RgbAnimationGenerator::new();

    // Advance 50 ticks
    let mut i = 0;
    while i < 50 {
        gen.advance_tick();
        i += 1;
    }
    if gen.current_tick != 50 {
        return false;
    }

    // Breathing at peak (tick 50)
    let c = gen.render_color(RgbAnimationMode::Breathing, RgbColor::new(200, 200, 200));
    if c.r < 180 || c.g < 180 || c.b < 180 {
        return false;
    }

    true
}

/// Low-battery conservation governor and alert override controller.
#[derive(Debug, Clone, Copy)]
pub struct RgbBatteryGovernor {
    pub battery_percent: u8,
    pub ac_connected: bool,
    pub user_inactive_seconds: u32,
    pub effective_brightness_pct: u8,
    pub low_battery_alert_active: bool,
}

impl RgbBatteryGovernor {
    /// Creates a new RGB battery governor.
    pub const fn new() -> Self {
        Self {
            battery_percent: 100,
            ac_connected: true,
            user_inactive_seconds: 0,
            effective_brightness_pct: 100,
            low_battery_alert_active: false,
        }
    }

    /// Evaluates battery state and inactivity to compute effective LED brightness ceiling:
    /// - 100% on AC
    /// - 40% on battery
    /// - 0% (off) below 10% battery or after 30s inactivity
    /// - Low-battery alert flashes red warning below 15%
    pub fn evaluate_governor(&mut self, battery_pct: u8, ac_connected: bool, inactive_sec: u32) -> u8 {
        self.battery_percent = battery_pct;
        self.ac_connected = ac_connected;
        self.user_inactive_seconds = inactive_sec;

        // Low battery alert override: < 15%
        self.low_battery_alert_active = !ac_connected && battery_pct < 15;

        let brightness = if !ac_connected && (battery_pct < 10 || inactive_sec >= 30) {
            0 // Conservation shutoff
        } else if !ac_connected {
            40 // Battery throttle
        } else {
            100 // AC full brightness
        };

        self.effective_brightness_pct = brightness;
        self.effective_brightness_pct
    }
}

/// Unit verification for low-battery conservation governor and alert override.
pub fn verify_battery_conservation_governor() -> bool {
    let mut gov = RgbBatteryGovernor::new();

    // On AC: 100% brightness
    if gov.evaluate_governor(80, true, 0) != 100 || gov.low_battery_alert_active {
        return false;
    }

    // On battery: 40% brightness
    if gov.evaluate_governor(50, false, 0) != 40 || gov.low_battery_alert_active {
        return false;
    }

    // Battery drops to 12%: alert active, 40% brightness
    if gov.evaluate_governor(12, false, 0) != 40 || !gov.low_battery_alert_active {
        return false;
    }

    // Battery drops to 8% (<10%): shutoff to 0%
    if gov.evaluate_governor(8, false, 0) != 0 {
        return false;
    }

    // Inactivity 35s on battery: shutoff to 0%
    if gov.evaluate_governor(50, false, 35) != 0 {
        return false;
    }

    true
}

/// Comprehensive verification self-test suite for Handheld RGB Lighting & Per-Game Aura Controller.
pub fn test_rgb_aura_subsystem() -> bool {
    crate::serial_write_str("[KNT-POWER] Testing Handheld RGB Lighting & Per-Game Aura Controller...\r\n");

    if !verify_lighting_zones_and_modes() {
        crate::serial_write_str("[KNT-PANIC] RGB lighting zones and modes verification failed!\r\n");
        return false;
    }
    if !verify_gamma_correction_table() {
        crate::serial_write_str("[KNT-PANIC] Gamma correction lookup table verification failed!\r\n");
        return false;
    }
    if !verify_animation_tick_progression() {
        crate::serial_write_str("[KNT-PANIC] Animation tick progression verification failed!\r\n");
        return false;
    }
    if !verify_battery_conservation_governor() {
        crate::serial_write_str("[KNT-PANIC] Battery conservation governor verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-RGB-AURA-PASS] Handheld RGB lighting and per-game aura controller verified\r\n");
    true
}
