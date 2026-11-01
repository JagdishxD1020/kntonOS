// Handheld Game Mode UI compositor and hardware settings interface.

pub mod launcher;
pub use launcher::*;

// Handheld performance overlay metrics.
#[derive(Debug, Clone, Copy)]
pub struct QuickSettings {
    pub target_tdp_watts: u8,
    pub display_hz: u16,
    pub hud_level: u8,
    pub volume_pct: u8,
}

impl QuickSettings {
    pub const fn new() -> Self {
        Self {
            target_tdp_watts: 15, // Default 15W balanced handheld TDP
            display_hz: 60,       // Default 60Hz
            hud_level: 1,         // FPS & Battery only
            volume_pct: 70,
        }
    }

    pub fn set_tdp(&mut self, tdp: u8) {
        self.target_tdp_watts = tdp.clamp(5, 30);
    }

    pub fn set_refresh_rate(&mut self, hz: u16) {
        if hz == 40 || hz == 60 || hz == 120 {
            self.display_hz = hz;
        }
    }
}

// Verification test for game mode shell.
pub fn verify_game_mode_shell() -> bool {
    let mut qs = QuickSettings::new();
    if qs.target_tdp_watts != 15 || qs.display_hz != 60 {
        return false;
    }

    qs.set_tdp(25);
    if qs.target_tdp_watts != 25 {
        return false;
    }

    qs.set_refresh_rate(120);
    if qs.display_hz != 120 {
        return false;
    }

    true
}
