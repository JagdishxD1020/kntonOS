//! Win32 Thread Local Storage (TLS) and Fiber Cooperative Multitasking Subsystem.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;

pub use crate::pe::synch::HANDLE;

pub const TLS_OUT_OF_INDEXES: u32 = 0xFFFF_FFFF;
pub const TLS_MINIMUM_AVAILABLE: usize = 64;
pub const TLS_EXPANSION_SLOTS: usize = 1024;
pub const TLS_MAX_SLOTS: usize = TLS_MINIMUM_AVAILABLE + TLS_EXPANSION_SLOTS;

const BITMAP_WORDS: usize = (TLS_MAX_SLOTS + 63) / 64;

struct TlsManager {
    allocated_bitmap: [u64; BITMAP_WORDS],
    thread_values: BTreeMap<u32, Vec<usize>>,
}

impl TlsManager {
    const fn new() -> Self {
        Self {
            allocated_bitmap: [0; BITMAP_WORDS],
            thread_values: BTreeMap::new(),
        }
    }

    fn is_slot_allocated(&self, slot: usize) -> bool {
        if slot >= TLS_MAX_SLOTS {
            return false;
        }
        let word = slot / 64;
        let bit = slot % 64;
        (self.allocated_bitmap[word] & (1 << bit)) != 0
    }

    fn allocate_slot(&mut self) -> u32 {
        for slot in 0..TLS_MAX_SLOTS {
            let word = slot / 64;
            let bit = slot % 64;
            if (self.allocated_bitmap[word] & (1 << bit)) == 0 {
                self.allocated_bitmap[word] |= 1 << bit;
                return slot as u32;
            }
        }
        TLS_OUT_OF_INDEXES
    }

    fn free_slot(&mut self, slot: usize) -> bool {
        if slot >= TLS_MAX_SLOTS || !self.is_slot_allocated(slot) {
            return false;
        }
        let word = slot / 64;
        let bit = slot % 64;
        self.allocated_bitmap[word] &= !(1 << bit);

        for values in self.thread_values.values_mut() {
            if slot < values.len() {
                values[slot] = 0;
            }
        }
        true
    }

    fn get_value(&self, tid: u32, slot: usize) -> usize {
        if !self.is_slot_allocated(slot) {
            return 0;
        }
        if let Some(values) = self.thread_values.get(&tid) {
            if slot < values.len() {
                return values[slot];
            }
        }
        0
    }

    fn set_value(&mut self, tid: u32, slot: usize, value: usize) -> bool {
        if !self.is_slot_allocated(slot) {
            return false;
        }
        let values = self.thread_values.entry(tid).or_insert_with(Vec::new);
        if slot >= values.len() {
            values.resize(slot + 1, 0);
        }
        values[slot] = value;
        true
    }
}

static TLS_MANAGER: Spinlock<TlsManager> = Spinlock::new(TlsManager::new());

fn get_current_tid() -> u32 {
    let tid = crate::process::scheduler::current_thread_id() as u32;
    if tid == 0 { 1 } else { tid }
}

/// Allocates a dynamic TLS index for the calling process.
pub fn win32_tls_alloc() -> u32 {
    let mut mgr = TLS_MANAGER.lock();
    let slot = mgr.allocate_slot();
    if slot == TLS_OUT_OF_INDEXES {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_NOT_ENOUGH_MEMORY);
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    }
    slot
}

/// Releases a TLS index previously allocated with win32_tls_alloc.
pub fn win32_tls_free(dw_tls_index: u32) -> bool {
    let mut mgr = TLS_MANAGER.lock();
    let ok = mgr.free_slot(dw_tls_index as usize);
    if ok {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
    }
    ok
}

/// Retrieves the value in the calling thread's TLS slot for the specified TLS index.
pub fn win32_tls_get_value(dw_tls_index: u32) -> usize {
    let mgr = TLS_MANAGER.lock();
    let tid = get_current_tid();
    let val = mgr.get_value(tid, dw_tls_index as usize);
    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    val
}

/// Stores a value in the calling thread's TLS slot for the specified TLS index.
pub fn win32_tls_set_value(dw_tls_index: u32, lp_tls_value: usize) -> bool {
    let mut mgr = TLS_MANAGER.lock();
    let tid = get_current_tid();
    let ok = mgr.set_value(tid, dw_tls_index as usize, lp_tls_value);
    if ok {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
    }
    ok
}

/// Retrieves a thread's TLS value for a given explicit thread ID (used for testing).
pub fn win32_tls_get_value_for_thread(tid: u32, dw_tls_index: u32) -> usize {
    let mgr = TLS_MANAGER.lock();
    mgr.get_value(tid, dw_tls_index as usize)
}

/// Sets a thread's TLS value for a given explicit thread ID (used for testing).
pub fn win32_tls_set_value_for_thread(tid: u32, dw_tls_index: u32, lp_tls_value: usize) -> bool {
    let mut mgr = TLS_MANAGER.lock();
    mgr.set_value(tid, dw_tls_index as usize, lp_tls_value)
}
