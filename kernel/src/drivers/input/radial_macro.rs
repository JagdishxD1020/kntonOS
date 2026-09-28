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


// ============================================================================
// Sub-Millisecond Virtual Radial Menu Angular Deflection Resolver
// ============================================================================

/// Number of slots for radial selection wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadialSlotCount {
    Slots8 = 8,
    Slots16 = 16,
}

/// Angular deflection resolver and virtual radial menu (weapon wheel / hotkeys).
#[derive(Debug, Clone)]
pub struct VirtualRadialMenu {
    pub slot_count: RadialSlotCount,
    pub deadzone_radius: i32,
    pub is_open: bool,
    pub active_slot: Option<u8>,
    pub slot_actions: [u16; 16],
}

impl VirtualRadialMenu {
    pub const fn new(slot_count: RadialSlotCount, deadzone_radius: i32) -> Self {
        Self {
            slot_count,
            deadzone_radius,
            is_open: false,
            active_slot: None,
            slot_actions: [0; 16],
        }
    }

    /// Assigns a virtual hotkey action code to a radial slot.
    pub fn bind_action(&mut self, slot_index: usize, action_code: u16) -> Result<(), &'static str> {
        if slot_index >= (self.slot_count as usize) {
            return Err("Slot index out of bounds");
        }
        self.slot_actions[slot_index] = action_code;
        Ok(())
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.active_slot = None;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.active_slot = None;
    }

    /// Approximates angle in degrees [0, 360) from stick/trackpad X and Y coordinates.
    pub fn compute_angle_deg(x: i32, y: i32) -> u16 {
        if x == 0 && y == 0 {
            return 0;
        }

        // Fast fixed-point rational atan2 approximation
        let abs_x = x.abs();
        let abs_y = y.abs();

        let ratio = if abs_x >= abs_y {
            (abs_y * 1024) / abs_x
        } else {
            (abs_x * 1024) / abs_y
        };

        // Linear interpolation for octant angle (0 to 45 deg, where 1024 ratio = 45 deg)
        let octant_deg = (ratio * 45) / 1024;

        let base_deg = if abs_x >= abs_y {
            octant_deg
        } else {
            90 - octant_deg
        };

        // Map to full 360 degrees based on quadrant
        let full_deg = if x >= 0 && y >= 0 {
            base_deg // Quadrant 1 (0 to 90 deg)
        } else if x < 0 && y >= 0 {
            180 - base_deg // Quadrant 2 (90 to 180 deg)
        } else if x < 0 && y < 0 {
            180 + base_deg // Quadrant 3 (180 to 270 deg)
        } else {
            360 - base_deg // Quadrant 4 (270 to 360 deg)
        };

        (full_deg % 360) as u16
    }

    /// Resolves stick/trackpad deflection into an active radial slot.
    pub fn resolve_deflection(&mut self, stick_x: i16, stick_y: i16) -> Option<u8> {
        if !self.is_open {
            return None;
        }

        let sx = stick_x as i32;
        let sy = stick_y as i32;
        let dist_sq = sx * sx + sy * sy;
        let deadzone_sq = self.deadzone_radius * self.deadzone_radius;

        if dist_sq < deadzone_sq {
            // Stick within deadzone: keep previous active slot or None
            return self.active_slot;
        }

        let angle_deg = Self::compute_angle_deg(sx, sy);
        let n_slots = self.slot_count as u16;
        let sector_width = 360 / n_slots;
        let half_sector = sector_width / 2;

        // Center slot 0 on East (0 deg) with half-sector offset
        let slot = ((angle_deg + half_sector) % 360) / sector_width;
        let slot_u8 = slot as u8;
        self.active_slot = Some(slot_u8);
        Some(slot_u8)
    }

    /// Confirms and commits the currently highlighted slot upon button release.
    pub fn confirm_selection(&mut self) -> Option<u16> {
        if let Some(slot) = self.active_slot {
            let action = self.slot_actions[slot as usize];
            self.close();
            Some(action)
        } else {
            self.close();
            None
        }
    }
}

/// Unit test verifying virtual radial menu angular resolution and slot selection.
pub fn verify_virtual_radial_menu() -> bool {
    let mut menu = VirtualRadialMenu::new(RadialSlotCount::Slots8, 8000);

    // Bind action codes (e.g. 101 to 108 for weapons)
    for i in 0..8 {
        if menu.bind_action(i, (100 + i) as u16).is_err() {
            return false;
        }
    }

    menu.open();
    if !menu.is_open || menu.active_slot.is_some() {
        return false;
    }

    // 1. Inside deadzone: returns None
    let s_deadzone = menu.resolve_deflection(3000, -2000);
    if s_deadzone.is_some() {
        return false;
    }

    // 2. East deflection (+20000, 0): angle ~0 deg -> Slot 0
    let s_east = menu.resolve_deflection(20000, 0);
    if s_east != Some(0) {
        return false;
    }

    // 3. North-East deflection (+18000, +18000): angle ~45 deg -> Slot 1
    let s_ne = menu.resolve_deflection(18000, 18000);
    if s_ne != Some(1) {
        return false;
    }

    // 4. North deflection (0, +22000): angle ~90 deg -> Slot 2
    let s_north = menu.resolve_deflection(0, 22000);
    if s_north != Some(2) {
        return false;
    }

    // 5. West deflection (-20000, 0): angle ~180 deg -> Slot 4
    let s_west = menu.resolve_deflection(-20000, 0);
    if s_west != Some(4) {
        return false;
    }

    // 6. Confirm selection: returns action code for Slot 4 (104) and closes menu
    let action = menu.confirm_selection();
    if action != Some(104) || menu.is_open {
        return false;
    }

    true
}
