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

/// Waits until the specified object is in the signaled state or the time-out interval elapses.
pub unsafe fn WaitForSingleObject(h_handle: HANDLE, dw_milliseconds: u32) -> u32 {
    WaitForMultipleObjectsEx(1, &h_handle, 0, dw_milliseconds, 0)
}

/// Waits until one or all of the specified objects are in the signaled state or time-out occurs.
pub unsafe fn WaitForMultipleObjects(
    count: u32,
    lp_handles: *const HANDLE,
    b_wait_all: i32,
    dw_milliseconds: u32,
) -> u32 {
    WaitForMultipleObjectsEx(count, lp_handles, b_wait_all, dw_milliseconds, 0)
}

/// Waits until one or all of the specified objects are in the signaled state, an I/O completion
/// routine or APC is queued, or the time-out interval elapses.
pub unsafe fn WaitForMultipleObjectsEx(
    count: u32,
    lp_handles: *const HANDLE,
    b_wait_all: i32,
    dw_milliseconds: u32,
    b_alertable: i32,
) -> u32 {
    if count == 0 || count > MAXIMUM_WAIT_OBJECTS || lp_handles.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return WAIT_FAILED;
    }

    let handles = core::slice::from_raw_parts(lp_handles, count as usize);
    let cur_tid = get_current_wait_tid();
    let wait_all = b_wait_all != 0;
    let alertable = b_alertable != 0;

    let start_tick = crate::pe::perf_counter::win32_get_tick_count64();
    let timeout_tick = if dw_milliseconds == INFINITE {
        u64::MAX
    } else {
        start_tick.saturating_add(dw_milliseconds as u64)
    };

    loop {
        let now = crate::pe::perf_counter::win32_get_tick_count64();
        update_active_waitable_timers(now);

        // Check and dispatch queued User APCs if in alertable wait
        if alertable {
            let pending_apc = {
                let mut apc_table = THREAD_APC_QUEUES.lock();
                apc_table.get_mut(&cur_tid).and_then(|q| q.pop_front())
            };
            if let Some(apc) = pending_apc {
                if apc.pfn_apc != 0 {
                    let apc_fn: unsafe extern "system" fn(usize) = core::mem::transmute(apc.pfn_apc);
                    apc_fn(apc.dw_data);
                }
                win32_set_last_error(ERROR_SUCCESS);
                return WAIT_IO_COMPLETION;
            }
        }

        // Evaluate waitable handles
        if let Some(res) = evaluate_wait_handles(handles, wait_all, cur_tid) {
            win32_set_last_error(ERROR_SUCCESS);
            return res;
        }

        if dw_milliseconds == 0 {
            win32_set_last_error(ERROR_SUCCESS);
            return WAIT_TIMEOUT;
        }

        if dw_milliseconds != INFINITE && now >= timeout_tick {
            win32_set_last_error(ERROR_SUCCESS);
            return WAIT_TIMEOUT;
        }

        core::hint::spin_loop();
        crate::process::yield_now();
    }
}

/// Sets the specified waitable event object to the signaled state.
pub fn win32_set_waitable_event(h_event: HANDLE) -> bool {
    let mut table = WAITABLE_OBJECTS.lock();
    if let Some(WaitableObject::Event(ref mut ev)) = table.get_mut(&h_event) {
        ev.signaled = true;
        win32_set_last_error(ERROR_SUCCESS);
        true
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        false
    }
}

/// Sets the specified waitable event object to the nonsignaled state.
pub fn win32_reset_waitable_event(h_event: HANDLE) -> bool {
    let mut table = WAITABLE_OBJECTS.lock();
    if let Some(WaitableObject::Event(ref mut ev)) = table.get_mut(&h_event) {
        ev.signaled = false;
        win32_set_last_error(ERROR_SUCCESS);
        true
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        false
    }
}

/// Releases ownership of the specified waitable mutex object.
pub fn win32_release_waitable_mutex(h_mutex: HANDLE) -> bool {
    let cur_tid = get_current_wait_tid();
    let mut table = WAITABLE_OBJECTS.lock();
    if let Some(WaitableObject::Mutex(ref mut m)) = table.get_mut(&h_mutex) {
        if m.owner_tid != cur_tid && m.owner_tid != 0 {
            win32_set_last_error(ERROR_ACCESS_DENIED);
            return false;
        }
        if m.recursion_count > 0 {
            m.recursion_count -= 1;
            if m.recursion_count == 0 {
                m.owner_tid = 0;
            }
            win32_set_last_error(ERROR_SUCCESS);
            true
        } else {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            false
        }
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        false
    }
}

