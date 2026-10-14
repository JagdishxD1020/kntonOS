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


pub use crate::pe::object_namespace::ProcessTrustLevel;

/// Violations detected by the anti-cheat APC injection governor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntiCheatApcViolationKind {
    None,
    CrossProcessInjection,
    UnbackedCodePageInjection,
    UnauthorizedThreadDetour,
    SuspiciousPayloadTarget,
}

/// Audit event capturing an anti-cheat APC injection attempt.
#[derive(Debug, Clone)]
pub struct ApcTamperViolationRecord {
    pub timestamp: u64,
    pub caller_pid: u64,
    pub target_pid: u64,
    pub target_tid: u64,
    pub apc_routine: usize,
    pub caller_trust_level: ProcessTrustLevel,
    pub violation_kind: AntiCheatApcViolationKind,
}

/// Address range descriptor for signed/backed executable PE modules.
#[derive(Debug, Clone)]
pub struct LoadedModuleRange {
    pub name: String,
    pub base_address: usize,
    pub size: usize,
    pub is_system_backed: bool,
}

/// Anti-cheat APC injection and thread hijacking defense governor.
pub struct AntiCheatApcGovernor {
    pub backed_modules: Vec<LoadedModuleRange>,
    pub tamper_records: Vec<ApcTamperViolationRecord>,
    pub tamper_count: AtomicU32,
    pub strict_enforcement: AtomicBool,
}

impl AntiCheatApcGovernor {
    pub fn new() -> Self {
        let mut gov = Self {
            backed_modules: Vec::new(),
            tamper_records: Vec::new(),
            tamper_count: AtomicU32::new(0),
            strict_enforcement: AtomicBool::new(true),
        };

        // Pre-register standard system module code regions (e.g. ntdll.dll, kernel32.dll, kntmain.exe)
        gov.register_backed_module("ntdll.dll", 0x00007FFF_8000_0000, 0x0020_0000);
        gov.register_backed_module("kernel32.dll", 0x00007FFF_8040_0000, 0x0018_0000);
        gov.register_backed_module("init.exe", 0x00000000_0040_0000, 0x0010_0000);
        gov
    }

    /// Registers a validated PE module virtual address range backed by file system storage.
    pub fn register_backed_module(&mut self, name: &str, base: usize, size: usize) {
        self.backed_modules.push(LoadedModuleRange {
            name: name.to_string(),
            base_address: base,
            size,
            is_system_backed: true,
        });
    }

    /// Checks if a code address resides within a verified file-backed executable module section.
    pub fn is_address_module_backed(&self, addr: usize) -> bool {
        self.backed_modules.iter().any(|m| {
            addr >= m.base_address && addr < (m.base_address + m.size)
        })
    }

    /// Validates an incoming APC queue request against anti-cheat injection policies.
    pub fn verify_apc_request(
        &mut self,
        caller_pid: u64,
        caller_trust: ProcessTrustLevel,
        target_pid: u64,
        target_tid: u64,
        apc_routine: usize,
    ) -> Result<(), NtStatus> {
        // 1. Cross-process APC injection protection
        if caller_pid != target_pid {
            if caller_trust < ProcessTrustLevel::System {
                self.record_violation(
                    caller_pid,
                    target_pid,
                    target_tid,
                    apc_routine,
                    caller_trust,
                    AntiCheatApcViolationKind::CrossProcessInjection,
                );
                return Err(STATUS_ACCESS_DENIED);
            }
        }

        // 2. Unbacked code page execution detection (heap/stack/unbacked RWX memory injection)
        if !self.is_address_module_backed(apc_routine) {
            self.record_violation(
                caller_pid,
                target_pid,
                target_tid,
                apc_routine,
                caller_trust,
                AntiCheatApcViolationKind::UnbackedCodePageInjection,
            );
            return Err(STATUS_ACCESS_DENIED);
        }

        Ok(())
    }

    /// Records an anti-cheat tampering violation event.
    pub fn record_violation(
        &mut self,
        caller_pid: u64,
        target_pid: u64,
        target_tid: u64,
        apc_routine: usize,
        caller_trust: ProcessTrustLevel,
        kind: AntiCheatApcViolationKind,
    ) {
        self.tamper_count.fetch_add(1, Ordering::SeqCst);
        self.tamper_records.push(ApcTamperViolationRecord {
            timestamp: 0,
            caller_pid,
            target_pid,
            target_tid,
            apc_routine,
            caller_trust_level: caller_trust,
            violation_kind: kind,
        });
    }
}

