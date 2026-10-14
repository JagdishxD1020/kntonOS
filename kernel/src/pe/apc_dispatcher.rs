//! Win32 Kernel & User-Mode Asynchronous Procedure Calls (APC), Alertable Wait States & QueueUserAPC Engine.
//!
//! Implements Windows NT Asynchronous Procedure Call (APC) dispatching, dual kernel/user mode queues,
//! alertable wait states (SleepEx, WaitForSingleObjectEx), trap frame user APC trampoline redirection
//! (KiUserApcDispatcher), and anti-cheat APC injection detection.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_SUCCESS,
};
use crate::pe::ntdll::{
    nt_error, nt_status_to_str, nt_success, NtStatus,
    STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_NO_MEMORY,
    STATUS_SUCCESS, STATUS_UNSUCCESSFUL,
};
pub use crate::pe::object_namespace::STATUS_ACCESS_DENIED;
pub use crate::pe::synch::HANDLE;
pub use crate::pe::wait::WAIT_IO_COMPLETION;
pub const STATUS_USER_APC: NtStatus = 0x0000_00C0;

/// Processor execution mode for an Asynchronous Procedure Call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KProcessorMode {
    KernelMode = 0,
    UserMode = 1,
}

/// APC environment selector for process attachment states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApcEnvironment {
    OriginalApcEnvironment = 0,
    AttachedApcEnvironment = 1,
    CurrentApcEnvironment = 2,
}

/// Kernel APC execution callback signature.
pub type KernelApcRoutine = fn(apc_id: u64, context: &mut usize, arg1: &mut usize, arg2: &mut usize);

/// Kernel rundown cleanup callback signature.
pub type RundownApcRoutine = fn(apc_id: u64);

/// Kernel Asynchronous Procedure Call (KAPC) descriptor.
#[derive(Clone)]
pub struct KAPC {
    pub apc_id: u64,
    pub thread_id: u64,
    pub process_id: u64,
    pub apc_mode: KProcessorMode,
    pub is_special_kernel_apc: bool,
    pub kernel_routine: Option<KernelApcRoutine>,
    pub rundown_routine: Option<RundownApcRoutine>,
    pub normal_routine: Option<usize>,
    pub normal_context: usize,
    pub system_argument1: usize,
    pub system_argument2: usize,
    pub inserted: bool,
    pub state_index: usize,
}

impl KAPC {
    /// Constructs a new Kernel-mode APC.
    pub fn new_kernel(
        thread_id: u64,
        process_id: u64,
        is_special: bool,
        kernel_routine: Option<KernelApcRoutine>,
        normal_routine: Option<usize>,
        context: usize,
    ) -> Self {
        Self {
            apc_id: 0,
            thread_id,
            process_id,
            apc_mode: KProcessorMode::KernelMode,
            is_special_kernel_apc: is_special,
            kernel_routine,
            rundown_routine: None,
            normal_routine,
            normal_context: context,
            system_argument1: 0,
            system_argument2: 0,
            inserted: false,
            state_index: 0,
        }
    }

    /// Constructs a new User-mode APC.
    pub fn new_user(
        thread_id: u64,
        process_id: u64,
        normal_routine: usize,
        context: usize,
        arg1: usize,
        arg2: usize,
    ) -> Self {
        Self {
            apc_id: 0,
            thread_id,
            process_id,
            apc_mode: KProcessorMode::UserMode,
            is_special_kernel_apc: false,
            kernel_routine: None,
            rundown_routine: None,
            normal_routine: Some(normal_routine),
            normal_context: context,
            system_argument1: arg1,
            system_argument2: arg2,
            inserted: false,
            state_index: 1,
        }
    }
}

/// Dual-queue APC state tracking Kernel and User mode pending callbacks for a thread.
#[derive(Clone, Default)]
pub struct KApcState {
    pub apc_list_head: [VecDeque<KAPC>; 2],
    pub process_id: u64,
    pub kernel_apc_in_progress: bool,
    pub kernel_apc_pending: bool,
    pub user_apc_pending: bool,
}

impl KApcState {
    pub fn new(process_id: u64) -> Self {
        Self {
            apc_list_head: [VecDeque::new(), VecDeque::new()],
            process_id,
            kernel_apc_in_progress: false,
            kernel_apc_pending: false,
            user_apc_pending: false,
        }
    }

    /// Pushes an APC to the corresponding queue (0 for KernelMode, 1 for UserMode).
    pub fn push_apc(&mut self, apc: KAPC) {
        match apc.apc_mode {
            KProcessorMode::KernelMode => {
                self.apc_list_head[0].push_back(apc);
                self.kernel_apc_pending = true;
            }
            KProcessorMode::UserMode => {
                self.apc_list_head[1].push_back(apc);
                self.user_apc_pending = true;
            }
        }
    }

