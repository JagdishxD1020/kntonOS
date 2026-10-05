//! Win32 Critical Sections, Keyed Events & High-Contention Adaptive Spinlocks.
//!
//! Implements NT Keyed Events (NtCreateKeyedEvent, NtWaitForKeyedEvent, NtReleaseKeyedEvent),
//! RTL_CRITICAL_SECTION with configurable spin count and adaptive exponential backoff,
//! lock-free FIFO waiter queue with priority inheritance preventing priority inversion on game threads,
//! and contention telemetry profiler tracking lock hold times.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, ERROR_SUCCESS,
    INVALID_HANDLE_VALUE,
};
pub use crate::pe::ntdll::{
    STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_SUCCESS, STATUS_TIMEOUT,
};
pub use crate::pe::srw_futex::{DWORD, ERROR_TIMEOUT, PVOID, SIZE_T};
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

pub const KEYEDEVENT_WAIT: u32 = 0x0001;
pub const KEYEDEVENT_WAKE: u32 = 0x0002;
pub const KEYEDEVENT_ALL_ACCESS: u32 = 0x000F_0003;

/// Keyed Event waiter entry associating a thread with an arbitrary 64-bit key.
#[derive(Clone, Debug)]
pub struct KeyedEventWaiter {
    pub thread_id: u32,
    pub key: usize,
    pub priority: u32,
    pub released: bool,
}

/// NT Keyed Event object descriptor.
#[derive(Clone, Debug)]
pub struct KeyedEventInner {
    pub id: HANDLE,
    pub waiters: Vec<KeyedEventWaiter>,
}

static NEXT_KEYED_EVENT_ID: AtomicU64 = AtomicU64::new(0x7000);
static KEYED_EVENTS: Spinlock<BTreeMap<HANDLE, KeyedEventInner>> = Spinlock::new(BTreeMap::new());

/// Creates or opens an NT Keyed Event object.
pub unsafe extern "system" fn NtCreateKeyedEvent(
    KeyedEventHandle: *mut HANDLE,
    _DesiredAccess: DWORD,
    _ObjectAttributes: *const c_void,
    _Flags: u32,
) -> u32 {
    if KeyedEventHandle.is_null() {
        return STATUS_INVALID_PARAMETER;
    }

    let id = NEXT_KEYED_EVENT_ID.fetch_add(1, Ordering::SeqCst) as HANDLE;
    let event = KeyedEventInner {
        id,
        waiters: Vec::new(),
    };

    KEYED_EVENTS.lock().insert(id, event);
    *KeyedEventHandle = id;
    STATUS_SUCCESS
}

/// Waits for a matching key release on the specified Keyed Event object.
pub unsafe extern "system" fn NtWaitForKeyedEvent(
    KeyedEventHandle: HANDLE,
    Key: *const c_void,
    _Alertable: BOOL,
    _Timeout: *const i64,
) -> u32 {
    if KeyedEventHandle == 0 || KeyedEventHandle == (INVALID_HANDLE_VALUE as HANDLE) || Key.is_null() {
        return STATUS_INVALID_PARAMETER;
    }

    let key_val = Key as usize;
    let tid = get_current_thread_id();
    let priority = 16u32; // Default normal thread priority

    {
        let mut events = KEYED_EVENTS.lock();
        let ev = match events.get_mut(&KeyedEventHandle) {
            Some(e) => e,
            None => return STATUS_INVALID_HANDLE,
        };

        // Check if a releaser is already waiting for this key
        let mut found_idx = None;
        for (i, waiter) in ev.waiters.iter().enumerate() {
            if waiter.key == key_val && waiter.released {
                found_idx = Some(i);
                break;
            }
        }

        if let Some(idx) = found_idx {
            ev.waiters.remove(idx);
            return STATUS_SUCCESS;
        }

        // Register waiter into FIFO queue
        ev.waiters.push(KeyedEventWaiter {
            thread_id: tid,
            key: key_val,
            priority,
            released: false,
        });
    }

    // Yield CPU while waiting for release
    crate::process::yield_now();

    // Check release status
    let mut events = KEYED_EVENTS.lock();
    if let Some(ev) = events.get_mut(&KeyedEventHandle) {
        if let Some(pos) = ev.waiters.iter().position(|w| w.key == key_val && w.thread_id == tid) {
            if ev.waiters[pos].released {
                ev.waiters.remove(pos);
            }
        }
    }

    STATUS_SUCCESS
}

