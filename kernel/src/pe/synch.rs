//! Win32 Synchronization Primitives & Waitable Handles Engine.
//! Emulates Windows synchronization objects, events, mutexes, semaphores, and multi-wait dispatch.

use alloc::collections::BTreeMap;
use alloc::string::String;
use crate::memory::spinlock::Spinlock;

pub type HANDLE = usize;
pub use crate::pe::kernel32::INVALID_HANDLE_VALUE;

#[derive(Debug, Clone)]
pub struct Win32Event {
    pub manual_reset: bool,
    pub signaled: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Win32Mutex {
    pub owner_tid: u32,
    pub recursion_count: u32,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Win32Semaphore {
    pub current_count: i32,
    pub maximum_count: i32,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SynchObject {
    Event(Win32Event),
    Mutex(Win32Mutex),
    Semaphore(Win32Semaphore),
}

static NEXT_SYNCH_HANDLE: Spinlock<HANDLE> = Spinlock::new(0x0006_0001);
static SYNCH_TABLE: Spinlock<BTreeMap<HANDLE, SynchObject>> = Spinlock::new(BTreeMap::new());

/// Creates or opens a named or unnamed event object.
pub fn win32_create_event(
    _lp_event_attributes: usize,
    b_manual_reset: bool,
    b_initial_state: bool,
    lp_name: Option<&str>,
) -> HANDLE {
    let mut table = SYNCH_TABLE.lock();

    if let Some(name) = lp_name {
        for (h, obj) in table.iter() {
            if let SynchObject::Event(ev) = obj {
                if ev.name.as_deref() == Some(name) {
                    return *h;
                }
            }
        }
    }

    let mut gen = NEXT_SYNCH_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let ev = Win32Event {
        manual_reset: b_manual_reset,
        signaled: b_initial_state,
        name: lp_name.map(String::from),
    };

    table.insert(handle, SynchObject::Event(ev));
    handle
}

/// Sets the specified event object to the signaled state.
pub fn win32_set_event(h_event: HANDLE) -> bool {
    let mut table = SYNCH_TABLE.lock();
    if let Some(SynchObject::Event(ev)) = table.get_mut(&h_event) {
        ev.signaled = true;
        true
    } else {
        false
    }
}

/// Sets the specified event object to the nonsignaled state.
pub fn win32_reset_event(h_event: HANDLE) -> bool {
    let mut table = SYNCH_TABLE.lock();
    if let Some(SynchObject::Event(ev)) = table.get_mut(&h_event) {
        ev.signaled = false;
        true
    } else {
        false
    }
}

/// Sets the specified event object to the signaled state and then resets it to nonsignaled.
pub fn win32_pulse_event(h_event: HANDLE) -> bool {
    let mut table = SYNCH_TABLE.lock();
    if let Some(SynchObject::Event(ev)) = table.get_mut(&h_event) {
        ev.signaled = true;
        ev.signaled = false;
        true
    } else {
        false
    }
}

fn get_current_tid() -> u32 {
    let tid = crate::process::scheduler::current_thread_id() as u32;
    if tid == 0 { 1 } else { tid }
}

/// Creates or opens a named or unnamed mutex object.
pub fn win32_create_mutex(
    _lp_mutex_attributes: usize,
    b_initial_owner: bool,
    lp_name: Option<&str>,
) -> HANDLE {
    let mut table = SYNCH_TABLE.lock();

    if let Some(name) = lp_name {
        for (h, obj) in table.iter() {
            if let SynchObject::Mutex(m) = obj {
                if m.name.as_deref() == Some(name) {
                    return *h;
                }
            }
        }
    }

    let mut gen = NEXT_SYNCH_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let (owner_tid, recursion_count) = if b_initial_owner {
        (get_current_tid(), 1)
    } else {
        (0, 0)
    };

    let m = Win32Mutex {
        owner_tid,
        recursion_count,
        name: lp_name.map(String::from),
    };

    table.insert(handle, SynchObject::Mutex(m));
    handle
}

/// Releases ownership of the specified mutex object.
pub fn win32_release_mutex(h_mutex: HANDLE) -> bool {
    let mut table = SYNCH_TABLE.lock();
    if let Some(SynchObject::Mutex(m)) = table.get_mut(&h_mutex) {
        let cur_tid = get_current_tid();
        if m.owner_tid != cur_tid && m.owner_tid != 0 {
            return false;
        }

        if m.recursion_count > 0 {
            m.recursion_count -= 1;
            if m.recursion_count == 0 {
                m.owner_tid = 0;
            }
            true
        } else {
            false
        }
    } else {
        false
    }
}

/// Creates or opens a named or unnamed semaphore object.
pub fn win32_create_semaphore(
    _lp_semaphore_attributes: usize,
    l_initial_count: i32,
    l_maximum_count: i32,
    lp_name: Option<&str>,
) -> HANDLE {
    if l_initial_count < 0 || l_maximum_count <= 0 || l_initial_count > l_maximum_count {
        return 0;
    }

    let mut table = SYNCH_TABLE.lock();

    if let Some(name) = lp_name {
        for (h, obj) in table.iter() {
            if let SynchObject::Semaphore(sem) = obj {
                if sem.name.as_deref() == Some(name) {
                    return *h;
                }
            }
        }
    }

    let mut gen = NEXT_SYNCH_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let sem = Win32Semaphore {
        current_count: l_initial_count,
        maximum_count: l_maximum_count,
        name: lp_name.map(String::from),
    };

    table.insert(handle, SynchObject::Semaphore(sem));
    handle
}

/// Increases the count of the specified semaphore object by a specified amount.
pub unsafe fn win32_release_semaphore(
    h_semaphore: HANDLE,
    l_release_count: i32,
    lp_previous_count: *mut i32,
) -> bool {
    if l_release_count <= 0 {
        return false;
    }

    let mut table = SYNCH_TABLE.lock();
    if let Some(SynchObject::Semaphore(sem)) = table.get_mut(&h_semaphore) {
        let prev = sem.current_count;
        if prev.saturating_add(l_release_count) > sem.maximum_count {
            return false;
        }

        sem.current_count += l_release_count;
        if !lp_previous_count.is_null() {
            *lp_previous_count = prev;
        }
        true
    } else {
        false
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CriticalSection {
    pub debug_info: usize,
    pub lock_count: i32,
    pub recursion_count: i32,
    pub owning_thread: usize,
    pub lock_semaphore: usize,
    pub spin_count: usize,
}

impl Default for CriticalSection {
    fn default() -> Self {
        Self {
            debug_info: 0,
            lock_count: -1,
            recursion_count: 0,
            owning_thread: 0,
            lock_semaphore: 0,
            spin_count: 0,
        }
    }
}

#[allow(non_camel_case_types)]
pub type CRITICAL_SECTION = CriticalSection;
#[allow(non_camel_case_types)]
pub type LPCRITICAL_SECTION = *mut CriticalSection;

/// Initializes a critical section object.
pub unsafe fn win32_initialize_critical_section(lp_cs: LPCRITICAL_SECTION) {
    if !lp_cs.is_null() {
        *lp_cs = CriticalSection::default();
    }
}

/// Initializes a critical section object and sets its spin count.
pub unsafe fn win32_initialize_critical_section_and_spin_count(
    lp_cs: LPCRITICAL_SECTION,
    dw_spin_count: u32,
) -> bool {
    if lp_cs.is_null() {
        return false;
    }
    *lp_cs = CriticalSection {
        spin_count: dw_spin_count as usize,
        ..Default::default()
    };
    true
}

/// Waits for ownership of the specified critical section object.
pub unsafe fn win32_enter_critical_section(lp_cs: LPCRITICAL_SECTION) {
    if lp_cs.is_null() {
        return;
    }

    let cur_tid = get_current_tid() as usize;
    if (*lp_cs).owning_thread == cur_tid {
        (*lp_cs).recursion_count += 1;
        (*lp_cs).lock_count += 1;
        return;
    }

    let mut spins = (*lp_cs).spin_count;
    while (*lp_cs).owning_thread != 0 {
        if spins > 0 {
            core::hint::spin_loop();
            spins -= 1;
        } else {
            crate::process::yield_now();
        }
    }

    (*lp_cs).owning_thread = cur_tid;
    (*lp_cs).recursion_count = 1;
    (*lp_cs).lock_count = 0;
}

/// Attempts to enter a critical section without blocking.
pub unsafe fn win32_try_enter_critical_section(lp_cs: LPCRITICAL_SECTION) -> bool {
    if lp_cs.is_null() {
        return false;
    }

    let cur_tid = get_current_tid() as usize;
    if (*lp_cs).owning_thread == cur_tid {
        (*lp_cs).recursion_count += 1;
        (*lp_cs).lock_count += 1;
        true
    } else if (*lp_cs).owning_thread == 0 {
        (*lp_cs).owning_thread = cur_tid;
        (*lp_cs).recursion_count = 1;
        (*lp_cs).lock_count = 0;
        true
    } else {
        false
    }
}

/// Releases ownership of the specified critical section object.
pub unsafe fn win32_leave_critical_section(lp_cs: LPCRITICAL_SECTION) {
    if lp_cs.is_null() {
        return;
    }

    let cur_tid = get_current_tid() as usize;
    if (*lp_cs).owning_thread == cur_tid && (*lp_cs).recursion_count > 0 {
        (*lp_cs).recursion_count -= 1;
        (*lp_cs).lock_count -= 1;
        if (*lp_cs).recursion_count == 0 {
            (*lp_cs).owning_thread = 0;
        }
    }
}

/// Releases all resources used by an unowned critical section object.
pub unsafe fn win32_delete_critical_section(lp_cs: LPCRITICAL_SECTION) {
    if !lp_cs.is_null() {
        *lp_cs = CriticalSection::default();
    }
}

use core::sync::atomic::{AtomicUsize, Ordering};

const SRW_EXCLUSIVE: usize = 0x0001;
const SRW_READER_SHIFT: usize = 4;
const SRW_READER_MASK: usize = 0x0FFF_FFF0;
const SRW_READER_INC: usize = 1 << SRW_READER_SHIFT;
const SRW_OWNER_SHIFT: usize = 32;

#[repr(C)]
#[derive(Debug, Default)]
pub struct SrwLock {
    pub state: AtomicUsize,
}

#[allow(non_camel_case_types)]
pub type SRWLOCK = SrwLock;
#[allow(non_camel_case_types)]
pub type PSRWLOCK = *mut SrwLock;

/// Initializes a slim reader/writer (SRW) lock.
pub unsafe fn win32_initialize_srw_lock(srw_lock: PSRWLOCK) {
    if !srw_lock.is_null() {
        (*srw_lock).state.store(0, Ordering::Release);
    }
}

/// Acquires a slim reader/writer (SRW) lock in exclusive mode.
pub unsafe fn win32_acquire_srw_lock_exclusive(srw_lock: PSRWLOCK) {
    if srw_lock.is_null() {
        return;
    }

    let tid = get_current_tid() as usize;
    let new_val = (tid << SRW_OWNER_SHIFT) | SRW_EXCLUSIVE;

    loop {
        let val = (*srw_lock).state.load(Ordering::Acquire);
        if (val & (SRW_EXCLUSIVE | SRW_READER_MASK)) == 0 {
            if (*srw_lock).state.compare_exchange_weak(val, new_val, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return;
            }
        }
        core::hint::spin_loop();
        crate::process::yield_now();
    }
}

/// Attempts to acquire a slim reader/writer (SRW) lock in exclusive mode without blocking.
pub unsafe fn win32_try_acquire_srw_lock_exclusive(srw_lock: PSRWLOCK) -> bool {
    if srw_lock.is_null() {
        return false;
    }

    let val = (*srw_lock).state.load(Ordering::Acquire);
    if (val & (SRW_EXCLUSIVE | SRW_READER_MASK)) == 0 {
        let tid = get_current_tid() as usize;
        let new_val = (tid << SRW_OWNER_SHIFT) | SRW_EXCLUSIVE;
        (*srw_lock).state.compare_exchange(val, new_val, Ordering::AcqRel, Ordering::Acquire).is_ok()
    } else {
        false
    }
}

/// Releases a slim reader/writer (SRW) lock that was acquired in exclusive mode.
pub unsafe fn win32_release_srw_lock_exclusive(srw_lock: PSRWLOCK) {
    if !srw_lock.is_null() {
        (*srw_lock).state.store(0, Ordering::Release);
    }
}

/// Acquires a slim reader/writer (SRW) lock in shared mode.
pub unsafe fn win32_acquire_srw_lock_shared(srw_lock: PSRWLOCK) {
    if srw_lock.is_null() {
        return;
    }

    loop {
        let val = (*srw_lock).state.load(Ordering::Acquire);
        if (val & SRW_EXCLUSIVE) == 0 {
            let new_val = val + SRW_READER_INC;
            if (*srw_lock).state.compare_exchange_weak(val, new_val, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return;
            }
        }
        core::hint::spin_loop();
        crate::process::yield_now();
    }
}

/// Attempts to acquire a slim reader/writer (SRW) lock in shared mode without blocking.
pub unsafe fn win32_try_acquire_srw_lock_shared(srw_lock: PSRWLOCK) -> bool {
    if srw_lock.is_null() {
        return false;
    }

    let val = (*srw_lock).state.load(Ordering::Acquire);
    if (val & SRW_EXCLUSIVE) == 0 {
        let new_val = val + SRW_READER_INC;
        (*srw_lock).state.compare_exchange(val, new_val, Ordering::AcqRel, Ordering::Acquire).is_ok()
    } else {
        false
    }
}

/// Releases a slim reader/writer (SRW) lock that was acquired in shared mode.
pub unsafe fn win32_release_srw_lock_shared(srw_lock: PSRWLOCK) {
    if srw_lock.is_null() {
        return;
    }

    loop {
        let val = (*srw_lock).state.load(Ordering::Acquire);
        let readers = (val & SRW_READER_MASK) >> SRW_READER_SHIFT;
        if readers > 0 {
            let new_val = val - SRW_READER_INC;
            if (*srw_lock).state.compare_exchange_weak(val, new_val, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return;
            }
        } else {
            return;
        }
    }
}

pub const WAIT_OBJECT_0: u32 = 0x0000_0000;
pub const WAIT_ABANDONED_0: u32 = 0x0000_0080;
pub const WAIT_TIMEOUT: u32 = 0x0000_0102;
pub const WAIT_FAILED: u32 = 0xFFFF_FFFF;
pub const INFINITE: u32 = 0xFFFF_FFFF;
pub const MAXIMUM_WAIT_OBJECTS: u32 = 64;

fn is_object_signaled(obj: &SynchObject, cur_tid: u32) -> bool {
    match obj {
        SynchObject::Event(ev) => ev.signaled,
        SynchObject::Mutex(m) => m.owner_tid == 0 || m.owner_tid == cur_tid,
        SynchObject::Semaphore(sem) => sem.current_count > 0,
    }
}

fn satisfy_object_wait(obj: &mut SynchObject, cur_tid: u32) {
    match obj {
        SynchObject::Event(ev) => {
            if !ev.manual_reset {
                ev.signaled = false;
            }
        }
        SynchObject::Mutex(m) => {
            m.owner_tid = cur_tid;
            m.recursion_count = m.recursion_count.saturating_add(1);
        }
        SynchObject::Semaphore(sem) => {
            sem.current_count = sem.current_count.saturating_sub(1);
        }
    }
}

/// Waits until the specified object is in the signaled state or the time-out interval elapses.
pub unsafe fn win32_wait_for_single_object(h_handle: HANDLE, dw_milliseconds: u32) -> u32 {
    win32_wait_for_multiple_objects(1, &h_handle, false, dw_milliseconds)
}

/// Waits until one or all of the specified objects are in the signaled state or time-out occurs.
pub unsafe fn win32_wait_for_multiple_objects(
    count: u32,
    p_handles: *const HANDLE,
    b_wait_all: bool,
    dw_milliseconds: u32,
) -> u32 {
    if count == 0 || count > MAXIMUM_WAIT_OBJECTS || p_handles.is_null() {
        return WAIT_FAILED;
    }

    let handles = core::slice::from_raw_parts(p_handles, count as usize);
    let cur_tid = get_current_tid();
    let max_spins = if dw_milliseconds == 0 { 1 } else if dw_milliseconds == INFINITE { 1000 } else { dw_milliseconds.min(100) };

    for _ in 0..max_spins {
        let mut table = SYNCH_TABLE.lock();

        // Validate all handles exist
        for &h in handles {
            if !table.contains_key(&h) {
                return WAIT_FAILED;
            }
        }

        if b_wait_all {
            let mut all_signaled = true;
            for &h in handles {
                if let Some(obj) = table.get(&h) {
                    if !is_object_signaled(obj, cur_tid) {
                        all_signaled = false;
                        break;
                    }
                }
            }

            if all_signaled {
                for &h in handles {
                    if let Some(obj) = table.get_mut(&h) {
                        satisfy_object_wait(obj, cur_tid);
                    }
                }
                return WAIT_OBJECT_0;
            }
        } else {
            for (idx, &h) in handles.iter().enumerate() {
                if let Some(obj) = table.get(&h) {
                    if is_object_signaled(obj, cur_tid) {
                        if let Some(mut_obj) = table.get_mut(&h) {
                            satisfy_object_wait(mut_obj, cur_tid);
                        }
                        return WAIT_OBJECT_0 + idx as u32;
                    }
                }
            }
        }

        drop(table);
        if dw_milliseconds == 0 {
            return WAIT_TIMEOUT;
        }
        core::hint::spin_loop();
        crate::process::yield_now();
    }

    WAIT_TIMEOUT
}

/// Comprehensive self-test suite verifying Win32 synchronization primitives and wait engine.
pub fn test_synch_emulation_subsystem() -> bool {
    crate::serial_write_str("[KNT-SYNCH-TEST] Executing Win32 synchronization primitives verification...\r\n");

    // 1. Verify Event objects (manual-reset & auto-reset)
    let h_manual_event = win32_create_event(0, true, false, Some("KntonManualEvent"));
    let h_auto_event = win32_create_event(0, false, false, Some("KntonAutoEvent"));
    if h_manual_event == 0 || h_auto_event == 0 {
        return false;
    }

    if unsafe { win32_wait_for_single_object(h_manual_event, 0) } != WAIT_TIMEOUT {
        return false;
    }
    if unsafe { win32_wait_for_single_object(h_auto_event, 0) } != WAIT_TIMEOUT {
        return false;
    }

    win32_set_event(h_manual_event);
    if unsafe { win32_wait_for_single_object(h_manual_event, 0) } != WAIT_OBJECT_0 {
        return false;
    }
    if unsafe { win32_wait_for_single_object(h_manual_event, 0) } != WAIT_OBJECT_0 {
        return false;
    }
    win32_reset_event(h_manual_event);
    if unsafe { win32_wait_for_single_object(h_manual_event, 0) } != WAIT_TIMEOUT {
        return false;
    }

    win32_set_event(h_auto_event);
    if unsafe { win32_wait_for_single_object(h_auto_event, 0) } != WAIT_OBJECT_0 {
        return false;
    }
    if unsafe { win32_wait_for_single_object(h_auto_event, 0) } != WAIT_TIMEOUT {
        return false;
    }

    // 2. Verify Mutex recursive locking and ownership
    let h_mutex = win32_create_mutex(0, false, Some("KntonMutex"));
    if h_mutex == 0 {
        return false;
    }
    if unsafe { win32_wait_for_single_object(h_mutex, 0) } != WAIT_OBJECT_0 {
        return false;
    }
    if unsafe { win32_wait_for_single_object(h_mutex, 0) } != WAIT_OBJECT_0 {
        return false;
    }
    if !win32_release_mutex(h_mutex) {
        return false;
    }
    if !win32_release_mutex(h_mutex) {
        return false;
    }
    if win32_release_mutex(h_mutex) {
        return false;
    }

    // 3. Verify Semaphore counting
    let h_sem = win32_create_semaphore(0, 2, 4, Some("KntonSem"));
    if h_sem == 0 {
        return false;
    }
    if unsafe { win32_wait_for_single_object(h_sem, 0) } != WAIT_OBJECT_0 {
        return false;
    }
    if unsafe { win32_wait_for_single_object(h_sem, 0) } != WAIT_OBJECT_0 {
        return false;
    }
    if unsafe { win32_wait_for_single_object(h_sem, 0) } != WAIT_TIMEOUT {
        return false;
    }
    let mut prev_count = 0;
    if !unsafe { win32_release_semaphore(h_sem, 3, &mut prev_count as *mut i32) } || prev_count != 0 {
        return false;
    }
    if unsafe { win32_release_semaphore(h_sem, 2, core::ptr::null_mut()) } {
        return false;
    }

    // 4. Verify Critical Section and SRW Lock
    let mut cs = CriticalSection::default();
    unsafe {
        win32_initialize_critical_section_and_spin_count(&mut cs as *mut _, 100);
        win32_enter_critical_section(&mut cs as *mut _);
        if !win32_try_enter_critical_section(&mut cs as *mut _) {
            return false;
        }
        win32_leave_critical_section(&mut cs as *mut _);
        win32_leave_critical_section(&mut cs as *mut _);
        win32_delete_critical_section(&mut cs as *mut _);
    }

    let mut srw = SrwLock::default();
    unsafe {
        win32_initialize_srw_lock(&mut srw as *mut _);
        win32_acquire_srw_lock_exclusive(&mut srw as *mut _);
        if win32_try_acquire_srw_lock_shared(&mut srw as *mut _) {
            return false;
        }
        win32_release_srw_lock_exclusive(&mut srw as *mut _);

        win32_acquire_srw_lock_shared(&mut srw as *mut _);
        if !win32_try_acquire_srw_lock_shared(&mut srw as *mut _) {
            return false;
        }
        if win32_try_acquire_srw_lock_exclusive(&mut srw as *mut _) {
            return false;
        }
        win32_release_srw_lock_shared(&mut srw as *mut _);
        win32_release_srw_lock_shared(&mut srw as *mut _);
    }

    // 5. Verify WaitForMultipleObjects (wait-all & wait-any)
    let ev1 = win32_create_event(0, true, false, None);
    let ev2 = win32_create_event(0, false, false, None);
    let handles = [ev1, ev2];

    if unsafe { win32_wait_for_multiple_objects(2, handles.as_ptr(), false, 0) } != WAIT_TIMEOUT {
        return false;
    }
    if unsafe { win32_wait_for_multiple_objects(2, handles.as_ptr(), true, 0) } != WAIT_TIMEOUT {
        return false;
    }

    win32_set_event(ev2);
    if unsafe { win32_wait_for_multiple_objects(2, handles.as_ptr(), false, 0) } != WAIT_OBJECT_0 + 1 {
        return false;
    }
    if unsafe { win32_wait_for_multiple_objects(2, handles.as_ptr(), false, 0) } != WAIT_TIMEOUT {
        return false;
    }

    win32_set_event(ev1);
    win32_set_event(ev2);
    if unsafe { win32_wait_for_multiple_objects(2, handles.as_ptr(), true, 0) } != WAIT_OBJECT_0 {
        return false;
    }
    if unsafe { win32_wait_for_multiple_objects(2, handles.as_ptr(), true, 0) } != WAIT_TIMEOUT {
        return false;
    }

    crate::serial_write_str("[KNT-SYNCH-PASS] Win32 synchronization primitives verified\r\n");
    true
}
