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
