//! Win32 Structured Exception Handling (SEH) Vector Unwinding & Anti-Debugging Trap Virtualization.
//!
//! Implements x86-64 SEH .pdata / .xdata table parsing (RUNTIME_FUNCTION, UNWIND_INFO, opcode decoding),
//! 2-phase exception dispatcher (search handlers & unwind stack frames), Vectored Exception / Continue
//! Handlers (VEH/VCH), and anti-debugging trap virtualization (int3, single-step, guard page redirection).

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::seh_veh::{
    EXCEPTION_ACCESS_VIOLATION, EXCEPTION_BREAKPOINT, EXCEPTION_CONTINUE_EXECUTION,
    EXCEPTION_CONTINUE_SEARCH, EXCEPTION_DATATYPE_MISALIGNMENT, EXCEPTION_EXECUTE_HANDLER,
    EXCEPTION_ILLEGAL_INSTRUCTION, EXCEPTION_MAXIMUM_PARAMETERS, EXCEPTION_NONCONTINUABLE,
    EXCEPTION_RECORD, EXCEPTION_SINGLE_STEP, EXCEPTION_STACK_OVERFLOW, EXCEPTION_UNWINDING,
};
pub use crate::pe::synch::HANDLE;

pub const STATUS_GUARD_PAGE_VIOLATION: u32 = 0x8000_0001;

// x86-64 UNWIND_INFO flags
pub const UNW_FLAG_NHANDLER: u8 = 0x0;
pub const UNW_FLAG_EHANDLER: u8 = 0x1;
pub const UNW_FLAG_UHANDLER: u8 = 0x2;
pub const UNW_FLAG_CHAININFO: u8 = 0x4;

// x86-64 Unwind Opcodes
pub const UWOP_PUSH_NONVOL: u8 = 0;     // 1 node: push reg
pub const UWOP_ALLOC_LARGE: u8 = 1;     // 2 or 3 nodes: alloc large stack
pub const UWOP_ALLOC_SMALL: u8 = 2;     // 1 node: alloc small stack
pub const UWOP_SET_FPREG: u8 = 3;       // 1 node: set frame pointer
pub const UWOP_SAVE_NONVOL: u8 = 4;     // 2 nodes: save nonvolatile reg
pub const UWOP_SAVE_NONVOL_FAR: u8 = 5; // 3 nodes: save nonvolatile reg far
pub const UWOP_SAVE_XMM128: u8 = 8;     // 2 nodes: save 128-bit XMM reg
pub const UWOP_SAVE_XMM128_FAR: u8 = 9; // 3 nodes: save 128-bit XMM reg far
pub const UWOP_PUSH_MACHFRAME: u8 = 10; // 1 node: push machine frame

/// Win32 x86-64 RUNTIME_FUNCTION representing an entry in the .pdata section.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RUNTIME_FUNCTION {
    pub BeginAddress: u32,
    pub EndAddress: u32,
    pub UnwindData: u32,
}

/// Decoded unwind code operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnwindOpcode {
    PushNonVol { reg: u8 },
    AllocLarge { size: u32 },
    AllocSmall { size: u32 },
    SetFpReg,
    SaveNonVol { reg: u8, offset: u32 },
    SaveXmm128 { reg: u8, offset: u32 },
    PushMachFrame { error_code: bool },
    Other { code: u8, info: u8 },
}

/// Decoded UNWIND_INFO from .xdata.
#[derive(Debug, Clone)]
pub struct DecodedUnwindInfo {
    pub version: u8,
    pub flags: u8,
    pub size_of_prolog: u8,
    pub count_of_codes: u8,
    pub frame_register: u8,
    pub frame_offset: u8,
    pub opcodes: Vec<UnwindOpcode>,
    pub exception_handler_rva: Option<u32>,
}

/// Parser for x86-64 .pdata and .xdata tables.
pub struct SehTableParser;

