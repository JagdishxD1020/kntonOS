//! Win32 User32 & GDI32 Window Message Loop and Graphics Emulation Layer.
//! Implements Win32 window classes, window management, message loops, and GDI device contexts.

use alloc::string::String;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;

pub type HWND = usize;
pub type HDC = usize;
pub type HBITMAP = usize;
pub type HBRUSH = usize;
pub type HCURSOR = usize;
pub type HICON = usize;
pub type HINSTANCE = usize;
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;
pub type WNDPROC = Option<unsafe extern "C" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;

pub const HWND_INVALID: HWND = 0;
pub const HWND_DESKTOP: HWND = 0x0001_0000;
pub const HWND_TOP: HWND = 0;

pub const WS_OVERLAPPED: u32 = 0x0000_0000;
pub const WS_POPUP: u32 = 0x8000_0000;
pub const WS_CHILD: u32 = 0x4000_0000;
pub const WS_VISIBLE: u32 = 0x1000_0000;
pub const WS_DISABLED: u32 = 0x0800_0000;
pub const WS_CAPTION: u32 = 0x00C0_0000;
pub const WS_BORDER: u32 = 0x0080_0000;
pub const WS_SYSMENU: u32 = 0x0008_0000;
pub const WS_THICKFRAME: u32 = 0x0004_0000;
pub const WS_MINIMIZEBOX: u32 = 0x0002_0000;
pub const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
pub const WS_OVERLAPPEDWINDOW: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

pub const WM_NULL: u32 = 0x0000;
pub const WM_CREATE: u32 = 0x0001;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_MOVE: u32 = 0x0003;
pub const WM_SIZE: u32 = 0x0005;
pub const WM_ACTIVATE: u32 = 0x0006;
pub const WM_PAINT: u32 = 0x000F;
pub const WM_CLOSE: u32 = 0x0010;
pub const WM_QUIT: u32 = 0x0012;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;

pub const SW_HIDE: i32 = 0;
pub const SW_SHOWNORMAL: i32 = 1;
pub const SW_SHOW: i32 = 5;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Msg {
    pub hwnd: HWND,
    pub message: u32,
    pub wparam: WPARAM,
    pub lparam: LPARAM,
    pub time: u32,
    pub pt: Point,
}

