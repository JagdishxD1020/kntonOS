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

pub const PM_NOREMOVE: u32 = 0x0000;
pub const PM_REMOVE: u32 = 0x0001;
pub const PM_NOYIELD: u32 = 0x0002;

pub struct MessageQueue {
    pub messages: Vec<Msg>,
    pub quit_posted: bool,
    pub exit_code: i32,
}

static MESSAGE_QUEUE: Spinlock<MessageQueue> = Spinlock::new(MessageQueue {
    messages: Vec::new(),
    quit_posted: false,
    exit_code: 0,
});

/// Places a message in the message queue associated with the thread that created the window.
pub fn win32_post_message(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    let msg = Msg {
        hwnd,
        message,
        wparam,
        lparam,
        time: 0,
        pt: Point::default(),
    };

    let mut q = MESSAGE_QUEUE.lock();
    if message == WM_QUIT {
        q.quit_posted = true;
        q.exit_code = wparam as i32;
    }
    q.messages.push(msg);
    true
}

/// Indicates to the system that a thread has made a request to terminate.
pub fn win32_post_quit_message(exit_code: i32) {
    let mut q = MESSAGE_QUEUE.lock();
    q.quit_posted = true;
    q.exit_code = exit_code;
    q.messages.push(Msg {
        hwnd: HWND_INVALID,
        message: WM_QUIT,
        wparam: exit_code as usize,
        lparam: 0,
        time: 0,
        pt: Point::default(),
    });
}

/// Dispatches incoming sent messages and checks the thread message queue for a posted message.
pub fn win32_peek_message(
    lp_msg: *mut Msg,
    hwnd: HWND,
    msg_filter_min: u32,
    msg_filter_max: u32,
    remove_msg: u32,
) -> bool {
    if lp_msg.is_null() {
        return false;
    }

    let mut q = MESSAGE_QUEUE.lock();
    let mut found_idx = None;

    for (idx, m) in q.messages.iter().enumerate() {
        if hwnd != HWND_INVALID && m.hwnd != hwnd {
            continue;
        }
        if msg_filter_min != 0 || msg_filter_max != 0 {
            if m.message < msg_filter_min || m.message > msg_filter_max {
                continue;
            }
        }
        found_idx = Some(idx);
        break;
    }

    if let Some(idx) = found_idx {
        let msg = if (remove_msg & PM_REMOVE) != 0 {
            q.messages.remove(idx)
        } else {
            q.messages[idx]
        };
        unsafe {
            *lp_msg = msg;
        }
        true
    } else if q.quit_posted {
        let quit_msg = Msg {
            hwnd: HWND_INVALID,
            message: WM_QUIT,
            wparam: q.exit_code as usize,
            lparam: 0,
            time: 0,
            pt: Point::default(),
        };
        if (remove_msg & PM_REMOVE) != 0 {
            q.quit_posted = false;
        }
        unsafe {
            *lp_msg = quit_msg;
        }
        true
    } else {
        false
    }
}

/// Retrieves a message from the calling thread's message queue.
pub fn win32_get_message(
    lp_msg: *mut Msg,
    hwnd: HWND,
    msg_filter_min: u32,
    msg_filter_max: u32,
) -> i32 {
    if lp_msg.is_null() {
        return -1;
    }

    if win32_peek_message(lp_msg, hwnd, msg_filter_min, msg_filter_max, PM_REMOVE) {
        let msg = unsafe { &*lp_msg };
        if msg.message == WM_QUIT {
            0
        } else {
            1
        }
    } else {
        -1
    }
}

/// Translates virtual-key messages into character messages.
pub fn win32_translate_message(lp_msg: *const Msg) -> bool {
    if lp_msg.is_null() {
        return false;
    }

    let msg = unsafe { &*lp_msg };
    if msg.message == WM_KEYDOWN {
        let vk = msg.wparam as u8;
        if vk.is_ascii_graphic() || vk == b' ' {
            win32_post_message(msg.hwnd, WM_CHAR, vk as usize, msg.lparam);
            return true;
        }
    }
    false
}

/// Dispatches a message to a window procedure.
pub fn win32_dispatch_message(lp_msg: *const Msg) -> LRESULT {
    if lp_msg.is_null() {
        return 0;
    }

    let msg = unsafe { *lp_msg };
    if msg.hwnd != HWND_INVALID {
        let wnd_proc = {
            let windows = WINDOW_TABLE.lock();
            windows.iter().find(|w| w.hwnd == msg.hwnd).and_then(|w| w.wnd_proc)
        };

        if let Some(proc) = wnd_proc {
            unsafe { proc(msg.hwnd, msg.message, msg.wparam, msg.lparam) }
        } else {
            win32_def_window_proc(msg.hwnd, msg.message, msg.wparam, msg.lparam)
        }
    } else {
        win32_def_window_proc(msg.hwnd, msg.message, msg.wparam, msg.lparam)
    }
}

/// Provides default processing for any window messages that an application does not process.
pub extern "C" fn win32_def_window_proc(
    hwnd: HWND,
    msg: u32,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLOSE => {
            win32_destroy_window(hwnd);
            0
        }
        WM_DESTROY => {
            win32_post_quit_message(0);
            0
        }
        _ => 0,
    }
}

/// Sends the specified message to a window or windows synchronously.
pub fn win32_send_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let wnd_proc = {
        let windows = WINDOW_TABLE.lock();
        windows.iter().find(|w| w.hwnd == hwnd).and_then(|w| w.wnd_proc)
    };

    if let Some(proc) = wnd_proc {
        unsafe { proc(hwnd, msg, wparam, lparam) }
    } else {
        win32_def_window_proc(hwnd, msg, wparam, lparam)
    }
}
