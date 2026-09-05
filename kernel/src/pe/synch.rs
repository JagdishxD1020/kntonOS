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
