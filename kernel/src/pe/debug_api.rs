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

pub static PROCESS_DEBUG_MEMORY: Spinlock<BTreeMap<HANDLE, BTreeMap<usize, Vec<u8>>>> =
    Spinlock::new(BTreeMap::new());

/// Reads memory from the virtual address space of a specified process.
pub unsafe extern "system" fn ReadProcessMemory(
    hProcess: HANDLE,
    lpBaseAddress: *const c_void,
    lpBuffer: *mut c_void,
    nSize: usize,
    lpNumberOfBytesRead: *mut usize,
) -> BOOL {
    if hProcess == 0 || lpBaseAddress.is_null() || lpBuffer.is_null() || nSize == 0 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let addr = lpBaseAddress as usize;

    let mem_lock = PROCESS_DEBUG_MEMORY.lock();
    if let Some(pages) = mem_lock.get(&hProcess) {
        if let Some(data) = pages.get(&addr) {
            let copy_len = nSize.min(data.len());
            core::ptr::copy_nonoverlapping(data.as_ptr(), lpBuffer as *mut u8, copy_len);
            if !lpNumberOfBytesRead.is_null() {
                *lpNumberOfBytesRead = copy_len;
            }
            win32_set_last_error(ERROR_SUCCESS);
            return TRUE;
        }
    }
    drop(mem_lock);

    // Fallback: direct virtual memory read.
    core::ptr::copy_nonoverlapping(lpBaseAddress as *const u8, lpBuffer as *mut u8, nSize);
    if !lpNumberOfBytesRead.is_null() {
        *lpNumberOfBytesRead = nSize;
    }

    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Writes memory to the virtual address space of a specified process.
pub unsafe extern "system" fn WriteProcessMemory(
    hProcess: HANDLE,
    lpBaseAddress: *mut c_void,
    lpBuffer: *const c_void,
    nSize: usize,
    lpNumberOfBytesWritten: *mut usize,
) -> BOOL {
    if hProcess == 0 || lpBaseAddress.is_null() || lpBuffer.is_null() || nSize == 0 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let addr = lpBaseAddress as usize;
    let mut slice = Vec::with_capacity(nSize);
    slice.resize(nSize, 0);
    core::ptr::copy_nonoverlapping(lpBuffer as *const u8, slice.as_mut_ptr(), nSize);

    let mut mem_lock = PROCESS_DEBUG_MEMORY.lock();
    let proc_pages = mem_lock.entry(hProcess).or_insert_with(BTreeMap::new);
    proc_pages.insert(addr, slice);
    drop(mem_lock);

    if !lpNumberOfBytesWritten.is_null() {
        *lpNumberOfBytesWritten = nSize;
    }

    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Breakpoint trigger condition for x86-64 debug registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwBpCondition {
    Execute = 0b00,
    Write = 0b01,
    IoPort = 0b10,
    ReadWrite = 0b11,
}

/// Watchpoint data size for x86-64 debug registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwBpSize {
    Byte1 = 0b00,
    Byte2 = 0b01,
    Byte8 = 0b10,
    Byte4 = 0b11,
}

/// Virtualized x86-64 hardware debug registers (DR0-DR3, DR6, DR7).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HardwareDebugRegisters {
    pub dr0: usize,
    pub dr1: usize,
    pub dr2: usize,
    pub dr3: usize,
    pub dr6: usize,
    pub dr7: usize,
}

pub static THREAD_HW_BREAKPOINTS: Spinlock<BTreeMap<u32, HardwareDebugRegisters>> =
    Spinlock::new(BTreeMap::new());

/// Configures an x86-64 hardware watchpoint register for the specified thread.
pub fn SetHardwareBreakpoint(
    tid: u32,
    dr_index: usize,
    address: usize,
    condition: HwBpCondition,
    size: HwBpSize,
) -> bool {
    if dr_index >= 4 {
        return false;
    }

    let mut lock = THREAD_HW_BREAKPOINTS.lock();
    let regs = lock.entry(tid).or_insert_with(HardwareDebugRegisters::default);

    match dr_index {
        0 => regs.dr0 = address,
        1 => regs.dr1 = address,
        2 => regs.dr2 = address,
        3 => regs.dr3 = address,
        _ => unreachable!(),
    }

    // Enable local breakpoint in DR7 (bit 0, 2, 4, 6) and LE (bit 8).
    regs.dr7 |= (1 << (dr_index * 2)) | (1 << 8);

    // Set condition (R/W) and size (LEN) fields in DR7.
    let rw_shift = 16 + (dr_index * 4);
    let len_shift = 18 + (dr_index * 4);
    regs.dr7 = (regs.dr7 & !(0x3 << rw_shift)) | ((condition as usize) << rw_shift);
    regs.dr7 = (regs.dr7 & !(0x3 << len_shift)) | ((size as usize) << len_shift);

    true
}

