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

pub use crate::pe::perf_counter::{TIMERR_NOCANDO, TIMERR_NOERROR, TIMERR_STRUCT};

pub const TIME_ONESHOT: u32 = 0x0000;
pub const TIME_PERIODIC: u32 = 0x0001;
pub const TIME_CALLBACK_FUNCTION: u32 = 0x0000;
pub const TIME_CALLBACK_EVENT_SET: u32 = 0x0010;
pub const TIME_CALLBACK_EVENT_PULSE: u32 = 0x0020;
pub const TIME_KILL_SYNCHRONOUS: u32 = 0x0100;

pub type LPTIMECALLBACK = Option<
    unsafe extern "system" fn(
        uTimerID: u32,
        uMsg: u32,
        dwUser: usize,
        dw1: usize,
        dw2: usize,
    ),
>;

pub struct MultimediaTimerInner {
    pub id: u32,
    pub delay_ms: u32,
    pub resolution_ms: u32,
    pub callback: usize,
    pub user_data: usize,
    pub event_type: u32,
    pub next_fire_tick: u64,
    pub is_active: bool,
    pub fired_count: u32,
}

static MM_MIN_PERIOD_MS: Spinlock<u32> = Spinlock::new(1);
static MM_NEXT_TIMER_ID: AtomicU32 = AtomicU32::new(1001);
static MM_TIMERS: Spinlock<BTreeMap<u32, MultimediaTimerInner>> = Spinlock::new(BTreeMap::new());

/// Requests a minimum resolution for periodic timers.
pub unsafe extern "system" fn timeBeginPeriod(uPeriod: u32) -> u32 {
    let mut min_period = MM_MIN_PERIOD_MS.lock();
    if uPeriod >= 1 && uPeriod <= 1000 {
        if uPeriod < *min_period {
            *min_period = uPeriod;
        }
        TIMERR_NOERROR
    } else {
        TIMERR_NOCANDO
    }
}

/// Clears a previously set minimum timer resolution.
pub unsafe extern "system" fn timeEndPeriod(uPeriod: u32) -> u32 {
    if uPeriod >= 1 && uPeriod <= 1000 {
        let mut min_period = MM_MIN_PERIOD_MS.lock();
        if *min_period == uPeriod {
            *min_period = 1;
        }
        TIMERR_NOERROR
    } else {
        TIMERR_NOCANDO
    }
}

/// Starts a specified multimedia timer event.
pub unsafe extern "system" fn timeSetEvent(
    uDelay: u32,
    uResolution: u32,
    lpTimeProc: LPTIMECALLBACK,
    dwUser: usize,
    fuEvent: u32,
) -> u32 {
    let id = MM_NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
    let now = crate::pe::perf_counter::win32_get_tick_count64();
    let delay = if uDelay == 0 { 1 } else { uDelay };
    let cb_addr = match lpTimeProc {
        Some(f) => f as usize,
        None => 0,
    };

    let timer = MultimediaTimerInner {
        id,
        delay_ms: delay,
        resolution_ms: uResolution,
        callback: cb_addr,
        user_data: dwUser,
        event_type: fuEvent,
        next_fire_tick: now + delay as u64,
        is_active: true,
        fired_count: 0,
    };

    MM_TIMERS.lock().insert(id, timer);
    id
}

/// Cancels a specified multimedia timer event.
pub unsafe extern "system" fn timeKillEvent(uTimerID: u32) -> u32 {
    let mut timers = MM_TIMERS.lock();
    if timers.remove(&uTimerID).is_some() {
        TIMERR_NOERROR
    } else {
        TIMERR_NOCANDO
    }
}

/// Scans and executes callbacks for due multimedia timers.
pub fn dispatch_multimedia_timers(current_tick: u64) -> usize {
    let mut due_callbacks = Vec::new();

    let mut timers = MM_TIMERS.lock();
    for timer in timers.values_mut() {
        if timer.is_active && current_tick >= timer.next_fire_tick {
            timer.fired_count += 1;
            due_callbacks.push((timer.id, timer.callback, timer.user_data, timer.event_type));

            if (timer.event_type & TIME_PERIODIC) != 0 {
                timer.next_fire_tick = current_tick + timer.delay_ms as u64;
            } else {
                timer.is_active = false;
            }
        }
    }
    drop(timers);

    let fired_count = due_callbacks.len();
    for (tid, cb_addr, user_data, ev_type) in due_callbacks {
        if (ev_type & TIME_CALLBACK_EVENT_SET) != 0 {
            if cb_addr != 0 {
                crate::pe::synch::win32_set_event(cb_addr as HANDLE);
            }
        } else if cb_addr != 0 {
            unsafe {
                let cb: unsafe extern "system" fn(u32, u32, usize, usize, usize) =
                    core::mem::transmute(cb_addr);
                cb(tid, 0, user_data, 0, 0);
            }
        }
    }

    fired_count
}

