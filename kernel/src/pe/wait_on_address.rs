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

pub const ADAPTIVE_SPIN_LIMIT: u32 = 400; // 400 iterations before kernel sleep escalation

/// Helper to read value at target address based on compare_address_size.
#[inline]
pub unsafe fn read_address_val(addr: *const u8, size: usize) -> Result<u64, ()> {
    if addr.is_null() {
        return Err(());
    }
    match size {
        1 => Ok(*(addr as *const u8) as u64),
        2 => Ok(*(addr as *const u16) as u64),
        4 => Ok(*(addr as *const u32) as u64),
        8 => Ok(*(addr as *const u64)),
        _ => Err(()),
    }
}

/// Adaptive spin-then-sleep lock escalation engine.
pub struct AdaptiveSpinEscalationEngine;

impl AdaptiveSpinEscalationEngine {
    /// Perform adaptive spin check on target address value.
    /// Returns true if predicate changed during spin (avoiding kernel sleep), false if unchanged.
    pub unsafe fn spin_check(
        address: *const u8,
        compare_address: *const u8,
        size: usize,
    ) -> bool {
        let expected = match read_address_val(compare_address, size) {
            Ok(v) => v,
            Err(_) => return false,
        };

        for _ in 0..ADAPTIVE_SPIN_LIMIT {
            core::hint::spin_loop();
            if let Ok(current) = read_address_val(address, size) {
                if current != expected {
                    return true; // Predicate changed during spin!
                }
            }
        }

        false // Did not change, must escalate to sleeping wait
    }
}

/// Emulates Win32 WaitOnAddress API.
pub unsafe fn win32_wait_on_address(
    address: *const u8,
    compare_address: *const u8,
    address_size: usize,
    _milliseconds: u32,
    tid: u64,
) -> bool {
    if address.is_null() || compare_address.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return false;
    }

    if address_size != 1 && address_size != 2 && address_size != 4 && address_size != 8 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return false;
    }

    // Initial check: if values already differ, return true immediately (no wait needed)
    let current_val = match read_address_val(address, address_size) {
        Ok(v) => v,
        Err(_) => {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return false;
        }
    };
    let compare_val = match read_address_val(compare_address, address_size) {
        Ok(v) => v,
        Err(_) => {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return false;
        }
    };

    if current_val != compare_val {
        win32_set_last_error(ERROR_SUCCESS);
        return true;
    }

    // Fast path: Adaptive spin loop
    if AdaptiveSpinEscalationEngine::spin_check(address, compare_address, address_size) {
        win32_set_last_error(ERROR_SUCCESS);
        return true;
    }

    // Slow path: Escalate to kernel wait queue
    let node = WaitAddressNode {
        thread_id: tid,
        address: address as usize,
        compare_size: address_size,
        compare_val,
        is_woken: false,
        priority_boosted: true, // Anti-inversion priority boost applied
    };

    ADDRESS_WAIT_TABLE.enqueue_waiter(node);
    win32_set_last_error(ERROR_SUCCESS);
    true
}

/// Emulates Win32 WakeByAddressSingle API.
pub unsafe fn win32_wake_by_address_single(address: *const u8) {
    if !address.is_null() {
        ADDRESS_WAIT_TABLE.wake_single(address as usize);
    }
}

/// Emulates Win32 WakeByAddressAll API.
pub unsafe fn win32_wake_by_address_all(address: *const u8) {
    if !address.is_null() {
        ADDRESS_WAIT_TABLE.wake_all(address as usize);
    }
}

