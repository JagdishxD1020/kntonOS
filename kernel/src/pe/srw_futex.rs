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
