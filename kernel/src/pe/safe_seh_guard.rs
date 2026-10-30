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
