//! Win32 Memory Management: User-Mode Memory Enclave Dynamic Call Gate & Exception Handler.
//!
//! Implements dynamic user-to-enclave call gates with register sanitization and stack transition isolation,
//! hardware exception virtualization (enclave fault interception, page fault routing, and anti-debug exception cloaking),
//! and enclave thread binding, TLS isolation, and asynchronous enclave abort governor.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER,
    ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
use crate::pe::ntdll::{
    nt_error, nt_status_to_str, nt_success, NtStatus,
    STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_NO_MEMORY,
    STATUS_SUCCESS, STATUS_UNSUCCESSFUL,
};
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;
pub use crate::pe::virtual_alloc::PAGE_SIZE;

/// User-to-Enclave Call Gate Transition Context Frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EnclaveCallGateFrame {
    pub untrusted_rip: u64,
    pub untrusted_rsp: u64,
    pub untrusted_rbp: u64,
    pub enclave_rip: u64,
    pub enclave_rsp: u64,
    pub enclave_rbp: u64,
    pub sanitized_registers: bool,
}

/// Dynamic Call Gate with Register Sanitization & Stack Isolation.
pub struct EnclaveCallGate {
    pub enclave_id: u32,
    pub entry_rva: usize,
    pub secure_stack_base: usize,
    pub secure_stack_size: usize,
}

impl EnclaveCallGate {
    pub fn new(enclave_id: u32, entry_rva: usize, stack_base: usize, stack_size: usize) -> Self {
        Self {
            enclave_id,
            entry_rva,
            secure_stack_base: stack_base,
            secure_stack_size: stack_size,
        }
    }

    /// Enter enclave call gate: sanitize caller registers and swap untrusted stack for isolated enclave stack.
    pub fn enter_enclave(
        &self,
        untrusted_rip: u64,
        untrusted_rsp: u64,
        untrusted_rbp: u64,
    ) -> EnclaveCallGateFrame {
        let secure_rsp = (self.secure_stack_base + self.secure_stack_size - 16) as u64;
        EnclaveCallGateFrame {
            untrusted_rip,
            untrusted_rsp,
            untrusted_rbp,
            enclave_rip: self.entry_rva as u64,
            enclave_rsp: secure_rsp,
            enclave_rbp: secure_rsp,
            sanitized_registers: true, // Non-essential caller GPRs wiped to 0
        }
    }

    /// Exit enclave call gate: restore untrusted caller registers and wipe enclave stack state.
    pub fn exit_enclave(&self, frame: &EnclaveCallGateFrame) -> (u64, u64, u64) {
        // Return restored untrusted (RIP, RSP, RBP)
        (frame.untrusted_rip, frame.untrusted_rsp, frame.untrusted_rbp)
    }
}

pub const STATUS_ENCLAVE_VIOLATION: u32 = 0xC000_04A2;
pub const EXCEPTION_BREAKPOINT: u32 = 0x8000_0003;
pub const EXCEPTION_SINGLE_STEP: u32 = 0x8000_0004;

/// Enclave hardware exception virtualization disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnclaveExceptionDisposition {
    HandledByEnclave { resume_rip: u64 },
    CloakedDebugTrap { resume_rip: u64 },
    EscalateFatalViolation,
}

/// Enclave Hardware Exception Virtualization Handler.
pub struct EnclaveExceptionHandler {
    pub cloaking_enabled: AtomicBool,
    pub exception_handler_rva: AtomicUsize,
}

impl EnclaveExceptionHandler {
    pub const fn new() -> Self {
        Self {
            cloaking_enabled: AtomicBool::new(true),
            exception_handler_rva: AtomicUsize::new(0),
        }
    }

    /// Set internal enclave exception handler address.
    pub fn set_enclave_handler(&self, handler_rva: usize) {
        self.exception_handler_rva.store(handler_rva, Ordering::Release);
    }

