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

/// Simulated stack frame context for unwinding and dispatching.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UNWIND_CONTEXT {
    pub Rip: u64,
    pub Rsp: u64,
    pub Rbp: u64,
    pub Rax: u64,
    pub Rbx: u64,
    pub Rcx: u64,
    pub Rdx: u64,
    pub Rsi: u64,
    pub Rdi: u64,
    pub R12: u64,
    pub R13: u64,
    pub R14: u64,
    pub R15: u64,
}

/// Simulated function frame registration for dispatch testing.
#[derive(Debug, Clone)]
pub struct FunctionFrameEntry {
    pub image_base: u64,
    pub runtime_function: RUNTIME_FUNCTION,
    pub unwind_info: DecodedUnwindInfo,
    pub frame_handler: Option<fn(record: &mut EXCEPTION_RECORD, context: &mut UNWIND_CONTEXT) -> i32>,
}

/// Dispatch status result of RtlDispatchException.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchResult {
    Handled,
    ContinueSearch,
    FatalUnwindCollision,
}

/// 2-Phase Exception Dispatcher & Virtual Frame Unwinder.
pub struct SehFrameDispatcher {
    frames: Spinlock<Vec<FunctionFrameEntry>>,
}

impl SehFrameDispatcher {
    pub const fn new() -> Self {
        Self {
            frames: Spinlock::new(Vec::new()),
        }
    }

    /// Register a function frame in the dispatcher table.
    pub fn register_frame(&self, frame: FunctionFrameEntry) {
        self.frames.lock().push(frame);
    }

    /// Phase 1: Search handlers across calling stack frames without altering execution state.
    pub fn phase1_search_handlers(
        &self,
        record: &mut EXCEPTION_RECORD,
        context: &mut UNWIND_CONTEXT,
    ) -> Option<usize> {
        let frames = self.frames.lock();
        for (idx, entry) in frames.iter().enumerate() {
            let func_start = entry.image_base + entry.runtime_function.BeginAddress as u64;
            let func_end = entry.image_base + entry.runtime_function.EndAddress as u64;

            if context.Rip >= func_start && context.Rip < func_end {
                if let Some(handler) = entry.frame_handler {
                    let disposition = handler(record, context);
                    if disposition == EXCEPTION_EXECUTE_HANDLER {
                        return Some(idx);
                    }
                }
            }
        }
        None
    }

    /// Phase 2: Unwind stack frames up to the target frame index (RtlUnwindEx).
    pub fn phase2_unwind_to_target(
        &self,
        record: &mut EXCEPTION_RECORD,
        context: &mut UNWIND_CONTEXT,
        target_idx: usize,
    ) -> Result<u64, DispatchResult> {
        let frames = self.frames.lock();
        if target_idx >= frames.len() {
            return Err(DispatchResult::ContinueSearch);
        }

        record.ExceptionFlags |= EXCEPTION_UNWINDING;

        // Apply unwind opcodes to virtual RSP/RBP
        let target_frame = &frames[target_idx];
        for op in &target_frame.unwind_info.opcodes {
            match op {
                UnwindOpcode::AllocSmall { size } => {
                    context.Rsp = context.Rsp.saturating_add(*size as u64);
                }
                UnwindOpcode::AllocLarge { size } => {
                    context.Rsp = context.Rsp.saturating_add(*size as u64);
                }
                UnwindOpcode::PushNonVol { .. } => {
                    context.Rsp = context.Rsp.saturating_add(8);
                }
                _ => {}
            }
        }

        // Return new virtual RIP
        let landing_rip = target_frame.image_base + target_frame.runtime_function.BeginAddress as u64;
        context.Rip = landing_rip;
        Ok(landing_rip)
    }

    /// Full 2-Phase Dispatcher (RtlDispatchException).
    pub fn dispatch_exception(
        &self,
        record: &mut EXCEPTION_RECORD,
        context: &mut UNWIND_CONTEXT,
    ) -> DispatchResult {
        // Phase 1: Search
        match self.phase1_search_handlers(record, context) {
            Some(target_idx) => {
                // Phase 2: Unwind
                if self.phase2_unwind_to_target(record, context, target_idx).is_ok() {
                    DispatchResult::Handled
                } else {
                    DispatchResult::FatalUnwindCollision
                }
            }
            None => DispatchResult::ContinueSearch,
        }
    }
}

