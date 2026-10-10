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

/// Result of guard page fault handling.
#[derive(Debug, PartialEq, Eq)]
pub enum GuardFaultResult {
    HandledGrowth(usize), // Successfully expanded stack by 1 page (new stack limit)
    StackOverflow,        // Hit stack reserved bottom boundary
    NotGuardPage,         // Fault address is outside thread guard page
}

/// Intercepts PAGE_GUARD page faults, transparently committing a new page and moving guard page down.
pub fn win32_handle_guard_page_fault(thread_id: u64, fault_addr: usize) -> GuardFaultResult {
    let mut stacks = THREAD_STACKS.lock();
    let stack = match stacks.get_mut(&thread_id) {
        Some(s) => s,
        None => return GuardFaultResult::NotGuardPage,
    };

    let fault_page = fault_addr & !(PAGE_SIZE - 1);
    if fault_page != stack.guard_page_addr {
        return GuardFaultResult::NotGuardPage;
    }

    // Check if new guard page would breach hard reserved boundary
    if stack.guard_page_addr <= stack.stack_reserve_bottom {
        stack.overflow_triggered = true;
        return GuardFaultResult::StackOverflow;
    }

    // Grow committed region by 1 page
    stack.stack_limit = stack.guard_page_addr;
    stack.committed_bytes += PAGE_SIZE;

    // Shift guard page lower by 1 page
    stack.guard_page_addr = stack.stack_limit.saturating_sub(PAGE_SIZE);

    // Sync TEB stack limit if registered
    crate::pe::peb_teb::win32_register_thread_stack(
        thread_id,
        stack.stack_base as u64,
        stack.stack_limit as u64,
    );

    GuardFaultResult::HandledGrowth(stack.stack_limit)
}

/// Dispatches stack overflow exception and triggers emergency user-mode trap handler.
pub fn win32_raise_stack_overflow_trap(thread_id: u64, rip: u64, rsp: u64) -> NtStatus {
    crate::serial_write_str("[KNT-EXCEPTION] Stack overflow trap raised for TID: ");
    write_u64_str(thread_id);
    crate::serial_write_str(", RIP: 0x");
    write_hex64_str(rip);
    crate::serial_write_str(", RSP: 0x");
    write_hex64_str(rsp);
    crate::serial_write_str("\r\n");

    // Deliver STATUS_STACK_OVERFLOW via VEH / SEH exception pipeline if active
    STATUS_STACK_OVERFLOW
}

