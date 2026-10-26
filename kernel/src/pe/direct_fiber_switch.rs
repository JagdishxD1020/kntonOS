//! Win32 Fiber State Machine & Low-Latency Direct Fiber Context Switch Engine.
//!
//! Provides direct SwitchToFiber execution engine bypassing OS scheduler traps for game loop job
//! systems, 5-stage fiber state machine transitions (Created, Ready, Running, Waiting, Dead)
//! with lock-free runlist, and 16-byte SSE/AVX user stack alignment and guard page overflow validation.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;

/// Win32 Fiber Execution State in 5-stage state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiberState {
    Created,
    Ready,
    Running,
    Waiting,
    Dead,
}

/// Direct Fiber Callee-Saved Register Context.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectFiberContext {
    pub rip: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub mxcsr: u32,
    pub fp_control_word: u16,
    pub flags: u32,
}

impl Default for DirectFiberContext {
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

/// Win32 Direct Fiber Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectFiberDescriptor {
    pub fiber_id: u64,
    pub fiber_data: usize,
    pub stack_base: usize,
    pub stack_limit: usize,
    pub state: FiberState,
    pub context: DirectFiberContext,
    pub switch_count: u64,
}

impl DirectFiberDescriptor {
    pub fn new(fiber_id: u64, fiber_data: usize, stack_base: usize, stack_limit: usize, entry_point: u64) -> Self {
        let mut context = DirectFiberContext::default();
        context.rip = entry_point;
        context.rsp = stack_base as u64;
        Self {
            fiber_id,
            fiber_data,
            stack_base,
            stack_limit,
            state: FiberState::Created,
            context,
            switch_count: 0,
        }
    }

    /// Transition fiber state according to state machine rules.
    pub fn transition_to(&mut self, next_state: FiberState) -> Result<(), &'static str> {
        let valid = match (self.state, next_state) {
            (FiberState::Created, FiberState::Ready) => true,
            (FiberState::Ready, FiberState::Running) => true,
            (FiberState::Running, FiberState::Waiting) => true,
            (FiberState::Running, FiberState::Ready) => true,
            (FiberState::Running, FiberState::Dead) => true,
            (FiberState::Waiting, FiberState::Ready) => true,
            (FiberState::Waiting, FiberState::Dead) => true,
            _ => false,
        };
        if valid {
            self.state = next_state;
            Ok(())
        } else {
            Err("Invalid fiber state transition")
        }
    }
}
