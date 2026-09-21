//! Win32 Waitable Objects & Multi-Thread Synchronization Dispatcher.
//!
//! Emulates Windows waitable object dispatching, single/multi-object wait multiplexing,
//! alertable waits, user APC delivery, and waitable timers.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports)]

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;

use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_SUCCESS,
};
use crate::pe::synch::{
    INFINITE, MAXIMUM_WAIT_OBJECTS, WAIT_ABANDONED_0, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};

type HANDLE = u64;

pub const WAIT_IO_COMPLETION: u32 = 0x0000_00C0;

/// Synchronization event descriptor.
#[derive(Debug, Clone)]
pub struct WaitEvent {
    pub manual_reset: bool,
    pub signaled: bool,
    pub name: Option<String>,
}

/// Mutual exclusion lock descriptor.
#[derive(Debug, Clone)]
pub struct WaitMutex {
    pub owner_tid: u32,
    pub recursion_count: u32,
    pub abandoned: bool,
    pub name: Option<String>,
}

/// Counting semaphore descriptor.
#[derive(Debug, Clone)]
pub struct WaitSemaphore {
    pub current_count: i32,
    pub maximum_count: i32,
    pub name: Option<String>,
}

/// High-resolution waitable timer descriptor.
#[derive(Debug, Clone)]
pub struct WaitTimer {
    pub manual_reset: bool,
    pub signaled: bool,
    pub due_tick: u64,
    pub period_ms: u32,
    pub active: bool,
    pub name: Option<String>,
}

/// Thread execution state waitable descriptor.
#[derive(Debug, Clone)]
pub struct WaitThread {
    pub tid: u32,
    pub terminated: bool,
    pub exit_code: u32,
}

/// Process execution state waitable descriptor.
#[derive(Debug, Clone)]
pub struct WaitProcess {
    pub pid: u32,
    pub terminated: bool,
    pub exit_code: u32,
}

/// Unified waitable object enum.
#[derive(Debug, Clone)]
pub enum WaitableObject {
    Event(WaitEvent),
    Mutex(WaitMutex),
    Semaphore(WaitSemaphore),
    Timer(WaitTimer),
    Thread(WaitThread),
    Process(WaitProcess),
}

/// Queued User APC (Asynchronous Procedure Call) callback entry.
#[derive(Debug, Clone, Copy)]
pub struct ApcEntry {
    pub pfn_apc: usize,
    pub dw_data: usize,
}

pub static WAITABLE_OBJECTS: Spinlock<BTreeMap<HANDLE, WaitableObject>> =
    Spinlock::new(BTreeMap::new());

pub static NAMED_WAITABLES: Spinlock<BTreeMap<String, HANDLE>> =
    Spinlock::new(BTreeMap::new());

pub static THREAD_APC_QUEUES: Spinlock<BTreeMap<u32, VecDeque<ApcEntry>>> =
    Spinlock::new(BTreeMap::new());

pub static NEXT_WAIT_HANDLE: Spinlock<HANDLE> = Spinlock::new(0x0008_0001);

/// Returns the calling thread ID.
pub fn get_current_wait_tid() -> u32 {
    let tid = crate::process::scheduler::current_thread_id() as u32;
    if tid == 0 {
        1
    } else {
        tid
    }
}

/// Creates a new waitable event.
pub fn win32_create_waitable_event(
    manual_reset: bool,
    initial_state: bool,
    name: Option<&str>,
) -> HANDLE {
    let mut named = NAMED_WAITABLES.lock();
    if let Some(n) = name {
        if let Some(&h) = named.get(n) {
            win32_set_last_error(ERROR_ALREADY_EXISTS);
            return h;
        }
    }

    let mut h_lock = NEXT_WAIT_HANDLE.lock();
    let handle = *h_lock;
    *h_lock = handle.saturating_add(1);
    drop(h_lock);

    let ev = WaitEvent {
        manual_reset,
        signaled: initial_state,
        name: name.map(String::from),
    };

    WAITABLE_OBJECTS.lock().insert(handle, WaitableObject::Event(ev));
    if let Some(n) = name {
        named.insert(String::from(n), handle);
    }
    win32_set_last_error(ERROR_SUCCESS);
    handle
}

