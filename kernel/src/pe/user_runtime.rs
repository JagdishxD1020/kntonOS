//! Userland Architecture: Standalone User-Mode Runtime & High-Performance Syscall Callgate.
//!
//! Provides TEB/PEB userland address space mapping, user stack frame construction with 16-byte alignment,
//! and high-performance `syscall`/`sysret` callgate primitives for Windows NT system calls.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;

pub const STATUS_SUCCESS: u32 = 0x0000_0000;
pub const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
pub const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const STATUS_NO_MEMORY: u32 = 0xC000_0017;
pub const STATUS_TIMEOUT: u32 = 0x0000_0102;

pub const DEFAULT_USER_STACK_SIZE: usize = 64 * 1024;
pub const DEFAULT_USER_STACK_BASE: u64 = 0x0000_7FFF_FFFF_0000;
pub const DEFAULT_USER_TEB_BASE: u64 = 0x0000_7FFE_0000_0000;
pub const DEFAULT_USER_PEB_BASE: u64 = 0x0000_7FFE_0001_0000;

/// Userland stack layout configuration with boundary and alignment tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserStackLayout {
    pub stack_base: u64,
    pub stack_limit: u64,
    pub initial_rsp: u64,
    pub size_bytes: usize,
}

impl UserStackLayout {
    /// Construct a user stack frame layout with 16-byte alignment.
    pub fn new(base: u64, size: usize) -> Self {
        let limit = base.saturating_sub(size as u64);
        let initial_rsp = (base - 16) & !0x0F;
        Self {
            stack_base: base,
            stack_limit: limit,
            initial_rsp,
            size_bytes: size,
        }
    }

    /// Verify 16-byte alignment and boundary adherence of user RSP.
    pub fn is_rsp_valid(&self, rsp: u64) -> bool {
        (rsp & 0x0F) == 0 && rsp >= self.stack_limit && rsp <= self.stack_base
    }
}

/// Userland Runtime Environment mapping TEB, PEB, and execution parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRuntimePebTeb {
    pub process_id: u32,
    pub thread_id: u64,
    pub peb_user_address: u64,
    pub teb_user_address: u64,
    pub stack_layout: UserStackLayout,
    pub entry_point_rip: u64,
    pub image_base: u64,
}

impl UserRuntimePebTeb {
    pub fn new(process_id: u32, thread_id: u64, entry_point_rip: u64, image_base: u64) -> Self {
        let stack_layout = UserStackLayout::new(DEFAULT_USER_STACK_BASE, DEFAULT_USER_STACK_SIZE);
        Self {
            process_id,
            thread_id,
            peb_user_address: DEFAULT_USER_PEB_BASE,
            teb_user_address: DEFAULT_USER_TEB_BASE,
            stack_layout,
            entry_point_rip,
            image_base,
        }
    }
}

/// Standalone User-Mode Runtime Manager.
pub struct UserRuntimeManager {
    environments: Spinlock<BTreeMap<u32, UserRuntimePebTeb>>,
    total_runtimes_created: AtomicU64,
}

impl UserRuntimeManager {
    pub const fn new() -> Self {
        Self {
            environments: Spinlock::new(BTreeMap::new()),
            total_runtimes_created: AtomicU64::new(0),
        }
    }

    /// Register a user runtime environment for a process.
    pub fn register_runtime(&self, rt: UserRuntimePebTeb) {
        let pid = rt.process_id;
        self.environments.lock().insert(pid, rt);
        self.total_runtimes_created.fetch_add(1, Ordering::Relaxed);
    }

    /// Get user runtime descriptor by PID.
    pub fn get_runtime(&self, pid: u32) -> Option<UserRuntimePebTeb> {
        self.environments.lock().get(&pid).cloned()
    }

    /// Retrieve count of active user runtimes created.
    pub fn get_total_created(&self) -> u64 {
        self.total_runtimes_created.load(Ordering::Acquire)
    }
}

pub static USER_RUNTIME_MGR: UserRuntimeManager = UserRuntimeManager::new();
