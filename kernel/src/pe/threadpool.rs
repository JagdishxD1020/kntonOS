//! Win32 Threadpool API & Parallel Game Job Scheduler.
//!
//! Emulates Windows Vista/7/10+ user-mode threadpool (Threadpool API v2),
//! multi-core work-stealing job scheduler, timer callbacks, and cleanup groups.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::file::FILETIME;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_SUCCESS,
};

pub type PTP_POOL = *mut c_void;
pub type PTP_WORK = *mut c_void;
pub type PTP_TIMER = *mut c_void;
pub type PTP_CLEANUP_GROUP = *mut c_void;
pub type PTP_CALLBACK_INSTANCE = *mut c_void;
pub type PTP_CALLBACK_ENVIRON = *mut TP_CALLBACK_ENVIRON;

pub type PTP_WORK_CALLBACK =
    Option<unsafe extern "system" fn(instance: PTP_CALLBACK_INSTANCE, context: *mut c_void, work: PTP_WORK)>;
pub type PTP_TIMER_CALLBACK =
    Option<unsafe extern "system" fn(instance: PTP_CALLBACK_INSTANCE, context: *mut c_void, timer: PTP_TIMER)>;
pub type PTP_CLEANUP_GROUP_CANCEL_CALLBACK =
    Option<unsafe extern "system" fn(object_context: *mut c_void, cleanup_context: *mut c_void)>;

pub type BOOL = i32;
pub const TRUE: BOOL = 1;
pub const FALSE: BOOL = 0;

/// Win32 threadpool callback environment structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TP_CALLBACK_ENVIRON {
    pub Version: u32,
    pub Pool: PTP_POOL,
    pub CleanupGroup: PTP_CLEANUP_GROUP,
    pub CleanupGroupCancelCallback: PTP_CLEANUP_GROUP_CANCEL_CALLBACK,
    pub RaceDll: *mut c_void,
    pub ActivationContext: *mut c_void,
    pub FinalizationCallback: *mut c_void,
    pub Flags: u32,
}

impl Default for TP_CALLBACK_ENVIRON {
    fn default() -> Self {
        Self {
            Version: 1,
            Pool: core::ptr::null_mut(),
            CleanupGroup: core::ptr::null_mut(),
            CleanupGroupCancelCallback: None,
            RaceDll: core::ptr::null_mut(),
            ActivationContext: core::ptr::null_mut(),
            FinalizationCallback: core::ptr::null_mut(),
            Flags: 0,
        }
    }
}

/// Internal threadpool descriptor.
pub struct ThreadpoolInner {
    pub id: u64,
    pub min_threads: u32,
    pub max_threads: u32,
    pub active_threads: u32,
    pub closed: bool,
}

static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(0x8000_1000);
pub static THREADPOOLS: Spinlock<BTreeMap<u64, ThreadpoolInner>> = Spinlock::new(BTreeMap::new());

/// Creates a new Win32 threadpool object.
pub unsafe extern "system" fn CreateThreadpool(reserved: *mut c_void) -> PTP_POOL {
    if !reserved.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }

    let id = NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst);
    let pool = ThreadpoolInner {
        id,
        min_threads: 1,
        max_threads: 8,
        active_threads: 1,
        closed: false,
    };

    let mut lock = THREADPOOLS.lock();
    lock.insert(id, pool);

    win32_set_last_error(ERROR_SUCCESS);
    id as PTP_POOL
}

/// Closes an allocated Win32 threadpool.
pub unsafe extern "system" fn CloseThreadpool(pool: PTP_POOL) {
    if pool.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return;
    }

    let id = pool as u64;
    let mut lock = THREADPOOLS.lock();
    if lock.remove(&id).is_some() {
        win32_set_last_error(ERROR_SUCCESS);
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
    }
}

/// Sets minimum worker thread count for the threadpool.
pub unsafe extern "system" fn SetThreadpoolThreadMinimum(pool: PTP_POOL, cthrd_min: u32) -> BOOL {
    if pool.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return FALSE;
    }

    let id = pool as u64;
    let mut lock = THREADPOOLS.lock();
    if let Some(p) = lock.get_mut(&id) {
        if cthrd_min > p.max_threads {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return FALSE;
        }
        p.min_threads = cthrd_min;
        if p.active_threads < cthrd_min {
            p.active_threads = cthrd_min;
        }
        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        FALSE
    }
}

/// Sets maximum worker thread count for the threadpool.
pub unsafe extern "system" fn SetThreadpoolThreadMaximum(pool: PTP_POOL, cthrd_max: u32) {
    if pool.is_null() || cthrd_max == 0 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return;
    }

    let id = pool as u64;
    let mut lock = THREADPOOLS.lock();
    if let Some(p) = lock.get_mut(&id) {
        p.max_threads = cthrd_max;
        if p.min_threads > cthrd_max {
            p.min_threads = cthrd_max;
        }
        if p.active_threads > cthrd_max {
            p.active_threads = cthrd_max;
        }
        win32_set_last_error(ERROR_SUCCESS);
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
    }
}

/// Initializes a callback environment with default settings.
pub unsafe extern "system" fn InitializeThreadpoolEnvironment(pcbe: *mut TP_CALLBACK_ENVIRON) {
    if !pcbe.is_null() {
        *pcbe = TP_CALLBACK_ENVIRON::default();
    }
}

/// Destroys and cleans up a callback environment.
pub unsafe extern "system" fn DestroyThreadpoolEnvironment(pcbe: *mut TP_CALLBACK_ENVIRON) {
    if !pcbe.is_null() {
        *pcbe = TP_CALLBACK_ENVIRON::default();
    }
}

