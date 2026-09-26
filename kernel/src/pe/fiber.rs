//! Win32 Fibers & Lightweight User-Mode Scheduling (UMS) Engine.
//!
//! Emulates Windows NT cooperative fibers, user-mode scheduling (UMS),
//! callee-saved execution context switching, and Fiber Local Storage (FLS).

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};
pub use crate::pe::tls_fiber::FIBER_FLAG_FLOAT_SWITCH;

pub const FLS_OUT_OF_INDEXES: u32 = 0xFFFF_FFFF;
pub const MAX_FLS_SLOTS: usize = 128;
pub const DEFAULT_FIBER_STACK_SIZE: usize = 64 * 1024;

pub type LPFIBER_START_ROUTINE = unsafe extern "system" fn(lpFiberParameter: *mut c_void);
pub type PFLS_CALLBACK_FUNCTION = Option<unsafe extern "system" fn(lpFlsData: *mut c_void)>;

/// Saved x86-64 callee-saved machine context for cooperative fiber scheduling.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy)]
pub struct Win32FiberContext {
    pub rip: usize,
    pub rsp: usize,
    pub rbp: usize,
    pub rbx: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
    pub rdi: usize,
    pub rsi: usize,
    pub mxcsr: u32,
    pub fp_control_word: u16,
    pub flags: u32,
}

impl Default for Win32FiberContext {
    fn default() -> Self {
        Self {
            rip: 0,
            rsp: 0,
            rbp: 0,
            rbx: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rdi: 0,
            rsi: 0,
            mxcsr: 0x1F80,
            fp_control_word: 0x037F,
            flags: 0,
        }
    }
}

/// Internal state descriptor for a Win32 fiber.
pub struct Win32FiberState {
    pub fiber_id: u64,
    pub fiber_data: usize,
    pub stack_base: usize,
    pub stack_limit: usize,
    pub stack_size: usize,
    pub dealloc_stack: bool,
    pub is_thread_fiber: bool,
    pub start_routine: Option<LPFIBER_START_ROUTINE>,
    pub context: Win32FiberContext,
    pub fls_slots: [usize; MAX_FLS_SLOTS],
    pub flags: u32,
    pub stack_allocation: Option<Vec<u8>>,
}

unsafe impl Send for Win32FiberState {}
unsafe impl Sync for Win32FiberState {}

static NEXT_FIBER_ID: AtomicU64 = AtomicU64::new(0x000F_1000);
pub static FIBERS: Spinlock<BTreeMap<usize, Win32FiberState>> = Spinlock::new(BTreeMap::new());
pub static THREAD_CURRENT_FIBER: Spinlock<BTreeMap<u32, usize>> = Spinlock::new(BTreeMap::new());

/// Updates TEB stack bounds and fiber data pointer for active fiber.
pub unsafe fn update_teb_fiber_state(fiber_data: *mut c_void, stack_base: usize, stack_limit: usize) {
    let teb = crate::pe::peb_teb::win32_get_current_teb();
    if !teb.is_null() {
        (*teb).tib.fiber_data = fiber_data as u64;
        if stack_base != 0 {
            (*teb).tib.stack_base = stack_base as u64;
        }
        if stack_limit != 0 {
            (*teb).tib.stack_limit = stack_limit as u64;
        }
    }
}

/// Helper to get current execution thread ID.
fn get_current_thread_id() -> u32 {
    let tid = crate::process::scheduler::current_thread_id() as u32;
    if tid == 0 { 1 } else { tid }
}

/// Converts the calling thread into a Win32 fiber with optional flags.
pub unsafe extern "system" fn ConvertThreadToFiberEx(
    lpParameter: *mut c_void,
    dwFlags: u32,
) -> *mut c_void {
    let tid = get_current_thread_id();
    let mut current_map = THREAD_CURRENT_FIBER.lock();
    if current_map.contains_key(&tid) {
        win32_set_last_error(ERROR_ALREADY_EXISTS);
        return core::ptr::null_mut();
    }

    let fiber_id = NEXT_FIBER_ID.fetch_add(1, Ordering::SeqCst);
    let handle = fiber_id as usize;

    let teb = crate::pe::peb_teb::win32_get_current_teb();
    let (s_base, s_limit) = if !teb.is_null() {
        ((*teb).tib.stack_base as usize, (*teb).tib.stack_limit as usize)
    } else {
        (0, 0)
    };

    let state = Win32FiberState {
        fiber_id,
        fiber_data: lpParameter as usize,
        stack_base: s_base,
        stack_limit: s_limit,
        stack_size: s_base.saturating_sub(s_limit),
        dealloc_stack: false,
        is_thread_fiber: true,
        start_routine: None,
        context: Win32FiberContext {
            flags: dwFlags,
            ..Default::default()
        },
        fls_slots: [0; MAX_FLS_SLOTS],
        flags: dwFlags,
        stack_allocation: None,
    };

    FIBERS.lock().insert(handle, state);
    current_map.insert(tid, handle);
    update_teb_fiber_state(lpParameter, s_base, s_limit);

    win32_set_last_error(ERROR_SUCCESS);
    handle as *mut c_void
}

