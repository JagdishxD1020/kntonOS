//! Win32 Process & Thread Fiber Local Storage (FLS) & Dynamic Callback Destruction Governor.
//!
//! Implements Win32 Fiber Local Storage API (FlsAlloc, FlsFree, FlsGetValue, FlsSetValue),
//! dynamic termination callback registration, TEB/fiber slot array expansion up to 4096 slots,
//! and anti-cheat FLS callback integrity verification against unauthorized hook injection.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER,
    ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
use crate::pe::ntdll::{
    nt_error, nt_status_to_str, nt_success, NtStatus,
    STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_NO_MEMORY,
    STATUS_SUCCESS, STATUS_UNSUCCESSFUL,
};
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;

pub const FLS_OUT_OF_INDEXES: u32 = 0xFFFF_FFFF;
pub const FLS_MAX_SLOTS: usize = 4096;
const FLS_BITMAP_WORDS: usize = (FLS_MAX_SLOTS + 63) / 64; // 64 words

/// Fiber Local Storage callback type for slot cleanup.
pub type PFLS_CALLBACK_FUNCTION = Option<fn(lpFlsData: usize)>;

/// Slot descriptor tracking allocated FLS index and callback integrity.
#[derive(Clone, Copy)]
pub struct FlsSlotDescriptor {
    pub allocated: bool,
    pub callback: PFLS_CALLBACK_FUNCTION,
    pub callback_hash: u64,
}

impl Default for FlsSlotDescriptor {
    fn default() -> Self {
        Self {
            allocated: false,
            callback: None,
            callback_hash: 0,
        }
    }
}

/// Compute simple non-zero hash of callback function pointer.
#[inline]
pub fn compute_callback_hash(cb: PFLS_CALLBACK_FUNCTION) -> u64 {
    match cb {
        Some(f) => {
            let addr = f as usize as u64;
            addr ^ 0x55AA_55AA_33CC_33CC
        }
        None => 0,
    }
}

/// Fiber Local Storage Global Slot Governor.
pub struct FlsSlotGovernor {
    slots: Spinlock<[FlsSlotDescriptor; FLS_MAX_SLOTS]>,
    allocated_bitmap: Spinlock<[u64; FLS_BITMAP_WORDS]>,
}

impl FlsSlotGovernor {
    pub const fn new() -> Self {
        Self {
            slots: Spinlock::new([FlsSlotDescriptor {
                allocated: false,
                callback: None,
                callback_hash: 0,
            }; FLS_MAX_SLOTS]),
            allocated_bitmap: Spinlock::new([0; FLS_BITMAP_WORDS]),
        }
    }

    /// Allocate a new FLS slot with optional termination callback.
    pub fn fls_alloc(&self, lp_callback: PFLS_CALLBACK_FUNCTION) -> u32 {
        let mut bitmap = self.allocated_bitmap.lock();
        let mut slots = self.slots.lock();

        for slot in 0..FLS_MAX_SLOTS {
            let word = slot / 64;
            let bit = slot % 64;

            if (bitmap[word] & (1 << bit)) == 0 {
                bitmap[word] |= 1 << bit;
                slots[slot] = FlsSlotDescriptor {
                    allocated: true,
                    callback: lp_callback,
                    callback_hash: compute_callback_hash(lp_callback),
                };
                return slot as u32;
            }
        }

        FLS_OUT_OF_INDEXES
    }

    /// Free an allocated FLS slot.
    pub fn fls_free(&self, dw_fls_index: u32) -> bool {
        let slot = dw_fls_index as usize;
        if slot >= FLS_MAX_SLOTS {
            return false;
        }

        let mut bitmap = self.allocated_bitmap.lock();
        let mut slots = self.slots.lock();

        let word = slot / 64;
        let bit = slot % 64;

        if (bitmap[word] & (1 << bit)) == 0 {
            return false; // Not allocated
        }

        bitmap[word] &= !(1 << bit);
        slots[slot] = FlsSlotDescriptor::default();
        true
    }

    /// Check if slot is currently allocated.
    pub fn is_allocated(&self, dw_fls_index: u32) -> bool {
        let slot = dw_fls_index as usize;
        if slot >= FLS_MAX_SLOTS {
            return false;
        }
        let bitmap = self.allocated_bitmap.lock();
        let word = slot / 64;
        let bit = slot % 64;
        (bitmap[word] & (1 << bit)) != 0
    }

    /// Get slot descriptor.
    pub fn get_slot_descriptor(&self, dw_fls_index: u32) -> Option<FlsSlotDescriptor> {
        let slot = dw_fls_index as usize;
        if slot >= FLS_MAX_SLOTS {
            return None;
        }
        let slots = self.slots.lock();
        if slots[slot].allocated {
            Some(slots[slot])
        } else {
            None
        }
    }
}

pub static FLS_SLOT_GOVERNOR: FlsSlotGovernor = FlsSlotGovernor::new();

/// Thread / Fiber termination context for invoking FLS callbacks.
pub struct FlsTerminationGovernor;

impl FlsTerminationGovernor {
    /// Invoke all registered FLS callbacks for non-null slot values of terminating thread/fiber.
    pub fn invoke_termination_callbacks(
        governor: &FlsSlotGovernor,
        fls_data: &[usize],
    ) -> usize {
        let mut invoked_count = 0;
        let limit = fls_data.len().min(FLS_MAX_SLOTS);

        for slot_idx in 0..limit {
            let val = fls_data[slot_idx];
            if val == 0 {
                continue;
            }

            if let Some(desc) = governor.get_slot_descriptor(slot_idx as u32) {
                if let Some(cb) = desc.callback {
                    // Invoke destructor callback
                    cb(val);
                    invoked_count += 1;
                }
            }
        }

        invoked_count
    }
}