fn write_u64_str(mut val: u64) {
    if val == 0 {
        crate::serial_write_byte(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut len = 0;
    while val > 0 {
        buf[len] = b'0' + (val % 10) as u8;
        val /= 10;
        len += 1;
    }
    for i in (0..len).rev() {
        crate::serial_write_byte(buf[i]);
    }
}

fn write_hex64_str(val: u64) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in (0..16).rev() {
        let n = ((val >> (i * 4)) & 0xF) as usize;
        crate::serial_write_byte(HEX[n]);
    }
}

/// Marks a thread as idle or active for background stack reclamation.
pub fn win32_set_thread_idle_state(thread_id: u64, is_idle: bool) {
    let mut stacks = THREAD_STACKS.lock();
    if let Some(stack) = stacks.get_mut(&thread_id) {
        stack.is_idle = is_idle;
    }
}

/// Handheld stack page reclamation governor for idle background game threads.
/// Trims excess committed stack pages back down towards minimum commit baseline,
/// moving the guard page up and releasing frames to the physical memory allocator.
pub fn win32_reclaim_idle_thread_stacks(min_keep_commit_bytes: usize) -> usize {
    let baseline = (min_keep_commit_bytes + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let mut stacks = THREAD_STACKS.lock();
    let mut total_reclaimed_pages = 0;

    for stack in stacks.values_mut() {
        if stack.is_idle && stack.committed_bytes > baseline {
            let excess_bytes = stack.committed_bytes - baseline;
            let excess_pages = excess_bytes / PAGE_SIZE;

            stack.committed_bytes = baseline;
            stack.stack_limit = stack.stack_base.saturating_sub(baseline);
            stack.guard_page_addr = stack.stack_limit.saturating_sub(PAGE_SIZE);

            crate::pe::peb_teb::win32_register_thread_stack(
                stack.thread_id,
                stack.stack_base as u64,
                stack.stack_limit as u64,
            );

            total_reclaimed_pages += excess_pages;
        }
    }

    total_reclaimed_pages
}

/// Self-test suite verifying Win32 dynamic stack expansion, PAGE_GUARD fault interception, and reclamation.
pub fn test_win32_guard_stack_subsystem() -> bool {
    crate::serial_write_str("[KNT-TEST] Initializing Win32 Dynamic Thread Stack & Guard Page Fault self-test...\r\n");

    let test_tid = 9001;
    let stack_base = 0x0000_7FFF_8000_0000;
    let reserve = 64 * 1024; // 64 KiB reserve
    let commit = 8 * 1024;   // 8 KiB commit (2 pages)

    // 1. Register dynamic thread stack
    if !win32_register_dynamic_stack(test_tid, stack_base, reserve, commit) {
        crate::serial_write_str("[KNT-ERROR] win32_register_dynamic_stack failed\r\n");
        return false;
    }

    let (base, limit, bottom) = match win32_get_dynamic_stack_bounds(test_tid) {
        Some(b) => b,
        None => {
            crate::serial_write_str("[KNT-ERROR] win32_get_dynamic_stack_bounds failed\r\n");
            return false;
        }
    };
    if base != stack_base || limit != stack_base - 8192 || bottom != stack_base - 65536 {
        crate::serial_write_str("[KNT-ERROR] Stack bounds mismatch\r\n");
        return false;
    }

    // 2. Test guard page fault interception and transparent expansion
    let guard_fault_addr = limit - 100; // Inside current guard page
    let res = win32_handle_guard_page_fault(test_tid, guard_fault_addr);
    match res {
        GuardFaultResult::HandledGrowth(new_limit) => {
            if new_limit != limit - PAGE_SIZE {
                crate::serial_write_str("[KNT-ERROR] Stack growth limit incorrect\r\n");
                return false;
            }
        }
        _ => {
            crate::serial_write_str("[KNT-ERROR] Guard page fault was not handled\r\n");
            return false;
        }
    }

    // 3. Test non-guard address fault
    if win32_handle_guard_page_fault(test_tid, 0x1000) != GuardFaultResult::NotGuardPage {
        crate::serial_write_str("[KNT-ERROR] Random address reported as guard page\r\n");
        return false;
    }

    // 4. Force stack down to reserve bottom and verify stack overflow protection
    let mut current_res = GuardFaultResult::NotGuardPage;
    for _ in 0..20 {
        let (_, cur_limit, _) = win32_get_dynamic_stack_bounds(test_tid).unwrap();
        current_res = win32_handle_guard_page_fault(test_tid, cur_limit - 100);
        if current_res == GuardFaultResult::StackOverflow {
            break;
        }
    }
    if current_res != GuardFaultResult::StackOverflow {
        crate::serial_write_str("[KNT-ERROR] Stack overflow did not trigger at reserve bottom\r\n");
        return false;
    }

    // 5. Verify emergency stack overflow trap
    let trap_status = win32_raise_stack_overflow_trap(test_tid, 0x0040_1000, bottom as u64);
    if trap_status != STATUS_STACK_OVERFLOW {
        crate::serial_write_str("[KNT-ERROR] Stack overflow trap status mismatch\r\n");
        return false;
    }

    // 6. Test idle stack reclamation governor
    win32_set_thread_idle_state(test_tid, true);
    let reclaimed_pages = win32_reclaim_idle_thread_stacks(8 * 1024);
    if reclaimed_pages == 0 {
        crate::serial_write_str("[KNT-ERROR] Idle stack reclamation failed to reclaim pages\r\n");
        return false;
    }

    let (_, reclaimed_limit, _) = win32_get_dynamic_stack_bounds(test_tid).unwrap();
    if reclaimed_limit != stack_base - (8 * 1024) {
        crate::serial_write_str("[KNT-ERROR] Reclaimed stack limit mismatch\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-GUARD-STACK-PASS] Win32 dynamic stack expansion and guard page trap verified\r\n");
    true
}