/// Creates a new waitable mutex.
pub fn win32_create_waitable_mutex(initial_owner: bool, name: Option<&str>) -> HANDLE {
    let mut named = NAMED_WAITABLES.lock();
    if let Some(n) = name {
        if let Some(&h) = named.get(n) {
            win32_set_last_error(ERROR_ALREADY_EXISTS);
            return h;
        }
    }

    let mut h_lock = NEXT_WAIT_HANDLE.lock();
    let handle = *h_lock;
    *h_lock = handle.saturating_add(1);
    drop(h_lock);

    let (owner_tid, recursion_count) = if initial_owner {
        (get_current_wait_tid(), 1)
    } else {
        (0, 0)
    };

    let m = WaitMutex {
        owner_tid,
        recursion_count,
        abandoned: false,
        name: name.map(String::from),
    };

    WAITABLE_OBJECTS.lock().insert(handle, WaitableObject::Mutex(m));
    if let Some(n) = name {
        named.insert(String::from(n), handle);
    }
    win32_set_last_error(ERROR_SUCCESS);
    handle
}

/// Creates a new waitable semaphore.
pub fn win32_create_waitable_semaphore(
    initial_count: i32,
    maximum_count: i32,
    name: Option<&str>,
) -> HANDLE {
    if initial_count < 0 || maximum_count <= 0 || initial_count > maximum_count {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }

    let mut named = NAMED_WAITABLES.lock();
    if let Some(n) = name {
        if let Some(&h) = named.get(n) {
            win32_set_last_error(ERROR_ALREADY_EXISTS);
            return h;
        }
    }

    let mut h_lock = NEXT_WAIT_HANDLE.lock();
    let handle = *h_lock;
    *h_lock = handle.saturating_add(1);
    drop(h_lock);

    let sem = WaitSemaphore {
        current_count: initial_count,
        maximum_count,
        name: name.map(String::from),
    };

    WAITABLE_OBJECTS.lock().insert(handle, WaitableObject::Semaphore(sem));
    if let Some(n) = name {
        named.insert(String::from(n), handle);
    }
    win32_set_last_error(ERROR_SUCCESS);
    handle
}

/// Creates or opens a waitable timer object (ANSI).
pub unsafe fn CreateWaitableTimerA(
    _lp_timer_attributes: usize,
    b_manual_reset: i32,
    lp_timer_name: *const u8,
) -> HANDLE {
    let name = if lp_timer_name.is_null() {
        None
    } else {
        let mut len = 0;
        while *lp_timer_name.add(len) != 0 && len < 260 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(lp_timer_name, len);
        core::str::from_utf8(slice).ok()
    };

    let mut named = NAMED_WAITABLES.lock();
    if let Some(n) = name {
        if let Some(&h) = named.get(n) {
            win32_set_last_error(ERROR_ALREADY_EXISTS);
            return h;
        }
    }

    let mut h_lock = NEXT_WAIT_HANDLE.lock();
    let handle = *h_lock;
    *h_lock = handle.saturating_add(1);
    drop(h_lock);

    let timer = WaitTimer {
        manual_reset: b_manual_reset != 0,
        signaled: false,
        due_tick: 0,
        period_ms: 0,
        active: false,
        name: name.map(String::from),
    };

    WAITABLE_OBJECTS.lock().insert(handle, WaitableObject::Timer(timer));
    if let Some(n) = name {
        named.insert(String::from(n), handle);
    }
    win32_set_last_error(ERROR_SUCCESS);
    handle
}

/// Activates the specified waitable timer.
pub unsafe fn SetWaitableTimer(
    h_timer: HANDLE,
    p_due_time_ms: u64,
    l_period_ms: u32,
) -> bool {
    let now = crate::pe::perf_counter::win32_get_tick_count64();
    let due_tick = now.saturating_add(p_due_time_ms);

    let mut table = WAITABLE_OBJECTS.lock();
    if let Some(WaitableObject::Timer(ref mut t)) = table.get_mut(&h_timer) {
        t.due_tick = due_tick;
        t.period_ms = l_period_ms;
        t.signaled = false;
        t.active = true;
        win32_set_last_error(ERROR_SUCCESS);
        true
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        false
    }
}

/// Sets the specified waitable timer to the inactive state.
pub fn CancelWaitableTimer(h_timer: HANDLE) -> bool {
    let mut table = WAITABLE_OBJECTS.lock();
    if let Some(WaitableObject::Timer(ref mut t)) = table.get_mut(&h_timer) {
        t.active = false;
        t.signaled = false;
        win32_set_last_error(ERROR_SUCCESS);
        true
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        false
    }
}

