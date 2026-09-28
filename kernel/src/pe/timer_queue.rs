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

pub const DEFAULT_TIMER_QUEUE: HANDLE = 0x000E_0001;

/// Ensures the default process timer queue is registered and available.
fn ensure_default_timer_queue() -> HANDLE {
    let mut queues = TIMER_QUEUES.lock();
    if !queues.contains_key(&DEFAULT_TIMER_QUEUE) {
        queues.insert(
            DEFAULT_TIMER_QUEUE,
            TimerQueueInner {
                id: DEFAULT_TIMER_QUEUE,
                timers: Vec::new(),
                closed: false,
            },
        );
    }
    DEFAULT_TIMER_QUEUE
}

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

/// Creates a timer-queue timer and assigns it to the designated timer queue.
pub unsafe extern "system" fn CreateTimerQueueTimer(
    phNewTimer: *mut HANDLE,
    TimerQueue: HANDLE,
    Callback: WAITORTIMERCALLBACK,
    Parameter: *mut c_void,
    DueTime: u32,
    Period: u32,
    Flags: u32,
) -> BOOL {
    if phNewTimer.is_null() || Callback.is_none() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let q_id = if TimerQueue == 0 {
        ensure_default_timer_queue()
    } else {
        TimerQueue
    };

    let now = crate::pe::perf_counter::win32_get_tick_count64();
    let timer_id = NEXT_TIMER_QUEUE_ID.fetch_add(1, Ordering::SeqCst) as HANDLE;

    let timer = TimerQueueTimerInner {
        id: timer_id,
        queue_id: q_id,
        callback: core::mem::transmute(Callback),
        parameter: Parameter as usize,
        due_time_ms: DueTime,
        period_ms: Period,
        flags: Flags,
        next_fire_tick: now + DueTime as u64,
        fired_count: 0,
        is_active: true,
        deleted: false,
    };

    TIMER_QUEUE_TIMERS.lock().insert(timer_id, timer);

    let mut queues = TIMER_QUEUES.lock();
    if let Some(queue) = queues.get_mut(&q_id) {
        queue.timers.push(timer_id);
    } else {
        let mut new_queue = TimerQueueInner {
            id: q_id,
            timers: Vec::new(),
            closed: false,
        };
        new_queue.timers.push(timer_id);
        queues.insert(q_id, new_queue);
    }

    *phNewTimer = timer_id;
    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Updates the due time and period of an active timer-queue timer.
pub unsafe extern "system" fn ChangeTimerQueueTimer(
    _TimerQueue: HANDLE,
    Timer: HANDLE,
    DueTime: u32,
    Period: u32,
) -> BOOL {
    if Timer == 0 {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return FALSE;
    }

    let mut timers = TIMER_QUEUE_TIMERS.lock();
    if let Some(timer) = timers.get_mut(&Timer) {
        if timer.deleted {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }

        let now = crate::pe::perf_counter::win32_get_tick_count64();
        timer.due_time_ms = DueTime;
        timer.period_ms = Period;
        timer.next_fire_tick = now + DueTime as u64;
        timer.is_active = true;

        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        FALSE
    }
}

/// Cancels and deletes a timer-queue timer, optionally signaling a completion event.
pub unsafe extern "system" fn DeleteTimerQueueTimer(
    TimerQueue: HANDLE,
    Timer: HANDLE,
    CompletionEvent: HANDLE,
) -> BOOL {
    if Timer == 0 {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return FALSE;
    }

    let q_id = if TimerQueue == 0 {
        DEFAULT_TIMER_QUEUE
    } else {
        TimerQueue
    };

    let removed = TIMER_QUEUE_TIMERS.lock().remove(&Timer);
    if removed.is_some() {
        let mut queues = TIMER_QUEUES.lock();
        if let Some(queue) = queues.get_mut(&q_id) {
            queue.timers.retain(|&id| id != Timer);
        }
        drop(queues);

        if CompletionEvent != 0 && CompletionEvent != (INVALID_HANDLE_VALUE as HANDLE) {
            crate::pe::synch::win32_set_event(CompletionEvent);
        }

        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        FALSE
    }
}

/// Destroys a timer queue and cancels all associated timers.
pub unsafe extern "system" fn DeleteTimerQueueEx(
    TimerQueue: HANDLE,
    CompletionEvent: HANDLE,
) -> BOOL {
    if TimerQueue == 0 {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return FALSE;
    }

    let mut queues = TIMER_QUEUES.lock();
    if let Some(queue) = queues.remove(&TimerQueue) {
        let mut timers = TIMER_QUEUE_TIMERS.lock();
        for tid in queue.timers {
            timers.remove(&tid);
        }
        drop(timers);

        if CompletionEvent != 0 && CompletionEvent != (INVALID_HANDLE_VALUE as HANDLE) {
            crate::pe::synch::win32_set_event(CompletionEvent);
        }

        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        FALSE
    }
}

static TOTAL_TIMERS_DISPATCHED: AtomicU64 = AtomicU64::new(0);

/// Scans timer queues and executes callbacks for all timers that have reached due time.
pub fn dispatch_timer_queue_timers(current_tick: u64) -> usize {
    let mut due_callbacks = Vec::new();

    let mut timers = TIMER_QUEUE_TIMERS.lock();
    for timer in timers.values_mut() {
        if timer.is_active && !timer.deleted && current_tick >= timer.next_fire_tick {
            timer.fired_count += 1;
            due_callbacks.push((timer.callback, timer.parameter));

            if (timer.flags & WT_EXECUTEONLYONCE) != 0 || timer.period_ms == 0 {
                timer.is_active = false;
            } else {
                timer.next_fire_tick = current_tick + timer.period_ms as u64;
            }
        }
    }
    drop(timers);

    let fired_count = due_callbacks.len();
    for (cb_addr, param) in due_callbacks {
        if cb_addr != 0 {
            unsafe {
                let cb: unsafe extern "system" fn(*mut c_void, BOOL) = core::mem::transmute(cb_addr);
                cb(param as *mut c_void, TRUE);
            }
            TOTAL_TIMERS_DISPATCHED.fetch_add(1, Ordering::SeqCst);
        }
    }

    fired_count
}

/// Advances virtual timer tick count and executes due timer queue callbacks.
pub fn win32_step_timer_queue_ticks(step_ms: u32) -> usize {
    let now = crate::pe::perf_counter::win32_get_tick_count64() + step_ms as u64;
    dispatch_timer_queue_timers(now)
}

/// Returns the number of times a timer-queue timer callback has fired.
pub fn win32_get_timer_fired_count(Timer: HANDLE) -> u32 {
    let timers = TIMER_QUEUE_TIMERS.lock();
    timers.get(&Timer).map(|t| t.fired_count).unwrap_or(0)
}

/// Returns whether a timer-queue timer is currently armed and active.
pub fn win32_is_timer_active(Timer: HANDLE) -> bool {
    let timers = TIMER_QUEUE_TIMERS.lock();
    timers.get(&Timer).map(|t| t.is_active && !t.deleted).unwrap_or(false)
}
