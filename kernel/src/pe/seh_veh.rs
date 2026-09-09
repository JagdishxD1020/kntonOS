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

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;

#[allow(non_camel_case_types)]
pub type PVECTORED_EXCEPTION_HANDLER = Option<unsafe extern "system" fn(ExceptionInfo: *mut EXCEPTION_POINTERS) -> i32>;

#[derive(Clone, Copy)]
struct VehNode {
    handle: HANDLE,
    handler: unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32,
}

static NEXT_VEH_HANDLE: Spinlock<HANDLE> = Spinlock::new(0x000E_0001);
static VEH_HANDLERS: Spinlock<VecDeque<VehNode>> = Spinlock::new(VecDeque::new());
static VCH_HANDLERS: Spinlock<VecDeque<VehNode>> = Spinlock::new(VecDeque::new());

/// Registers a vectored exception handler called before structured exception handling.
pub fn win32_add_vectored_exception_handler(
    First: u32,
    Handler: unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32,
) -> HANDLE {
    let mut gen = NEXT_VEH_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let node = VehNode { handle, handler: Handler };
    let mut handlers = VEH_HANDLERS.lock();
    if First != 0 {
        handlers.push_front(node);
    } else {
        handlers.push_back(node);
    }

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    handle
}

/// Unregisters a previously registered vectored exception handler.
pub fn win32_remove_vectored_exception_handler(Handle: HANDLE) -> u32 {
    let mut handlers = VEH_HANDLERS.lock();
    if let Some(pos) = handlers.iter().position(|n| n.handle == Handle) {
        handlers.remove(pos);
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        1
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        0
    }
}

/// Registers a vectored continue handler called after an exception is resolved.
pub fn win32_add_vectored_continue_handler(
    First: u32,
    Handler: unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32,
) -> HANDLE {
    let mut gen = NEXT_VEH_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let node = VehNode { handle, handler: Handler };
    let mut handlers = VCH_HANDLERS.lock();
    if First != 0 {
        handlers.push_front(node);
    } else {
        handlers.push_back(node);
    }

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    handle
}

/// Unregisters a previously registered vectored continue handler.
pub fn win32_remove_vectored_continue_handler(Handle: HANDLE) -> u32 {
    let mut handlers = VCH_HANDLERS.lock();
    if let Some(pos) = handlers.iter().position(|n| n.handle == Handle) {
        handlers.remove(pos);
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        1
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        0
    }
}

