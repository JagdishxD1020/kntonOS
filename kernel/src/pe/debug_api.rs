//! Win32 Debugging & Exception Instrumentation API Subsystem.
//!
//! Emulates Windows NT user-mode debugging, debug event generation,
//! cross-process memory inspection, and virtualized hardware watchpoints.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_SUCCESS,
};
pub use crate::pe::ntdll::STATUS_ACCESS_VIOLATION;
pub use crate::pe::seh_veh::EXCEPTION_RECORD;
pub use crate::pe::synch::{HANDLE, WAIT_TIMEOUT};
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

pub const DBG_CONTINUE: u32 = 0x0001_0002;
pub const DBG_EXCEPTION_NOT_HANDLED: u32 = 0x8001_0001;
pub const DBG_REPLY_LATER: u32 = 0x4001_0001;

pub const STATUS_BREAKPOINT: u32 = 0x8000_0003;
pub const STATUS_SINGLE_STEP: u32 = 0x8000_0004;

pub const EXCEPTION_DEBUG_EVENT: u32 = 1;
pub const CREATE_THREAD_DEBUG_EVENT: u32 = 2;
pub const CREATE_PROCESS_DEBUG_EVENT: u32 = 3;
pub const EXIT_THREAD_DEBUG_EVENT: u32 = 4;
pub const EXIT_PROCESS_DEBUG_EVENT: u32 = 5;
pub const LOAD_DLL_DEBUG_EVENT: u32 = 6;
pub const UNLOAD_DLL_DEBUG_EVENT: u32 = 7;
pub const OUTPUT_DEBUG_STRING_EVENT: u32 = 8;
pub const RIP_EVENT: u32 = 9;

/// Debug event payload for exception notifications.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EXCEPTION_DEBUG_INFO {
    pub ExceptionRecord: EXCEPTION_RECORD,
    pub dwFirstChance: u32,
}

/// Debug event payload for thread creation events.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CREATE_THREAD_DEBUG_INFO {
    pub hThread: HANDLE,
    pub lpThreadLocalBase: *mut c_void,
    pub lpStartAddress: *mut c_void,
}

/// Debug event payload for process creation events.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CREATE_PROCESS_DEBUG_INFO {
    pub hFile: HANDLE,
    pub hProcess: HANDLE,
    pub hThread: HANDLE,
    pub lpBaseOfImage: *mut c_void,
    pub dwDebugInfoFileOffset: u32,
    pub nDebugInfoSize: u32,
    pub lpThreadLocalBase: *mut c_void,
    pub lpStartAddress: *mut c_void,
    pub lpImageName: *mut c_void,
    pub fUnicode: u16,
}

/// Debug event payload for thread termination events.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EXIT_THREAD_DEBUG_INFO {
    pub dwExitCode: u32,
}

/// Debug event payload for process termination events.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EXIT_PROCESS_DEBUG_INFO {
    pub dwExitCode: u32,
}

/// Debug event payload for output debug string calls.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct OUTPUT_DEBUG_STRING_INFO {
    pub lpDebugStringData: *mut u8,
    pub fUnicode: u16,
    pub nDebugStringLength: u16,
}

/// Union of debug event payload structures.
#[repr(C)]
#[derive(Clone, Copy)]
pub union DEBUG_EVENT_UNION {
    pub Exception: EXCEPTION_DEBUG_INFO,
    pub CreateThread: CREATE_THREAD_DEBUG_INFO,
    pub CreateProcessInfo: CREATE_PROCESS_DEBUG_INFO,
    pub ExitThread: EXIT_THREAD_DEBUG_INFO,
    pub ExitProcess: EXIT_PROCESS_DEBUG_INFO,
    pub DebugString: OUTPUT_DEBUG_STRING_INFO,
}

impl Default for DEBUG_EVENT_UNION {
    fn default() -> Self {
        Self {
            Exception: EXCEPTION_DEBUG_INFO::default(),
        }
    }
}

/// Central Win32 debug event packet dispatched to attached debuggers.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DEBUG_EVENT {
    pub dwDebugEventCode: u32,
    pub dwProcessId: u32,
    pub dwThreadId: u32,
    pub u: DEBUG_EVENT_UNION,
}

unsafe impl Send for DEBUG_EVENT {}
unsafe impl Sync for DEBUG_EVENT {}

/// State tracking for active debugger sessions and attached targets.
pub struct DebuggerSession {
    pub debugger_pid: u32,
    pub attached_processes: BTreeMap<u32, bool>,
    pub event_queue: VecDeque<DEBUG_EVENT>,
    pub pending_continue: Option<(u32, u32)>,
}

impl DebuggerSession {
    pub fn new(debugger_pid: u32) -> Self {
        Self {
            debugger_pid,
            attached_processes: BTreeMap::new(),
            event_queue: VecDeque::new(),
            pending_continue: None,
        }
    }
}

pub static DEBUG_SESSIONS: Spinlock<BTreeMap<u32, DebuggerSession>> =
    Spinlock::new(BTreeMap::new());

/// Enqueues a debug event for the debugger attached to the target process.
pub fn post_debug_event(event: DEBUG_EVENT) -> bool {
    let mut sessions = DEBUG_SESSIONS.lock();
    for session in sessions.values_mut() {
        if session.attached_processes.contains_key(&event.dwProcessId) {
            session.event_queue.push_back(event);
            return true;
        }
    }
    false
}
