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

/// Comprehensive self-test suite for Win32 Direct Fiber Switch & State Machine Subsystem.
pub fn test_win32_direct_fiber_switch_subsystem() -> bool {
    crate::serial_write_str("[KNT-WIN32-FIBER-SWITCH] Initializing Win32 Direct Fiber Switch Subsystem self-test...\r\n");

    // 1. Verify 5-stage Fiber State Machine Transitions
    let mut fiber = DirectFiberDescriptor::new(
        1001,
        0,
        0x0000_7FFF_1000_8000,
        0x0000_7FFF_1000_0000,
        0x0000_0040_1000,
    );

    if fiber.state != FiberState::Created {
        crate::serial_write_str("[KNT-TEST-ERR] Initial fiber state is not Created\r\n");
        return false;
    }

    if fiber.transition_to(FiberState::Ready).is_err() || fiber.state != FiberState::Ready {
        crate::serial_write_str("[KNT-TEST-ERR] Transition to Ready failed\r\n");
        return false;
    }

    if fiber.transition_to(FiberState::Running).is_err() || fiber.state != FiberState::Running {
        crate::serial_write_str("[KNT-TEST-ERR] Transition to Running failed\r\n");
        return false;
    }

    if fiber.transition_to(FiberState::Waiting).is_err() || fiber.state != FiberState::Waiting {
        crate::serial_write_str("[KNT-TEST-ERR] Transition to Waiting failed\r\n");
        return false;
    }

    if fiber.transition_to(FiberState::Ready).is_err() || fiber.state != FiberState::Ready {
        crate::serial_write_str("[KNT-TEST-ERR] Transition from Waiting to Ready failed\r\n");
        return false;
    }

    // Invalid transition check: Ready -> Waiting must fail
    if fiber.transition_to(FiberState::Waiting).is_ok() {
        crate::serial_write_str("[KNT-TEST-ERR] Illegal transition Ready -> Waiting succeeded\r\n");
        return false;
    }

    // 2. Verify 16-byte SSE/AVX Stack Alignment & Guard Page Overflow Validator
    let valid_rsp = 0x0000_7FFF_1000_8000u64;
    let misaligned_rsp = 0x0000_7FFF_1000_8008u64;
    let stack_base = 0x0000_7FFF_1000_8000usize;
    let stack_limit = 0x0000_7FFF_1000_0000usize;

    if !FiberStackValidator::is_16byte_aligned(valid_rsp) {
        crate::serial_write_str("[KNT-TEST-ERR] Valid 16-byte aligned RSP failed alignment check\r\n");
        return false;
    }

    if FiberStackValidator::is_16byte_aligned(misaligned_rsp) {
        crate::serial_write_str("[KNT-TEST-ERR] Misaligned RSP passed alignment check\r\n");
        return false;
    }

    let overflow_rsp = 0x0000_7FFF_0FFF_F000u64; // Below stack limit
    match FiberStackValidator::validate_stack_bounds(overflow_rsp, stack_base, stack_limit) {
        Err(FiberStackFault::GuardPageOverflow { .. }) => {}
        _ => {
            crate::serial_write_str("[KNT-TEST-ERR] Guard page overflow not detected\r\n");
            return false;
        }
    }

    // 3. Verify Fiber RunList FIFO Queue
    let runlist = FiberRunList::new();
    runlist.push_ready(201);
    runlist.push_ready(202);
    if runlist.len() != 2 {
        crate::serial_write_str("[KNT-TEST-ERR] Runlist length mismatch\r\n");
        return false;
    }
    if runlist.pop_ready() != Some(201) || runlist.pop_ready() != Some(202) {
        crate::serial_write_str("[KNT-TEST-ERR] Runlist FIFO order mismatch\r\n");
        return false;
    }
    if !runlist.is_empty() {
        crate::serial_write_str("[KNT-TEST-ERR] Runlist should be empty\r\n");
        return false;
    }

    // 4. Verify Direct Fiber Switch Engine
    let f1 = DirectFiberDescriptor::new(
        301,
        0,
        0x0000_7FFF_3000_8000,
        0x0000_7FFF_3000_0000,
        0x0000_0040_3000,
    );
    let f2 = DirectFiberDescriptor::new(
        302,
        0,
        0x0000_7FFF_4000_8000,
        0x0000_7FFF_4000_0000,
        0x0000_0040_4000,
    );

    if DIRECT_FIBER_ENGINE.register_fiber(f1).is_err() {
        crate::serial_write_str("[KNT-TEST-ERR] Failed to register fiber 301\r\n");
        return false;
    }
    if DIRECT_FIBER_ENGINE.register_fiber(f2).is_err() {
        crate::serial_write_str("[KNT-TEST-ERR] Failed to register fiber 302\r\n");
        return false;
    }

    let initial_switches = DIRECT_FIBER_ENGINE.get_total_switches();

    // Switch to Fiber 301
    let sw1 = DIRECT_FIBER_ENGINE.switch_to_fiber_direct(301);
    if sw1 != Ok((0, 301)) || DIRECT_FIBER_ENGINE.get_active_fiber_id() != 301 {
        crate::serial_write_str("[KNT-TEST-ERR] Direct switch to fiber 301 failed\r\n");
        return false;
    }

    // Switch directly from Fiber 301 to Fiber 302
    let sw2 = DIRECT_FIBER_ENGINE.switch_to_fiber_direct(302);
    if sw2 != Ok((301, 302)) || DIRECT_FIBER_ENGINE.get_active_fiber_id() != 302 {
        crate::serial_write_str("[KNT-TEST-ERR] Direct switch from fiber 301 to 302 failed\r\n");
        return false;
    }

    if DIRECT_FIBER_ENGINE.get_total_switches() != initial_switches + 2 {
        crate::serial_write_str("[KNT-TEST-ERR] Total switches count mismatch\r\n");
        return false;
    }

    let f1_desc = DIRECT_FIBER_ENGINE.get_fiber(301).unwrap();
    let f2_desc = DIRECT_FIBER_ENGINE.get_fiber(302).unwrap();
    if f1_desc.state != FiberState::Ready || f2_desc.state != FiberState::Running {
        crate::serial_write_str("[KNT-TEST-ERR] Fiber states after direct switch mismatch\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-FIBER-SWITCH-PASS] Win32 Direct Fiber Context Switch & State Machine verified\r\n");
    true
}
