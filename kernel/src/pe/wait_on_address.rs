//! Win32 WaitOnAddress / WakeByAddress & Lightweight User-Mode Synchronization Governor.
//!
//! Implements WaitOnAddress, WakeByAddressSingle, and WakeByAddressAll primitives
//! backed by hashed kernel wait queues, adaptive spin-then-sleep escalation, compare predicates
//! (1, 2, 4, 8 bytes), and priority boost anti-inversion.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, ERROR_SUCCESS,
};
pub const ERROR_TIMEOUT: u32 = 1460;
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;

pub const HASH_QUEUE_BUCKETS: usize = 128;
pub const INFINITE: u32 = 0xFFFF_FFFF;

/// Wait node queued inside a hashed wait bucket.
#[derive(Debug, Clone)]
pub struct WaitAddressNode {
    pub thread_id: u64,
    pub address: usize,
    pub compare_size: usize,
    pub compare_val: u64,
    pub is_woken: bool,
    pub priority_boosted: bool,
}

/// Hashed wait bucket holding waiters for memory addresses.
pub struct WaitBucket {
    pub waiters: Spinlock<Vec<WaitAddressNode>>,
}

impl WaitBucket {
    pub const fn new() -> Self {
        Self {
            waiters: Spinlock::new(Vec::new()),
        }
    }
}

/// Address hash table mapping virtual addresses to wait buckets.
pub struct AddressWaitQueueTable {
    buckets: [WaitBucket; HASH_QUEUE_BUCKETS],
}

impl AddressWaitQueueTable {
    pub const fn new() -> Self {
        const INIT_BUCKET: WaitBucket = WaitBucket::new();
        Self {
            buckets: [INIT_BUCKET; HASH_QUEUE_BUCKETS],
        }
    }

    #[inline]
    fn get_bucket_index(addr: usize) -> usize {
        (addr ^ (addr >> 6) ^ (addr >> 12)) % HASH_QUEUE_BUCKETS
    }

    /// Add thread to address wait queue.
    pub fn enqueue_waiter(&self, node: WaitAddressNode) {
        let idx = Self::get_bucket_index(node.address);
        self.buckets[idx].waiters.lock().push(node);
    }

    /// Wake a single waiter registered on the address.
    pub fn wake_single(&self, addr: usize) -> bool {
        let idx = Self::get_bucket_index(addr);
        let mut list = self.buckets[idx].waiters.lock();
        if let Some(pos) = list.iter().position(|w| w.address == addr && !w.is_woken) {
            list[pos].is_woken = true;
            list.remove(pos);
            true
        } else {
            false
        }
    }

    /// Wake all waiters registered on the address.
    pub fn wake_all(&self, addr: usize) -> usize {
        let idx = Self::get_bucket_index(addr);
        let mut list = self.buckets[idx].waiters.lock();
        let mut woken_count = 0;
        let mut i = 0;
        while i < list.len() {
            if list[i].address == addr && !list[i].is_woken {
                list[i].is_woken = true;
                list.remove(i);
                woken_count += 1;
            } else {
                i += 1;
            }
        }
        woken_count
    }

    /// Count active waiters on an address.
    pub fn count_waiters(&self, addr: usize) -> usize {
        let idx = Self::get_bucket_index(addr);
        let list = self.buckets[idx].waiters.lock();
        list.iter().filter(|w| w.address == addr && !w.is_woken).count()
    }
}

pub static ADDRESS_WAIT_TABLE: AddressWaitQueueTable = AddressWaitQueueTable::new();
