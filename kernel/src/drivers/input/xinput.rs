//! Userland Game Controller and Win32 XInput Runtime Service.

#![allow(non_snake_case)]

use core::mem::size_of;

// Standard Win32 XInput button bitmasks.
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

// Standard Win32 XInput deadzone and trigger thresholds.
pub const XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE: i16 = 7849;
pub const XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE: i16 = 8689;
pub const XINPUT_GAMEPAD_TRIGGER_THRESHOLD: u8 = 30;

// Standard Win32 error codes for XInput APIs.
pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_DEVICE_NOT_CONNECTED: u32 = 1167;
pub const ERROR_EMPTY: u32 = 4306;

// Device types and subtypes.
pub const XINPUT_DEVTYPE_GAMEPAD: u8 = 0x01;
pub const XINPUT_DEVSUBTYPE_GAMEPAD: u8 = 0x01;
pub const XINPUT_FLAG_GAMEPAD: u16 = 0x0001;

// Battery types and levels.
pub const BATTERY_TYPE_DISCONNECTED: u8 = 0x00;
pub const BATTERY_TYPE_WIRED: u8 = 0x01;
pub const BATTERY_TYPE_ALKALINE: u8 = 0x02;
pub const BATTERY_TYPE_NIMH: u8 = 0x03;
pub const BATTERY_LEVEL_EMPTY: u8 = 0x00;
pub const BATTERY_LEVEL_LOW: u8 = 0x01;
pub const BATTERY_LEVEL_MEDIUM: u8 = 0x02;
pub const BATTERY_LEVEL_FULL: u8 = 0x03;

/// Describes current state of standard Win32 XInput controller buttons, triggers, and thumbsticks.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputGamepad {
    pub wButtons: u16,
    pub bLeftTrigger: u8,
    pub bRightTrigger: u8,
    pub sThumbLX: i16,
    pub sThumbLY: i16,
    pub sThumbRX: i16,
    pub sThumbRY: i16,
}

impl XInputGamepad {
    // Tests whether a specific button bitflag is set.
    #[inline]
    pub const fn has_button(&self, mask: u16) -> bool {
        (self.wButtons & mask) != 0
    }

    // Sets or clears a specific button bitflag.
    #[inline]
    pub fn set_button(&mut self, mask: u16, pressed: bool) {
        if pressed {
            self.wButtons |= mask;
        } else {
            self.wButtons &= !mask;
        }
    }
}

/// Represents state packet of XInput controller with monotonic sequence counter.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputState {
    pub dwPacketNumber: u32,
    pub Gamepad: XInputGamepad,
}

/// Vibration speeds for controller dual rumble motors.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputVibration {
    pub wLeftMotorSpeed: u16,
    pub wRightMotorSpeed: u16,
}

/// Capabilities descriptor for connected XInput device.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputCapabilities {
    pub Type: u8,
    pub SubType: u8,
    pub Flags: u16,
    pub Gamepad: XInputGamepad,
    pub Vibration: XInputVibration,
}

/// Battery status report for wireless or wired controller.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputBatteryInformation {
    pub BatteryType: u8,
    pub BatteryLevel: u8,
}

/// Verifies memory layout, field alignments, and structure sizes for standard Win32 XInput compatibility.
pub fn verify_xinput_structures() -> bool {
    if size_of::<XInputGamepad>() != 12 {
        return false;
    }
    if size_of::<XInputState>() != 16 {
        return false;
    }
    if size_of::<XInputVibration>() != 4 {
        return false;
    }
    if size_of::<XInputBatteryInformation>() != 2 {
        return false;
    }

    let mut pad = XInputGamepad::default();
    pad.set_button(XINPUT_GAMEPAD_A | XINPUT_GAMEPAD_DPAD_UP, true);
    if !pad.has_button(XINPUT_GAMEPAD_A) || !pad.has_button(XINPUT_GAMEPAD_DPAD_UP) {
        return false;
    }
    if pad.has_button(XINPUT_GAMEPAD_B) {
        return false;
    }
    pad.set_button(XINPUT_GAMEPAD_A, false);
    if pad.has_button(XINPUT_GAMEPAD_A) || !pad.has_button(XINPUT_GAMEPAD_DPAD_UP) {
        return false;
    }

    true
}
