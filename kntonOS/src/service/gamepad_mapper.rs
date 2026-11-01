// SteamDeck Virtual Gamepad Input Device Driver and Mapper.
// Implements XInput and DirectInput dual-mode emulation and haptic event pipelines.

pub const XINPUT_GAMEPAD_DPAD_UP: u16 = 0x0001;
pub const XINPUT_GAMEPAD_DPAD_DOWN: u16 = 0x0002;
pub const XINPUT_GAMEPAD_DPAD_LEFT: u16 = 0x0004;
pub const XINPUT_GAMEPAD_DPAD_RIGHT: u16 = 0x0008;
pub const XINPUT_GAMEPAD_START: u16 = 0x0010;
pub const XINPUT_GAMEPAD_BACK: u16 = 0x0020;
pub const XINPUT_GAMEPAD_LEFT_THUMB: u16 = 0x0040;
pub const XINPUT_GAMEPAD_RIGHT_THUMB: u16 = 0x0080;
pub const XINPUT_GAMEPAD_LEFT_SHOULDER: u16 = 0x0100;
pub const XINPUT_GAMEPAD_RIGHT_SHOULDER: u16 = 0x0200;
pub const XINPUT_GAMEPAD_A: u16 = 0x1000;
pub const XINPUT_GAMEPAD_B: u16 = 0x2000;
pub const XINPUT_GAMEPAD_X: u16 = 0x4000;
pub const XINPUT_GAMEPAD_Y: u16 = 0x8000;

pub const DECK_BUTTON_L4: u32 = 0x0001_0000;
pub const DECK_BUTTON_L5: u32 = 0x0002_0000;
pub const DECK_BUTTON_R4: u32 = 0x0004_0000;
pub const DECK_BUTTON_R5: u32 = 0x0008_0000;
pub const DECK_BUTTON_QAM: u32 = 0x0010_0000;
pub const DECK_BUTTON_STEAM: u32 = 0x0020_0000;

pub const DINPUT_POV_CENTER: u32 = 0xFFFF_FFFF;

// XInput standard gamepad state report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XInputGamepadState {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
    pub packet_number: u32,
}

impl XInputGamepadState {
    // Construct zeroed XInput state snapshot.
    pub const fn new() -> Self {
        Self {
            buttons: 0,
            left_trigger: 0,
            right_trigger: 0,
            thumb_lx: 0,
            thumb_ly: 0,
            thumb_rx: 0,
            thumb_ry: 0,
            packet_number: 0,
        }
    }
}

// DirectInput extended 32-button gamepad report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectInputGamepadState {
    pub buttons: u32,
    pub axis_x: i32,
    pub axis_y: i32,
    pub axis_z: i32,
    pub axis_rx: i32,
    pub axis_ry: i32,
    pub axis_rz: i32,
    pub pov_hat: u32,
}

impl DirectInputGamepadState {
    // Construct zeroed DirectInput state snapshot.
    pub const fn new() -> Self {
        Self {
            buttons: 0,
            axis_x: 0,
            axis_y: 0,
            axis_z: 0,
            axis_rx: 0,
            axis_ry: 0,
            axis_rz: 0,
            pov_hat: DINPUT_POV_CENTER,
        }
    }
}

// Controller protocol emulation profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmulationMode {
    XInputOnly,
    DirectInputOnly,
    DualMode,
}

// Virtual gamepad input device driver handling dual-mode protocol translation.
pub struct VirtualGamepadDriver {
    pub connected: bool,
    pub emulation_mode: EmulationMode,
    pub deck_extra_buttons: u32,
    pub xinput_state: XInputGamepadState,
    pub dinput_state: DirectInputGamepadState,
    pub rumble_left_motor: u16,
    pub rumble_right_motor: u16,
}

impl VirtualGamepadDriver {
    // Construct a new virtual gamepad driver instance.
    pub const fn new(mode: EmulationMode) -> Self {
        Self {
            connected: true,
            emulation_mode: mode,
            deck_extra_buttons: 0,
            xinput_state: XInputGamepadState::new(),
            dinput_state: DirectInputGamepadState::new(),
            rumble_left_motor: 0,
            rumble_right_motor: 0,
        }
    }

    // Set connection status of the virtual controller.
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    // Check if the virtual controller is currently reported as connected.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    // Press one or more standard XInput buttons and increment packet sequence.
    pub fn press_button(&mut self, mask: u16) {
        self.xinput_state.buttons |= mask;
        self.xinput_state.packet_number = self.xinput_state.packet_number.wrapping_add(1);
        self.sync_dinput();
    }

    // Release one or more standard XInput buttons.
    pub fn release_button(&mut self, mask: u16) {
        self.xinput_state.buttons &= !mask;
        self.xinput_state.packet_number = self.xinput_state.packet_number.wrapping_add(1);
        self.sync_dinput();
    }

    // Press SteamDeck rear grip or system button.
    pub fn press_deck_button(&mut self, mask: u32) {
        self.deck_extra_buttons |= mask;
        self.sync_dinput();
    }