/// Increases the count of the specified waitable semaphore.
pub unsafe fn win32_release_waitable_semaphore(
    h_semaphore: HANDLE,
    release_count: i32,
    previous_count: *mut i32,
) -> bool {
    if release_count <= 0 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return false;
    }

    let mut table = WAITABLE_OBJECTS.lock();
    if let Some(WaitableObject::Semaphore(ref mut sem)) = table.get_mut(&h_semaphore) {
        let prev = sem.current_count;
        if prev.saturating_add(release_count) > sem.maximum_count {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return false;
        }
        sem.current_count += release_count;
        if !previous_count.is_null() {
            *previous_count = prev;
        }
        win32_set_last_error(ERROR_SUCCESS);
        true
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        false
    }
}

/// Signals one object and waits on another object as an atomic operation.
pub unsafe fn SignalObjectAndWait(
    h_object_to_signal: HANDLE,
    h_object_to_wait_on: HANDLE,
    dw_milliseconds: u32,
    b_alertable: i32,
) -> u32 {
    let cur_tid = get_current_wait_tid();
    {
        let mut table = WAITABLE_OBJECTS.lock();
        let obj = match table.get_mut(&h_object_to_signal) {
            Some(o) => o,
            None => {
                win32_set_last_error(ERROR_INVALID_HANDLE);
                return WAIT_FAILED;
            }
        };

        match obj {
            WaitableObject::Event(ref mut ev) => {
                ev.signaled = true;
            }
            WaitableObject::Mutex(ref mut m) => {
                if m.owner_tid != cur_tid && m.owner_tid != 0 {
                    win32_set_last_error(ERROR_ACCESS_DENIED);
                    return WAIT_FAILED;
                }
                if m.recursion_count > 0 {
                    m.recursion_count -= 1;
                    if m.recursion_count == 0 {
                        m.owner_tid = 0;
                    }
                }
            }
            WaitableObject::Semaphore(ref mut sem) => {
                if sem.current_count < sem.maximum_count {
                    sem.current_count += 1;
                }
            }
            WaitableObject::Timer(ref mut t) => {
                t.signaled = true;
            }
            WaitableObject::Thread(_) | WaitableObject::Process(_) => {
                win32_set_last_error(ERROR_INVALID_PARAMETER);
                return WAIT_FAILED;
            }
        }
    }

    WaitForMultipleObjectsEx(1, &h_object_to_wait_on, 0, dw_milliseconds, b_alertable)
}

/// Adds a user-mode asynchronous procedure call (APC) object to the APC queue of the specified thread.
pub unsafe fn QueueUserAPC(
    pfn_apc: usize,
    h_thread: HANDLE,
    dw_data: usize,
) -> u32 {
    if pfn_apc == 0 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }

    let target_tid = {
        let table = WAITABLE_OBJECTS.lock();
        if let Some(WaitableObject::Thread(th)) = table.get(&h_thread) {
            th.tid
        } else if h_thread == 0 || h_thread == !0u64 {
            get_current_wait_tid()
        } else {
            h_thread as u32
        }
    };

    let entry = ApcEntry {
        pfn_apc,
        dw_data,
    };

    THREAD_APC_QUEUES
        .lock()
        .entry(target_tid)
        .or_insert_with(VecDeque::new)
        .push_back(entry);

    win32_set_last_error(ERROR_SUCCESS);
    1
}

static mut TEST_APC_INVOCATIONS: u32 = 0;

unsafe extern "system" fn test_apc_routine(dw_data: usize) {
    TEST_APC_INVOCATIONS = TEST_APC_INVOCATIONS.saturating_add(dw_data as u32);
}

