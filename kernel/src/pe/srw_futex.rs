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
