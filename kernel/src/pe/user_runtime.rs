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

pub const SYS_NT_ALLOCATE_VIRTUAL_MEMORY: u32 = 0x0018;
pub const SYS_NT_CREATE_THREAD_EX: u32 = 0x00B4;
pub const SYS_NT_WAIT_FOR_SINGLE_OBJECT: u32 = 0x0004;
pub const SYS_NT_YIELD_EXECUTION: u32 = 0x0046;

/// High-performance Syscall Callgate Dispatcher processing NT syscall requests.
pub struct SyscallCallgateDispatcher {
    total_dispatched: AtomicU64,
    allocated_virtual_bytes: AtomicU64,
    created_threads_count: AtomicU64,
    yield_count: AtomicU64,
}

impl SyscallCallgateDispatcher {
    pub const fn new() -> Self {
        Self {
            total_dispatched: AtomicU64::new(0),
            allocated_virtual_bytes: AtomicU64::new(0),
            created_threads_count: AtomicU64::new(0),
            yield_count: AtomicU64::new(0),
        }
    }

    /// Fast syscall dispatch matching Microsoft x64 calling convention with registers RCX, RDX, R8, R9.
    pub fn dispatch(
        &self,
        syscall_id: u32,
        arg1_rcx: u64,
        arg2_rdx: u64,
        arg3_r8: u64,
        arg4_r9: u64,
    ) -> u32 {
        self.total_dispatched.fetch_add(1, Ordering::Relaxed);

        match syscall_id {
            SYS_NT_ALLOCATE_VIRTUAL_MEMORY => {
                // arg1: ProcessHandle, arg2: BaseAddress, arg3: RegionSize, arg4: Protect
                let process_handle = arg1_rcx;
                let _base_address = arg2_rdx;
                let region_size = arg3_r8;
                let _protect = arg4_r9;
                if process_handle == 0 || region_size == 0 {
                    return STATUS_INVALID_PARAMETER;
                }
                self.allocated_virtual_bytes.fetch_add(region_size, Ordering::Relaxed);
                STATUS_SUCCESS
            }
            SYS_NT_CREATE_THREAD_EX => {
                // arg1: ThreadHandle, arg2: DesiredAccess, arg3: StartRoutine, arg4: ProcessHandle
                let _thread_handle = arg1_rcx;
                let _access = arg2_rdx;
                let start_routine = arg3_r8;
                let process_handle = arg4_r9;
                if process_handle == 0 || start_routine == 0 {
                    return STATUS_INVALID_PARAMETER;
                }
                self.created_threads_count.fetch_add(1, Ordering::Relaxed);
                STATUS_SUCCESS
            }
            SYS_NT_WAIT_FOR_SINGLE_OBJECT => {
                // arg1: Handle, arg2: Alertable, arg3: Timeout, arg4: Reserved
                let handle = arg1_rcx;
                let _alertable = arg2_rdx;
                let _timeout = arg3_r8;
                let _reserved = arg4_r9;
                if handle == 0 {
                    return STATUS_INVALID_PARAMETER;
                }
                STATUS_SUCCESS
            }
            SYS_NT_YIELD_EXECUTION => {
                self.yield_count.fetch_add(1, Ordering::Relaxed);
                STATUS_SUCCESS
            }
            _ => STATUS_INVALID_PARAMETER,
        }
    }

    /// Retrieve total syscall requests processed.
    pub fn get_total_dispatched(&self) -> u64 {
        self.total_dispatched.load(Ordering::Acquire)
    }

    /// Retrieve total threads created via NtCreateThreadEx.
    pub fn get_created_threads(&self) -> u64 {
        self.created_threads_count.load(Ordering::Acquire)
    }

    /// Retrieve total yields via NtYieldExecution.
    pub fn get_yield_count(&self) -> u64 {
        self.yield_count.load(Ordering::Acquire)
    }
}

pub static SYSCALL_CALLGATE: SyscallCallgateDispatcher = SyscallCallgateDispatcher::new();

/// Userland CPU execution context holding user registers across syscall/sysret transitions.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserExecutionContext {
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
}

/// Userland Syscall and Sysret Transition Handler.
pub struct UserSyscallCallgate;

impl UserSyscallCallgate {
    /// Construct initial user-mode execution frame.
    pub fn setup_user_entry_frame(
        entry_point: u64,
        stack_base: u64,
        stack_size: usize,
    ) -> UserExecutionContext {
        let layout = UserStackLayout::new(stack_base, stack_size);
        UserExecutionContext {
            rip: entry_point,
            rsp: layout.initial_rsp,
            rflags: 0x0202,
            rax: 0,
            rcx: 0,
            rdx: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0x0202,
        }
    }

    /// Execute a simulated high-performance syscall/sysret context switch.
    pub fn execute_user_syscall(
        ctx: &mut UserExecutionContext,
        syscall_id: u32,
        arg1: u64,
        arg2: u64,
        arg3: u64,
        arg4: u64,
    ) -> u32 {
        // Syscall entry: RCX captures user RIP, R11 captures user RFLAGS
        ctx.rcx = ctx.rip;
        ctx.r11 = ctx.rflags;
        ctx.rax = syscall_id as u64;
        ctx.r10 = arg1;
        ctx.rdx = arg2;
        ctx.r8 = arg3;
        ctx.r9 = arg4;

        // Execute kernel callgate dispatch
        let status = SYSCALL_CALLGATE.dispatch(syscall_id, arg1, arg2, arg3, arg4);

        // Sysret return: RAX stores NTSTATUS, resume execution at RCX
        ctx.rax = status as u64;
        ctx.rip = ctx.rcx;
        ctx.rflags = ctx.r11;

        status
    }
}
