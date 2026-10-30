//! Win32 Structured Exception Handling: Dynamic Safe SEH Handler Validation & Stack Pivoting Guard.
//!
//! Implements dynamic runtime validation of SEH exception handler targets against PE SafeSEH tables
//! and image boundaries, stack pivoting and ROP chain anomaly detection during exception dispatch,
//! and anti-tampering exception filtering to defeat hook redirection and debugger detection traps.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;

/// Loaded module image boundary descriptor for SafeSEH verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafeSehImageDescriptor {
    pub image_base: u64,
    pub image_size: usize,
    pub has_safe_seh_table: bool,
}

/// Dynamic SafeSEH Handler Target Validator.
pub struct SafeSehValidator {
    images: Spinlock<BTreeMap<u64, SafeSehImageDescriptor>>,
    registered_handlers: Spinlock<BTreeSet<u64>>,
    total_validations: AtomicU64,
    total_violations_blocked: AtomicU64,
}

impl SafeSehValidator {
    pub const fn new() -> Self {
        Self {
            images: Spinlock::new(BTreeMap::new()),
            registered_handlers: Spinlock::new(BTreeSet::new()),
            total_validations: AtomicU64::new(0),
            total_violations_blocked: AtomicU64::new(0),
        }
    }

    /// Register a PE module and its SafeSEH status.
    pub fn register_image(&self, image_base: u64, image_size: usize, has_safe_seh_table: bool) {
        let mut img = self.images.lock();
        img.insert(image_base, SafeSehImageDescriptor {
            image_base,
            image_size,
            has_safe_seh_table,
        });
    }

    /// Register an allowed exception handler entry point.
    pub fn register_safe_handler(&self, handler_addr: u64) {
        self.registered_handlers.lock().insert(handler_addr);
    }

    /// Validate whether an exception handler target is safe to dispatch.
    pub fn validate_handler_target(&self, handler_addr: u64) -> Result<(), &'static str> {
        self.total_validations.fetch_add(1, Ordering::Relaxed);

        // Handler outside any loaded image -> rejected (e.g. heap/stack shellcode)
        let images = self.images.lock();
        let mut matching_image = None;
        for (&base, img) in images.iter() {
            if handler_addr >= base && handler_addr < base + img.image_size as u64 {
                matching_image = Some(*img);
                break;
            }
        }

        let img = match matching_image {
            Some(i) => i,
            None => {
                self.total_violations_blocked.fetch_add(1, Ordering::Relaxed);
                return Err("Handler target resides outside valid PE image boundaries");
            }
        };

        // If the image enforces SafeSEH, handler must be in registered table
        if img.has_safe_seh_table {
            let handlers = self.registered_handlers.lock();
            if !handlers.contains(&handler_addr) {
                self.total_violations_blocked.fetch_add(1, Ordering::Relaxed);
                return Err("Handler target not present in module SafeSEH table");
            }
        }

        Ok(())
    }

    /// Retrieve count of validated handlers.
    pub fn get_total_validations(&self) -> u64 {
        self.total_validations.load(Ordering::Acquire)
    }

    /// Retrieve count of blocked SafeSEH violations.
    pub fn get_total_blocked(&self) -> u64 {
        self.total_violations_blocked.load(Ordering::Acquire)
    }
}

pub static SAFE_SEH_VALIDATOR: SafeSehValidator = SafeSehValidator::new();

/// Fault classification for stack pivoting and frame alignment violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackPivotFault {
    RspMisaligned(u64),
    RbpMisaligned(u64),
    StackPivotDetected { rsp: u64, stack_limit: u64, stack_base: u64 },
    InvertedFramePointers { rsp: u64, rbp: u64 },
}

/// Stack pivoting and ROP chain anomaly detection engine.
pub struct StackPivotGuard {
    total_frame_checks: AtomicU64,
    total_pivots_blocked: AtomicU64,
}

impl StackPivotGuard {
    pub const fn new() -> Self {
        Self {
            total_frame_checks: AtomicU64::new(0),
            total_pivots_blocked: AtomicU64::new(0),
        }
    }

    /// Verify thread stack frame registers during exception dispatch.
    pub fn verify_exception_stack_frame(
        &self,
        rsp: u64,
        rbp: u64,
        stack_base: u64,
        stack_limit: u64,
    ) -> Result<(), StackPivotFault> {
        self.total_frame_checks.fetch_add(1, Ordering::Relaxed);

        // 1. Verify 16-byte RSP alignment
        if (rsp & 0x0F) != 0 {
            self.total_pivots_blocked.fetch_add(1, Ordering::Relaxed);
            return Err(StackPivotFault::RspMisaligned(rsp));
        }

        // 2. Check for stack pivot (RSP outside legitimate thread stack bounds)
        if rsp < stack_limit || rsp > stack_base {
            self.total_pivots_blocked.fetch_add(1, Ordering::Relaxed);
            return Err(StackPivotFault::StackPivotDetected {
                rsp,
                stack_limit,
                stack_base,
            });
        }

        // 3. Verify RBP frame pointer constraints if active
        if rbp != 0 {
            if (rbp & 0x0F) != 0 {
                self.total_pivots_blocked.fetch_add(1, Ordering::Relaxed);
                return Err(StackPivotFault::RbpMisaligned(rbp));
            }
            if rbp < rsp || rbp > stack_base {
                self.total_pivots_blocked.fetch_add(1, Ordering::Relaxed);
                return Err(StackPivotFault::InvertedFramePointers { rsp, rbp });
            }
        }

        Ok(())
    }

