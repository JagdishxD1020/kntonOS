//! Win32 Thread Pool High-Priority Game Worker Queues & Work-Stealing Fiber Scheduler.
//!
//! Implements a Chase-Lev work-stealing double-ended queue (deque) for thread pool workers,
//! high-priority game task scheduling with performance core pinning, and fiber migration.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;

pub const CHASE_LEV_INITIAL_CAPACITY: usize = 64;

/// Work item task wrapper for worker queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadPoolWorkItem {
    pub work_id: u64,
    pub task_fn: usize,
    pub context: usize,
    pub priority: u8, // 0 = normal, 1 = high, 2 = critical game render/audio
    pub enqueue_tsc: u64,
}

/// Chase-Lev work-stealing deque for thread pool worker threads.
/// Single producer (worker thread) pushes/pops at bottom; multiple thieves steal at top.
pub struct ChaseLevDeque {
    buffer: Spinlock<Vec<ThreadPoolWorkItem>>,
    top: AtomicIsize,
    bottom: AtomicIsize,
}

impl ChaseLevDeque {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Spinlock::new(Vec::with_capacity(capacity)),
            top: AtomicIsize::new(0),
            bottom: AtomicIsize::new(0),
        }
    }

    /// Owner thread pushes a work item onto the bottom of the deque.
    pub fn push(&self, item: ThreadPoolWorkItem) {
        let mut b = self.bottom.load(Ordering::Relaxed);
        let t = self.top.load(Ordering::Acquire);
        let mut buf = self.buffer.lock();

        let size = b - t;
        if size < 0 {
            // Underflow reset
            self.top.store(0, Ordering::Relaxed);
            self.bottom.store(0, Ordering::Relaxed);
            b = 0;
        }

        buf.push(item);
        self.bottom.store(b + 1, Ordering::Release);
    }

    /// Owner thread pops a work item from the bottom of the deque (LIFO order).
    pub fn pop(&self) -> Option<ThreadPoolWorkItem> {
        let b = self.bottom.load(Ordering::Relaxed) - 1;
        self.bottom.store(b, Ordering::Relaxed);
        core::sync::atomic::fence(Ordering::SeqCst);

        let t = self.top.load(Ordering::Relaxed);
        if t <= b {
            // Non-empty deque
            let mut buf = self.buffer.lock();
            if let Some(item) = buf.pop() {
                if t == b {
                    // Single item contention with concurrent thief
                    if self.top.compare_exchange(t, t + 1, Ordering::SeqCst, Ordering::Relaxed).is_err() {
                        // Lost race to a thief
                        self.bottom.store(t + 1, Ordering::Relaxed);
                        return None;
                    }
                    self.bottom.store(t + 1, Ordering::Relaxed);
                }
                return Some(item);
            }
            self.bottom.store(t, Ordering::Relaxed);
            None
        } else {
            // Empty deque
            self.bottom.store(t, Ordering::Relaxed);
            None
        }
    }

    /// Thief thread steals a work item from the top of the deque (FIFO order).
    pub fn steal(&self) -> Option<ThreadPoolWorkItem> {
        let t = self.top.load(Ordering::Acquire);
        core::sync::atomic::fence(Ordering::SeqCst);
        let b = self.bottom.load(Ordering::Acquire);

        if t < b {
            let mut buf = self.buffer.lock();
            if !buf.is_empty() {
                if self.top.compare_exchange(t, t + 1, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
                    return Some(buf.remove(0));
                }
            }
            None
        } else {
            None
        }
    }

    /// Check if deque is empty.
    pub fn is_empty(&self) -> bool {
        self.top.load(Ordering::Acquire) >= self.bottom.load(Ordering::Acquire)
    }
}

pub const P_CORE_AFFINITY_MASK: usize = 0x0000_0003; // Cores 0 & 1 (P-cores)
pub const E_CORE_AFFINITY_MASK: usize = 0x0000_000C; // Cores 2 & 3 (E-cores)

/// High-Priority Game Worker Thread Descriptor.
pub struct GameWorkerThread {
    pub thread_id: u64,
    pub assigned_core: usize,
    pub affinity_mask: usize,
    pub is_performance_core: bool,
    pub local_deque: ChaseLevDeque,
    pub processed_items: AtomicU64,
    pub stolen_items: AtomicU64,
}

impl GameWorkerThread {
    pub fn new(tid: u64, core: usize, is_p_core: bool) -> Self {
        let mask = if is_p_core { P_CORE_AFFINITY_MASK } else { E_CORE_AFFINITY_MASK };
        Self {
            thread_id: tid,
            assigned_core: core,
            affinity_mask: mask,
            is_performance_core: is_p_core,
            local_deque: ChaseLevDeque::new(CHASE_LEV_INITIAL_CAPACITY),
            processed_items: AtomicU64::new(0),
            stolen_items: AtomicU64::new(0),
        }
    }
}

/// High-Priority Game Worker Scheduler.
pub struct HighPriorityGameWorkerScheduler {
    workers: Spinlock<Vec<GameWorkerThread>>,
    next_work_id: AtomicU64,
}