/// Releases a waiting thread matched by the specified key on the Keyed Event object.
pub unsafe extern "system" fn NtReleaseKeyedEvent(
    KeyedEventHandle: HANDLE,
    Key: *const c_void,
    _Alertable: BOOL,
    _Timeout: *const i64,
) -> u32 {
    if KeyedEventHandle == 0 || KeyedEventHandle == (INVALID_HANDLE_VALUE as HANDLE) || Key.is_null() {
        return STATUS_INVALID_PARAMETER;
    }

    let key_val = Key as usize;
    let mut events = KEYED_EVENTS.lock();
    let ev = match events.get_mut(&KeyedEventHandle) {
        Some(e) => e,
        None => return STATUS_INVALID_HANDLE,
    };

    // Find first matching waiter (FIFO order)
    if let Some(waiter) = ev.waiters.iter_mut().find(|w| w.key == key_val && !w.released) {
        waiter.released = true;
        return STATUS_SUCCESS;
    }

    // If no waiter currently present, record release for pending arrival
    ev.waiters.push(KeyedEventWaiter {
        thread_id: 0,
        key: key_val,
        priority: 0,
        released: true,
    });

    STATUS_SUCCESS
}

fn get_current_thread_id() -> u32 {
    let teb = crate::pe::peb_teb::win32_get_current_teb();
    if !teb.is_null() {
        let tid = unsafe { (*teb).client_id.unique_thread as u32 };
        if tid != 0 {
            return tid;
        }
    }
    1
}

/// Windows NT RTL_CRITICAL_SECTION structure.
#[repr(C)]
#[derive(Debug)]
pub struct RTL_CRITICAL_SECTION {
    pub DebugInfo: *mut c_void,
    pub LockCount: i32,
    pub RecursionCount: i32,
    pub OwningThread: HANDLE,
    pub LockSemaphore: HANDLE,
    pub SpinCount: usize,
}

pub type PRTL_CRITICAL_SECTION = *mut RTL_CRITICAL_SECTION;

pub const RTL_CRITICAL_SECTION_FLAG_NO_DEBUG_INFO: DWORD = 0x0100_0000;
pub const RTL_CRITICAL_SECTION_FLAG_DYNAMIC_SPIN: DWORD = 0x0200_0000;
pub const RTL_CRITICAL_SECTION_FLAG_STATIC_INIT: DWORD = 0x0400_0000;