impl Default for Msg {
    fn default() -> Self {
        Self {
            hwnd: HWND_INVALID,
            message: WM_NULL,
            wparam: 0,
            lparam: 0,
            time: 0,
            pt: Point::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WndClassEx {
    pub cb_size: u32,
    pub style: u32,
    pub lpfn_wnd_proc: WNDPROC,
    pub cb_cls_extra: i32,
    pub cb_wnd_extra: i32,
    pub h_instance: HINSTANCE,
    pub h_icon: HICON,
    pub h_cursor: HCURSOR,
    pub hbr_background: HBRUSH,
    pub lpsz_menu_name: *const u8,
    pub lpsz_class_name: *const u8,
    pub h_icon_sm: HICON,
}

pub struct RegisteredClass {
    pub name: String,
    pub wnd_proc: WNDPROC,
    pub style: u32,
    pub h_instance: HINSTANCE,
    pub hbr_background: HBRUSH,
}

pub struct WindowState {
    pub hwnd: HWND,
    pub class_name: String,
    pub title: String,
    pub style: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub wnd_proc: WNDPROC,
    pub h_instance: HINSTANCE,
    pub is_visible: bool,
    pub client_rect: Rect,
    pub surface_buffer: Vec<u32>,
}

static NEXT_HWND: Spinlock<HWND> = Spinlock::new(0x0002_0001);
static CLASS_TABLE: Spinlock<Vec<RegisteredClass>> = Spinlock::new(Vec::new());
static WINDOW_TABLE: Spinlock<Vec<WindowState>> = Spinlock::new(Vec::new());

/// Registers a window class for subsequent use in CreateWindowEx calls.
pub unsafe fn win32_register_class_ex(lp_wcex: *const WndClassEx) -> u16 {
    if lp_wcex.is_null() {
        return 0;
    }
    let wcex = &*lp_wcex;
    if wcex.lpsz_class_name.is_null() {
        return 0;
    }

    let mut len = 0;
    while *wcex.lpsz_class_name.add(len) != 0 && len < 256 {
        len += 1;
    }
    let name_bytes = core::slice::from_raw_parts(wcex.lpsz_class_name, len);
    let class_name = match core::str::from_utf8(name_bytes) {
        Ok(s) => String::from(s),
        Err(_) => return 0,
    };

    let mut classes = CLASS_TABLE.lock();
    for (idx, c) in classes.iter().enumerate() {
        if c.name == class_name {
            return (idx + 1) as u16;
        }
    }

    classes.push(RegisteredClass {
        name: class_name,
        wnd_proc: wcex.lpfn_wnd_proc,
        style: wcex.style,
        h_instance: wcex.h_instance,
        hbr_background: wcex.hbr_background,
    });

    classes.len() as u16
}

/// Creates an overlapped, pop-up, or child window with an extended window style.
pub fn win32_create_window_ex(
    _ex_style: u32,
    class_name: &str,
    window_name: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    _hwnd_parent: HWND,
    _h_menu: usize,
    h_instance: HINSTANCE,
    _lp_param: usize,
) -> HWND {
    let classes = CLASS_TABLE.lock();
    let class_info = match classes.iter().find(|c| c.name == class_name) {
        Some(c) => (c.wnd_proc, c.h_instance),
        None => return HWND_INVALID,
    };
    drop(classes);

    let mut hwnd_gen = NEXT_HWND.lock();
    let hwnd = *hwnd_gen;
    *hwnd_gen = hwnd.saturating_add(1);
    drop(hwnd_gen);

    let w = if width > 0 { width } else { 640 };
    let h = if height > 0 { height } else { 480 };
    let buffer_size = (w as usize) * (h as usize);
    let mut surface = Vec::with_capacity(buffer_size);
    surface.resize(buffer_size, 0x0020_2020);

    let client_rect = Rect {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };

    let state = WindowState {
        hwnd,
        class_name: String::from(class_name),
        title: String::from(window_name),
        style,
        x,
        y,
        width: w,
        height: h,
        wnd_proc: class_info.0,
        h_instance: if h_instance != 0 { h_instance } else { class_info.1 },
        is_visible: (style & WS_VISIBLE) != 0,
        client_rect,
        surface_buffer: surface,
    };

    let mut windows = WINDOW_TABLE.lock();
    windows.push(state);
    drop(windows);

    hwnd
}

/// Destroys the specified window handle.
pub fn win32_destroy_window(hwnd: HWND) -> bool {
    let mut windows = WINDOW_TABLE.lock();
    if let Some(pos) = windows.iter().position(|w| w.hwnd == hwnd) {
        windows.remove(pos);
        true
    } else {
        false
    }
}

/// Determines whether the specified window handle identifies an existing window.
pub fn win32_is_window(hwnd: HWND) -> bool {
    let windows = WINDOW_TABLE.lock();
    windows.iter().any(|w| w.hwnd == hwnd)
}

/// Retrieves the dimensions of the bounding rectangle of the specified window.
pub fn win32_get_window_rect(hwnd: HWND, lp_rect: *mut Rect) -> bool {
    if lp_rect.is_null() {
        return false;
    }
    let windows = WINDOW_TABLE.lock();
    if let Some(win) = windows.iter().find(|w| w.hwnd == hwnd) {
        unsafe {
            *lp_rect = Rect {
                left: win.x,
                top: win.y,
                right: win.x + win.width,
                bottom: win.y + win.height,
            };
        }
        true
    } else {
        false
    }
}

/// Retrieves the coordinates of a window's client area.
pub fn win32_get_client_rect(hwnd: HWND, lp_rect: *mut Rect) -> bool {
    if lp_rect.is_null() {
        return false;
    }
    let windows = WINDOW_TABLE.lock();
    if let Some(win) = windows.iter().find(|w| w.hwnd == hwnd) {
        unsafe {
            *lp_rect = win.client_rect;
        }
        true
    } else {
        false
    }
}

/// Sets the specified window's show state.
pub fn win32_show_window(hwnd: HWND, cmd_show: i32) -> bool {
    let mut windows = WINDOW_TABLE.lock();
    if let Some(win) = windows.iter_mut().find(|w| w.hwnd == hwnd) {
        let prev = win.is_visible;
        win.is_visible = cmd_show != SW_HIDE;
        prev
    } else {
        false
    }
}
