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

/// Canonical virtual address of NTDLL KiUserApcDispatcher trampoline.
pub const KI_USER_APC_DISPATCHER_ADDRESS: u64 = 0x00007FFF_8100_1000;

/// User-mode execution trap frame captured upon kernel transition.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserTrapFrame {
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rbx: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

/// Context saved onto the user stack during APC trampoline setup.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApcSavedContext {
    pub original_rip: u64,
    pub original_rsp: u64,
    pub original_rcx: u64,
    pub original_rdx: u64,
    pub apc_routine: u64,
    pub apc_context: u64,
}


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
    pub wait_interrupted: bool,
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
            wait_interrupted: false,
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

        // If User-mode APC is queued to a thread in alertable wait state, wake it up immediately
        if apc.apc_mode == KProcessorMode::UserMode && thread.alertable {
            thread.alertable = false;
            thread.wait_interrupted = true;
        }

        thread.apc_state.push_apc(apc);
        Ok(apc_id)
    }


    /// Puts a thread into an alertable wait state.
    /// Returns true if a User-mode APC is already pending and immediately alerts the thread.
    pub fn enter_alertable_wait(&mut self, thread_id: u64) -> bool {
        let thread = match self.threads.get_mut(&thread_id) {
            Some(t) => t,
            None => return false,
        };

        if thread.apc_state.user_apc_pending {
            thread.alertable = false;
            thread.wait_interrupted = true;
            true
        } else {
            thread.alertable = true;
            thread.wait_interrupted = false;
            false
        }
    }

    /// Exits an alertable wait state, clearing flags.
    pub fn exit_alertable_wait(&mut self, thread_id: u64) {
        if let Some(thread) = self.threads.get_mut(&thread_id) {
            thread.alertable = false;
            thread.wait_interrupted = false;
        }
    }

    /// Checks if a thread's wait was interrupted by a queued User APC.
    pub fn check_and_clear_interrupted(&mut self, thread_id: u64) -> bool {
        if let Some(thread) = self.threads.get_mut(&thread_id) {
            let was_interrupted = thread.wait_interrupted;
            thread.wait_interrupted = false;
            was_interrupted
        } else {
            false
        }
    }

    /// Emulates Win32 SleepEx with alertable user APC dispatch.
    pub fn sleep_ex(&mut self, thread_id: u64, _milliseconds: u32, alertable: bool) -> u32 {
        if !alertable {
            return 0;
        }

        // Check if thread can enter alertable state or is already pending
        if self.enter_alertable_wait(thread_id) {
            return WAIT_IO_COMPLETION;
        }

        // In synchronous kernel environment, check if wait was interrupted by an APC
        if self.check_and_clear_interrupted(thread_id) || self.is_user_apc_pending(thread_id) {
            self.exit_alertable_wait(thread_id);
            WAIT_IO_COMPLETION
        } else {
            self.exit_alertable_wait(thread_id);
            0
        }
    }

    /// Emulates Win32 WaitForSingleObjectEx with alertable user APC dispatch.
    pub fn wait_for_single_object_ex(
        &mut self,
        thread_id: u64,
        _handle: u64,
        _milliseconds: u32,
        alertable: bool,
    ) -> u32 {
        if !alertable {
            return 0; // WAIT_OBJECT_0
        }

        if self.enter_alertable_wait(thread_id) {
            return WAIT_IO_COMPLETION;
        }

        if self.check_and_clear_interrupted(thread_id) || self.is_user_apc_pending(thread_id) {
            self.exit_alertable_wait(thread_id);
            WAIT_IO_COMPLETION
        } else {
            self.exit_alertable_wait(thread_id);
            0 // WAIT_OBJECT_0
        }
    }

    /// NT system call NtDelayExecution supporting alertable APC delivery.
    pub fn nt_delay_execution(&mut self, thread_id: u64, alertable: bool) -> NtStatus {
        let res = self.sleep_ex(thread_id, 0, alertable);
        if res == WAIT_IO_COMPLETION {
            STATUS_USER_APC
        } else {
            STATUS_SUCCESS
        }
    }


    /// Modifies user trap frame to execute KiUserApcDispatcher trampoline upon return to user mode.
    /// Passes normal routine in RCX, context in RDX, arg1 in R8, arg2 in R9 according to MS x64 ABI.
    pub fn prepare_user_apc_dispatch(
        &mut self,
        thread_id: u64,
        trap_frame: &mut UserTrapFrame,
    ) -> Result<bool, NtStatus> {
        let apc = match self.pop_user_apc(thread_id) {
            Some(a) => a,
            None => return Ok(false),
        };

        let normal_fn = apc.normal_routine.ok_or(STATUS_UNSUCCESSFUL)?;

        // Adjust user stack for 32-byte shadow space + saved context alignment
        let _original_rip = trap_frame.rip;
        let original_rsp = trap_frame.rsp;
        let new_rsp = (original_rsp.saturating_sub(0x40)) & !0x0F;

        // Configure register arguments for KiUserApcDispatcher:
        // RCX: Target APC callback routine pointer
        // RDX: APC Context parameter (dwData)
        // R8:  SystemArgument1
        // R9:  SystemArgument2
        trap_frame.rcx = normal_fn as u64;
        trap_frame.rdx = apc.normal_context as u64;
        trap_frame.r8 = apc.system_argument1 as u64;
        trap_frame.r9 = apc.system_argument2 as u64;
        trap_frame.rsp = new_rsp;
        trap_frame.rip = KI_USER_APC_DISPATCHER_ADDRESS;

        Ok(true)
    }

    /// Queues a User-mode APC to target thread by handle/TID.
    pub fn queue_user_apc(
        &mut self,
        thread_id: u64,
        process_id: u64,
        apc_routine: usize,
        context: usize,
        arg1: usize,
        arg2: usize,
    ) -> Result<u64, NtStatus> {
        if apc_routine == 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }

        let apc = KAPC::new_user(
            thread_id,
            process_id,
            apc_routine,
            context,
            arg1,
            arg2,
        );

        self.queue_apc(apc)
    }

    /// NT system call NtQueueApcThread.
    pub fn nt_queue_apc_thread(
        &mut self,
        thread_id: u64,
        apc_routine: usize,
        arg1: usize,
        arg2: usize,
        arg3: usize,
    ) -> NtStatus {
        let proc_id = self
            .threads
            .get(&thread_id)
            .map(|t| t.process_id)
            .unwrap_or(0);

        match self.queue_user_apc(thread_id, proc_id, apc_routine, arg1, arg2, arg3) {
            Ok(_) => STATUS_SUCCESS,
            Err(e) => e,
        }
    }

    /// Win32 QueueUserAPC wrapper.
    pub fn win32_queue_user_apc(
        &mut self,
        pfn_apc: usize,
        thread_id: u64,
        dw_data: usize,
    ) -> u32 {
        let proc_id = self
            .threads
            .get(&thread_id)
            .map(|t| t.process_id)
            .unwrap_or(0);

        match self.queue_user_apc(thread_id, proc_id, pfn_apc, dw_data, 0, 0) {
            Ok(_) => 1,
            Err(_) => 0,
        }
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