/// Returns the current minimum period governor resolution in milliseconds.
pub fn win32_get_multimedia_period_governor() -> u32 {
    *MM_MIN_PERIOD_MS.lock()
}

static TEST_TIMER_CALLBACK_FIRED: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn test_timer_callback(_param: *mut c_void, _timer_or_wait: BOOL) {
    TEST_TIMER_CALLBACK_FIRED.fetch_add(1, Ordering::SeqCst);
}

static TEST_MM_CALLBACK_FIRED: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn test_mm_callback(
    _timer_id: u32,
    _msg: u32,
    _user: usize,
    _dw1: usize,
    _dw2: usize,
) {
    TEST_MM_CALLBACK_FIRED.fetch_add(1, Ordering::SeqCst);
}

/// Self-test suite verifying Win32 timer queues and multimedia timer synchronization.
pub fn test_win32_timer_queue_subsystem() -> bool {
    unsafe {
        // 1. Test CreateTimerQueue
        let tq = CreateTimerQueue();
        if tq == 0 {
            return false;
        }

        // 2. Test CreateTimerQueueTimer with periodic firing
        let mut timer_handle: HANDLE = 0;
        let created = CreateTimerQueueTimer(
            &mut timer_handle,
            tq,
            Some(test_timer_callback),
            core::ptr::null_mut(),
            10,
            20,
            WT_EXECUTEDEFAULT,
        );
        if created != TRUE || timer_handle == 0 {
            return false;
        }

        if !win32_is_timer_active(timer_handle) {
            return false;
        }

        // 3. Step virtual ticks and verify callback dispatching
        let initial_fires = win32_step_timer_queue_ticks(5);
        if initial_fires != 0 {
            return false;
        }

        let due_fires = win32_step_timer_queue_ticks(15);
        if due_fires != 1 || TEST_TIMER_CALLBACK_FIRED.load(Ordering::SeqCst) < 1 {
            return false;
        }

        // 4. Test ChangeTimerQueueTimer
        let changed = ChangeTimerQueueTimer(tq, timer_handle, 5, 10);
        if changed != TRUE {
            return false;
        }

        // 5. Test DeleteTimerQueueTimer with event notification
        let comp_event = crate::pe::synch::win32_create_event(0, false, false, None);
        let deleted = DeleteTimerQueueTimer(tq, timer_handle, comp_event);
        if deleted != TRUE {
            return false;
        }

        if win32_is_timer_active(timer_handle) {
            return false;
        }

        // Verify completion event was signaled
        if crate::pe::synch::win32_wait_for_single_object(comp_event, 0) != 0 {
            return false;
        }

        // 6. Test DeleteTimerQueueEx
        let tq_del = DeleteTimerQueueEx(tq, 0);
        if tq_del != TRUE {
            return false;
        }

        // 7. Test Multimedia Timer period governor
        if timeBeginPeriod(1) != TIMERR_NOERROR {
            return false;
        }
        if win32_get_multimedia_period_governor() != 1 {
            return false;
        }
        if timeEndPeriod(1) != TIMERR_NOERROR {
            return false;
        }

        // 8. Test timeSetEvent and timeKillEvent
        let mm_id = timeSetEvent(10, 1, Some(test_mm_callback), 42, TIME_PERIODIC);
        if mm_id == 0 {
            return false;
        }

        let now = crate::pe::perf_counter::win32_get_tick_count64() + 25;
        dispatch_multimedia_timers(now);
        if TEST_MM_CALLBACK_FIRED.load(Ordering::SeqCst) < 1 {
            return false;
        }

        if timeKillEvent(mm_id) != TIMERR_NOERROR {
            return false;
        }
    }

    crate::serial_write_str(
        "[KNT-WIN32-TIMER-QUEUE-PASS] Win32 timer queue and multimedia timer subsystem verified\r\n",
    );
    true
}
