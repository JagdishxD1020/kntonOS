//! Handheld Gamepad Dynamic Button Remapping, Macro Sequencer & Virtual Radial Menu.

#![allow(dead_code)]

/// Extended digital button bitmask including back-paddles L4/L5/R4/R5.
pub const PAD_BTN_A: u32 = 1 << 0;
pub const PAD_BTN_B: u32 = 1 << 1;
pub const PAD_BTN_X: u32 = 1 << 2;
pub const PAD_BTN_Y: u32 = 1 << 3;
pub const PAD_BTN_DPAD_UP: u32 = 1 << 4;
pub const PAD_BTN_DPAD_DOWN: u32 = 1 << 5;
pub const PAD_BTN_DPAD_LEFT: u32 = 1 << 6;
pub const PAD_BTN_DPAD_RIGHT: u32 = 1 << 7;
pub const PAD_BTN_LB: u32 = 1 << 8;
pub const PAD_BTN_RB: u32 = 1 << 9;
pub const PAD_BTN_THUMB_L: u32 = 1 << 10;
pub const PAD_BTN_THUMB_R: u32 = 1 << 11;
pub const PAD_BTN_MENU: u32 = 1 << 12;
pub const PAD_BTN_VIEW: u32 = 1 << 13;
pub const PAD_BTN_L4: u32 = 1 << 16;
pub const PAD_BTN_L5: u32 = 1 << 17;
pub const PAD_BTN_R4: u32 = 1 << 18;
pub const PAD_BTN_R5: u32 = 1 << 19;

/// Turbo frequency options in Hertz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurboFrequency {
    Hz5 = 5,
    Hz10 = 10,
    Hz20 = 20,
}

impl TurboFrequency {
    /// Half-period duration in microseconds for 50% duty cycle.
    pub const fn half_period_us(&self) -> u32 {
        match self {
            TurboFrequency::Hz5 => 100_000, // 100ms ON / 100ms OFF
            TurboFrequency::Hz10 => 50_000, // 50ms ON / 50ms OFF
            TurboFrequency::Hz20 => 25_000, // 25ms ON / 25ms OFF
        }
    }
}

/// Dynamic button remapping entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemapEntry {
    /// Source physical button bitmask.
    pub source_button: u32,
    /// Destination mapped button bitmask.
    pub target_button: u32,
    /// Whether turbo oscillation is active on this mapped target.
    pub turbo_enabled: bool,
}

/// Dynamic Button & Back-Paddle Remapper with Turbo Generator.
#[derive(Debug, Clone)]
pub struct ButtonRemapper {
    pub mappings: [RemapEntry; 16],
    pub mapping_count: usize,
    pub turbo_frequency: TurboFrequency,
    pub turbo_oscillator_on: bool,
    pub last_turbo_toggle_us: u64,
}

impl ButtonRemapper {
    pub const fn new() -> Self {
        Self {
            mappings: [RemapEntry { source_button: 0, target_button: 0, turbo_enabled: false }; 16],
            mapping_count: 0,
            turbo_frequency: TurboFrequency::Hz10,
            turbo_oscillator_on: true,
            last_turbo_toggle_us: 0,
        }
    }

    /// Configures a single button or back-paddle remapping rule.
    pub fn add_mapping(&mut self, source_button: u32, target_button: u32, turbo_enabled: bool) -> Result<(), &'static str> {
        if self.mapping_count >= self.mappings.len() {
            return Err("Mapping table full");
        }
        // Update existing if present
        for i in 0..self.mapping_count {
            if self.mappings[i].source_button == source_button {
                self.mappings[i].target_button = target_button;
                self.mappings[i].turbo_enabled = turbo_enabled;
                return Ok(());
            }
        }
        self.mappings[self.mapping_count] = RemapEntry { source_button, target_button, turbo_enabled };
        self.mapping_count += 1;
        Ok(())
    }

    /// Sets turbo toggle frequency.
    pub fn set_turbo_frequency(&mut self, freq: TurboFrequency) {
        self.turbo_frequency = freq;
    }

    /// Applies remappings and turbo toggles to raw hardware button state.
    pub fn process_buttons(&mut self, raw_buttons: u32, timestamp_us: u64) -> u32 {
        // Advance turbo oscillator state
        let half_period = self.turbo_frequency.half_period_us() as u64;
        if timestamp_us >= self.last_turbo_toggle_us + half_period {
            self.turbo_oscillator_on = !self.turbo_oscillator_on;
            self.last_turbo_toggle_us = timestamp_us;
        }

        let mut output_buttons = 0u32;
        let mut remapped_sources = 0u32;

        for i in 0..self.mapping_count {
            let entry = &self.mappings[i];
            if (raw_buttons & entry.source_button) != 0 {
                remapped_sources |= entry.source_button;
                if entry.turbo_enabled {
                    if self.turbo_oscillator_on {
                        output_buttons |= entry.target_button;
                    }
                } else {
                    output_buttons |= entry.target_button;
                }
            }
        }

        // Pass-through any unmapped buttons directly
        output_buttons |= raw_buttons & !remapped_sources;
        output_buttons
    }
}

/// Unit test verifying dynamic button remapping and turbo pulse generation.
pub fn verify_button_remapper_and_turbo() -> bool {
    let mut remapper = ButtonRemapper::new();

    // 1. Map Back Paddle L4 -> PAD_BTN_A, and Back Paddle R4 -> PAD_BTN_X with 10Hz Turbo
    if remapper.add_mapping(PAD_BTN_L4, PAD_BTN_A, false).is_err() {
        return false;
    }
    if remapper.add_mapping(PAD_BTN_R4, PAD_BTN_X, true).is_err() {
        return false;
    }

    // 2. Press L4: outputs PAD_BTN_A
    let out_l4 = remapper.process_buttons(PAD_BTN_L4, 1000);
    if out_l4 != PAD_BTN_A {
        return false;
    }

    // 3. Press R4 (turbo on): at t=1000, oscillator is ON -> outputs PAD_BTN_X
    let out_r4_on = remapper.process_buttons(PAD_BTN_R4, 1000);
    if out_r4_on != PAD_BTN_X {
        return false;
    }

    // 4. Advance time past 50ms (50,000us) for 10Hz half-period -> oscillator toggles OFF
    let out_r4_off = remapper.process_buttons(PAD_BTN_R4, 52_000);
    if out_r4_off != 0 {
        return false;
    }

    // 5. Advance another 50ms (102,000us) -> oscillator toggles back ON
    let out_r4_on2 = remapper.process_buttons(PAD_BTN_R4, 104_000);
    if out_r4_on2 != PAD_BTN_X {
        return false;
    }

    // 6. Unmapped button pass-through: press PAD_BTN_B alongside L4
    let out_combined = remapper.process_buttons(PAD_BTN_L4 | PAD_BTN_B, 104_000);
    if out_combined != (PAD_BTN_A | PAD_BTN_B) {
        return false;
    }

    true
}
