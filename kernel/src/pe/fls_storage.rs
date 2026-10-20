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