    /// Retrieve count of frame validations performed.
    pub fn get_total_frame_checks(&self) -> u64 {
        self.total_frame_checks.load(Ordering::Acquire)
    }

    /// Retrieve count of blocked stack pivots.
    pub fn get_total_pivots_blocked(&self) -> u64 {
        self.total_pivots_blocked.load(Ordering::Acquire)
    }
}

pub static STACK_PIVOT_GUARD: StackPivotGuard = StackPivotGuard::new();

pub const STATUS_BREAKPOINT: u32 = 0x8000_0003;
pub const STATUS_SINGLE_STEP: u32 = 0x8000_0004;

/// Exception filter verdict for anti-tamper and debugger protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterAction {
    ForwardToHandler,
    CloakDebuggerTrap { resume_rip: u64 },
    BlockMaliciousHook { reason: &'static str },
}

/// Anti-tampering exception filter preventing hook redirection and debugger detection traps.
pub struct AntiTamperExceptionFilter {
    protected_hooks: Spinlock<BTreeSet<u64>>,
    debugger_traps_cloaked: AtomicU64,
    malicious_hooks_thwarted: AtomicU64,
}

impl AntiTamperExceptionFilter {
    pub const fn new() -> Self {
        Self {
            protected_hooks: Spinlock::new(BTreeSet::new()),
            debugger_traps_cloaked: AtomicU64::new(0),
            malicious_hooks_thwarted: AtomicU64::new(0),
        }
    }

    /// Protect a function entry address against SEH hook hijacking.
    pub fn protect_hook_target(&self, rip: u64) {
        self.protected_hooks.lock().insert(rip);
    }

    /// Filter an exception and determine anti-tampering disposition.
    pub fn filter_exception(
        &self,
        exception_code: u32,
        fault_rip: u64,
        target_handler: u64,
    ) -> FilterAction {
        // Virtualize debugger traps (INT3 / single step)
        if exception_code == STATUS_BREAKPOINT || exception_code == STATUS_SINGLE_STEP {
            self.debugger_traps_cloaked.fetch_add(1, Ordering::Relaxed);
            return FilterAction::CloakDebuggerTrap {
                resume_rip: fault_rip.saturating_add(1),
            };
        }

        // Prevent hook hijacking on protected sensitive game hooks
        if self.protected_hooks.lock().contains(&fault_rip) {
            if SAFE_SEH_VALIDATOR.validate_handler_target(target_handler).is_err() {
                self.malicious_hooks_thwarted.fetch_add(1, Ordering::Relaxed);
                return FilterAction::BlockMaliciousHook {
                    reason: "Unauthorized SEH redirection on protected game hook",
                };
            }
        }

        FilterAction::ForwardToHandler
    }

    /// Retrieve count of cloaked debugger traps.
    pub fn get_cloaked_traps(&self) -> u64 {
        self.debugger_traps_cloaked.load(Ordering::Acquire)
    }

    /// Retrieve count of thwarted hook hijackings.
    pub fn get_thwarted_hooks(&self) -> u64 {
        self.malicious_hooks_thwarted.load(Ordering::Acquire)
    }

    /// Clear protected hook list.
    pub fn clear(&self) {
        self.protected_hooks.lock().clear();
        self.debugger_traps_cloaked.store(0, Ordering::Relaxed);
        self.malicious_hooks_thwarted.store(0, Ordering::Relaxed);
    }
}

pub static ANTI_TAMPER_FILTER: AntiTamperExceptionFilter = AntiTamperExceptionFilter::new();