/// Disables an x86-64 hardware watchpoint register for the specified thread.
pub fn ClearHardwareBreakpoint(tid: u32, dr_index: usize) -> bool {
    if dr_index >= 4 {
        return false;
    }

    let mut lock = THREAD_HW_BREAKPOINTS.lock();
    if let Some(regs) = lock.get_mut(&tid) {
        match dr_index {
            0 => regs.dr0 = 0,
            1 => regs.dr1 = 0,
            2 => regs.dr2 = 0,
            3 => regs.dr3 = 0,
            _ => unreachable!(),
        }

        // Disable local and global enable bits for this DR.
        regs.dr7 &= !((1 << (dr_index * 2)) | (1 << (dr_index * 2 + 1)));

        // Clear R/W and LEN fields.
        let rw_shift = 16 + (dr_index * 4);
        regs.dr7 &= !(0xF << rw_shift);

        true
    } else {
        false
    }
}

/// Retrieves the current hardware debug register configuration for a thread.
pub fn GetHardwareBreakpoints(tid: u32) -> HardwareDebugRegisters {
    let lock = THREAD_HW_BREAKPOINTS.lock();
    lock.get(&tid).copied().unwrap_or_default()
}

/// Checks if an address access triggers any active hardware watchpoint.
pub fn CheckHardwareBreakpointTrigger(tid: u32, address: usize, is_write: bool) -> Option<usize> {
    let mut lock = THREAD_HW_BREAKPOINTS.lock();
    let regs = lock.get_mut(&tid)?;

    let addresses = [regs.dr0, regs.dr1, regs.dr2, regs.dr3];
    for (i, &watch_addr) in addresses.iter().enumerate() {
        let is_enabled = (regs.dr7 & (1 << (i * 2))) != 0;
        if is_enabled && watch_addr == address {
            let rw = (regs.dr7 >> (16 + i * 4)) & 0x3;
            let triggered = match rw {
                0b00 => !is_write,              // Execute
                0b01 => is_write,               // Write only
                0b11 => true,                   // Read/Write
                _ => false,
            };
            if triggered {
                regs.dr6 |= 1 << i;
                return Some(i);
            }
        }
    }
    None
}

/// Comprehensive self-test for Win32 debugging, exception trapping, and hardware watchpoints.
pub fn test_win32_debug_subsystem() -> bool {
    unsafe {
        let test_pid = 0x4242u32;
        let test_hprocess = test_pid as HANDLE;

        // 1. Attach debugger to target process.
        if DebugActiveProcess(test_pid) != TRUE {
            return false;
        }

        // 2. Wait for initial CREATE_PROCESS debug event.
        let mut event = DEBUG_EVENT::default();
        if WaitForDebugEvent(&mut event, 100) != TRUE {
            return false;
        }

        if event.dwDebugEventCode != CREATE_PROCESS_DEBUG_EVENT || event.dwProcessId != test_pid {
            return false;
        }

        // 3. Continue debug event.
        if ContinueDebugEvent(test_pid, event.dwThreadId, DBG_CONTINUE) != TRUE {
            return false;
        }

        // 4. Cross-process memory inspection and modification.
        let target_addr = 0x0040_5000usize as *mut c_void;
        let write_pattern: [u8; 8] = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22];
        let mut bytes_written = 0usize;
        if WriteProcessMemory(
            test_hprocess,
            target_addr,
            write_pattern.as_ptr() as *const c_void,
            8,
            &mut bytes_written,
        ) != TRUE {
            return false;
        }

        if bytes_written != 8 {
            return false;
        }

        let mut read_buf = [0u8; 8];
        let mut bytes_read = 0usize;
        if ReadProcessMemory(
            test_hprocess,
            target_addr as *const c_void,
            read_buf.as_mut_ptr() as *mut c_void,
            8,
            &mut bytes_read,
        ) != TRUE {
            return false;
        }

        if bytes_read != 8 || read_buf != write_pattern {
            return false;
        }

        // 5. Hardware debug registers and watchpoint trapping.
        let thread_id = 1u32;
        let watch_addr = 0x0040_5000usize;
        if !SetHardwareBreakpoint(thread_id, 0, watch_addr, HwBpCondition::Write, HwBpSize::Byte4) {
            return false;
        }

        let hw_regs = GetHardwareBreakpoints(thread_id);
        if hw_regs.dr0 != watch_addr || (hw_regs.dr7 & 0x1) == 0 {
            return false;
        }

        if CheckHardwareBreakpointTrigger(thread_id, watch_addr, true) != Some(0) {
            return false;
        }

        if !ClearHardwareBreakpoint(thread_id, 0) {
            return false;
        }

        let cleared_regs = GetHardwareBreakpoints(thread_id);
        if cleared_regs.dr0 != 0 || (cleared_regs.dr7 & 0x1) != 0 {
            return false;
        }

        // 6. Breakpoint injection and exception handling.
        if DebugBreakProcess(test_hprocess) != TRUE {
            return false;
        }

        if WaitForDebugEvent(&mut event, 100) != TRUE {
            return false;
        }

        if event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT
            || event.u.Exception.ExceptionRecord.ExceptionCode != STATUS_BREAKPOINT
        {
            return false;
        }

        if ContinueDebugEvent(test_pid, event.dwThreadId, DBG_CONTINUE) != TRUE {
            return false;
        }

        // 7. Detach debugger session.
        if DebugActiveProcessStop(test_pid) != TRUE {
            return false;
        }

        crate::serial_write_str(
            "[KNT-WIN32-DEBUG-PASS] Win32 debugging and exception instrumentation subsystem verified\r\n",
        );
        true
    }
}