/// Global system-wide Asynchronous Procedure Call governor.
pub struct GlobalApcManager {
    pub threads: BTreeMap<u64, ThreadApcContext>,
    pub next_apc_id: AtomicU64,
    pub anti_cheat_gov: AntiCheatApcGovernor,
}

impl GlobalApcManager {
    pub fn new() -> Self {
        Self {
            threads: BTreeMap::new(),
            next_apc_id: AtomicU64::new(1),
            anti_cheat_gov: AntiCheatApcGovernor::new(),
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


    /// Queues a User-mode APC with anti-cheat origin and code-backing validation.
    pub fn queue_user_apc_secure(
        &mut self,
        caller_pid: u64,
        caller_trust: ProcessTrustLevel,
        target_tid: u64,
        apc_routine: usize,
        context: usize,
        arg1: usize,
        arg2: usize,
    ) -> Result<u64, NtStatus> {
        let target_pid = self
            .threads
            .get(&target_tid)
            .map(|t| t.process_id)
            .ok_or(STATUS_INVALID_HANDLE)?;

        // Validate via anti-cheat governor
        self.anti_cheat_gov.verify_apc_request(
            caller_pid,
            caller_trust,
            target_pid,
            target_tid,
            apc_routine,
        )?;

        self.queue_user_apc(target_tid, target_pid, apc_routine, context, arg1, arg2)
    }

    /// Queries total count of anti-cheat APC injection violations and the last recorded event.
    pub fn get_anti_cheat_apc_stats(&self) -> (u32, Option<ApcTamperViolationRecord>) {
        let count = self.anti_cheat_gov.tamper_count.load(Ordering::SeqCst);
        let last = self.anti_cheat_gov.tamper_records.last().cloned();
        (count, last)
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

static KERNEL_APC_INVOCATION_COUNT: AtomicU64 = AtomicU64::new(0);

fn sample_kernel_apc_routine(_apc_id: u64, context: &mut usize, arg1: &mut usize, _arg2: &mut usize) {
    *context = context.wrapping_add(42);
    *arg1 = arg1.wrapping_add(100);
    KERNEL_APC_INVOCATION_COUNT.fetch_add(1, Ordering::SeqCst);
}

/// Public self-test suite validating Win32 Kernel & User-Mode APC Dispatching,
/// Alertable Wait States (SleepEx), Trap Frame Redirection & Anti-Cheat Injection Defense.
pub fn test_win32_apc_subsystem() -> bool {
    crate::serial_write_str("[KNT-TEST] Initializing Win32 APC Dispatcher self-test suite...\r\n");

    let mut mgr = GlobalApcManager::new();

    // 1. Verify Kernel Special APC delivery
    mgr.register_thread(100, 10);
    let mut kernel_apc = KAPC::new_kernel(
        100,
        10,
        true,
        Some(sample_kernel_apc_routine),
        None,
        1000,
    );
    kernel_apc.system_argument1 = 500;

    let initial_count = KERNEL_APC_INVOCATION_COUNT.load(Ordering::SeqCst);
    if let Err(_) = mgr.queue_apc(kernel_apc) {
        crate::serial_write_str("[KNT-TEST-ERR] Failed to queue special kernel APC\r\n");
        return false;
    }

    if KERNEL_APC_INVOCATION_COUNT.load(Ordering::SeqCst) != initial_count + 1 {
        crate::serial_write_str("[KNT-TEST-ERR] Special kernel APC was not immediately executed\r\n");
        return false;
    }

    // 2. Verify User alertable wait state delivery
    mgr.register_thread(101, 10);

    // Initial sleep without APC should return 0 (timeout)
    if mgr.sleep_ex(101, 10, true) != 0 {
        crate::serial_write_str("[KNT-TEST-ERR] SleepEx returned alerted without pending APC\r\n");
        return false;
    }

    // Queue backed user APC and enter alertable sleep
    let backed_ntdll_code = 0x00007FFF_8000_1500;
    if let Err(_) = mgr.queue_user_apc(101, 10, backed_ntdll_code, 777, 0, 0) {
        crate::serial_write_str("[KNT-TEST-ERR] Failed to queue user APC\r\n");
        return false;
    }

    if mgr.sleep_ex(101, 100, true) != WAIT_IO_COMPLETION {
        crate::serial_write_str("[KNT-TEST-ERR] SleepEx did not return WAIT_IO_COMPLETION on pending APC\r\n");
        return false;
    }

    // 3. Verify Multi-APC FIFO queueing
    mgr.register_thread(102, 10);
    let _ = mgr.queue_user_apc(102, 10, backed_ntdll_code, 10, 0, 0);
    let _ = mgr.queue_user_apc(102, 10, backed_ntdll_code, 20, 0, 0);
    let _ = mgr.queue_user_apc(102, 10, backed_ntdll_code, 30, 0, 0);

    let apc1 = mgr.pop_user_apc(102).map(|a| a.normal_context);
    let apc2 = mgr.pop_user_apc(102).map(|a| a.normal_context);
    let apc3 = mgr.pop_user_apc(102).map(|a| a.normal_context);

    if apc1 != Some(10) || apc2 != Some(20) || apc3 != Some(30) {
        crate::serial_write_str("[KNT-TEST-ERR] Multi-APC FIFO ordering violated\r\n");
        return false;
    }

    // 4. Verify User trap frame redirection to KiUserApcDispatcher
    let target_callback = 0x00007FFF_8000_2500;
    let target_context = 0xCAFE_BABE_usize;
    let _ = mgr.queue_user_apc(102, 10, target_callback, target_context, 0x111, 0x222);

    let mut trap_frame = UserTrapFrame {
        rip: 0x0040_1050,
        rsp: 0x00007FFF_FFFF_0000,
        rcx: 0,
        rdx: 0,
        r8: 0,
        r9: 0,
        ..Default::default()
    };

    match mgr.prepare_user_apc_dispatch(102, &mut trap_frame) {
        Ok(true) => {
            if trap_frame.rip != KI_USER_APC_DISPATCHER_ADDRESS {
                crate::serial_write_str("[KNT-TEST-ERR] Trap frame RIP not redirected to KiUserApcDispatcher\r\n");
                return false;
            }
            if trap_frame.rcx != target_callback as u64 {
                crate::serial_write_str("[KNT-TEST-ERR] Trap frame RCX does not hold target callback routine\r\n");
                return false;
            }
            if trap_frame.rdx != target_context as u64 {
                crate::serial_write_str("[KNT-TEST-ERR] Trap frame RDX does not hold APC context parameter\r\n");
                return false;
            }
            if trap_frame.rsp >= 0x00007FFF_FFFF_0000 {
                crate::serial_write_str("[KNT-TEST-ERR] Trap frame RSP was not adjusted for user stack frame\r\n");
                return false;
            }
        }
        _ => {
            crate::serial_write_str("[KNT-TEST-ERR] prepare_user_apc_dispatch failed\r\n");
            return false;
        }
    }

    // 5. Verify Anti-Cheat rogue APC injection rejection
    // Case A: Untrusted external process attempting cross-process APC injection
    let cross_proc_res = mgr.queue_user_apc_secure(
        999,
        ProcessTrustLevel::Untrusted,
        100,
        backed_ntdll_code,
        0,
        0,
        0,
    );
    if cross_proc_res != Err(STATUS_ACCESS_DENIED) {
        crate::serial_write_str("[KNT-TEST-ERR] Cross-process untrusted APC injection was not blocked\r\n");
        return false;
    }

    // Case B: Unbacked shellcode memory injection
    let unbacked_heap_code = 0x00000200_1234_0000;
    let unbacked_res = mgr.queue_user_apc_secure(
        10,
        ProcessTrustLevel::Medium,
        100,
        unbacked_heap_code,
        0,
        0,
        0,
    );
    if unbacked_res != Err(STATUS_ACCESS_DENIED) {
        crate::serial_write_str("[KNT-TEST-ERR] Unbacked code page APC injection was not blocked\r\n");
        return false;
    }

    let (tamper_count, last_record) = mgr.get_anti_cheat_apc_stats();
    if tamper_count != 2 || last_record.is_none() {
        crate::serial_write_str("[KNT-TEST-ERR] Anti-cheat APC tamper events not recorded correctly\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-APC-PASS] Win32 Kernel & User-Mode APC Dispatcher & Alertable Wait Engine verified\r\n");
    true
}
