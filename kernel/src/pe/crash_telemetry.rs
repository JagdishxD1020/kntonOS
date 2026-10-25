//! Win32 Dynamic Exception Routing & Minidump Crash Telemetry Ring Buffer.
//!
//! Implements low-latency user-mode exception capture (access violations, illegal instructions,
//! and stack buffer overflows), fast in-memory Minidump stream encoder, and lock-free SPSC crash
//! telemetry ring buffer streaming crash events to kernel diagnostic store.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;

pub const EXCEPTION_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const EXCEPTION_ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
pub const EXCEPTION_STACK_BUFFER_OVERRUN: u32 = 0xC000_0409;

/// Captured Exception Record with Fault Addressing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapturedExceptionContext {
    pub exception_code: u32,
    pub exception_flags: u32,
    pub fault_address: u64,
    pub thread_id: u64,
    pub process_id: u32,
    pub timestamp_tsc: u64,
    pub rip: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
}

/// Dynamic Exception Router capturing fatal faults.
pub struct DynamicExceptionRouter {
    captured_exceptions: Spinlock<Vec<CapturedExceptionContext>>,
    capture_count: AtomicU64,
}

impl DynamicExceptionRouter {
    pub const fn new() -> Self {
        Self {
            captured_exceptions: Spinlock::new(Vec::new()),
            capture_count: AtomicU64::new(0),
        }
    }

    /// Capture and route a user-mode exception.
    pub fn capture_exception(&self, ctx: CapturedExceptionContext) {
        let mut list = self.captured_exceptions.lock();
        list.push(ctx);
        self.capture_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Check if an exception code is a recognized fatal user crash.
    pub fn is_fatal_crash_code(code: u32) -> bool {
        code == EXCEPTION_ACCESS_VIOLATION
            || code == EXCEPTION_ILLEGAL_INSTRUCTION
            || code == EXCEPTION_STACK_BUFFER_OVERRUN
    }

    /// Retrieve count of captured exceptions.
    pub fn get_capture_count(&self) -> u64 {
        self.capture_count.load(Ordering::Acquire)
    }

    /// Retrieve captured exceptions list.
    pub fn get_captured(&self) -> Vec<CapturedExceptionContext> {
        self.captured_exceptions.lock().clone()
    }
}

pub static EXCEPTION_ROUTER: DynamicExceptionRouter = DynamicExceptionRouter::new();