/// TEB / Fiber Local Storage Data Array Manager.
/// Manages dynamically expanding FLS data arrays up to 4096 slots.
pub struct FlsDataManager {
    // Key: (ThreadID, FiberID) -> FLS values vector
    storage: Spinlock<BTreeMap<(u64, u64), Vec<usize>>>,
}

impl FlsDataManager {
    pub const fn new() -> Self {
        Self {
            storage: Spinlock::new(BTreeMap::new()),
        }
    }

    /// Retrieve value from fiber local storage.
    pub fn fls_get_value(&self, tid: u64, fiber_id: u64, slot: u32) -> Option<usize> {
        let slot_idx = slot as usize;
        if slot_idx >= FLS_MAX_SLOTS {
            return None;
        }

        let map = self.storage.lock();
        if let Some(vec) = map.get(&(tid, fiber_id)) {
            if slot_idx < vec.len() {
                return Some(vec[slot_idx]);
            }
        }
        Some(0)
    }

    /// Store value into fiber local storage, expanding dynamic slot array if necessary.
    pub fn fls_set_value(
        &self,
        tid: u64,
        fiber_id: u64,
        slot: u32,
        val: usize,
    ) -> Result<(), NtStatus> {
        let slot_idx = slot as usize;
        if slot_idx >= FLS_MAX_SLOTS {
            return Err(STATUS_INVALID_PARAMETER);
        }

        let mut map = self.storage.lock();
        let vec = map.entry((tid, fiber_id)).or_insert_with(Vec::new);

        if slot_idx >= vec.len() {
            vec.resize(slot_idx + 1, 0);
        }

        vec[slot_idx] = val;
        Ok(())
    }

    /// Retrieve snapshot of FLS data array for termination.
    pub fn take_fls_data(&self, tid: u64, fiber_id: u64) -> Vec<usize> {
        let mut map = self.storage.lock();
        map.remove(&(tid, fiber_id)).unwrap_or_default()
    }
}

pub static FLS_DATA_MANAGER: FlsDataManager = FlsDataManager::new();

/// Emulates Win32 FlsAlloc API.
pub fn win32_fls_alloc(lp_callback: PFLS_CALLBACK_FUNCTION) -> u32 {
    let idx = FLS_SLOT_GOVERNOR.fls_alloc(lp_callback);
    if idx == FLS_OUT_OF_INDEXES {
        win32_set_last_error(ERROR_NOT_ENOUGH_MEMORY);
    } else {
        win32_set_last_error(ERROR_SUCCESS);
    }
    idx
}

/// Emulates Win32 FlsFree API.
pub fn win32_fls_free(dw_fls_index: u32) -> bool {
    if FLS_SLOT_GOVERNOR.fls_free(dw_fls_index) {
        win32_set_last_error(ERROR_SUCCESS);
        true
    } else {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        false
    }
}

/// Emulates Win32 FlsGetValue API.
pub fn win32_fls_get_value(dw_fls_index: u32, tid: u64, fiber_id: u64) -> usize {
    if !FLS_SLOT_GOVERNOR.is_allocated(dw_fls_index) {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }

    match FLS_DATA_MANAGER.fls_get_value(tid, fiber_id, dw_fls_index) {
        Some(v) => {
            win32_set_last_error(ERROR_SUCCESS);
            v
        }
        None => {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

/// Emulates Win32 FlsSetValue API.
pub fn win32_fls_set_value(dw_fls_index: u32, lp_fls_data: usize, tid: u64, fiber_id: u64) -> bool {
    if !FLS_SLOT_GOVERNOR.is_allocated(dw_fls_index) {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return false;
    }

    match FLS_DATA_MANAGER.fls_set_value(tid, fiber_id, dw_fls_index, lp_fls_data) {
        Ok(_) => {
            win32_set_last_error(ERROR_SUCCESS);
            true
        }
        Err(_) => {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            false
        }
    }
}

/// Anti-cheat FLS callback violation types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlsTamperViolation {
    CallbackPointerCorrupted {
        slot: u32,
        expected_hash: u64,
        actual_hash: u64,
    },
    UnauthorizedCallbackInjection {
        slot: u32,
    },
}

/// Anti-Cheat FLS Callback Integrity Scanner.
pub struct AntiCheatFlsScanner {
    pub detection_enabled: AtomicBool,
    pub detected_violations: Spinlock<Vec<FlsTamperViolation>>,
}

impl AntiCheatFlsScanner {
    pub const fn new() -> Self {
        Self {
            detection_enabled: AtomicBool::new(true),
            detected_violations: Spinlock::new(Vec::new()),
        }
    }

    /// Verify integrity of all registered FLS callbacks against their recorded creation hashes.
    pub fn verify_callback_integrity(&self, governor: &FlsSlotGovernor) -> bool {
        if !self.detection_enabled.load(Ordering::Acquire) {
            return true;
        }

        let slots = governor.slots.lock();
        let mut violations = self.detected_violations.lock();
        let mut clean = true;

        for (idx, slot) in slots.iter().enumerate() {
            if slot.allocated {
                let current_hash = compute_callback_hash(slot.callback);
                if current_hash != slot.callback_hash {
                    clean = false;
                    violations.push(FlsTamperViolation::CallbackPointerCorrupted {
                        slot: idx as u32,
                        expected_hash: slot.callback_hash,
                        actual_hash: current_hash,
                    });
                }
            }
        }

        clean
    }

    /// Retrieve recorded violations.
    pub fn get_violations(&self) -> Vec<FlsTamperViolation> {
        self.detected_violations.lock().clone()
    }
}

pub static ANTI_CHEAT_FLS_SCANNER: AntiCheatFlsScanner = AntiCheatFlsScanner::new();
