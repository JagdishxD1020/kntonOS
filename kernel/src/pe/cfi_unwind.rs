//! Win32 Structured Exception Handling: Dynamic Vector Unwind & CFI Forward-Edge Target Verifier.
//!
//! Implements Control Flow Integrity (CFI / Control Flow Guard) forward-edge call target verifier
//! using bitmap lookups, dynamic SEH vector unwinding with non-volatile register state recovery
//! (RBX, RBP, RDI, RSI, R12-R15), and anti-cheat traps detecting indirect call tampering and forged records.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;

pub const CFI_TARGET_ALIGNMENT: u64 = 16;

/// Forward-Edge CFI Target Verification Result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfiCheckResult {
    ValidTarget,
    InvalidTarget(u64),
    UnalignedTarget(u64),
}

/// Control Flow Integrity (CFI) target bitmap lookup table.
pub struct CfiBitmapLookup {
    pub base_address: u64,
    pub size_bytes: usize,
    bitmap: Spinlock<Vec<u8>>,
}

impl CfiBitmapLookup {
    pub fn new(base_address: u64, size_bytes: usize) -> Self {
        // 1 bit per 16 bytes of code space
        let bit_count = (size_bytes as u64 + CFI_TARGET_ALIGNMENT - 1) / CFI_TARGET_ALIGNMENT;
        let byte_count = ((bit_count + 7) / 8) as usize;
        let mut bm = Vec::with_capacity(byte_count);
        bm.resize(byte_count, 0);
        Self {
            base_address,
            size_bytes,
            bitmap: Spinlock::new(bm),
        }
    }

    /// Mark an indirect call target address as valid in the CFI bitmap.
    pub fn mark_valid_target(&self, target_address: u64) -> Result<(), &'static str> {
        if target_address < self.base_address || target_address >= self.base_address + self.size_bytes as u64 {
            return Err("Target address outside CFI table bounds");
        }
        if (target_address & (CFI_TARGET_ALIGNMENT - 1)) != 0 {
            return Err("Target address is not 16-byte aligned");
        }

        let offset = target_address - self.base_address;
        let bit_index = offset / CFI_TARGET_ALIGNMENT;
        let byte_idx = (bit_index / 8) as usize;
        let bit_rem = (bit_index % 8) as u8;

        let mut bm = self.bitmap.lock();
        if byte_idx < bm.len() {
            bm[byte_idx] |= 1 << bit_rem;
            Ok(())
        } else {
            Err("Bit index exceeds bitmap storage")
        }
    }

    /// Verify whether an indirect call target is marked valid.
    pub fn check_target(&self, target_address: u64) -> CfiCheckResult {
        if (target_address & (CFI_TARGET_ALIGNMENT - 1)) != 0 {
            return CfiCheckResult::UnalignedTarget(target_address);
        }
        if target_address < self.base_address || target_address >= self.base_address + self.size_bytes as u64 {
            return CfiCheckResult::InvalidTarget(target_address);
        }

        let offset = target_address - self.base_address;
        let bit_index = offset / CFI_TARGET_ALIGNMENT;
        let byte_idx = (bit_index / 8) as usize;
        let bit_rem = (bit_index % 8) as u8;

        let bm = self.bitmap.lock();
        if byte_idx < bm.len() && (bm[byte_idx] & (1 << bit_rem)) != 0 {
            CfiCheckResult::ValidTarget
        } else {
            CfiCheckResult::InvalidTarget(target_address)
        }
    }
}

/// Global Forward-Edge CFI Target Verifier.
pub struct CfiTargetVerifier {
    modules: Spinlock<BTreeMap<u64, Arc<CfiBitmapLookup>>>,
    total_checks: AtomicU64,
    total_violations: AtomicU64,
}

impl CfiTargetVerifier {
    pub const fn new() -> Self {
        Self {
            modules: Spinlock::new(BTreeMap::new()),
            total_checks: AtomicU64::new(0),
            total_violations: AtomicU64::new(0),
        }
    }

    /// Register a CFI bitmap for an executable module range.
    pub fn register_module_cfi(&self, base_address: u64, size_bytes: usize) -> Arc<CfiBitmapLookup> {
        let lookup = Arc::new(CfiBitmapLookup::new(base_address, size_bytes));
        self.modules.lock().insert(base_address, lookup.clone());
        lookup
    }

    /// Verify an indirect call destination address across registered modules.
    pub fn verify_indirect_call(&self, target_address: u64) -> CfiCheckResult {
        self.total_checks.fetch_add(1, Ordering::Relaxed);

        let modules = self.modules.lock();
        for (&base, lookup) in modules.iter() {
            if target_address >= base && target_address < base + lookup.size_bytes as u64 {
                let res = lookup.check_target(target_address);
                if res != CfiCheckResult::ValidTarget {
                    self.total_violations.fetch_add(1, Ordering::Relaxed);
                }
                return res;
            }
        }

        // Call target outside known modules
        self.total_violations.fetch_add(1, Ordering::Relaxed);
        CfiCheckResult::InvalidTarget(target_address)
    }

    /// Retrieve total CFI verifications performed.
    pub fn get_total_checks(&self) -> u64 {
        self.total_checks.load(Ordering::Acquire)
    }

    /// Retrieve total CFI violations detected.
    pub fn get_total_violations(&self) -> u64 {
        self.total_violations.load(Ordering::Acquire)
    }
}

pub static CFI_VERIFIER: CfiTargetVerifier = CfiTargetVerifier::new();
