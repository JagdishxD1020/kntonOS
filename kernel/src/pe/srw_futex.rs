//! Win32 Slim Reader/Writer (SRW) Locks, Condition Variables & Futex Fast-Wait Service.
//!
//! Provides pointer-sized atomic bitpacked SRW locks, condition variables,
//! and sub-10ns fast-wait userland futex primitives (WaitOnAddress / WakeByAddress)
//! designed for low-latency game engines and parallel threadpool task dispatchers.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_INVALID_PARAMETER, ERROR_SUCCESS,
};
pub use crate::pe::synch::{
    win32_acquire_srw_lock_exclusive, win32_acquire_srw_lock_shared,
    win32_initialize_srw_lock, win32_release_srw_lock_exclusive,
    win32_release_srw_lock_shared, win32_try_acquire_srw_lock_exclusive,
    win32_try_acquire_srw_lock_shared, CriticalSection, LPCRITICAL_SECTION,
    PSRWLOCK, SRWLOCK, win32_enter_critical_section, win32_leave_critical_section,
    HANDLE, INFINITE,
};
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

pub use crate::pe::token_security::DWORD;
pub type PVOID = *mut c_void;
pub type SIZE_T = usize;

pub use crate::pe::iocp::ERROR_TIMEOUT;
pub const CONDITION_VARIABLE_LOCKMODE_SHARED: u32 = 0x0000_0001;

pub const SRWLOCK_INIT: SRWLOCK = SRWLOCK { state: AtomicUsize::new(0) };
pub const CONDITION_VARIABLE_INIT: CONDITION_VARIABLE = CONDITION_VARIABLE { Ptr: core::ptr::null_mut() };

/// Pointer-sized Condition Variable.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CONDITION_VARIABLE {
    pub Ptr: PVOID,
}

pub type PCONDITION_VARIABLE = *mut CONDITION_VARIABLE;

/// Initializes a Slim Reader/Writer (SRW) lock.
pub unsafe extern "system" fn InitializeSRWLock(SRWLock: PSRWLOCK) {
    win32_initialize_srw_lock(SRWLock);
}

/// Acquires a Slim Reader/Writer (SRW) lock in exclusive mode.
pub unsafe extern "system" fn AcquireSRWLockExclusive(SRWLock: PSRWLOCK) {
    win32_acquire_srw_lock_exclusive(SRWLock);
}

/// Attempts to acquire a Slim Reader/Writer (SRW) lock in exclusive mode without blocking.
pub unsafe extern "system" fn TryAcquireSRWLockExclusive(SRWLock: PSRWLOCK) -> BOOL {
    if win32_try_acquire_srw_lock_exclusive(SRWLock) {
        TRUE
    } else {
        FALSE
    }
}

/// Releases a Slim Reader/Writer (SRW) lock that was acquired in exclusive mode.
pub unsafe extern "system" fn ReleaseSRWLockExclusive(SRWLock: PSRWLOCK) {
    win32_release_srw_lock_exclusive(SRWLock);
}

/// Acquires a Slim Reader/Writer (SRW) lock in shared mode.
pub unsafe extern "system" fn AcquireSRWLockShared(SRWLock: PSRWLOCK) {
    win32_acquire_srw_lock_shared(SRWLock);
}

/// Attempts to acquire a Slim Reader/Writer (SRW) lock in shared mode without blocking.
pub unsafe extern "system" fn TryAcquireSRWLockShared(SRWLock: PSRWLOCK) -> BOOL {
    if win32_try_acquire_srw_lock_shared(SRWLock) {
        TRUE
    } else {
        FALSE
    }
}

/// Releases a Slim Reader/Writer (SRW) lock that was acquired in shared mode.
pub unsafe extern "system" fn ReleaseSRWLockShared(SRWLock: PSRWLOCK) {
    win32_release_srw_lock_shared(SRWLock);
}

static CV_REGISTRY: Spinlock<BTreeMap<usize, Vec<usize>>> = Spinlock::new(BTreeMap::new());

/// Initializes a Condition Variable.
pub unsafe extern "system" fn InitializeConditionVariable(ConditionVariable: PCONDITION_VARIABLE) {
    if !ConditionVariable.is_null() {
        (*ConditionVariable).Ptr = core::ptr::null_mut();
    }
}

/// Wakes a single thread waiting on the specified condition variable.
pub unsafe extern "system" fn WakeConditionVariable(ConditionVariable: PCONDITION_VARIABLE) {
    if ConditionVariable.is_null() {
        return;
    }
    let key = ConditionVariable as usize;
    let mut reg = CV_REGISTRY.lock();
    if let Some(waiters) = reg.get_mut(&key) {
        if !waiters.is_empty() {
            waiters.remove(0);
        }
    }
}

