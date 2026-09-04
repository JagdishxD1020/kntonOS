//! Win32 Raw Input (user32.dll) High-DPI Mouse & Keyboard Gaming Input Engine.
//! Emulates Windows Raw Input device registration, WM_INPUT event streams, and coordinate accumulation.

use crate::pe::user32::HWND;

pub const HID_USAGE_PAGE_GENERIC: u16 = 0x01;
pub const HID_USAGE_GENERIC_MOUSE: u16 = 0x02;
pub const HID_USAGE_GENERIC_JOYSTICK: u16 = 0x04;
pub const HID_USAGE_GENERIC_GAMEPAD: u16 = 0x05;
pub const HID_USAGE_GENERIC_KEYBOARD: u16 = 0x06;

pub const RIDEV_REMOVE: u32 = 0x0000_0001;
pub const RIDEV_EXCLUDE: u32 = 0x0000_0010;
pub const RIDEV_PAGEONLY: u32 = 0x0000_0020;
pub const RIDEV_NOLEGACY: u32 = 0x0000_0030;
pub const RIDEV_INPUTSINK: u32 = 0x0000_0100;
pub const RIDEV_CAPTUREMOUSE: u32 = 0x0000_0200;
pub const RIDEV_NOHOTKEYS: u32 = 0x0000_0200;
pub const RIDEV_APPKEYS: u32 = 0x0000_0400;
pub const RIDEV_EXINPUTSINK: u32 = 0x0000_1000;
pub const RIDEV_DEVNOTIFY: u32 = 0x0000_2000;

pub const RIM_TYPEMOUSE: u32 = 0;
pub const RIM_TYPEKEYBOARD: u32 = 1;
pub const RIM_TYPEHID: u32 = 2;

pub const RID_HEADER: u32 = 0x1000_0005;
pub const RID_INPUT: u32 = 0x1000_0003;

pub const RIM_INPUT: usize = 0;
pub const RIM_INPUTSINK: usize = 1;

pub const RI_MOUSE_LEFT_BUTTON_DOWN: u16 = 0x0001;
pub const RI_MOUSE_LEFT_BUTTON_UP: u16 = 0x0002;
pub const RI_MOUSE_RIGHT_BUTTON_DOWN: u16 = 0x0004;
pub const RI_MOUSE_RIGHT_BUTTON_UP: u16 = 0x0008;
pub const RI_MOUSE_MIDDLE_BUTTON_DOWN: u16 = 0x0010;
pub const RI_MOUSE_MIDDLE_BUTTON_UP: u16 = 0x0020;
pub const RI_MOUSE_BUTTON_4_DOWN: u16 = 0x0040;
pub const RI_MOUSE_BUTTON_4_UP: u16 = 0x0080;
pub const RI_MOUSE_BUTTON_5_DOWN: u16 = 0x0100;
pub const RI_MOUSE_BUTTON_5_UP: u16 = 0x0200;
pub const RI_MOUSE_WHEEL: u16 = 0x0400;
pub const RI_MOUSE_HWHEEL: u16 = 0x0800;

pub const MOUSE_MOVE_RELATIVE: u16 = 0x00;
pub const MOUSE_MOVE_ABSOLUTE: u16 = 0x01;
pub const MOUSE_VIRTUAL_DESKTOP: u16 = 0x02;
pub const MOUSE_ATTRIBUTES_CHANGED: u16 = 0x04;
pub const MOUSE_MOVE_NOCOALESCE: u16 = 0x08;

pub const RI_KEY_MAKE: u16 = 0;
pub const RI_KEY_BREAK: u16 = 1;
pub const RI_KEY_E0: u16 = 2;
pub const RI_KEY_E1: u16 = 4;

pub const WM_INPUT: u32 = 0x00FF;
pub const WM_INPUT_DEVICE_CHANGE: u32 = 0x00FE;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawInputDevice {
    pub us_usage_page: u16,
    pub us_usage: u16,
    pub dw_flags: u32,
    pub hwnd_target: HWND,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawInputHeader {
    pub dw_type: u32,
    pub dw_size: u32,
    pub h_device: usize,
    pub w_param: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawMouse {
    pub us_flags: u16,
    pub us_button_flags: u16,
    pub us_button_data: u16,
    pub ul_raw_buttons: u32,
    pub l_last_x: i32,
    pub l_last_y: i32,
    pub ul_extra_information: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawKeyboard {
    pub make_code: u16,
    pub flags: u16,
    pub reserved: u16,
    pub v_key: u16,
    pub message: u32,
    pub extra_information: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawHid {
    pub dw_size_hid: u32,
    pub dw_count: u32,
    pub b_raw_data: [u8; 1],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union RawInputData {
    pub mouse: RawMouse,
    pub keyboard: RawKeyboard,
    pub hid: RawHid,
}

impl Default for RawInputData {
    fn default() -> Self {
        Self {
            mouse: RawMouse::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawInput {
    pub header: RawInputHeader,
    pub data: RawInputData,
}

impl Default for RawInput {
    fn default() -> Self {
        Self {
            header: RawInputHeader {
                dw_type: RIM_TYPEMOUSE,
                dw_size: core::mem::size_of::<RawInput>() as u32,
                h_device: 0,
                w_param: RIM_INPUT,
            },
            data: RawInputData::default(),
        }
    }
}