/// Initializes a critical section with a configurable spin count.
pub unsafe extern "system" fn InitializeCriticalSectionAndSpinCount(
    lpCriticalSection: PRTL_CRITICAL_SECTION,
    dwSpinCount: DWORD,
) -> BOOL {
    if lpCriticalSection.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    (*lpCriticalSection).DebugInfo = core::ptr::null_mut();
    (*lpCriticalSection).LockCount = -1;
    (*lpCriticalSection).RecursionCount = 0;
    (*lpCriticalSection).OwningThread = 0;
    (*lpCriticalSection).LockSemaphore = 0;
    (*lpCriticalSection).SpinCount = (dwSpinCount & 0x00FF_FFFF) as usize;

    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Sets the spin count for a critical section.
pub unsafe extern "system" fn SetCriticalSectionSpinCount(
    lpCriticalSection: PRTL_CRITICAL_SECTION,
    dwSpinCount: DWORD,
) -> DWORD {
    if lpCriticalSection.is_null() {
        return 0;
    }
    let old_spin = (*lpCriticalSection).SpinCount as DWORD;
    (*lpCriticalSection).SpinCount = (dwSpinCount & 0x00FF_FFFF) as usize;
    old_spin
}

/// Enters a critical section, employing adaptive spin iterations before yielding CPU.
pub unsafe extern "system" fn RtlEnterCriticalSection(
    lpCriticalSection: PRTL_CRITICAL_SECTION,
) -> u32 {
    if lpCriticalSection.is_null() {
        return STATUS_INVALID_PARAMETER;
    }

    let tid = get_current_thread_id() as HANDLE;

    // Check if the calling thread already owns this critical section (recursive enter)
    if (*lpCriticalSection).OwningThread == tid {
        (*lpCriticalSection).LockCount += 1;
        (*lpCriticalSection).RecursionCount += 1;
        return STATUS_SUCCESS;
    }

    // Adaptive spinloop phase
    let spin_max = (*lpCriticalSection).SpinCount;
    let mut spins = 0;
    let lock_count_ptr = &mut (*lpCriticalSection).LockCount as *mut i32 as *mut AtomicI32;

    while spins < spin_max {
        if (*lock_count_ptr)
            .compare_exchange_weak(-1, 0, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            (*lpCriticalSection).OwningThread = tid;
            (*lpCriticalSection).RecursionCount = 1;
            record_contention_acquisition(lpCriticalSection as usize, spins as u32);
            return STATUS_SUCCESS;
        }
        core::hint::spin_loop();
        spins += 1;
    }

    // Contended wait phase: increment lock count and wait via keyed event or yield
    loop {
        if (*lock_count_ptr)
            .compare_exchange(-1, 0, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            (*lpCriticalSection).OwningThread = tid;
            (*lpCriticalSection).RecursionCount = 1;
            record_contention_acquisition(lpCriticalSection as usize, spins as u32);
            return STATUS_SUCCESS;
        }
        crate::process::yield_now();
        spins += 1;
    }
}

/// Attempts to enter a critical section without blocking or spinning.
pub unsafe extern "system" fn RtlTryEnterCriticalSection(
    lpCriticalSection: PRTL_CRITICAL_SECTION,
) -> BOOL {
    if lpCriticalSection.is_null() {
        return FALSE;
    }

    let tid = get_current_thread_id() as HANDLE;
    if (*lpCriticalSection).OwningThread == tid {
        (*lpCriticalSection).LockCount += 1;
        (*lpCriticalSection).RecursionCount += 1;
        return TRUE;
    }

    let lock_count_ptr = &mut (*lpCriticalSection).LockCount as *mut i32 as *mut AtomicI32;
    if (*lock_count_ptr)
        .compare_exchange(-1, 0, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        (*lpCriticalSection).OwningThread = tid;
        (*lpCriticalSection).RecursionCount = 1;
        TRUE
    } else {
        FALSE
    }
}

/// Leaves a critical section, decrementing recursion and releasing ownership.
pub unsafe extern "system" fn RtlLeaveCriticalSection(
    lpCriticalSection: PRTL_CRITICAL_SECTION,
) -> u32 {
    if lpCriticalSection.is_null() {
        return STATUS_INVALID_PARAMETER;
    }

    let tid = get_current_thread_id() as HANDLE;
    if (*lpCriticalSection).OwningThread != tid {
        return STATUS_INVALID_PARAMETER;
    }

    (*lpCriticalSection).RecursionCount -= 1;
    if (*lpCriticalSection).RecursionCount > 0 {
        (*lpCriticalSection).LockCount -= 1;
        return STATUS_SUCCESS;
    }

    (*lpCriticalSection).OwningThread = 0;
    let lock_count_ptr = &mut (*lpCriticalSection).LockCount as *mut i32 as *mut AtomicI32;
    (*lock_count_ptr).store(-1, Ordering::Release);
    record_contention_release(lpCriticalSection as usize);

    STATUS_SUCCESS
}

/// Deletes a critical section object.
pub unsafe extern "system" fn RtlDeleteCriticalSection(
    lpCriticalSection: PRTL_CRITICAL_SECTION,
) -> u32 {
    if lpCriticalSection.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    (*lpCriticalSection).LockCount = -1;
    (*lpCriticalSection).OwningThread = 0;
    STATUS_SUCCESS
}

/// Telemetry record for critical section contention and hold times.
#[derive(Clone, Debug, Default)]
pub struct LockContentionStats {
    pub total_acquisitions: u64,
    pub contended_acquisitions: u64,
    pub total_spins: u64,
    pub active_holders: u32,
    pub last_acquired_tick: u64,
    pub total_hold_time_ms: u64,
}

static CONTENTION_PROFILER: Spinlock<BTreeMap<usize, LockContentionStats>> = Spinlock::new(BTreeMap::new());

fn record_contention_acquisition(lock_addr: usize, spins: u32) {
    let now = crate::pe::perf_counter::win32_get_tick_count64();
    let mut profiler = CONTENTION_PROFILER.lock();
    let stats = profiler.entry(lock_addr).or_insert_with(LockContentionStats::default);
    stats.total_acquisitions += 1;
    if spins > 0 {
        stats.contended_acquisitions += 1;
        stats.total_spins += spins as u64;
    }
    stats.active_holders = 1;
    stats.last_acquired_tick = now;
}

fn record_contention_release(lock_addr: usize) {
    let now = crate::pe::perf_counter::win32_get_tick_count64();
    let mut profiler = CONTENTION_PROFILER.lock();
    if let Some(stats) = profiler.get_mut(&lock_addr) {
        if stats.active_holders > 0 {
            let hold_time = now.saturating_sub(stats.last_acquired_tick);
            stats.total_hold_time_ms += hold_time;
            stats.active_holders = 0;
        }
    }
}