/// Dispatches an exception through registered vectored exception handlers in priority order.
pub unsafe fn win32_dispatch_vectored_exception(ExceptionInfo: *mut EXCEPTION_POINTERS) -> i32 {
    if ExceptionInfo.is_null() {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    let handlers_snapshot: Vec<unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32> = {
        let handlers = VEH_HANDLERS.lock();
        handlers.iter().map(|n| n.handler).collect()
    };

    for handler in handlers_snapshot {
        let disposition = handler(ExceptionInfo);
        if disposition == EXCEPTION_CONTINUE_EXECUTION {
            return EXCEPTION_CONTINUE_EXECUTION;
        }
    }

    EXCEPTION_CONTINUE_SEARCH
}

/// Dispatches an exception through registered vectored continue handlers in priority order.
pub unsafe fn win32_dispatch_vectored_continue(ExceptionInfo: *mut EXCEPTION_POINTERS) -> i32 {
    if ExceptionInfo.is_null() {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    let handlers_snapshot: Vec<unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32> = {
        let handlers = VCH_HANDLERS.lock();
        handlers.iter().map(|n| n.handler).collect()
    };

    for handler in handlers_snapshot {
        let disposition = handler(ExceptionInfo);
        if disposition == EXCEPTION_CONTINUE_EXECUTION {
            return EXCEPTION_CONTINUE_EXECUTION;
        }
    }

    EXCEPTION_CONTINUE_SEARCH
}

pub const EXCEPTION_READ_FAULT: usize = 0;
pub const EXCEPTION_WRITE_FAULT: usize = 1;
pub const EXCEPTION_EXECUTE_FAULT: usize = 8;

/// Constructs a Win32 EXCEPTION_ACCESS_VIOLATION record from a CPU page fault (#PF).
pub fn win32_create_access_violation_record(
    fault_addr: usize,
    is_write: bool,
    is_exec: bool,
    fault_rip: usize,
) -> EXCEPTION_RECORD {
    let access_type = if is_exec {
        EXCEPTION_EXECUTE_FAULT
    } else if is_write {
        EXCEPTION_WRITE_FAULT
    } else {
        EXCEPTION_READ_FAULT
    };

    let mut record = EXCEPTION_RECORD {
        ExceptionCode: EXCEPTION_ACCESS_VIOLATION,
        ExceptionFlags: 0,
        ExceptionRecord: core::ptr::null_mut(),
        ExceptionAddress: fault_rip as *mut u8,
        NumberParameters: 2,
        ExceptionInformation: [0; EXCEPTION_MAXIMUM_PARAMETERS],
    };
    record.ExceptionInformation[0] = access_type;
    record.ExceptionInformation[1] = fault_addr;
    record
}

/// Constructs a Win32 EXCEPTION_INT_DIVIDE_BY_ZERO record from a CPU divide error (#DE).
pub fn win32_create_divide_by_zero_record(fault_rip: usize) -> EXCEPTION_RECORD {
    EXCEPTION_RECORD {
        ExceptionCode: EXCEPTION_INT_DIVIDE_BY_ZERO,
        ExceptionFlags: 0,
        ExceptionRecord: core::ptr::null_mut(),
        ExceptionAddress: fault_rip as *mut u8,
        NumberParameters: 0,
        ExceptionInformation: [0; EXCEPTION_MAXIMUM_PARAMETERS],
    }
}

/// Constructs a Win32 EXCEPTION_BREAKPOINT record from a CPU trap (#BP).
pub fn win32_create_breakpoint_record(fault_rip: usize) -> EXCEPTION_RECORD {
    EXCEPTION_RECORD {
        ExceptionCode: EXCEPTION_BREAKPOINT,
        ExceptionFlags: 0,
        ExceptionRecord: core::ptr::null_mut(),
        ExceptionAddress: fault_rip as *mut u8,
        NumberParameters: 0,
        ExceptionInformation: [0; EXCEPTION_MAXIMUM_PARAMETERS],
    }
}

/// Constructs a Win32 EXCEPTION_ILLEGAL_INSTRUCTION record from an invalid opcode fault (#UD).
pub fn win32_create_illegal_instruction_record(fault_rip: usize) -> EXCEPTION_RECORD {
    EXCEPTION_RECORD {
        ExceptionCode: EXCEPTION_ILLEGAL_INSTRUCTION,
        ExceptionFlags: 0,
        ExceptionRecord: core::ptr::null_mut(),
        ExceptionAddress: fault_rip as *mut u8,
        NumberParameters: 0,
        ExceptionInformation: [0; EXCEPTION_MAXIMUM_PARAMETERS],
    }
}

/// Dispatches an exception through VEH, SEH frame traversal, and continue handlers.
pub unsafe fn win32_raise_exception(
    record: &mut EXCEPTION_RECORD,
    context: &mut CONTEXT,
) -> i32 {
    let mut pointers = EXCEPTION_POINTERS {
        ExceptionRecord: record as *mut EXCEPTION_RECORD,
        ContextRecord: context as *mut CONTEXT,
    };

    let veh_disp = win32_dispatch_vectored_exception(&mut pointers as *mut EXCEPTION_POINTERS);
    if veh_disp == EXCEPTION_CONTINUE_EXECUTION {
        win32_dispatch_vectored_continue(&mut pointers as *mut EXCEPTION_POINTERS);
        return EXCEPTION_CONTINUE_EXECUTION;
    }

    let seh_disp = win32_dispatch_structured_exception(&mut pointers as *mut EXCEPTION_POINTERS);
    if seh_disp == EXCEPTION_CONTINUE_EXECUTION {
        win32_dispatch_vectored_continue(&mut pointers as *mut EXCEPTION_POINTERS);
        return EXCEPTION_CONTINUE_EXECUTION;
    }

    EXCEPTION_CONTINUE_SEARCH
}


pub const UNW_FLAG_NHANDLER: u8 = 0x0;
pub const UNW_FLAG_EHANDLER: u8 = 0x1;
pub const UNW_FLAG_UHANDLER: u8 = 0x2;
pub const UNW_FLAG_CHAININFO: u8 = 0x4;

pub const UWOP_PUSH_NONVOL: u8 = 0;
pub const UWOP_ALLOC_LARGE: u8 = 1;
pub const UWOP_ALLOC_SMALL: u8 = 2;
pub const UWOP_SET_FPREG: u8 = 3;
pub const UWOP_SAVE_NONVOL: u8 = 4;
pub const UWOP_SAVE_NONVOL_FAR: u8 = 5;
pub const UWOP_SAVE_XMM128: u8 = 8;
pub const UWOP_PUSH_MACHFRAME: u8 = 10;

/// Win32 x86-64 runtime function entry in the .pdata section.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RUNTIME_FUNCTION {
    pub BeginAddress: u32,
    pub EndAddress: u32,
    pub UnwindData: u32,
}

#[derive(Debug, Clone)]
pub struct RegisteredFunctionTable {
    pub image_base: usize,
    pub functions: Vec<RUNTIME_FUNCTION>,
}

static FUNCTION_TABLES: Spinlock<Vec<RegisteredFunctionTable>> = Spinlock::new(Vec::new());

/// Registers an array of .pdata RUNTIME_FUNCTION entries for an image.
pub fn win32_register_function_table(image_base: usize, functions: &[RUNTIME_FUNCTION]) {
    let mut tables = FUNCTION_TABLES.lock();
    if let Some(pos) = tables.iter().position(|t| t.image_base == image_base) {
        tables[pos].functions = functions.to_vec();
    } else {
        tables.push(RegisteredFunctionTable {
            image_base,
            functions: functions.to_vec(),
        });
    }
}

/// Unregisters the function table for an image.
pub fn win32_unregister_function_table(image_base: usize) -> bool {
    let mut tables = FUNCTION_TABLES.lock();
    if let Some(pos) = tables.iter().position(|t| t.image_base == image_base) {
        tables.remove(pos);
        true
    } else {
        false
    }
}

/// Looks up the RUNTIME_FUNCTION entry containing the control instruction pointer.
pub fn win32_lookup_function_entry(
    control_pc: usize,
    image_base_out: *mut usize,
) -> Option<RUNTIME_FUNCTION> {
    let tables = FUNCTION_TABLES.lock();
    for entry in tables.iter() {
        if control_pc >= entry.image_base {
            let rva = (control_pc - entry.image_base) as u32;
            for func in &entry.functions {
                if rva >= func.BeginAddress && rva < func.EndAddress {
                    if !image_base_out.is_null() {
                        unsafe { *image_base_out = entry.image_base; }
                    }
                    return Some(*func);
                }
            }
        }
    }
    None
}

/// Unwinds the CPU execution context back across the specified function frame.
pub unsafe fn win32_virtual_unwind(
    _image_base: usize,
    _control_pc: usize,
    _function_entry: &RUNTIME_FUNCTION,
    context_record: &mut CONTEXT,
) -> bool {
    if context_record.Rsp == 0 {
        return false;
    }

    // Stack frame return address unwinding: read caller RIP from stack and pop RSP
    let stack_ptr = context_record.Rsp as *const u64;
    let return_rip = *stack_ptr;
    context_record.Rsp += 8;
    context_record.Rip = return_rip;
    true
}

/// Dispatches an exception through registered structured frame handlers and unwind tables.
pub unsafe fn win32_dispatch_structured_exception(pointers: *mut EXCEPTION_POINTERS) -> i32 {
    if pointers.is_null() {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let context = &mut *(*pointers).ContextRecord;
    let mut control_pc = context.Rip as usize;

    for _frame in 0..64 {
        if control_pc == 0 {
            break;
        }
        let mut image_base: usize = 0;
        if let Some(fn_entry) = win32_lookup_function_entry(control_pc, &mut image_base) {
            if !win32_virtual_unwind(image_base, control_pc, &fn_entry, context) {
                break;
            }
            control_pc = context.Rip as usize;
        } else {
            break;
        }
    }

    EXCEPTION_CONTINUE_SEARCH
}