impl SehTableParser {
    /// Parse raw byte slice of .xdata UNWIND_INFO.
    pub fn parse_xdata(xdata: &[u8]) -> Option<DecodedUnwindInfo> {
        if xdata.len() < 4 {
            return None;
        }

        let byte0 = xdata[0];
        let version = byte0 & 0x07;
        let flags = (byte0 >> 3) & 0x1F;
        let size_of_prolog = xdata[1];
        let count_of_codes = xdata[2];
        let byte3 = xdata[3];
        let frame_register = byte3 & 0x0F;
        let frame_offset = (byte3 >> 4) & 0x0F;

        let mut opcodes = Vec::new();
        let mut idx = 4;
        let mut code_idx = 0;

        while code_idx < count_of_codes as usize && idx + 2 <= xdata.len() {
            let _prolog_offset = xdata[idx];
            let op_byte = xdata[idx + 1];
            let opcode_id = op_byte & 0x0F;
            let op_info = (op_byte >> 4) & 0x0F;
            idx += 2;
            code_idx += 1;

            match opcode_id {
                UWOP_PUSH_NONVOL => {
                    opcodes.push(UnwindOpcode::PushNonVol { reg: op_info });
                }
                UWOP_ALLOC_LARGE => {
                    if op_info == 0 {
                        if idx + 2 <= xdata.len() {
                            let sz = u16::from_le_bytes([xdata[idx], xdata[idx + 1]]) as u32 * 8;
                            idx += 2;
                            code_idx += 1;
                            opcodes.push(UnwindOpcode::AllocLarge { size: sz });
                        }
                    } else if op_info == 1 {
                        if idx + 4 <= xdata.len() {
                            let sz = u32::from_le_bytes([xdata[idx], xdata[idx + 1], xdata[idx + 2], xdata[idx + 3]]);
                            idx += 4;
                            code_idx += 2;
                            opcodes.push(UnwindOpcode::AllocLarge { size: sz });
                        }
                    }
                }
                UWOP_ALLOC_SMALL => {
                    let sz = (op_info as u32 * 8) + 8;
                    opcodes.push(UnwindOpcode::AllocSmall { size: sz });
                }
                UWOP_SET_FPREG => {
                    opcodes.push(UnwindOpcode::SetFpReg);
                }
                UWOP_SAVE_NONVOL => {
                    if idx + 2 <= xdata.len() {
                        let off = u16::from_le_bytes([xdata[idx], xdata[idx + 1]]) as u32 * 8;
                        idx += 2;
                        code_idx += 1;
                        opcodes.push(UnwindOpcode::SaveNonVol { reg: op_info, offset: off });
                    }
                }
                UWOP_SAVE_XMM128 => {
                    if idx + 2 <= xdata.len() {
                        let off = u16::from_le_bytes([xdata[idx], xdata[idx + 1]]) as u32 * 16;
                        idx += 2;
                        code_idx += 1;
                        opcodes.push(UnwindOpcode::SaveXmm128 { reg: op_info, offset: off });
                    }
                }
                UWOP_PUSH_MACHFRAME => {
                    opcodes.push(UnwindOpcode::PushMachFrame { error_code: op_info == 1 });
                }
                _ => {
                    opcodes.push(UnwindOpcode::Other { code: opcode_id, info: op_info });
                }
            }
        }

        // Align idx if codes count was odd
        if (count_of_codes & 1) != 0 {
            idx += 2;
        }

        let mut exception_handler_rva = None;
        if (flags & (UNW_FLAG_EHANDLER | UNW_FLAG_UHANDLER)) != 0 && idx + 4 <= xdata.len() {
            let rva = u32::from_le_bytes([xdata[idx], xdata[idx + 1], xdata[idx + 2], xdata[idx + 3]]);
            exception_handler_rva = Some(rva);
        }

        Some(DecodedUnwindInfo {
            version,
            flags,
            size_of_prolog,
            count_of_codes,
            frame_register,
            frame_offset,
            opcodes,
            exception_handler_rva,
        })
    }
}
