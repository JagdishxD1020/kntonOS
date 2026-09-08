//! Win32 Structured Exception Handling (SEH) & Vectored Exception Handling (VEH) Subsystem.

#![allow(non_snake_case, non_camel_case_types)]

pub use crate::pe::synch::HANDLE;

pub const EXCEPTION_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const EXCEPTION_DATATYPE_MISALIGNMENT: u32 = 0x8000_0002;
pub const EXCEPTION_BREAKPOINT: u32 = 0x8000_0003;
pub const EXCEPTION_SINGLE_STEP: u32 = 0x8000_0004;
pub const EXCEPTION_ARRAY_BOUNDS_EXCEEDED: u32 = 0xC000_008C;
pub const EXCEPTION_FLT_DENORMAL_OPERAND: u32 = 0xC000_008D;
pub const EXCEPTION_FLT_DIVIDE_BY_ZERO: u32 = 0xC000_008E;
pub const EXCEPTION_INT_DIVIDE_BY_ZERO: u32 = 0xC000_0094;
pub const EXCEPTION_INT_OVERFLOW: u32 = 0xC000_0095;
pub const EXCEPTION_ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
pub const EXCEPTION_IN_PAGE_ERROR: u32 = 0xC000_0006;
pub const EXCEPTION_NONCONTINUABLE_EXCEPTION: u32 = 0xC000_0025;
pub const EXCEPTION_STACK_OVERFLOW: u32 = 0xC000_00FD;

pub const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
pub const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
pub const EXCEPTION_EXECUTE_HANDLER: i32 = 1;

pub const EXCEPTION_NONCONTINUABLE: u32 = 0x1;
pub const EXCEPTION_UNWINDING: u32 = 0x2;
pub const EXCEPTION_EXIT_UNWIND: u32 = 0x4;
pub const EXCEPTION_STACK_INVALID: u32 = 0x8;
pub const EXCEPTION_NESTED_CALL: u32 = 0x10;
pub const EXCEPTION_TARGET_UNWIND: u32 = 0x20;
pub const EXCEPTION_COLLIDED_UNWIND: u32 = 0x40;

pub const EXCEPTION_MAXIMUM_PARAMETERS: usize = 15;

/// Win32 machine-independent exception description record.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EXCEPTION_RECORD {
    pub ExceptionCode: u32,
    pub ExceptionFlags: u32,
    pub ExceptionRecord: *mut EXCEPTION_RECORD,
    pub ExceptionAddress: *mut u8,
    pub NumberParameters: u32,
    pub ExceptionInformation: [usize; EXCEPTION_MAXIMUM_PARAMETERS],
}

impl Default for EXCEPTION_RECORD {
    fn default() -> Self {
        Self {
            ExceptionCode: 0,
            ExceptionFlags: 0,
            ExceptionRecord: core::ptr::null_mut(),
            ExceptionAddress: core::ptr::null_mut(),
            NumberParameters: 0,
            ExceptionInformation: [0; EXCEPTION_MAXIMUM_PARAMETERS],
        }
    }
}

/// x86-64 CPU execution context frame.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Default)]
pub struct CONTEXT {
    pub Rax: u64,
    pub Rcx: u64,
    pub Rdx: u64,
    pub Rbx: u64,
    pub Rsp: u64,
    pub Rbp: u64,
    pub Rsi: u64,
    pub Rdi: u64,
    pub R8: u64,
    pub R9: u64,
    pub R10: u64,
    pub R11: u64,
    pub R12: u64,
    pub R13: u64,
    pub R14: u64,
    pub R15: u64,
    pub Rip: u64,
    pub EFlags: u32,
}

/// Win32 exception pointers structure passed to exception filters and handlers.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EXCEPTION_POINTERS {
    pub ExceptionRecord: *mut EXCEPTION_RECORD,
    pub ContextRecord: *mut CONTEXT,
}
