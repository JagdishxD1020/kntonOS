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

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;

static RAW_INPUT_REGISTRY: Spinlock<Vec<RawInputDevice>> = Spinlock::new(Vec::new());
static RAW_INPUT_BUFFER: Spinlock<BTreeMap<usize, RawInput>> = Spinlock::new(BTreeMap::new());
static NEXT_RAW_INPUT_HANDLE: Spinlock<usize> = Spinlock::new(0x0005_0001);

/// Registers the devices that supply the Raw Input data.
pub unsafe fn win32_register_raw_input_devices(
    p_raw_input_devices: *const RawInputDevice,
    ui_num_devices: u32,
    cb_size: u32,
) -> bool {
    if p_raw_input_devices.is_null() || cb_size != core::mem::size_of::<RawInputDevice>() as u32 {
        return false;
    }

    let mut registry = RAW_INPUT_REGISTRY.lock();
    for i in 0..(ui_num_devices as usize) {
        let dev = &*p_raw_input_devices.add(i);

        if (dev.dw_flags & RIDEV_REMOVE) != 0 {
            if let Some(pos) = registry.iter().position(|d| {
                d.us_usage_page == dev.us_usage_page && d.us_usage == dev.us_usage
            }) {
                registry.remove(pos);
            }
        } else {
            if let Some(existing) = registry.iter_mut().find(|d| {
                d.us_usage_page == dev.us_usage_page && d.us_usage == dev.us_usage
            }) {
                *existing = *dev;
            } else {
                registry.push(*dev);
            }
        }
    }

    true
}

/// Retrieves the information about the Raw Input devices for the current process.
pub unsafe fn win32_get_registered_raw_input_devices(
    p_raw_input_devices: *mut RawInputDevice,
    pui_num_devices: *mut u32,
    cb_size: u32,
) -> u32 {
    if pui_num_devices.is_null() || cb_size != core::mem::size_of::<RawInputDevice>() as u32 {
        return 0xFFFF_FFFF;
    }

    let registry = RAW_INPUT_REGISTRY.lock();
    let total = registry.len() as u32;

    if p_raw_input_devices.is_null() {
        *pui_num_devices = total;
        return 0;
    }

    let capacity = *pui_num_devices;
    if capacity < total {
        *pui_num_devices = total;
        return 0xFFFF_FFFF;
    }

    let copy_count = capacity.min(total) as usize;
    for i in 0..copy_count {
        *p_raw_input_devices.add(i) = registry[i];
    }
    *pui_num_devices = copy_count as u32;

    copy_count as u32
}

/// Retrieves the raw input from the specified device.
pub unsafe fn win32_get_raw_input_data(
    h_raw_input: usize,
    ui_command: u32,
    p_data: *mut u8,
    pcb_size: *mut u32,
    cb_size_header: u32,
) -> u32 {
    if pcb_size.is_null() || cb_size_header != core::mem::size_of::<RawInputHeader>() as u32 {
        return 0xFFFF_FFFF;
    }

    let header_size = core::mem::size_of::<RawInputHeader>() as u32;
    let full_size = core::mem::size_of::<RawInput>() as u32;

    if p_data.is_null() {
        match ui_command {
            RID_HEADER => {
                *pcb_size = header_size;
                0
            }
            RID_INPUT => {
                *pcb_size = full_size;
                0
            }
            _ => 0xFFFF_FFFF,
        }
    } else {
        let buffer = RAW_INPUT_BUFFER.lock();
        let packet = match buffer.get(&h_raw_input) {
            Some(p) => *p,
            None => return 0xFFFF_FFFF,
        };

        match ui_command {
            RID_HEADER => {
                if *pcb_size < header_size {
                    *pcb_size = header_size;
                    return 0xFFFF_FFFF;
                }
                core::ptr::copy_nonoverlapping(
                    &packet.header as *const _ as *const u8,
                    p_data,
                    header_size as usize,
                );
                header_size
            }
            RID_INPUT => {
                if *pcb_size < full_size {
                    *pcb_size = full_size;
                    return 0xFFFF_FFFF;
                }
                core::ptr::copy_nonoverlapping(
                    &packet as *const _ as *const u8,
                    p_data,
                    full_size as usize,
                );
                full_size
            }
            _ => 0xFFFF_FFFF,
        }
    }
}

/// Dispatches a raw input packet to registered target windows via WM_INPUT.
pub fn win32_dispatch_raw_input(raw: &RawInput) -> bool {
    let (usage_page, usage) = match raw.header.dw_type {
        RIM_TYPEMOUSE => (HID_USAGE_PAGE_GENERIC, HID_USAGE_GENERIC_MOUSE),
        RIM_TYPEKEYBOARD => (HID_USAGE_PAGE_GENERIC, HID_USAGE_GENERIC_KEYBOARD),
        _ => return false,
    };

    let target_hwnd = {
        let registry = RAW_INPUT_REGISTRY.lock();
        registry.iter().find(|d| d.us_usage_page == usage_page && d.us_usage == usage).map(|d| d.hwnd_target)
    };

    let hwnd = match target_hwnd {
        Some(h) if h != 0 => h,
        _ => return false,
    };

    let handle = {
        let mut gen = NEXT_RAW_INPUT_HANDLE.lock();
        let h = *gen;
        *gen = h.saturating_add(1);
        h
    };

    let mut packet = *raw;
    packet.header.h_device = handle;

    {
        let mut buffer = RAW_INPUT_BUFFER.lock();
        buffer.insert(handle, packet);
    }

    crate::pe::user32::win32_post_message(
        hwnd,
        WM_INPUT,
        raw.header.w_param,
        handle as isize,
    )
}
