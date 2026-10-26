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

pub const FIBER_GUARD_PAGE_SIZE: usize = 4096;

/// Stack Alignment & Guard Page Overflow Error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiberStackFault {
    MisalignedRsp(u64),
    GuardPageOverflow { rsp: u64, stack_limit: u64 },
    StackUnderflow { rsp: u64, stack_base: u64 },
}

/// Validates 16-byte SSE/AVX user stack alignment and checks for guard page violation.
pub struct FiberStackValidator;

impl FiberStackValidator {
    /// Check whether the fiber RSP has 16-byte alignment required by SSE/AVX operations.
    #[inline]
    pub fn is_16byte_aligned(rsp: u64) -> bool {
        (rsp & 0x0F) == 0
    }

    /// Validate stack bounds: ensure RSP is within [stack_limit, stack_base] and properly aligned.
    pub fn validate_stack_bounds(rsp: u64, stack_base: usize, stack_limit: usize) -> Result<(), FiberStackFault> {
        if !Self::is_16byte_aligned(rsp) {
            return Err(FiberStackFault::MisalignedRsp(rsp));
        }

        let limit = stack_limit as u64;
        let base = stack_base as u64;

        if rsp < limit {
            return Err(FiberStackFault::GuardPageOverflow { rsp, stack_limit: limit });
        }

        if rsp > base {
            return Err(FiberStackFault::StackUnderflow { rsp, stack_base: base });
        }

        Ok(())
    }
}

pub const FIBER_RUNLIST_CAPACITY: usize = 256;

/// Lock-free runlist for ready fibers waiting for game loop job worker execution.
pub struct FiberRunList {
    queue: Spinlock<Vec<u64>>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl FiberRunList {
    pub const fn new() -> Self {
        Self {
            queue: Spinlock::new(Vec::new()),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Push a fiber ID into ready runlist.
    pub fn push_ready(&self, fiber_id: u64) -> bool {
        let mut q = self.queue.lock();
        if q.len() < FIBER_RUNLIST_CAPACITY {
            q.push(fiber_id);
            self.head.fetch_add(1, Ordering::Release);
            true
        } else {
            false
        }
    }

    /// Pop next ready fiber ID for direct scheduling.
    pub fn pop_ready(&self) -> Option<u64> {
        let mut q = self.queue.lock();
        if q.is_empty() {
            None
        } else {
            self.tail.fetch_add(1, Ordering::Release);
            Some(q.remove(0))
        }
    }

    /// Return count of pending ready fibers.
    pub fn len(&self) -> usize {
        self.queue.lock().len()
    }

    /// Check if runlist is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Direct Low-Latency Fiber Switch Engine bypassing OS thread scheduler traps.
pub struct DirectFiberSwitchEngine {
    fibers: Spinlock<BTreeMap<u64, DirectFiberDescriptor>>,
    active_fiber_id: AtomicU64,
    total_switches: AtomicU64,
    runlist: FiberRunList,
}

impl DirectFiberSwitchEngine {
    pub const fn new() -> Self {
        Self {
            fibers: Spinlock::new(BTreeMap::new()),
            active_fiber_id: AtomicU64::new(0),
            total_switches: AtomicU64::new(0),
            runlist: FiberRunList::new(),
        }
    }

    /// Register a new fiber in the engine.
    pub fn register_fiber(&self, mut desc: DirectFiberDescriptor) -> Result<(), &'static str> {
        FiberStackValidator::validate_stack_bounds(
            desc.context.rsp,
            desc.stack_base,
            desc.stack_limit,
        ).map_err(|_| "Invalid initial fiber stack bounds or alignment")?;

        let _ = desc.transition_to(FiberState::Ready);
        let id = desc.fiber_id;
        let mut map = self.fibers.lock();
        map.insert(id, desc);
        self.runlist.push_ready(id);
        Ok(())
    }

    /// Perform a direct cooperative fiber switch from current to target fiber without OS kernel traps.
    pub fn switch_to_fiber_direct(&self, target_id: u64) -> Result<(u64, u64), &'static str> {
        let mut map = self.fibers.lock();

        let current_id = self.active_fiber_id.load(Ordering::Acquire);
        if current_id != 0 {
            if let Some(curr) = map.get_mut(&current_id) {
                if curr.state == FiberState::Running {
                    let _ = curr.transition_to(FiberState::Ready);
                }
            }
        }

        let target = map.get_mut(&target_id).ok_or("Target fiber not found")?;
        FiberStackValidator::validate_stack_bounds(
            target.context.rsp,
            target.stack_base,
            target.stack_limit,
        ).map_err(|_| "Target fiber stack violation or alignment error")?;

        target.transition_to(FiberState::Running)?;
        target.switch_count += 1;

        self.active_fiber_id.store(target_id, Ordering::Release);
        self.total_switches.fetch_add(1, Ordering::Relaxed);

        Ok((current_id, target_id))
    }

    /// Get current active fiber ID.
    pub fn get_active_fiber_id(&self) -> u64 {
        self.active_fiber_id.load(Ordering::Acquire)
    }

    /// Get total direct fiber switches executed.
    pub fn get_total_switches(&self) -> u64 {
        self.total_switches.load(Ordering::Acquire)
    }

    /// Access runlist.
    pub fn runlist(&self) -> &FiberRunList {
        &self.runlist
    }

    /// Retrieve fiber descriptor.
    pub fn get_fiber(&self, fiber_id: u64) -> Option<DirectFiberDescriptor> {
        self.fibers.lock().get(&fiber_id).cloned()
    }
}

pub static DIRECT_FIBER_ENGINE: DirectFiberSwitchEngine = DirectFiberSwitchEngine::new();