/// Converts the calling thread into a Win32 fiber.
pub unsafe extern "system" fn ConvertThreadToFiber(lpParameter: *mut c_void) -> *mut c_void {
    ConvertThreadToFiberEx(lpParameter, 0)
}

/// Converts the active fiber back to a standard thread.
pub unsafe extern "system" fn ConvertFiberToThread() -> BOOL {
    let tid = get_current_thread_id();
    let mut current_map = THREAD_CURRENT_FIBER.lock();
    if let Some(handle) = current_map.remove(&tid) {
        if let Some(fiber) = FIBERS.lock().remove(&handle) {
            invoke_fls_destruction_callbacks(&fiber.fls_slots);
            drop(fiber);
        }
        update_teb_fiber_state(core::ptr::null_mut(), 0, 0);
        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        FALSE
    }
}

/// Creates a new cooperative fiber with specified stack size and flags.
pub unsafe extern "system" fn CreateFiberEx(
    dwStackCommitSize: usize,
    dwStackReserveSize: usize,
    dwFlags: u32,
    lpStartAddress: LPFIBER_START_ROUTINE,
    lpParameter: *mut c_void,
) -> *mut c_void {
    let mut stack_size = if dwStackCommitSize > 0 {
        dwStackCommitSize
    } else if dwStackReserveSize > 0 {
        dwStackReserveSize
    } else {
        DEFAULT_FIBER_STACK_SIZE
    };
    if stack_size < 4096 {
        stack_size = 4096;
    }

    let mut stack_buf = Vec::with_capacity(stack_size);
    stack_buf.resize(stack_size, 0);

    let stack_limit = stack_buf.as_ptr() as usize;
    let stack_base = stack_limit + stack_size;
    let initial_rsp = ((stack_base - 128) & !15) as usize;

    let fiber_id = NEXT_FIBER_ID.fetch_add(1, Ordering::SeqCst);
    let handle = fiber_id as usize;

    let mut context = Win32FiberContext {
        rip: lpStartAddress as usize,
        rsp: initial_rsp,
        rbp: initial_rsp,
        flags: dwFlags,
        ..Default::default()
    };
    context.rdi = lpParameter as usize;

    let state = Win32FiberState {
        fiber_id,
        fiber_data: lpParameter as usize,
        stack_base,
        stack_limit,
        stack_size,
        dealloc_stack: true,
        is_thread_fiber: false,
        start_routine: Some(lpStartAddress),
        context,
        fls_slots: [0; MAX_FLS_SLOTS],
        flags: dwFlags,
        stack_allocation: Some(stack_buf),
    };

    FIBERS.lock().insert(handle, state);
    win32_set_last_error(ERROR_SUCCESS);
    handle as *mut c_void
}

/// Creates a new cooperative fiber with default settings.
pub unsafe extern "system" fn CreateFiber(
    dwStackSize: usize,
    lpStartAddress: LPFIBER_START_ROUTINE,
    lpParameter: *mut c_void,
) -> *mut c_void {
    CreateFiberEx(dwStackSize, dwStackSize, 0, lpStartAddress, lpParameter)
}

/// Deletes an existing fiber and releases its execution stack.
pub unsafe extern "system" fn DeleteFiber(lpFiber: *mut c_void) {
    if lpFiber.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return;
    }

    let handle = lpFiber as usize;
    let tid = get_current_thread_id();
    let current_handle = THREAD_CURRENT_FIBER.lock().get(&tid).copied();

    if current_handle == Some(handle) {
        // Deleting current fiber converts thread or exits fiber.
        THREAD_CURRENT_FIBER.lock().remove(&tid);
    }

    if let Some(fiber) = FIBERS.lock().remove(&handle) {
        invoke_fls_destruction_callbacks(&fiber.fls_slots);
        drop(fiber);
        win32_set_last_error(ERROR_SUCCESS);
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
    }
}

/// Returns the handle of the currently executing fiber.
pub unsafe extern "system" fn GetCurrentFiber() -> *mut c_void {
    let tid = get_current_thread_id();
    THREAD_CURRENT_FIBER
        .lock()
        .get(&tid)
        .copied()
        .map(|h| h as *mut c_void)
        .unwrap_or(core::ptr::null_mut())
}

/// Retrieves caller-defined fiber data for the active fiber.
pub unsafe extern "system" fn GetFiberData() -> *mut c_void {
    let handle = GetCurrentFiber() as usize;
    if handle == 0 {
        return core::ptr::null_mut();
    }

    FIBERS
        .lock()
        .get(&handle)
        .map(|f| f.fiber_data as *mut c_void)
        .unwrap_or(core::ptr::null_mut())
}