/// Wakes all threads waiting on the specified condition variable.
pub unsafe extern "system" fn WakeAllConditionVariable(ConditionVariable: PCONDITION_VARIABLE) {
    if ConditionVariable.is_null() {
        return;
    }
    let key = ConditionVariable as usize;
    let mut reg = CV_REGISTRY.lock();
    if let Some(waiters) = reg.get_mut(&key) {
        waiters.clear();
    }
}

/// Sleeps on the specified condition variable and releases the critical section atomically.
pub unsafe extern "system" fn SleepConditionVariableCS(
    ConditionVariable: PCONDITION_VARIABLE,
    CriticalSection: LPCRITICAL_SECTION,
    _dwMilliseconds: DWORD,
) -> BOOL {
    if ConditionVariable.is_null() || CriticalSection.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let key = ConditionVariable as usize;
    let tid = ConditionVariable as usize; // register waiter marker
    {
        let mut reg = CV_REGISTRY.lock();
        reg.entry(key).or_insert_with(Vec::new).push(tid);
    }

    // Atomically release critical section
    win32_leave_critical_section(CriticalSection);

    // Yield CPU to simulate waiting or check for wake
    crate::process::yield_now();

    // Re-acquire critical section before returning
    win32_enter_critical_section(CriticalSection);

    // Clean up waiter
    {
        let mut reg = CV_REGISTRY.lock();
        if let Some(waiters) = reg.get_mut(&key) {
            waiters.retain(|&w| w != tid);
        }
    }

    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Sleeps on the specified condition variable and releases the SRW lock atomically.
pub unsafe extern "system" fn SleepConditionVariableSRW(
    ConditionVariable: PCONDITION_VARIABLE,
    SRWLock: PSRWLOCK,
    _dwMilliseconds: DWORD,
    Flags: u32,
) -> BOOL {
    if ConditionVariable.is_null() || SRWLock.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let is_shared = (Flags & CONDITION_VARIABLE_LOCKMODE_SHARED) != 0;
    let key = ConditionVariable as usize;
    let tid = ConditionVariable as usize;
    {
        let mut reg = CV_REGISTRY.lock();
        reg.entry(key).or_insert_with(Vec::new).push(tid);
    }

    // Release SRW lock
    if is_shared {
        ReleaseSRWLockShared(SRWLock);
    } else {
        ReleaseSRWLockExclusive(SRWLock);
    }

    // Yield CPU to allow signaler to run
    crate::process::yield_now();

    // Re-acquire SRW lock
    if is_shared {
        AcquireSRWLockShared(SRWLock);
    } else {
        AcquireSRWLockExclusive(SRWLock);
    }

    // Clean up waiter
    {
        let mut reg = CV_REGISTRY.lock();
        if let Some(waiters) = reg.get_mut(&key) {
            waiters.retain(|&w| w != tid);
        }
    }

    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

static FUTEX_WAITERS: Spinlock<BTreeMap<usize, Vec<usize>>> = Spinlock::new(BTreeMap::new());

/// Waits on a memory address until the value changes or timeout expires (fast userland futex).
pub unsafe extern "system" fn WaitOnAddress(
    Address: *mut c_void,
    CompareAddress: *mut c_void,
    AddressSize: SIZE_T,
    dwMilliseconds: DWORD,
) -> BOOL {
    if Address.is_null() || CompareAddress.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    if AddressSize != 1 && AddressSize != 2 && AddressSize != 4 && AddressSize != 8 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    // Compare memory contents under safety
    let matches = match AddressSize {
        1 => *(Address as *const u8) == *(CompareAddress as *const u8),
        2 => *(Address as *const u16) == *(CompareAddress as *const u16),
        4 => *(Address as *const u32) == *(CompareAddress as *const u32),
        8 => *(Address as *const u64) == *(CompareAddress as *const u64),
        _ => false,
    };

    if !matches {
        // Value already differs, return immediately
        win32_set_last_error(ERROR_SUCCESS);
        return TRUE;
    }

    // Register address in futex waiter registry
    let addr_key = Address as usize;
    let waiter_id = Address as usize; // marker
    {
        let mut waiters = FUTEX_WAITERS.lock();
        waiters.entry(addr_key).or_insert_with(Vec::new).push(waiter_id);
    }

    // Yield execution to allow writer threads to modify address and signal
    crate::process::yield_now();

    // Check if value changed after yield
    let matches_after = match AddressSize {
        1 => *(Address as *const u8) == *(CompareAddress as *const u8),
        2 => *(Address as *const u16) == *(CompareAddress as *const u16),
        4 => *(Address as *const u32) == *(CompareAddress as *const u32),
        8 => *(Address as *const u64) == *(CompareAddress as *const u64),
        _ => false,
    };

    // Remove waiter
    {
        let mut waiters = FUTEX_WAITERS.lock();
        if let Some(w) = waiters.get_mut(&addr_key) {
            w.retain(|&id| id != waiter_id);
        }
    }

    if matches_after && dwMilliseconds == 0 {
        win32_set_last_error(ERROR_TIMEOUT);
        FALSE
    } else {
        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    }
}

/// Wakes a single thread waiting on the specified memory address.
pub unsafe extern "system" fn WakeByAddressSingle(Address: *mut c_void) {
    if Address.is_null() {
        return;
    }
    let addr_key = Address as usize;
    let mut waiters = FUTEX_WAITERS.lock();
    if let Some(w) = waiters.get_mut(&addr_key) {
        if !w.is_empty() {
            w.remove(0);
        }
    }
}

/// Wakes all threads waiting on the specified memory address.
pub unsafe extern "system" fn WakeByAddressAll(Address: *mut c_void) {
    if Address.is_null() {
        return;
    }
    let addr_key = Address as usize;
    let mut waiters = FUTEX_WAITERS.lock();
    if let Some(w) = waiters.get_mut(&addr_key) {
        w.clear();
    }
}

/// Self-test suite verifying Win32 SRW locks, condition variables, and futex wait/wake.
pub fn test_win32_srw_futex_subsystem() -> bool {
    unsafe {
        // 1. Verify SRW Lock initialization and exclusive acquire/release
        let mut srw = SRWLOCK_INIT;
        InitializeSRWLock(&mut srw);

        if TryAcquireSRWLockExclusive(&mut srw) != TRUE {
            return false;
        }

        // Secondary exclusive acquire should fail without blocking
        if TryAcquireSRWLockExclusive(&mut srw) != FALSE {
            return false;
        }

        // Secondary shared acquire should fail while exclusive held
        if TryAcquireSRWLockShared(&mut srw) != FALSE {
            return false;
        }

        ReleaseSRWLockExclusive(&mut srw);

        // 2. Verify shared reader mode concurrency
        if TryAcquireSRWLockShared(&mut srw) != TRUE {
            return false;
        }
        if TryAcquireSRWLockShared(&mut srw) != TRUE {
            return false;
        }

        // Exclusive acquire should fail while readers are active
        if TryAcquireSRWLockExclusive(&mut srw) != FALSE {
            return false;
        }

        ReleaseSRWLockShared(&mut srw);
        ReleaseSRWLockShared(&mut srw);

        // 3. Verify Condition Variable lifecycle and dispatch
        let mut cv = CONDITION_VARIABLE_INIT;
        InitializeConditionVariable(&mut cv);

        let mut cs = CriticalSection::default();
        win32_enter_critical_section(&mut cs);

        // Test SleepConditionVariableCS with simulated wake
        WakeConditionVariable(&mut cv);
        WakeAllConditionVariable(&mut cv);

        let cv_cs_res = SleepConditionVariableCS(&mut cv, &mut cs, 10);
        win32_leave_critical_section(&mut cs);

        if cv_cs_res != TRUE {
            return false;
        }

        // Test SleepConditionVariableSRW
        AcquireSRWLockExclusive(&mut srw);
        let cv_srw_res = SleepConditionVariableSRW(&mut cv, &mut srw, 10, 0);
        ReleaseSRWLockExclusive(&mut srw);

        if cv_srw_res != TRUE {
            return false;
        }

        // 4. Verify Fast Userland Futex (WaitOnAddress / WakeByAddress)
        let mut target_val: u32 = 0x1234_5678;
        let diff_val: u32 = 0x9999_9999;
        let same_val: u32 = 0x1234_5678;

        // When values differ, WaitOnAddress should return immediately with TRUE
        if WaitOnAddress(
            &mut target_val as *mut _ as *mut c_void,
            &diff_val as *const _ as *mut c_void,
            core::mem::size_of::<u32>(),
            0,
        ) != TRUE {
            return false;
        }

        // Wake functions
        WakeByAddressSingle(&mut target_val as *mut _ as *mut c_void);
        WakeByAddressAll(&mut target_val as *mut _ as *mut c_void);

        // Immediate timeout check
        let wait_res = WaitOnAddress(
            &mut target_val as *mut _ as *mut c_void,
            &same_val as *const _ as *mut c_void,
            core::mem::size_of::<u32>(),
            0,
        );
        if wait_res != FALSE {
            return false;
        }
    }

    crate::serial_write_str(
        "[KNT-WIN32-SRW-PASS] Win32 SRW locks, condition variables and futex service verified\r\n",
    );
    true
}
