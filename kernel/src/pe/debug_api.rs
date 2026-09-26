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

/// Helper to get current execution process ID.
fn get_current_process_id() -> u32 {
    let teb = crate::pe::peb_teb::win32_get_current_teb();
    if !teb.is_null() {
        let pid = unsafe { (*teb).client_id.unique_process as u32 };
        if pid != 0 {
            return pid;
        }
    }
    1
}

/// Attaches the calling process as an active debugger to the target process.
pub unsafe extern "system" fn DebugActiveProcess(dwProcessId: u32) -> BOOL {
    if dwProcessId == 0 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let debugger_pid = get_current_process_id();
    let mut sessions = DEBUG_SESSIONS.lock();
    let session = sessions
        .entry(debugger_pid)
        .or_insert_with(|| DebuggerSession::new(debugger_pid));

    if session.attached_processes.contains_key(&dwProcessId) {
        win32_set_last_error(ERROR_ALREADY_EXISTS);
        return FALSE;
    }

    session.attached_processes.insert(dwProcessId, true);

    // Generate initial CREATE_PROCESS debug event for the target.
    let create_proc_event = DEBUG_EVENT {
        dwDebugEventCode: CREATE_PROCESS_DEBUG_EVENT,
        dwProcessId,
        dwThreadId: 1,
        u: DEBUG_EVENT_UNION {
            CreateProcessInfo: CREATE_PROCESS_DEBUG_INFO {
                hFile: 0x100,
                hProcess: dwProcessId as HANDLE,
                hThread: 1 as HANDLE,
                lpBaseOfImage: 0x0040_0000 as *mut c_void,
                dwDebugInfoFileOffset: 0,
                nDebugInfoSize: 0,
                lpThreadLocalBase: core::ptr::null_mut(),
                lpStartAddress: 0x0040_1000 as *mut c_void,
                lpImageName: core::ptr::null_mut(),
                fUnicode: 0,
            },
        },
    };
    session.event_queue.push_back(create_proc_event);

    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Detaches the calling debugger from the target process.
pub unsafe extern "system" fn DebugActiveProcessStop(dwProcessId: u32) -> BOOL {
    let debugger_pid = get_current_process_id();
    let mut sessions = DEBUG_SESSIONS.lock();
    if let Some(session) = sessions.get_mut(&debugger_pid) {
        if session.attached_processes.remove(&dwProcessId).is_some() {
            win32_set_last_error(ERROR_SUCCESS);
            TRUE
        } else {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            FALSE
        }
    } else {
        win32_set_last_error(ERROR_ACCESS_DENIED);
        FALSE
    }
}

/// Injects a breakpoint exception into the target process.
pub unsafe extern "system" fn DebugBreakProcess(Process: HANDLE) -> BOOL {
    if Process == 0 {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return FALSE;
    }

    let target_pid = Process as u32;
    let mut event = DEBUG_EVENT {
        dwDebugEventCode: EXCEPTION_DEBUG_EVENT,
        dwProcessId: target_pid,
        dwThreadId: 1,
        u: DEBUG_EVENT_UNION::default(),
    };
    event.u.Exception = EXCEPTION_DEBUG_INFO {
        ExceptionRecord: EXCEPTION_RECORD {
            ExceptionCode: STATUS_BREAKPOINT,
            ExceptionFlags: 0,
            ExceptionRecord: core::ptr::null_mut(),
            ExceptionAddress: 0x7FFF_0000 as *mut u8,
            NumberParameters: 0,
            ExceptionInformation: [0; 15],
        },
        dwFirstChance: 1,
    };

    if post_debug_event(event) {
        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        FALSE
    }
}

/// Waits for a debug event to occur in a process being debugged.
pub unsafe extern "system" fn WaitForDebugEvent(
    lpDebugEvent: *mut DEBUG_EVENT,
    dwMilliseconds: u32,
) -> BOOL {
    WaitForDebugEventEx(lpDebugEvent, dwMilliseconds)
}

/// Extended debug event retrieval with timeout support.
pub unsafe extern "system" fn WaitForDebugEventEx(
    lpDebugEvent: *mut DEBUG_EVENT,
    _dwMilliseconds: u32,
) -> BOOL {
    if lpDebugEvent.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let debugger_pid = get_current_process_id();
    let mut sessions = DEBUG_SESSIONS.lock();
    if let Some(session) = sessions.get_mut(&debugger_pid) {
        if let Some(event) = session.event_queue.pop_front() {
            session.pending_continue = Some((event.dwProcessId, event.dwThreadId));
            *lpDebugEvent = event;
            win32_set_last_error(ERROR_SUCCESS);
            TRUE
        } else {
            win32_set_last_error(WAIT_TIMEOUT);
            FALSE
        }
    } else {
        win32_set_last_error(ERROR_ACCESS_DENIED);
        FALSE
    }
}

/// Resumes a thread that previously generated a debug event.
pub unsafe extern "system" fn ContinueDebugEvent(
    dwProcessId: u32,
    dwThreadId: u32,
    dwContinueStatus: u32,
) -> BOOL {
    if dwContinueStatus != DBG_CONTINUE
        && dwContinueStatus != DBG_EXCEPTION_NOT_HANDLED
        && dwContinueStatus != DBG_REPLY_LATER
    {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let debugger_pid = get_current_process_id();
    let mut sessions = DEBUG_SESSIONS.lock();
    if let Some(session) = sessions.get_mut(&debugger_pid) {
        if session.pending_continue == Some((dwProcessId, dwThreadId)) {
            session.pending_continue = None;
            win32_set_last_error(ERROR_SUCCESS);
            TRUE
        } else {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            FALSE
        }
    } else {
        win32_set_last_error(ERROR_ACCESS_DENIED);
        FALSE
    }
}