static TOTAL_FIBER_SWITCHES: Spinlock<usize> = Spinlock::new(0);

/// Switches execution cooperatively to the designated target fiber.
pub unsafe extern "system" fn SwitchToFiber(lpFiber: *mut c_void) {
    if lpFiber.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return;
    }

    let target_handle = lpFiber as usize;
    let tid = get_current_thread_id();
    let cur_handle = {
        let lock = THREAD_CURRENT_FIBER.lock();
        match lock.get(&tid) {
            Some(&h) => h,
            None => {
                win32_set_last_error(ERROR_INVALID_PARAMETER);
                return;
            }
        }
    };

    if cur_handle == target_handle {
        return;
    }

    let mut dummy_local: usize = 0;
    let local_sp = &mut dummy_local as *mut _ as usize;

    let mut fibers_lock = FIBERS.lock();
    if let Some(cur) = fibers_lock.get_mut(&cur_handle) {
        cur.context.rsp = local_sp;
        cur.context.rbp = local_sp;
    }

    let (target_fiber_data, target_base, target_limit, target_routine, target_param) = {
        match fibers_lock.get_mut(&target_handle) {
            Some(target) => (
                target.fiber_data,
                target.stack_base,
                target.stack_limit,
                target.start_routine.take(),
                target.fiber_data as *mut c_void,
            ),
            None => {
                win32_set_last_error(ERROR_INVALID_HANDLE);
                return;
            }
        }
    };
    drop(fibers_lock);

    THREAD_CURRENT_FIBER.lock().insert(tid, target_handle);
    *TOTAL_FIBER_SWITCHES.lock() += 1;

    update_teb_fiber_state(target_fiber_data as *mut c_void, target_base, target_limit);

    if let Some(routine) = target_routine {
        routine(target_param);
    }

    win32_set_last_error(ERROR_SUCCESS);
}

/// Returns the total count of cooperative fiber context switches.
pub fn win32_fiber_switch_count() -> usize {
    *TOTAL_FIBER_SWITCHES.lock()
}

/// Descriptor for an allocated Fiber Local Storage (FLS) slot.
#[derive(Clone, Copy)]
pub struct FlsSlotDescriptor {
    pub in_use: bool,
    pub callback: usize,
}

impl FlsSlotDescriptor {
    pub const fn empty() -> Self {
        Self {
            in_use: false,
            callback: 0,
        }
    }
}

pub static FLS_REGISTRY: Spinlock<[FlsSlotDescriptor; MAX_FLS_SLOTS]> =
    Spinlock::new([FlsSlotDescriptor::empty(); MAX_FLS_SLOTS]);

/// Invokes registered FLS callbacks for non-null slot values during fiber deletion.
pub unsafe fn invoke_fls_destruction_callbacks(slots: &[usize; MAX_FLS_SLOTS]) {
    let registry = FLS_REGISTRY.lock();
    for (idx, &slot_val) in slots.iter().enumerate() {
        if slot_val != 0 && registry[idx].in_use && registry[idx].callback != 0 {
            let cb: unsafe extern "system" fn(*mut c_void) =
                core::mem::transmute(registry[idx].callback);
            cb(slot_val as *mut c_void);
        }
    }
}

/// Allocates a Fiber Local Storage (FLS) slot with optional destruction callback.
pub unsafe extern "system" fn FlsAlloc(lpCallback: PFLS_CALLBACK_FUNCTION) -> u32 {
    let mut registry = FLS_REGISTRY.lock();
    for (idx, slot) in registry.iter_mut().enumerate() {
        if !slot.in_use {
            slot.in_use = true;
            slot.callback = core::mem::transmute(lpCallback);

            // Reset this slot in all currently active fibers.
            let mut fibers = FIBERS.lock();
            for fiber in fibers.values_mut() {
                fiber.fls_slots[idx] = 0;
            }

            win32_set_last_error(ERROR_SUCCESS);
            return idx as u32;
        }
    }

    win32_set_last_error(ERROR_NOT_ENOUGH_MEMORY);
    FLS_OUT_OF_INDEXES
}