/// Comprehensive self-test suite for Win32 SafeSEH Target Validation & Stack Pivot Guard.
pub fn test_win32_safe_seh_subsystem() -> bool {
    crate::serial_write_str("[KNT-WIN32-SAFE-SEH] Initializing Win32 SafeSEH Subsystem self-test...\r\n");

    // 1. Verify SafeSEH Table Registration & Target Validation
    let image_base = 0x0000_0000_0040_0000u64;
    let image_size = 0x0002_0000usize; // 128 KB
    SAFE_SEH_VALIDATOR.register_image(image_base, image_size, true);

    let valid_handler = 0x0000_0000_0040_1500u64;
    let unregistered_handler = 0x0000_0000_0040_1900u64;
    let foreign_handler = 0x0000_7FFF_1111_0000u64;

    SAFE_SEH_VALIDATOR.register_safe_handler(valid_handler);

    if SAFE_SEH_VALIDATOR.validate_handler_target(valid_handler).is_err() {
        crate::serial_write_str("[KNT-TEST-ERR] Valid registered SafeSEH handler failed validation\r\n");
        return false;
    }

    if SAFE_SEH_VALIDATOR.validate_handler_target(unregistered_handler).is_ok() {
        crate::serial_write_str("[KNT-TEST-ERR] Unregistered handler within SafeSEH module was allowed\r\n");
        return false;
    }

    if SAFE_SEH_VALIDATOR.validate_handler_target(foreign_handler).is_ok() {
        crate::serial_write_str("[KNT-TEST-ERR] Foreign handler outside image boundaries was allowed\r\n");
        return false;
    }

    if SAFE_SEH_VALIDATOR.get_total_blocked() < 2 {
        crate::serial_write_str("[KNT-TEST-ERR] Blocked SafeSEH violations count mismatch\r\n");
        return false;
    }

    // 2. Verify Stack Pivoting & ROP Chain Frame Alignment Guard
    let stack_base = 0x0000_7FFF_8000_8000u64;
    let stack_limit = 0x0000_7FFF_8000_0000u64;

    let valid_rsp = 0x0000_7FFF_8000_2000u64;
    let valid_rbp = 0x0000_7FFF_8000_2020u64;

    if STACK_PIVOT_GUARD.verify_exception_stack_frame(valid_rsp, valid_rbp, stack_base, stack_limit).is_err() {
        crate::serial_write_str("[KNT-TEST-ERR] Valid stack frame failed alignment verification\r\n");
        return false;
    }

    // Misaligned RSP
    let misaligned_rsp = 0x0000_7FFF_8000_2007u64;
    match STACK_PIVOT_GUARD.verify_exception_stack_frame(misaligned_rsp, valid_rbp, stack_base, stack_limit) {
        Err(StackPivotFault::RspMisaligned(_)) => {}
        _ => {
            crate::serial_write_str("[KNT-TEST-ERR] Misaligned RSP not detected\r\n");
            return false;
        }
    }

    // Stack pivot into heap
    let pivoted_rsp = 0x0000_0000_1000_0000u64;
    match STACK_PIVOT_GUARD.verify_exception_stack_frame(pivoted_rsp, 0, stack_base, stack_limit) {
        Err(StackPivotFault::StackPivotDetected { .. }) => {}
        _ => {
            crate::serial_write_str("[KNT-TEST-ERR] Stack pivot outside bounds not detected\r\n");
            return false;
        }
    }

    // Inverted RBP
    let inverted_rbp = 0x0000_7FFF_8000_1000u64; // Below RSP
    match STACK_PIVOT_GUARD.verify_exception_stack_frame(valid_rsp, inverted_rbp, stack_base, stack_limit) {
        Err(StackPivotFault::InvertedFramePointers { .. }) => {}
        _ => {
            crate::serial_write_str("[KNT-TEST-ERR] Inverted RBP frame pointer not detected\r\n");
            return false;
        }
    }

    if STACK_PIVOT_GUARD.get_total_pivots_blocked() < 3 {
        crate::serial_write_str("[KNT-TEST-ERR] Blocked stack pivots counter mismatch\r\n");
        return false;
    }

    // 3. Verify Anti-Tampering Exception Filter & Debugger Trap Cloaking
    let bp_action = ANTI_TAMPER_FILTER.filter_exception(STATUS_BREAKPOINT, 0x0000_0040_1000, 0);
    match bp_action {
        FilterAction::CloakDebuggerTrap { resume_rip } if resume_rip == 0x0000_0040_1001 => {}
        _ => {
            crate::serial_write_str("[KNT-TEST-ERR] Breakpoint exception was not cloaked\r\n");
            return false;
        }
    }

    let hook_entry = 0x0000_0040_3000u64;
    ANTI_TAMPER_FILTER.protect_hook_target(hook_entry);

    let hijack_action = ANTI_TAMPER_FILTER.filter_exception(0xC000_0005, hook_entry, foreign_handler);
    match hijack_action {
        FilterAction::BlockMaliciousHook { .. } => {}
        _ => {
            crate::serial_write_str("[KNT-TEST-ERR] Malicious SEH hook redirection was not blocked\r\n");
            return false;
        }
    }

    if ANTI_TAMPER_FILTER.get_cloaked_traps() == 0 || ANTI_TAMPER_FILTER.get_thwarted_hooks() == 0 {
        crate::serial_write_str("[KNT-TEST-ERR] Anti-tamper filter counters mismatch\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-SAFE-SEH-PASS] Win32 SafeSEH Validation & Stack Pivoting Guard verified\r\n");
    true
}