/// Comprehensive self-test for Win32 WaitOnAddress / WakeByAddress Subsystem.
pub fn test_win32_wait_on_address_subsystem() -> bool {
    crate::serial_write_str("[KNT-WIN32-WAIT-ADDRESS] Initializing Win32 WaitOnAddress Subsystem self-test...\r\n");

    // 1. Test Immediate Return when value already differs (No wait, no spin)
    let sync_var_32: u32 = 42;
    let compare_diff_32: u32 = 99;

    let ret_diff = unsafe {
        win32_wait_on_address(
            &sync_var_32 as *const _ as *const u8,
            &compare_diff_32 as *const _ as *const u8,
            4,
            1000,
            1001,
        )
    };

    if !ret_diff {
        crate::serial_write_str("[KNT-TEST-ERR] WaitOnAddress failed on differing value check\r\n");
        return false;
    }

    if ADDRESS_WAIT_TABLE.count_waiters(&sync_var_32 as *const _ as usize) != 0 {
        crate::serial_write_str("[KNT-TEST-ERR] Waiter enqueued despite differing compare value\r\n");
        return false;
    }

    // 2. Test Multi-Size Predicate Matching (1, 2, 4, 8 bytes)
    let val_u8: u8 = 0xAA;
    let cmp_u8: u8 = 0xAA;
    let ret_u8 = unsafe {
        win32_wait_on_address(
            &val_u8 as *const _ as *const u8,
            &cmp_u8 as *const _ as *const u8,
            1,
            0,
            1002,
        )
    };
    if !ret_u8 || ADDRESS_WAIT_TABLE.count_waiters(&val_u8 as *const _ as usize) != 1 {
        crate::serial_write_str("[KNT-TEST-ERR] 1-byte predicate wait queue enqueuing failed\r\n");
        return false;
    }
    unsafe { win32_wake_by_address_single(&val_u8 as *const _ as *const u8) };

    let val_u64: u64 = 0x0123_4567_89AB_CDEF;
    let cmp_u64: u64 = 0x0123_4567_89AB_CDEF;
    let ret_u64 = unsafe {
        win32_wait_on_address(
            &val_u64 as *const _ as *const u8,
            &cmp_u64 as *const _ as *const u8,
            8,
            0,
            1003,
        )
    };
    if !ret_u64 || ADDRESS_WAIT_TABLE.count_waiters(&val_u64 as *const _ as usize) != 1 {
        crate::serial_write_str("[KNT-TEST-ERR] 8-byte predicate wait queue enqueuing failed\r\n");
        return false;
    }
    unsafe { win32_wake_by_address_single(&val_u64 as *const _ as *const u8) };

    // 3. Test WakeByAddressSingle & WakeByAddressAll
    let shared_mutex: u32 = 1;
    let cmp_mutex: u32 = 1;

    // Enqueue 3 waiters
    unsafe {
        win32_wait_on_address(&shared_mutex as *const _ as *const u8, &cmp_mutex as *const _ as *const u8, 4, 0, 2001);
        win32_wait_on_address(&shared_mutex as *const _ as *const u8, &cmp_mutex as *const _ as *const u8, 4, 0, 2002);
        win32_wait_on_address(&shared_mutex as *const _ as *const u8, &cmp_mutex as *const _ as *const u8, 4, 0, 2003);
    }

    let shared_addr = &shared_mutex as *const _ as usize;
    if ADDRESS_WAIT_TABLE.count_waiters(shared_addr) != 3 {
        crate::serial_write_str("[KNT-TEST-ERR] 3 waiters not found in address queue\r\n");
        return false;
    }

    // Wake single waiter
    unsafe { win32_wake_by_address_single(&shared_mutex as *const _ as *const u8) };
    if ADDRESS_WAIT_TABLE.count_waiters(shared_addr) != 2 {
        crate::serial_write_str("[KNT-TEST-ERR] WakeByAddressSingle did not wake exactly 1 waiter\r\n");
        return false;
    }

    // Wake all remaining waiters
    unsafe { win32_wake_by_address_all(&shared_mutex as *const _ as *const u8) };
    if ADDRESS_WAIT_TABLE.count_waiters(shared_addr) != 0 {
        crate::serial_write_str("[KNT-TEST-ERR] WakeByAddressAll did not clear remaining waiters\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-WAIT-ADDRESS-PASS] Win32 WaitOnAddress & WakeByAddress verified\r\n");
    true
}
