//! Win32 Dynamic Thread Stack Expansion & Guard Page Fault Trap Handler.
//!
//! Implements dynamic user thread stack growth with committed/reserved memory bounds,
//! `PAGE_GUARD` page fault interception and transparent page allocation,
//! stack overflow protection (`STATUS_STACK_OVERFLOW`) and emergency user-mode trap handler,
//! and handheld stack page reclamation governor for idle background game threads.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER,
    ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
pub use crate::pe::ntdll::NtStatus;
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

pub const STATUS_GUARD_PAGE_VIOLATION: NtStatus = 0x8000_0001;
pub const STATUS_STACK_OVERFLOW: NtStatus = 0xC000_00FD;
pub const STATUS_STACK_BUFFER_OVERRUN: NtStatus = 0xC000_0409;

pub use crate::pe::virtual_alloc::PAGE_SIZE;

pub const DEFAULT_STACK_RESERVE: usize = 1024 * 1024; // 1 MiB standard Win32 thread reserve
pub const DEFAULT_STACK_COMMIT: usize = 64 * 1024;    // 64 KiB initial commit

/// Dynamic thread stack representation.
#[derive(Debug, Clone)]
pub struct DynamicThreadStack {
    pub thread_id: u64,
    pub stack_base: usize,       // Highest address (top of stack memory where RSP starts)
    pub stack_limit: usize,      // Lowest committed boundary address
    pub stack_reserve_bottom: usize, // Hard bottom (cannot grow lower than this)
    pub guard_page_addr: usize,  // Address of current guard page
    pub committed_bytes: usize,
    pub reserved_bytes: usize,
    pub overflow_triggered: bool,
    pub is_idle: bool,
}

impl DynamicThreadStack {
    pub fn new(thread_id: u64, stack_base: usize, reserved_bytes: usize, initial_commit_bytes: usize) -> Self {
        let reserved = (reserved_bytes + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let commit = (initial_commit_bytes + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let stack_reserve_bottom = stack_base.saturating_sub(reserved);
        let stack_limit = stack_base.saturating_sub(commit);
        let guard_page_addr = stack_limit.saturating_sub(PAGE_SIZE);

        Self {
            thread_id,
            stack_base,
            stack_limit,
            stack_reserve_bottom,
            guard_page_addr,
            committed_bytes: commit,
            reserved_bytes: reserved,
            overflow_triggered: false,
            is_idle: false,
        }
    }
}

static THREAD_STACKS: Spinlock<BTreeMap<u64, DynamicThreadStack>> = Spinlock::new(BTreeMap::new());

/// Registers a thread stack with specified dynamic reserved and initial committed bounds.
pub fn win32_register_dynamic_stack(
    thread_id: u64,
    stack_base: usize,
    reserved_bytes: usize,
    initial_commit_bytes: usize,
) -> bool {
    let stack = DynamicThreadStack::new(thread_id, stack_base, reserved_bytes, initial_commit_bytes);
    THREAD_STACKS.lock().insert(thread_id, stack);
    true
}

/// Retrieves stack bounds (base, limit, reserve bottom) for a thread.
pub fn win32_get_dynamic_stack_bounds(thread_id: u64) -> Option<(usize, usize, usize)> {
    let stacks = THREAD_STACKS.lock();
    stacks.get(&thread_id).map(|s| (s.stack_base, s.stack_limit, s.stack_reserve_bottom))
}