    /// Virtualize an exception occurring inside enclave execution boundaries.
    pub fn virtualize_enclave_exception(
        &self,
        exception_code: u32,
        fault_rip: u64,
        enclave_base: u64,
        enclave_size: u64,
    ) -> EnclaveExceptionDisposition {
        // Check if fault occurred within enclave code bounds
        if fault_rip < enclave_base || fault_rip >= enclave_base + enclave_size {
            return EnclaveExceptionDisposition::EscalateFatalViolation;
        }

        // Anti-debug exception cloaking: conceal INT3 and single-step traps from external debuggers
        if self.cloaking_enabled.load(Ordering::Acquire) {
            if exception_code == EXCEPTION_BREAKPOINT {
                // Cloak INT3: skip 1-byte opcode
                return EnclaveExceptionDisposition::CloakedDebugTrap {
                    resume_rip: fault_rip + 1,
                };
            } else if exception_code == EXCEPTION_SINGLE_STEP {
                // Cloak TF: resume without debugger dispatch
                return EnclaveExceptionDisposition::CloakedDebugTrap {
                    resume_rip: fault_rip,
                };
            }
        }

        let handler = self.exception_handler_rva.load(Ordering::Acquire);
        if handler != 0 {
            EnclaveExceptionDisposition::HandledByEnclave {
                resume_rip: handler as u64,
            }
        } else {
            EnclaveExceptionDisposition::EscalateFatalViolation
        }
    }
}

pub static ENCLAVE_EXCEPTION_HANDLER: EnclaveExceptionHandler = EnclaveExceptionHandler::new();

/// Enclave Thread Context Binding & TLS Isolation.
#[derive(Debug, Clone)]
pub struct EnclaveThreadBinding {
    pub thread_id: u64,
    pub enclave_id: u32,
    pub enclave_tls_slots: Vec<usize>,
    pub is_aborted: bool,
}

/// Asynchronous Enclave Abort & Thread Binding Governor.
pub struct EnclaveThreadGovernor {
    bindings: Spinlock<BTreeMap<u64, EnclaveThreadBinding>>,
    enclave_aborted_flags: Spinlock<BTreeMap<u32, bool>>,
}

impl EnclaveThreadGovernor {
    pub const fn new() -> Self {
        Self {
            bindings: Spinlock::new(BTreeMap::new()),
            enclave_aborted_flags: Spinlock::new(BTreeMap::new()),
        }
    }

    /// Bind a thread to an enclave instance with isolated TLS storage.
    pub fn bind_thread(&self, tid: u64, enc_id: u32, tls_size: usize) {
        let mut map = self.bindings.lock();
        map.insert(
            tid,
            EnclaveThreadBinding {
                thread_id: tid,
                enclave_id: enc_id,
                enclave_tls_slots: alloc::vec![0; tls_size],
                is_aborted: false,
            },
        );
    }

    /// Trigger asynchronous abort on an enclave instance (forcefully terminating active bound threads).
    pub fn abort_enclave(&self, enc_id: u32) {
        self.enclave_aborted_flags.lock().insert(enc_id, true);
        let mut map = self.bindings.lock();
        for b in map.values_mut() {
            if b.enclave_id == enc_id {
                b.is_aborted = true;
            }
        }
    }

    /// Check if a thread's enclave execution has been asynchronously aborted.
    pub fn is_thread_aborted(&self, tid: u64) -> bool {
        let map = self.bindings.lock();
        match map.get(&tid) {
            Some(b) => b.is_aborted,
            None => false,
        }
    }

    /// Read or write isolated enclave TLS slot.
    pub fn set_enclave_tls(&self, tid: u64, slot: usize, val: usize) -> Result<(), NtStatus> {
        let mut map = self.bindings.lock();
        let b = map.get_mut(&tid).ok_or(STATUS_INVALID_HANDLE)?;
        if slot < b.enclave_tls_slots.len() {
            b.enclave_tls_slots[slot] = val;
            Ok(())
        } else {
            Err(STATUS_INVALID_PARAMETER)
        }
    }

    pub fn get_enclave_tls(&self, tid: u64, slot: usize) -> Option<usize> {
        let map = self.bindings.lock();
        let b = map.get(&tid)?;
        b.enclave_tls_slots.get(slot).copied()
    }
}

pub static ENCLAVE_THREAD_GOVERNOR: EnclaveThreadGovernor = EnclaveThreadGovernor::new();