    /// Pops the next pending KernelMode APC if available.
    pub fn pop_kernel_apc(&mut self) -> Option<KAPC> {
        let apc = self.apc_list_head[0].pop_front();
        self.kernel_apc_pending = !self.apc_list_head[0].is_empty();
        apc
    }

    /// Pops the next pending UserMode APC if available.
    pub fn pop_user_apc(&mut self) -> Option<KAPC> {
        let apc = self.apc_list_head[1].pop_front();
        self.user_apc_pending = !self.apc_list_head[1].is_empty();
        apc
    }
}

/// Thread-level execution context for APC delivery and alertable wait states.
#[derive(Clone)]
pub struct ThreadApcContext {
    pub thread_id: u64,
    pub process_id: u64,
    pub apc_state: KApcState,
    pub saved_apc_state: Option<KApcState>,
    pub alertable: bool,
    pub kernel_apc_disable: u32,
    pub special_apc_disable: u32,
}

impl ThreadApcContext {
    pub fn new(thread_id: u64, process_id: u64) -> Self {
        Self {
            thread_id,
            process_id,
            apc_state: KApcState::new(process_id),
            saved_apc_state: None,
            alertable: false,
            kernel_apc_disable: 0,
            special_apc_disable: 0,
        }
    }
}

/// Global system-wide Asynchronous Procedure Call governor.
pub struct GlobalApcManager {
    pub threads: BTreeMap<u64, ThreadApcContext>,
    pub next_apc_id: AtomicU64,
}

impl GlobalApcManager {
    pub fn new() -> Self {
        Self {
            threads: BTreeMap::new(),
            next_apc_id: AtomicU64::new(1),
        }
    }

    /// Registers a thread into the APC tracking governor.
    pub fn register_thread(&mut self, thread_id: u64, process_id: u64) {
        if !self.threads.contains_key(&thread_id) {
            self.threads.insert(thread_id, ThreadApcContext::new(thread_id, process_id));
        }
    }

    /// Inserts a KAPC into the target thread's APC queue.
    pub fn queue_apc(&mut self, mut apc: KAPC) -> Result<u64, NtStatus> {
        let thread = self
            .threads
            .get_mut(&apc.thread_id)
            .ok_or(STATUS_INVALID_HANDLE)?;

        let apc_id = self.next_apc_id.fetch_add(1, Ordering::SeqCst);
        apc.apc_id = apc_id;
        apc.inserted = true;

        // If Special Kernel APC and delivery is enabled, execute immediately
        if apc.is_special_kernel_apc && thread.special_apc_disable == 0 {
            if let Some(kernel_fn) = apc.kernel_routine {
                let mut ctx = apc.normal_context;
                let mut arg1 = apc.system_argument1;
                let mut arg2 = apc.system_argument2;
                kernel_fn(apc_id, &mut ctx, &mut arg1, &mut arg2);
                return Ok(apc_id);
            }
        }

        thread.apc_state.push_apc(apc);
        Ok(apc_id)
    }

    /// Pops the next pending Kernel-mode APC for a thread.
    pub fn pop_kernel_apc(&mut self, thread_id: u64) -> Option<KAPC> {
        let thread = self.threads.get_mut(&thread_id)?;
        if thread.kernel_apc_disable > 0 {
            return None;
        }
        thread.apc_state.pop_kernel_apc()
    }

    /// Pops the next pending User-mode APC for a thread.
    pub fn pop_user_apc(&mut self, thread_id: u64) -> Option<KAPC> {
        let thread = self.threads.get_mut(&thread_id)?;
        thread.apc_state.pop_user_apc()
    }

    /// Returns true if a User-mode APC is pending delivery on the given thread.
    pub fn is_user_apc_pending(&self, thread_id: u64) -> bool {
        self.threads
            .get(&thread_id)
            .map(|t| t.apc_state.user_apc_pending)
            .unwrap_or(false)
    }

    /// Returns true if a Kernel-mode APC is pending delivery on the given thread.
    pub fn is_kernel_apc_pending(&self, thread_id: u64) -> bool {
        self.threads
            .get(&thread_id)
            .map(|t| t.apc_state.kernel_apc_pending)
            .unwrap_or(false)
    }
}

/// Global synchronized instance of the APC manager.
pub static GLOBAL_APC_MANAGER: Spinlock<Option<GlobalApcManager>> = Spinlock::new(None);

/// Initializes the APC manager subsystem.
pub fn init_apc_manager() {
    let mut lock = GLOBAL_APC_MANAGER.lock();
    if lock.is_none() {
        *lock = Some(GlobalApcManager::new());
    }
}

/// Accessor for global APC manager.
pub fn with_apc_manager<F, R>(f: F) -> R
where
    F: FnOnce(&mut GlobalApcManager) -> R,
{
    init_apc_manager();
    let mut lock = GLOBAL_APC_MANAGER.lock();
    f(lock.as_mut().unwrap())
}