/// Registers a thread execution waitable handle.
pub fn win32_register_thread_waitable(tid: u32) -> HANDLE {
    let mut h_lock = NEXT_WAIT_HANDLE.lock();
    let handle = *h_lock;
    *h_lock = handle.saturating_add(1);
    drop(h_lock);

    let thread = WaitThread {
        tid,
        terminated: false,
        exit_code: 0,
    };
    WAITABLE_OBJECTS.lock().insert(handle, WaitableObject::Thread(thread));
    handle
}

/// Marks a thread as terminated, signaling any waiting threads.
pub fn win32_set_thread_terminated(tid: u32, exit_code: u32) {
    let mut table = WAITABLE_OBJECTS.lock();
    for obj in table.values_mut() {
        if let WaitableObject::Thread(ref mut th) = obj {
            if th.tid == tid {
                th.terminated = true;
                th.exit_code = exit_code;
            }
        }
    }
}

/// Updates active timers against current hardware tick count.
pub fn update_active_waitable_timers(now_ticks: u64) {
    let mut table = WAITABLE_OBJECTS.lock();
    for obj in table.values_mut() {
        if let WaitableObject::Timer(ref mut t) = obj {
            if t.active && !t.signaled && now_ticks >= t.due_tick {
                t.signaled = true;
                if t.period_ms > 0 {
                    t.due_tick = now_ticks.saturating_add(t.period_ms as u64);
                } else if !t.manual_reset {
                    t.active = false;
                }
            }
        }
    }
}

/// Evaluates if a waitable object is in the signaled state.
pub fn is_waitable_signaled(obj: &WaitableObject, cur_tid: u32) -> bool {
    match obj {
        WaitableObject::Event(ev) => ev.signaled,
        WaitableObject::Mutex(m) => m.owner_tid == 0 || m.owner_tid == cur_tid || m.abandoned,
        WaitableObject::Semaphore(sem) => sem.current_count > 0,
        WaitableObject::Timer(t) => t.signaled,
        WaitableObject::Thread(th) => th.terminated,
        WaitableObject::Process(pr) => pr.terminated,
    }
}

/// Satisfies the wait on an object, updating its internal state.
pub fn satisfy_waitable_object(obj: &mut WaitableObject, cur_tid: u32) -> bool {
    match obj {
        WaitableObject::Event(ev) => {
            if !ev.manual_reset {
                ev.signaled = false;
            }
            false
        }
        WaitableObject::Mutex(m) => {
            let was_abandoned = m.abandoned;
            m.abandoned = false;
            m.owner_tid = cur_tid;
            m.recursion_count = m.recursion_count.saturating_add(1);
            was_abandoned
        }
        WaitableObject::Semaphore(sem) => {
            sem.current_count = sem.current_count.saturating_sub(1);
            false
        }
        WaitableObject::Timer(t) => {
            if !t.manual_reset {
                t.signaled = false;
            }
            false
        }
        WaitableObject::Thread(_) => false,
        WaitableObject::Process(_) => false,
    }
}

/// Dispatches wait evaluation across handles with WaitAny and WaitAll barrier semantics.
pub fn evaluate_wait_handles(
    handles: &[HANDLE],
    wait_all: bool,
    cur_tid: u32,
) -> Option<u32> {
    let mut table = WAITABLE_OBJECTS.lock();

    // Verify all handles exist
    for &h in handles {
        if !table.contains_key(&h) {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return Some(WAIT_FAILED);
        }
    }

    if wait_all {
        let mut all_signaled = true;
        for &h in handles {
            if let Some(obj) = table.get(&h) {
                if !is_waitable_signaled(obj, cur_tid) {
                    all_signaled = false;
                    break;
                }
            }
        }

        if all_signaled {
            let mut abandoned = false;
            for &h in handles {
                if let Some(obj) = table.get_mut(&h) {
                    if satisfy_waitable_object(obj, cur_tid) {
                        abandoned = true;
                    }
                }
            }
            return Some(if abandoned { WAIT_ABANDONED_0 } else { WAIT_OBJECT_0 });
        }
    } else {
        for (idx, &h) in handles.iter().enumerate() {
            if let Some(obj) = table.get(&h) {
                if is_waitable_signaled(obj, cur_tid) {
                    let mut abandoned = false;
                    if let Some(mut_obj) = table.get_mut(&h) {
                        abandoned = satisfy_waitable_object(mut_obj, cur_tid);
                    }
                    let base = if abandoned { WAIT_ABANDONED_0 } else { WAIT_OBJECT_0 };
                    return Some(base + idx as u32);
                }
            }
        }
    }

    None
}