    // Release SteamDeck rear grip or system button.
    pub fn release_deck_button(&mut self, mask: u32) {
        self.deck_extra_buttons &= !mask;
        self.sync_dinput();
    }

    // Set left analog trigger pressure [0..255].
    pub fn set_left_trigger(&mut self, value: u8) {
        self.xinput_state.left_trigger = value;
        self.sync_dinput();
    }

    // Set right analog trigger pressure [0..255].
    pub fn set_right_trigger(&mut self, value: u8) {
        self.xinput_state.right_trigger = value;
        self.sync_dinput();
    }

    // Set left analog thumbstick deflection coordinates.
    pub fn set_left_stick(&mut self, x: i16, y: i16) {
        self.xinput_state.thumb_lx = x;
        self.xinput_state.thumb_ly = y;
        self.sync_dinput();
    }

    // Set right analog thumbstick deflection coordinates.
    pub fn set_right_stick(&mut self, x: i16, y: i16) {
        self.xinput_state.thumb_rx = x;
        self.xinput_state.thumb_ry = y;
        self.sync_dinput();
    }

    // Update force-feedback rumble vibration motor speeds.
    pub fn set_vibration(&mut self, left_speed: u16, right_speed: u16) {
        self.rumble_left_motor = left_speed;
        self.rumble_right_motor = right_speed;
    }

    // Synchronize DirectInput state representation from XInput and Deck state.
    pub fn sync_dinput(&mut self) {
        if matches!(self.emulation_mode, EmulationMode::XInputOnly) {
            return;
        }

        // Map primary buttons: A -> Btn0, B -> Btn1, X -> Btn2, Y -> Btn3, LB -> Btn4, RB -> Btn5
        let mut d_buttons = 0u32;
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_A) != 0 { d_buttons |= 1 << 0; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_B) != 0 { d_buttons |= 1 << 1; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_X) != 0 { d_buttons |= 1 << 2; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_Y) != 0 { d_buttons |= 1 << 3; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_LEFT_SHOULDER) != 0 { d_buttons |= 1 << 4; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_RIGHT_SHOULDER) != 0 { d_buttons |= 1 << 5; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_BACK) != 0 { d_buttons |= 1 << 6; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_START) != 0 { d_buttons |= 1 << 7; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_LEFT_THUMB) != 0 { d_buttons |= 1 << 8; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_RIGHT_THUMB) != 0 { d_buttons |= 1 << 9; }

        // Map SteamDeck rear grip buttons to buttons 10..13
        if (self.deck_extra_buttons & DECK_BUTTON_L4) != 0 { d_buttons |= 1 << 10; }
        if (self.deck_extra_buttons & DECK_BUTTON_R4) != 0 { d_buttons |= 1 << 11; }
        if (self.deck_extra_buttons & DECK_BUTTON_L5) != 0 { d_buttons |= 1 << 12; }
        if (self.deck_extra_buttons & DECK_BUTTON_R5) != 0 { d_buttons |= 1 << 13; }
        if (self.deck_extra_buttons & DECK_BUTTON_QAM) != 0 { d_buttons |= 1 << 14; }
        if (self.deck_extra_buttons & DECK_BUTTON_STEAM) != 0 { d_buttons |= 1 << 15; }

        self.dinput_state.buttons = d_buttons;
        self.dinput_state.axis_x = self.xinput_state.thumb_lx as i32;
        self.dinput_state.axis_y = -(self.xinput_state.thumb_ly as i32);
        self.dinput_state.axis_rx = self.xinput_state.thumb_rx as i32;
        self.dinput_state.axis_ry = -(self.xinput_state.thumb_ry as i32);
        self.dinput_state.axis_z = (self.xinput_state.left_trigger as i32) * 257 - 32768;
        self.dinput_state.axis_rz = (self.xinput_state.right_trigger as i32) * 257 - 32768;

        // Map D-pad to POV hat angles (in hundredths of degrees)
        let dpad = self.xinput_state.buttons & 0x000F;
        self.dinput_state.pov_hat = match dpad {
            XINPUT_GAMEPAD_DPAD_UP => 0,
            x if x == (XINPUT_GAMEPAD_DPAD_UP | XINPUT_GAMEPAD_DPAD_RIGHT) => 4500,
            XINPUT_GAMEPAD_DPAD_RIGHT => 9000,
            x if x == (XINPUT_GAMEPAD_DPAD_DOWN | XINPUT_GAMEPAD_DPAD_RIGHT) => 13500,
            XINPUT_GAMEPAD_DPAD_DOWN => 18000,
            x if x == (XINPUT_GAMEPAD_DPAD_DOWN | XINPUT_GAMEPAD_DPAD_LEFT) => 22500,
            XINPUT_GAMEPAD_DPAD_LEFT => 27000,
            x if x == (XINPUT_GAMEPAD_DPAD_UP | XINPUT_GAMEPAD_DPAD_LEFT) => 31500,
            _ => DINPUT_POV_CENTER,
        };
    }
}
