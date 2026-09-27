//! Win32 High-Resolution Timer Queues & Multimedia Audio-Video Synchronization.
//!
//! Emulates Windows NT timer queues, worker thread execution flags,
//! sub-millisecond timer dispatching, and multimedia timer period governors.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_SUCCESS, INVALID_HANDLE_VALUE,
};
pub use crate::pe::synch::{HANDLE, WAIT_TIMEOUT};
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

pub const WT_EXECUTEDEFAULT: u32 = 0x0000_0000;
pub const WT_EXECUTEINIOTHREAD: u32 = 0x0000_0001;
pub const WT_EXECUTEINUITHREAD: u32 = 0x0000_0002;
pub const WT_EXECUTEINWAITTHREAD: u32 = 0x0000_0004;
pub const WT_EXECUTEONLYONCE: u32 = 0x0000_0008;
pub const WT_EXECUTELONGFUNCTION: u32 = 0x0000_0010;
pub const WT_EXECUTEINTIMERTHREAD: u32 = 0x0000_0020;
pub const WT_TRANSFER_IMPERSONATION: u32 = 0x0000_0100;

pub type WAITORTIMERCALLBACK =
    Option<unsafe extern "system" fn(lpParameter: *mut c_void, TimerOrWaitFired: BOOL)>;

/// Descriptor for an individual timer within a timer queue.
pub struct TimerQueueTimerInner {
    pub id: HANDLE,
    pub queue_id: HANDLE,
    pub callback: usize,
    pub parameter: usize,
    pub due_time_ms: u32,
    pub period_ms: u32,
    pub flags: u32,
    pub next_fire_tick: u64,
    pub fired_count: u32,
    pub is_active: bool,
    pub deleted: bool,
}

/// Internal state descriptor for a Win32 timer queue.
pub struct TimerQueueInner {
    pub id: HANDLE,
    pub timers: Vec<HANDLE>,
    pub closed: bool,
}

static NEXT_TIMER_QUEUE_ID: AtomicU64 = AtomicU64::new(0x000E_1000);
pub static TIMER_QUEUES: Spinlock<BTreeMap<HANDLE, TimerQueueInner>> =
    Spinlock::new(BTreeMap::new());
pub static TIMER_QUEUE_TIMERS: Spinlock<BTreeMap<HANDLE, TimerQueueTimerInner>> =
    Spinlock::new(BTreeMap::new());

/// Creates a queue for timer callbacks executed by worker threads.
pub unsafe extern "system" fn CreateTimerQueue() -> HANDLE {
    let id = NEXT_TIMER_QUEUE_ID.fetch_add(1, Ordering::SeqCst) as HANDLE;
    let queue = TimerQueueInner {
        id,
        timers: Vec::new(),
        closed: false,
    };

    TIMER_QUEUES.lock().insert(id, queue);
    win32_set_last_error(ERROR_SUCCESS);
    id
}