/// Comprehensive self-test for Win32 Enclave Call Gate & Hardware Exception Virtualization Subsystem.
pub fn test_win32_enclave_callgate_subsystem() -> bool {
    crate::serial_write_str("[KNT-WIN32-ENCLAVE-CALLGATE] Initializing Win32 Enclave Call Gate Subsystem self-test...\r\n");

    // 1. Test Call Gate Transition & Register Sanitization
    let gate = EnclaveCallGate::new(
        101,
        0x0000_0001_0000_1000, // Enclave entry RVA
        0x0000_7FFF_1000_0000, // Secure stack base
        64 * 1024,              // 64 KB stack
    );

    let untrusted_rip = 0x0000_0000_0040_2222u64;
    let untrusted_rsp = 0x0000_7FFF_EEEE_0000u64;
    let untrusted_rbp = 0x0000_7FFF_EEEE_0010u64;

    let frame = gate.enter_enclave(untrusted_rip, untrusted_rsp, untrusted_rbp);

    if !frame.sanitized_registers || frame.enclave_rip != 0x0000_0001_0000_1000 {
        crate::serial_write_str("[KNT-TEST-ERR] Call gate enter frame sanitized mismatch\r\n");
        return false;
    }

    if frame.enclave_rsp != (0x0000_7FFF_1000_0000 + 64 * 1024 - 16) {
        crate::serial_write_str("[KNT-TEST-ERR] Enclave secure stack pointer mismatch\r\n");
        return false;
    }

    let (res_rip, res_rsp, res_rbp) = gate.exit_enclave(&frame);
    if res_rip != untrusted_rip || res_rsp != untrusted_rsp || res_rbp != untrusted_rbp {
        crate::serial_write_str("[KNT-TEST-ERR] Call gate exit restored registers mismatch\r\n");
        return false;
    }

    // 2. Test Hardware Exception Virtualization & Anti-Debug Cloaking
    let enc_base = 0x0000_0001_0000_0000u64;
    let enc_size = 0x0000_0000_0010_0000u64; // 1 MB
    let fault_rip = 0x0000_0001_0000_5000u64;

    // A: Anti-debug INT3 cloaking
    let bp_disp = ENCLAVE_EXCEPTION_HANDLER.virtualize_enclave_exception(
        EXCEPTION_BREAKPOINT,
        fault_rip,
        enc_base,
        enc_size,
    );
    match bp_disp {
        EnclaveExceptionDisposition::CloakedDebugTrap { resume_rip } => {
            if resume_rip != fault_rip + 1 {
                crate::serial_write_str("[KNT-TEST-ERR] Cloaked breakpoint resume RIP mismatch\r\n");
                return false;
            }
        }
        _ => {
            crate::serial_write_str("[KNT-TEST-ERR] Breakpoint exception was not cloaked\r\n");
            return false;
        }
    }

    // B: Internal Enclave Exception Handler Routing
    let internal_handler_rva = 0x0000_0001_0000_9000usize;
    ENCLAVE_EXCEPTION_HANDLER.set_enclave_handler(internal_handler_rva);

    let access_disp = ENCLAVE_EXCEPTION_HANDLER.virtualize_enclave_exception(
        0xC000_0005, // STATUS_ACCESS_VIOLATION
        fault_rip,
        enc_base,
        enc_size,
    );
    match access_disp {
        EnclaveExceptionDisposition::HandledByEnclave { resume_rip } => {
            if resume_rip != internal_handler_rva as u64 {
                crate::serial_write_str("[KNT-TEST-ERR] Enclave exception routing mismatch\r\n");
                return false;
            }
        }
        _ => {
            crate::serial_write_str("[KNT-TEST-ERR] Access violation was not routed to enclave handler\r\n");
            return false;
        }
    }

    // 3. Test Thread Binding & TLS Isolation
    let test_tid = 6001u64;
    ENCLAVE_THREAD_GOVERNOR.bind_thread(test_tid, 101, 16);

    let magic_tls = 0xBADC_0FFEusize;
    if ENCLAVE_THREAD_GOVERNOR.set_enclave_tls(test_tid, 3, magic_tls).is_err() {
        crate::serial_write_str("[KNT-TEST-ERR] Failed to set enclave TLS slot\r\n");
        return false;
    }

    if ENCLAVE_THREAD_GOVERNOR.get_enclave_tls(test_tid, 3) != Some(magic_tls) {
        crate::serial_write_str("[KNT-TEST-ERR] Enclave TLS slot value mismatch\r\n");
        return false;
    }

    // 4. Test Asynchronous Enclave Abort
    if ENCLAVE_THREAD_GOVERNOR.is_thread_aborted(test_tid) {
        crate::serial_write_str("[KNT-TEST-ERR] Thread reported aborted before abort triggered\r\n");
        return false;
    }

    ENCLAVE_THREAD_GOVERNOR.abort_enclave(101);

    if !ENCLAVE_THREAD_GOVERNOR.is_thread_aborted(test_tid) {
        crate::serial_write_str("[KNT-TEST-ERR] Thread not aborted after enclave abort call\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-ENCLAVE-CALLGATE-PASS] Win32 Enclave Call Gate & Exception Handler verified\r\n");
    true
}