impl HighPriorityGameWorkerScheduler {
    pub const fn new() -> Self {
        Self {
            workers: Spinlock::new(Vec::new()),
            next_work_id: AtomicU64::new(1),
        }
    }

    /// Register a worker thread with P-core or E-core pinning.
    pub fn register_worker(&self, tid: u64, core: usize, is_p_core: bool) {
        self.workers.lock().push(GameWorkerThread::new(tid, core, is_p_core));
    }

    /// Dispatch a high-priority game task.
    /// Critical priority tasks are always enqueued to P-core worker deques for minimum latency.
    pub fn schedule_game_task(&self, task_fn: usize, ctx: usize, priority: u8) -> u64 {
        let work_id = self.next_work_id.fetch_add(1, Ordering::Relaxed);
        let item = ThreadPoolWorkItem {
            work_id,
            task_fn,
            context: ctx,
            priority,
            enqueue_tsc: 1000,
        };

        let workers = self.workers.lock();
        if workers.is_empty() {
            return work_id;
        }

        // For high/critical priority, target P-core workers first
        if priority >= 1 {
            if let Some(p_worker) = workers.iter().find(|w| w.is_performance_core) {
                p_worker.local_deque.push(item);
                return work_id;
            }
        }

        // Fallback to first available worker
        workers[0].local_deque.push(item);
        work_id
    }

    /// Worker execution step: try local pop, if empty attempt to steal from other workers.
    pub fn worker_execute_next(&self, worker_idx: usize) -> Option<ThreadPoolWorkItem> {
        let workers = self.workers.lock();
        if worker_idx >= workers.len() {
            return None;
        }

        // 1. Try popping from own bottom
        if let Some(item) = workers[worker_idx].local_deque.pop() {
            workers[worker_idx].processed_items.fetch_add(1, Ordering::Relaxed);
            return Some(item);
        }

        // 2. Work-stealing: steal from another worker's top
        for (i, victim) in workers.iter().enumerate() {
            if i != worker_idx {
                if let Some(stolen) = victim.local_deque.steal() {
                    workers[worker_idx].processed_items.fetch_add(1, Ordering::Relaxed);
                    workers[worker_idx].stolen_items.fetch_add(1, Ordering::Relaxed);
                    return Some(stolen);
                }
            }
        }

        None
    }
}

pub static GAME_WORKER_SCHEDULER: HighPriorityGameWorkerScheduler =
    HighPriorityGameWorkerScheduler::new();

/// Fiber State Snapshot for Work-Stealing Migration.
#[derive(Debug, Clone)]
pub struct StealableFiber {
    pub fiber_id: u64,
    pub owning_thread_id: u64,
    pub fls_slots: Vec<usize>,
    pub execution_quantum_ticks: u32,
    pub starvation_boost_applied: bool,
    pub is_yielded: bool,
}

/// Fiber-to-Thread Work-Stealing Migration Governor.
pub struct FiberMigrationGovernor {
    fibers: Spinlock<BTreeMap<u64, StealableFiber>>,
    starvation_threshold_ticks: u32,
}

impl FiberMigrationGovernor {
    pub const fn new() -> Self {
        Self {
            fibers: Spinlock::new(BTreeMap::new()),
            starvation_threshold_ticks: 100,
        }
    }

    /// Register a fiber with its current FLS state array.
    pub fn register_fiber(&self, fiber_id: u64, thread_id: u64, fls_data: Vec<usize>) {
        let fiber = StealableFiber {
            fiber_id,
            owning_thread_id: thread_id,
            fls_slots: fls_data,
            execution_quantum_ticks: 0,
            starvation_boost_applied: false,
            is_yielded: false,
        };
        self.fibers.lock().insert(fiber_id, fiber);
    }

    /// Yield execution from current worker thread.
    pub fn yield_fiber(&self, fiber_id: u64) {
        let mut map = self.fibers.lock();
        if let Some(f) = map.get_mut(&fiber_id) {
            f.is_yielded = true;
            f.execution_quantum_ticks += 1;
            if f.execution_quantum_ticks >= self.starvation_threshold_ticks {
                f.starvation_boost_applied = true;
            }
        }
    }

    /// Migrate yielded fiber to a new destination worker thread, preserving FLS continuity.
    pub fn migrate_fiber_to_thread(
        &self,
        fiber_id: u64,
        target_thread_id: u64,
    ) -> Result<Vec<usize>, ()> {
        let mut map = self.fibers.lock();
        let f = map.get_mut(&fiber_id).ok_or(())?;

        if !f.is_yielded && !f.starvation_boost_applied {
            return Err(());
        }

        // Migrate ownership
        f.owning_thread_id = target_thread_id;
        f.is_yielded = false;
        f.execution_quantum_ticks = 0;
        f.starvation_boost_applied = false;

        // Return preserved FLS slots
        Ok(f.fls_slots.clone())
    }

    /// Query fiber state.
    pub fn get_fiber(&self, fiber_id: u64) -> Option<StealableFiber> {
        self.fibers.lock().get(&fiber_id).cloned()
    }
}

pub static FIBER_MIGRATION_GOVERNOR: FiberMigrationGovernor = FiberMigrationGovernor::new();

