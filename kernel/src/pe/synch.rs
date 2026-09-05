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