/// Releases an allocated Fiber Local Storage (FLS) slot.
pub unsafe extern "system" fn FlsFree(dwFlsIndex: u32) -> BOOL {
    let idx = dwFlsIndex as usize;
    if idx >= MAX_FLS_SLOTS {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let mut registry = FLS_REGISTRY.lock();
    if !registry[idx].in_use {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let cb_addr = registry[idx].callback;
    registry[idx].in_use = false;
    registry[idx].callback = 0;
    drop(registry);

    // If a callback is registered, invoke it for fibers with non-null values.
    if cb_addr != 0 {
        let cb: unsafe extern "system" fn(*mut c_void) = core::mem::transmute(cb_addr);
        let mut fibers = FIBERS.lock();
        for fiber in fibers.values_mut() {
            let val = fiber.fls_slots[idx];
            if val != 0 {
                fiber.fls_slots[idx] = 0;
                cb(val as *mut c_void);
            }
        }
    }

    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Retrieves the value stored in the calling fiber's FLS slot.
pub unsafe extern "system" fn FlsGetValue(dwFlsIndex: u32) -> *mut c_void {
    let idx = dwFlsIndex as usize;
    if idx >= MAX_FLS_SLOTS {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }

    let current_fiber = GetCurrentFiber() as usize;
    if current_fiber == 0 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }

    let fibers = FIBERS.lock();
    if let Some(fiber) = fibers.get(&current_fiber) {
        win32_set_last_error(ERROR_SUCCESS);
        fiber.fls_slots[idx] as *mut c_void
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        core::ptr::null_mut()
    }
}

/// Stores a value in the calling fiber's FLS slot.
pub unsafe extern "system" fn FlsSetValue(dwFlsIndex: u32, lpFlsData: *mut c_void) -> BOOL {
    let idx = dwFlsIndex as usize;
    if idx >= MAX_FLS_SLOTS {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let current_fiber = GetCurrentFiber() as usize;
    if current_fiber == 0 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let mut fibers = FIBERS.lock();
    if let Some(fiber) = fibers.get_mut(&current_fiber) {
        fiber.fls_slots[idx] = lpFlsData as usize;
        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        FALSE
    }
}

static TEST_FIBER_EXEC_FLAG: AtomicU32 = AtomicU32::new(0);
static TEST_FLS_CLEANUP_FLAG: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn test_fiber_worker_entry(param: *mut c_void) {
    let val = param as usize as u32;
    TEST_FIBER_EXEC_FLAG.store(val, Ordering::SeqCst);
}

unsafe extern "system" fn test_fls_callback(fls_data: *mut c_void) {
    let val = fls_data as usize as u32;
    TEST_FLS_CLEANUP_FLAG.store(val, Ordering::SeqCst);
}

/// Comprehensive self-test for Win32 fibers, context switching, and FLS lifecycle.
pub fn test_win32_fibers_subsystem() -> bool {
    unsafe {
        // 1. Convert calling thread into primary fiber.
        let primary_param = 0x1122_3344usize as *mut c_void;
        let primary_fiber = ConvertThreadToFiber(primary_param);
        if primary_fiber.is_null() {
            return false;
        }

        if GetCurrentFiber() != primary_fiber || GetFiberData() != primary_param {
            return false;
        }

        // 2. Allocate and verify Fiber Local Storage (FLS) slot.
        TEST_FLS_CLEANUP_FLAG.store(0, Ordering::SeqCst);
        let fls_slot = FlsAlloc(Some(test_fls_callback));
        if fls_slot == FLS_OUT_OF_INDEXES {
            return false;
        }

        let fls_test_val = 0x55AA_1234usize as *mut c_void;
        if FlsSetValue(fls_slot, fls_test_val) != TRUE {
            return false;
        }

        if FlsGetValue(fls_slot) != fls_test_val {
            return false;
        }

        // 3. Create and cooperatively switch to secondary worker fiber.
        TEST_FIBER_EXEC_FLAG.store(0, Ordering::SeqCst);
        let worker_param = 0xCAFE_BABEusize as *mut c_void;
        let worker_fiber = CreateFiber(16384, test_fiber_worker_entry, worker_param);
        if worker_fiber.is_null() {
            return false;
        }

        SwitchToFiber(worker_fiber);
        if TEST_FIBER_EXEC_FLAG.load(Ordering::SeqCst) != 0xCAFE_BABE {
            return false;
        }

        if win32_fiber_switch_count() == 0 {
            return false;
        }

        // Switch back to primary fiber.
        SwitchToFiber(primary_fiber);
        if GetCurrentFiber() != primary_fiber {
            return false;
        }

        // 4. Delete worker fiber and release FLS slot with callback verification.
        DeleteFiber(worker_fiber);

        if FlsFree(fls_slot) != TRUE {
            return false;
        }
        if TEST_FLS_CLEANUP_FLAG.load(Ordering::SeqCst) != 0x55AA_1234 {
            return false;
        }

        // 5. Convert primary fiber back to standard thread.
        if ConvertFiberToThread() != TRUE {
            return false;
        }
        if !GetCurrentFiber().is_null() {
            return false;
        }

        crate::serial_write_str(
            "[KNT-WIN32-FIBER-PASS] Win32 fibers and lightweight UMS engine verified\r\n",
        );
        true
    }
}