/// Comprehensive self-test suite verifying Win32 waitable objects and multi-thread synchronization.
pub unsafe fn test_win32_wait_subsystem() -> bool {
    // 1. Single-object wait (manual-reset & auto-reset events)
    let h_manual = win32_create_waitable_event(true, false, Some("KntWaitManualEvent"));
    let h_auto = win32_create_waitable_event(false, true, Some("KntWaitAutoEvent"));
    if h_manual == 0 || h_auto == 0 {
        return false;
    }

    if WaitForSingleObject(h_manual, 0) != WAIT_TIMEOUT {
        return false;
    }
    if !win32_set_waitable_event(h_manual) {
        return false;
    }
    if WaitForSingleObject(h_manual, 0) != WAIT_OBJECT_0 {
        return false;
    }
    if WaitForSingleObject(h_manual, 0) != WAIT_OBJECT_0 {
        return false;
    }

    // Auto-reset event initially signaled
    if WaitForSingleObject(h_auto, 0) != WAIT_OBJECT_0 {
        return false;
    }
    // Auto-reset consumed: next wait must time out
    if WaitForSingleObject(h_auto, 0) != WAIT_TIMEOUT {
        return false;
    }

    // 2. Mutex ownership and recursive acquisition
    let h_mutex = win32_create_waitable_mutex(false, Some("KntWaitMutex"));
    if h_mutex == 0 {
        return false;
    }
    if WaitForSingleObject(h_mutex, 0) != WAIT_OBJECT_0 {
        return false;
    }
    // Recursive acquisition
    if WaitForSingleObject(h_mutex, 0) != WAIT_OBJECT_0 {
        return false;
    }
    if !win32_release_waitable_mutex(h_mutex) {
        return false;
    }
    if !win32_release_waitable_mutex(h_mutex) {
        return false;
    }

    // 3. WaitAny multiplexing across heterogeneous objects
    let h_sem = win32_create_waitable_semaphore(0, 10, Some("KntWaitSem"));
    let h_timer = CreateWaitableTimerA(0, 0, b"KntWaitTimer\0".as_ptr());
    if h_sem == 0 || h_timer == 0 {
        return false;
    }

    let multiplex_handles = [h_sem, h_auto, h_timer];
    if WaitForMultipleObjects(3, multiplex_handles.as_ptr(), 0, 0) != WAIT_TIMEOUT {
        return false;
    }

    // Signal semaphore: WaitAny should return WAIT_OBJECT_0 + 0
    if !win32_release_waitable_semaphore(h_sem, 1, core::ptr::null_mut()) {
        return false;
    }
    if WaitForMultipleObjects(3, multiplex_handles.as_ptr(), 0, 0) != WAIT_OBJECT_0 {
        return false;
    }

    // Signal auto event: WaitAny should return WAIT_OBJECT_0 + 1
    if !win32_set_waitable_event(h_auto) {
        return false;
    }
    if WaitForMultipleObjects(3, multiplex_handles.as_ptr(), 0, 0) != (WAIT_OBJECT_0 + 1) {
        return false;
    }

    // 4. WaitAll atomic barrier
    let barrier_handles = [h_sem, h_auto];
    // h_sem count is 0, h_auto is false
    if !win32_set_waitable_event(h_auto) {
        return false;
    }
    // Only 1 of 2 is signaled: WaitAll must time out without consuming h_auto
    if WaitForMultipleObjects(2, barrier_handles.as_ptr(), 1, 0) != WAIT_TIMEOUT {
        return false;
    }
    // Verify atomic barrier preserved h_auto signaled state
    if WaitForSingleObject(h_auto, 0) != WAIT_OBJECT_0 {
        return false;
    }

    // Now signal both objects
    if !win32_release_waitable_semaphore(h_sem, 1, core::ptr::null_mut()) {
        return false;
    }
    if !win32_set_waitable_event(h_auto) {
        return false;
    }
    if WaitForMultipleObjects(2, barrier_handles.as_ptr(), 1, 0) != WAIT_OBJECT_0 {
        return false;
    }

    // 5. SignalObjectAndWait atomic transition
    let h_sig_ev = win32_create_waitable_event(true, false, None);
    let h_wait_ev = win32_create_waitable_event(true, true, None);
    if SignalObjectAndWait(h_sig_ev, h_wait_ev, 0, 0) != WAIT_OBJECT_0 {
        return false;
    }
    if WaitForSingleObject(h_sig_ev, 0) != WAIT_OBJECT_0 {
        return false;
    }

    // 6. Alertable wait and User APC dispatch
    TEST_APC_INVOCATIONS = 0;
    if QueueUserAPC(test_apc_routine as *const () as usize, 0, 42) == 0 {
        return false;
    }

    let h_nonsignaled = win32_create_waitable_event(false, false, None);
    let wait_res = WaitForMultipleObjectsEx(1, &h_nonsignaled, 0, 10, 1);
    if wait_res != WAIT_IO_COMPLETION {
        return false;
    }
    if TEST_APC_INVOCATIONS != 42 {
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-WAIT-OK] Win32 waitable objects and multi-thread sync dispatcher verified\r\n");
    true
}
