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

/// Saved non-volatile machine register state for SEH unwinding.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NonVolatileContext {
    pub rip: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

/// Dynamic stack unwind operation descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnwindRegisterOp {
    RestoreRbx { stack_offset: u32 },
    RestoreRbp { stack_offset: u32 },
    RestoreRdi { stack_offset: u32 },
    RestoreRsi { stack_offset: u32 },
    RestoreR12 { stack_offset: u32 },
    RestoreR13 { stack_offset: u32 },
    RestoreR14 { stack_offset: u32 },
    RestoreR15 { stack_offset: u32 },
    DeallocStack { size: u32 },
}

/// Dynamic SEH Vector Unwinder recovering non-volatile machine registers.
pub struct SehVectorUnwinder;

impl SehVectorUnwinder {
    /// Unwind a stack frame and recover non-volatile register states.
    pub fn unwind_frame_step(
        ctx: &mut NonVolatileContext,
        stack_memory: &[u8],
        unwind_ops: &[UnwindRegisterOp],
        return_address: u64,
    ) -> Result<(), &'static str> {
        for op in unwind_ops {
            match *op {
                UnwindRegisterOp::RestoreRbx { stack_offset } => {
                    ctx.rbx = Self::read_u64_from_stack(stack_memory, stack_offset as usize)?;
                }
                UnwindRegisterOp::RestoreRbp { stack_offset } => {
                    ctx.rbp = Self::read_u64_from_stack(stack_memory, stack_offset as usize)?;
                }
                UnwindRegisterOp::RestoreRdi { stack_offset } => {
                    ctx.rdi = Self::read_u64_from_stack(stack_memory, stack_offset as usize)?;
                }
                UnwindRegisterOp::RestoreRsi { stack_offset } => {
                    ctx.rsi = Self::read_u64_from_stack(stack_memory, stack_offset as usize)?;
                }
                UnwindRegisterOp::RestoreR12 { stack_offset } => {
                    ctx.r12 = Self::read_u64_from_stack(stack_memory, stack_offset as usize)?;
                }
                UnwindRegisterOp::RestoreR13 { stack_offset } => {
                    ctx.r13 = Self::read_u64_from_stack(stack_memory, stack_offset as usize)?;
                }
                UnwindRegisterOp::RestoreR14 { stack_offset } => {
                    ctx.r14 = Self::read_u64_from_stack(stack_memory, stack_offset as usize)?;
                }
                UnwindRegisterOp::RestoreR15 { stack_offset } => {
                    ctx.r15 = Self::read_u64_from_stack(stack_memory, stack_offset as usize)?;
                }
                UnwindRegisterOp::DeallocStack { size } => {
                    ctx.rsp = ctx.rsp.saturating_add(size as u64);
                }
            }
        }
        ctx.rip = return_address;
        Ok(())
    }

    fn read_u64_from_stack(stack: &[u8], offset: usize) -> Result<u64, &'static str> {
        if offset + 8 <= stack.len() {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&stack[offset..offset + 8]);
            Ok(u64::from_le_bytes(bytes))
        } else {
            Err("Stack read out of bounds")
        }
    }
}

/// Anti-cheat violation classification for CFI and SEH tampering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfiTamperTrapType {
    IndirectCallTamper { target_addr: u64 },
    ForgedExceptionRecord { record_addr: u64, reason: &'static str },
}

/// Anti-cheat trap detecting CFI indirect call tampering and forged exception registration records.
pub struct CfiAntiCheatTrap {
    incidents: Spinlock<Vec<CfiTamperTrapType>>,
    total_traps: AtomicU64,
}

impl CfiAntiCheatTrap {
    pub const fn new() -> Self {
        Self {
            incidents: Spinlock::new(Vec::new()),
            total_traps: AtomicU64::new(0),
        }
    }

    /// Verify an indirect call destination against forward-edge CFI policies and raise trap on tampering.
    pub fn verify_call_or_trap(&self, target_address: u64) -> Result<(), CfiTamperTrapType> {
        match CFI_VERIFIER.verify_indirect_call(target_address) {
            CfiCheckResult::ValidTarget => Ok(()),
            _ => {
                let trap = CfiTamperTrapType::IndirectCallTamper { target_addr: target_address };
                self.incidents.lock().push(trap.clone());
                self.total_traps.fetch_add(1, Ordering::Relaxed);
                Err(trap)
            }
        }
    }

    /// Verify SEH exception registration record integrity against memory forging and unmapped pointers.
    pub fn verify_exception_record_or_trap(
        &self,
        record_addr: u64,
        handler_addr: u64,
    ) -> Result<(), CfiTamperTrapType> {
        // Record address must be 8-byte aligned and non-null
        if record_addr == 0 || (record_addr & 0x07) != 0 {
            let trap = CfiTamperTrapType::ForgedExceptionRecord {
                record_addr,
                reason: "Unaligned or null exception record address",
            };
            self.incidents.lock().push(trap.clone());
            self.total_traps.fetch_add(1, Ordering::Relaxed);
            return Err(trap);
        }

        // Handler address must be valid forward-edge CFI target
        match CFI_VERIFIER.verify_indirect_call(handler_addr) {
            CfiCheckResult::ValidTarget => Ok(()),
            _ => {
                let trap = CfiTamperTrapType::ForgedExceptionRecord {
                    record_addr,
                    reason: "Exception handler target failed CFI validation",
                };
                self.incidents.lock().push(trap.clone());
                self.total_traps.fetch_add(1, Ordering::Relaxed);
                Err(trap)
            }
        }
    }

    /// Retrieve total anti-cheat traps triggered.
    pub fn get_total_traps(&self) -> u64 {
        self.total_traps.load(Ordering::Acquire)
    }

    /// Clear all incident history.
    pub fn clear(&self) {
        self.incidents.lock().clear();
        self.total_traps.store(0, Ordering::Relaxed);
    }
}

pub static CFI_ANTI_CHEAT_TRAP: CfiAntiCheatTrap = CfiAntiCheatTrap::new();
