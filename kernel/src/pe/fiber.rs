//! Win32 Fibers & Lightweight User-Mode Scheduling (UMS) Engine.
//!
//! Emulates Windows NT cooperative fibers, user-mode scheduling (UMS),
//! callee-saved execution context switching, and Fiber Local Storage (FLS).

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};
pub use crate::pe::tls_fiber::FIBER_FLAG_FLOAT_SWITCH;

pub const FLS_OUT_OF_INDEXES: u32 = 0xFFFF_FFFF;
pub const MAX_FLS_SLOTS: usize = 128;
pub const DEFAULT_FIBER_STACK_SIZE: usize = 64 * 1024;

pub type LPFIBER_START_ROUTINE = unsafe extern "system" fn(lpFiberParameter: *mut c_void);
pub type PFLS_CALLBACK_FUNCTION = Option<unsafe extern "system" fn(lpFlsData: *mut c_void)>;

/// Saved x86-64 callee-saved machine context for cooperative fiber scheduling.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy)]
pub struct Win32FiberContext {
    pub rip: usize,
    pub rsp: usize,
    pub rbp: usize,
    pub rbx: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
    pub rdi: usize,
    pub rsi: usize,
    pub mxcsr: u32,
    pub fp_control_word: u16,
    pub flags: u32,
}

impl Default for Win32FiberContext {
    fn default() -> Self {
        Self {
            rip: 0,
            rsp: 0,
            rbp: 0,
            rbx: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rdi: 0,
            rsi: 0,
            mxcsr: 0x1F80,
            fp_control_word: 0x037F,
            flags: 0,
        }
    }
}

/// Internal state descriptor for a Win32 fiber.
pub struct Win32FiberState {
    pub fiber_id: u64,
    pub fiber_data: usize,
    pub stack_base: usize,
    pub stack_limit: usize,
    pub stack_size: usize,
    pub dealloc_stack: bool,
    pub is_thread_fiber: bool,
    pub start_routine: Option<LPFIBER_START_ROUTINE>,
    pub context: Win32FiberContext,
    pub fls_slots: [usize; MAX_FLS_SLOTS],
    pub flags: u32,
    pub stack_allocation: Option<Vec<u8>>,
}

unsafe impl Send for Win32FiberState {}
unsafe impl Sync for Win32FiberState {}

static NEXT_FIBER_ID: AtomicU64 = AtomicU64::new(0x000F_1000);
pub static FIBERS: Spinlock<BTreeMap<usize, Win32FiberState>> = Spinlock::new(BTreeMap::new());
pub static THREAD_CURRENT_FIBER: Spinlock<BTreeMap<u32, usize>> = Spinlock::new(BTreeMap::new());

/// Updates TEB stack bounds and fiber data pointer for active fiber.
pub unsafe fn update_teb_fiber_state(fiber_data: *mut c_void, stack_base: usize, stack_limit: usize) {
    let teb = crate::pe::peb_teb::win32_get_current_teb();
    if !teb.is_null() {
        (*teb).tib.fiber_data = fiber_data as u64;
        if stack_base != 0 {
            (*teb).tib.stack_base = stack_base as u64;
        }
        if stack_limit != 0 {
            (*teb).tib.stack_limit = stack_limit as u64;
        }
    }
}