/// Comprehensive self-test for Win32 Thread Pool High-Priority Worker Queues & Work-Stealing Scheduler.
pub fn test_win32_threadpool_steal_subsystem() -> bool {
    crate::serial_write_str("[KNT-WIN32-THREADPOOL-STEAL] Initializing Win32 Work-Stealing Subsystem self-test...\r\n");

    // 1. Test Chase-Lev Deque Push, Pop (LIFO), and Steal (FIFO)
    let deque = ChaseLevDeque::new(16);

    let item1 = ThreadPoolWorkItem {
        work_id: 1,
        task_fn: 0x0040_1000,
        context: 0x1111,
        priority: 0,
        enqueue_tsc: 100,
    };
    let item2 = ThreadPoolWorkItem {
        work_id: 2,
        task_fn: 0x0040_2000,
        context: 0x2222,
        priority: 1,
        enqueue_tsc: 200,
    };
    let item3 = ThreadPoolWorkItem {
        work_id: 3,
        task_fn: 0x0040_3000,
        context: 0x3333,
        priority: 2,
        enqueue_tsc: 300,
    };

    deque.push(item1);
    deque.push(item2);
    deque.push(item3);

    // Thief steals from top (FIFO: item1 first)
    let stolen = match deque.steal() {
        Some(s) => s,
        None => {
            crate::serial_write_str("[KNT-TEST-ERR] Chase-Lev steal failed\r\n");
            return false;
        }
    };
    if stolen.work_id != 1 {
        crate::serial_write_str("[KNT-TEST-ERR] Stolen item was not item 1 (FIFO order violation)\r\n");
        return false;
    }

    // Owner pops from bottom (LIFO: item3 first)
    let popped = match deque.pop() {
        Some(p) => p,
        None => {
            crate::serial_write_str("[KNT-TEST-ERR] Chase-Lev pop failed\r\n");
            return false;
        }
    };
    if popped.work_id != 3 {
        crate::serial_write_str("[KNT-TEST-ERR] Popped item was not item 3 (LIFO order violation)\r\n");
        return false;
    }

    // 2. Test High-Priority Game Worker Scheduling & P-Core Pinning
    let sched = &GAME_WORKER_SCHEDULER;
    sched.register_worker(5001, 0, true);  // Worker 0: P-core 0
    sched.register_worker(5002, 2, false); // Worker 1: E-core 2

    // Schedule critical game task (priority = 2) -> Must land on P-Core worker (Worker 0)
    let crit_id = sched.schedule_game_task(0x0040_9000, 0xCAFE, 2);

    // Worker 0 executes local task
    let exec_p = match sched.worker_execute_next(0) {
        Some(e) => e,
        None => {
            crate::serial_write_str("[KNT-TEST-ERR] Worker 0 failed to execute local task\r\n");
            return false;
        }
    };
    if exec_p.work_id != crit_id || exec_p.priority != 2 {
        crate::serial_write_str("[KNT-TEST-ERR] Executed task was not the critical game task\r\n");
        return false;
    }

    // 3. Test Inter-Worker Work-Stealing
    // Enqueue normal task onto Worker 0
    let normal_id = sched.schedule_game_task(0x0040_8000, 0xBEEF, 0);

    // Worker 1 (idle on E-core) steals task from Worker 0
    let exec_e = match sched.worker_execute_next(1) {
        Some(e) => e,
        None => {
            crate::serial_write_str("[KNT-TEST-ERR] Worker 1 failed to steal task from Worker 0\r\n");
            return false;
        }
    };
    if exec_e.work_id != normal_id {
        crate::serial_write_str("[KNT-TEST-ERR] Stolen task ID mismatch\r\n");
        return false;
    }

    // 4. Test Fiber-to-Thread Migration & FLS Continuity
    let fls_initial = alloc::vec![0x1234usize, 0x5678usize];
    FIBER_MIGRATION_GOVERNOR.register_fiber(777, 5001, fls_initial);

    // Yield fiber
    FIBER_MIGRATION_GOVERNOR.yield_fiber(777);

    // Migrate to Worker 1 (TID 5002)
    let migrated_fls = match FIBER_MIGRATION_GOVERNOR.migrate_fiber_to_thread(777, 5002) {
        Ok(slots) => slots,
        Err(_) => {
            crate::serial_write_str("[KNT-TEST-ERR] Fiber migration to thread 5002 failed\r\n");
            return false;
        }
    };

    if migrated_fls.len() != 2 || migrated_fls[0] != 0x1234 || migrated_fls[1] != 0x5678 {
        crate::serial_write_str("[KNT-TEST-ERR] Migrated fiber FLS state corrupted\r\n");
        return false;
    }

    let f_post = FIBER_MIGRATION_GOVERNOR.get_fiber(777).unwrap();
    if f_post.owning_thread_id != 5002 {
        crate::serial_write_str("[KNT-TEST-ERR] Fiber owning thread ID mismatch after migration\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-THREADPOOL-STEAL-PASS] Win32 Work-Stealing & Fiber Scheduler verified\r\n");
    true
}