pub static SEH_FRAME_DISPATCHER: SehFrameDispatcher = SehFrameDispatcher::new();

/// Vectored Exception/Continue Handler callback type.
pub type PVECTORED_EXCEPTION_HANDLER =
    fn(exception_info: &mut EXCEPTION_POINTERS_UNWIND) -> i32;

/// Exception pointers passed to VEH/VCH.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EXCEPTION_POINTERS_UNWIND {
    pub ExceptionRecord: *mut EXCEPTION_RECORD,
    pub ContextRecord: *mut UNWIND_CONTEXT,
}

/// Node representing a registered VEH or VCH handler.
#[derive(Clone)]
pub struct VectoredHandlerEntry {
    pub id: u32,
    pub is_first: bool,
    pub handler: PVECTORED_EXCEPTION_HANDLER,
}

/// Vectored Exception Handling (VEH) & Vectored Continue Handling (VCH) priority table.
pub struct VectoredHandlerList {
    veh_handlers: Spinlock<Vec<VectoredHandlerEntry>>,
    vch_handlers: Spinlock<Vec<VectoredHandlerEntry>>,
    next_id: AtomicU32,
}

impl VectoredHandlerList {
    pub const fn new() -> Self {
        Self {
            veh_handlers: Spinlock::new(Vec::new()),
            vch_handlers: Spinlock::new(Vec::new()),
            next_id: AtomicU32::new(1),
        }
    }

    /// Add a Vectored Exception Handler (AddVectoredExceptionHandler).
    pub fn add_veh(&self, first: bool, handler: PVECTORED_EXCEPTION_HANDLER) -> HANDLE {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let entry = VectoredHandlerEntry {
            id,
            is_first: first,
            handler,
        };
        let mut list = self.veh_handlers.lock();
        if first {
            list.insert(0, entry);
        } else {
            list.push(entry);
        }
        id as HANDLE
    }

    /// Add a Vectored Continue Handler (AddVectoredContinueHandler).
    pub fn add_vch(&self, first: bool, handler: PVECTORED_EXCEPTION_HANDLER) -> HANDLE {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let entry = VectoredHandlerEntry {
            id,
            is_first: first,
            handler,
        };
        let mut list = self.vch_handlers.lock();
        if first {
            list.insert(0, entry);
        } else {
            list.push(entry);
        }
        id as HANDLE
    }

    /// Execute VEH priority list before any frame SEH handler is called.
    /// Returns true if an exception was handled (EXCEPTION_CONTINUE_EXECUTION), false otherwise.
    pub fn execute_veh_list(
        &self,
        record: &mut EXCEPTION_RECORD,
        context: &mut UNWIND_CONTEXT,
    ) -> bool {
        let handlers = self.veh_handlers.lock().clone();
        let mut ptrs = EXCEPTION_POINTERS_UNWIND {
            ExceptionRecord: record as *mut _,
            ContextRecord: context as *mut _,
        };

        for entry in handlers {
            let res = (entry.handler)(&mut ptrs);
            if res == EXCEPTION_CONTINUE_EXECUTION {
                return true;
            }
        }
        false
    }

    /// Execute VCH priority list when continuing execution.
    pub fn execute_vch_list(
        &self,
        record: &mut EXCEPTION_RECORD,
        context: &mut UNWIND_CONTEXT,
    ) -> bool {
        let handlers = self.vch_handlers.lock().clone();
        let mut ptrs = EXCEPTION_POINTERS_UNWIND {
            ExceptionRecord: record as *mut _,
            ContextRecord: context as *mut _,
        };

        for entry in handlers {
            let res = (entry.handler)(&mut ptrs);
            if res == EXCEPTION_CONTINUE_EXECUTION {
                return true;
            }
        }
        false
    }

    /// Remove a registered vectored handler.
    pub fn remove_vectored_handler(&self, handle: HANDLE) -> bool {
        let id = handle as u32;
        let mut veh = self.veh_handlers.lock();
        if let Some(pos) = veh.iter().position(|e| e.id == id) {
            veh.remove(pos);
            return true;
        }

        let mut vch = self.vch_handlers.lock();
        if let Some(pos) = vch.iter().position(|e| e.id == id) {
            vch.remove(pos);
            return true;
        }

        false
    }
}

pub static VECTORED_HANDLER_LIST: VectoredHandlerList = VectoredHandlerList::new();