/// Associates a threadpool with a callback environment.
pub unsafe extern "system" fn SetThreadpoolCallbackPool(
    pcbe: *mut TP_CALLBACK_ENVIRON,
    ptpp: PTP_POOL,
) {
    if !pcbe.is_null() {
        (*pcbe).Pool = ptpp;
    }
}

/// Associates a cleanup group with a callback environment.
pub unsafe extern "system" fn SetThreadpoolCallbackCleanupGroup(
    pcbe: *mut TP_CALLBACK_ENVIRON,
    ptpcg: PTP_CLEANUP_GROUP,
    pfnc: PTP_CLEANUP_GROUP_CANCEL_CALLBACK,
) {
    if !pcbe.is_null() {
        (*pcbe).CleanupGroup = ptpcg;
        (*pcbe).CleanupGroupCancelCallback = pfnc;
    }
}

/// Job execution priority for game and physics dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum JobPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Descriptor for a dispatched job item in the threadpool queue.
#[derive(Debug, Clone, Copy)]
pub struct JobDescriptor {
    pub job_id: u64,
    pub pool_id: u64,
    pub callback: usize,
    pub context: usize,
    pub work_handle: u64,
    pub priority: JobPriority,
    pub affinity_mask: u64,
}

/// Per-core worker thread queue lane supporting local enqueue and remote stealing.
pub struct WorkerThreadLane {
    pub worker_id: usize,
    pub local_queue: alloc::collections::VecDeque<JobDescriptor>,
    pub jobs_executed: u64,
    pub jobs_stolen: u64,
}

/// Work-stealing MPMC job scheduler coordinating multi-core worker threads.
pub struct WorkStealingJobScheduler {
    pub num_workers: usize,
    pub workers: Vec<Spinlock<WorkerThreadLane>>,
    pub global_queue: Spinlock<alloc::collections::VecDeque<JobDescriptor>>,
    pub total_dispatched: AtomicU64,
    pub total_completed: AtomicU64,
}

impl WorkStealingJobScheduler {
    /// Initializes scheduler with the specified worker thread count.
    pub fn new(num_workers: usize) -> Self {
        let mut workers = Vec::with_capacity(num_workers);
        for id in 0..num_workers {
            workers.push(Spinlock::new(WorkerThreadLane {
                worker_id: id,
                local_queue: alloc::collections::VecDeque::new(),
                jobs_executed: 0,
                jobs_stolen: 0,
            }));
        }

        Self {
            num_workers,
            workers,
            global_queue: Spinlock::new(alloc::collections::VecDeque::new()),
            total_dispatched: AtomicU64::new(0),
            total_completed: AtomicU64::new(0),
        }
    }

    /// Enqueues a job into a preferred worker lane or the global fallback queue.
    pub fn push_job(&self, worker_hint: usize, job: JobDescriptor) {
        self.total_dispatched.fetch_add(1, Ordering::SeqCst);
        if worker_hint < self.num_workers {
            self.workers[worker_hint].lock().local_queue.push_back(job);
        } else {
            self.global_queue.lock().push_back(job);
        }
    }

    /// Pops a job for a worker, attempting local queue, then global, then stealing.
    pub fn pop_job(&self, worker_id: usize) -> Option<JobDescriptor> {
        // 1. Try local worker queue first (LIFO order for cache locality).
        if let Some(job) = self.workers[worker_id].lock().local_queue.pop_back() {
            return Some(job);
        }

        // 2. Try global shared queue (FIFO order).
        if let Some(job) = self.global_queue.lock().pop_front() {
            return Some(job);
        }

        // 3. Work stealing: steal from other worker lanes (FIFO order from head).
        for offset in 1..self.num_workers {
            let victim = (worker_id + offset) % self.num_workers;
            let mut victim_lane = self.workers[victim].lock();
            if let Some(stolen) = victim_lane.local_queue.pop_front() {
                self.workers[worker_id].lock().jobs_stolen += 1;
                return Some(stolen);
            }
        }

        None
    }

    /// Executes the next available job for the designated worker thread.
    pub fn execute_next_job(&self, worker_id: usize) -> bool {
        if let Some(job) = self.pop_job(worker_id) {
            if job.callback != 0 {
                unsafe {
                    let cb: unsafe extern "system" fn(
                        PTP_CALLBACK_INSTANCE,
                        *mut c_void,
                        PTP_WORK,
                    ) = core::mem::transmute(job.callback);
                    cb(
                        core::ptr::null_mut(),
                        job.context as *mut c_void,
                        job.work_handle as PTP_WORK,
                    );
                }
            }
            self.workers[worker_id].lock().jobs_executed += 1;
            self.total_completed.fetch_add(1, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// Drains all currently queued jobs across all worker lanes.
    pub fn drain_all(&self) {
        let mut executed_any = true;
        while executed_any {
            executed_any = false;
            for wid in 0..self.num_workers {
                if self.execute_next_job(wid) {
                    executed_any = true;
                }
            }
        }
    }
}

pub static GLOBAL_SCHEDULER: Spinlock<Option<WorkStealingJobScheduler>> = Spinlock::new(None);

/// Ensures the global job scheduler is initialized.
pub fn get_or_init_scheduler() -> &'static Spinlock<Option<WorkStealingJobScheduler>> {
    let mut lock = GLOBAL_SCHEDULER.lock();
    if lock.is_none() {
        *lock = Some(WorkStealingJobScheduler::new(4));
    }
    drop(lock);
    &GLOBAL_SCHEDULER
}
