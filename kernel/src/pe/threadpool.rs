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

/// Internal descriptor for a Win32 threadpool work object.
pub struct ThreadpoolWorkInner {
    pub id: u64,
    pub pool_id: u64,
    pub cleanup_group_id: u64,
    pub pfn_callback: usize,
    pub context: usize,
    pub submitted_count: AtomicU32,
    pub completed_count: AtomicU32,
    pub cancelled_count: AtomicU32,
}

pub static THREADPOOL_WORKS: Spinlock<BTreeMap<u64, ThreadpoolWorkInner>> =
    Spinlock::new(BTreeMap::new());

/// Creates a new work object for asynchronous execution in the threadpool.
pub unsafe extern "system" fn CreateThreadpoolWork(
    pfnwk: PTP_WORK_CALLBACK,
    pv: *mut c_void,
    pcbe: *const TP_CALLBACK_ENVIRON,
) -> PTP_WORK {
    if pfnwk.is_none() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }

    let mut pool_id = 0u64;
    let mut cleanup_group_id = 0u64;

    if !pcbe.is_null() {
        let env = &*pcbe;
        if !env.Pool.is_null() {
            pool_id = env.Pool as u64;
        }
        if !env.CleanupGroup.is_null() {
            cleanup_group_id = env.CleanupGroup as u64;
        }
    }

    let id = NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst);
    let work = ThreadpoolWorkInner {
        id,
        pool_id,
        cleanup_group_id,
        pfn_callback: core::mem::transmute(pfnwk),
        context: pv as usize,
        submitted_count: AtomicU32::new(0),
        completed_count: AtomicU32::new(0),
        cancelled_count: AtomicU32::new(0),
    };

    THREADPOOL_WORKS.lock().insert(id, work);
    if cleanup_group_id != 0 {
        let mut cg_lock = CLEANUP_GROUPS.lock();
        if let Some(cg) = cg_lock.get_mut(&cleanup_group_id) {
            cg.works.push(id);
        }
    }
    win32_set_last_error(ERROR_SUCCESS);
    id as PTP_WORK
}

/// Submits a work object to the threadpool job queue.
pub unsafe extern "system" fn SubmitThreadpoolWork(pwk: PTP_WORK) {
    if pwk.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return;
    }

    let id = pwk as u64;
    let lock = THREADPOOL_WORKS.lock();
    let (pfn, ctx, pool) = match lock.get(&id) {
        Some(w) => {
            w.submitted_count.fetch_add(1, Ordering::SeqCst);
            (w.pfn_callback, w.context, w.pool_id)
        }
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return;
        }
    };
    drop(lock);

    let sched_lock = get_or_init_scheduler();
    if let Some(ref sched) = *sched_lock.lock() {
        let job = JobDescriptor {
            job_id: NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst),
            pool_id: pool,
            callback: pfn,
            context: ctx,
            work_handle: id,
            priority: JobPriority::Normal,
            affinity_mask: 0xFFFF,
        };
        let lane = (id as usize) % sched.num_workers;
        sched.push_job(lane, job);
    }
    win32_set_last_error(ERROR_SUCCESS);
}

/// Waits for outstanding callbacks of a work object to complete or cancel.
pub unsafe extern "system" fn WaitForThreadpoolWorkCallbacks(
    pwk: PTP_WORK,
    f_cancel_pending_callbacks: BOOL,
) {
    if pwk.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return;
    }

    let id = pwk as u64;
    let sched_lock = get_or_init_scheduler();
    if let Some(ref sched) = *sched_lock.lock() {
        if f_cancel_pending_callbacks != 0 {
            // Cancel pending jobs remaining in worker lanes.
            let mut cancelled = 0u32;
            for wid in 0..sched.num_workers {
                let mut lane = sched.workers[wid].lock();
                let initial_len = lane.local_queue.len();
                lane.local_queue.retain(|j| j.work_handle != id);
                cancelled += (initial_len - lane.local_queue.len()) as u32;
            }
            let mut gq = sched.global_queue.lock();
            let gq_len = gq.len();
            gq.retain(|j| j.work_handle != id);
            cancelled += (gq_len - gq.len()) as u32;
            drop(gq);

            let lock = THREADPOOL_WORKS.lock();
            if let Some(w) = lock.get(&id) {
                w.cancelled_count.fetch_add(cancelled, Ordering::SeqCst);
            }
        } else {
            // Drain and complete all pending work callbacks.
            sched.drain_all();
            let lock = THREADPOOL_WORKS.lock();
            if let Some(w) = lock.get(&id) {
                let sub = w.submitted_count.load(Ordering::SeqCst);
                w.completed_count.store(sub, Ordering::SeqCst);
            }
        }
    }
    win32_set_last_error(ERROR_SUCCESS);
}

/// Releases an allocated threadpool work object.
pub unsafe extern "system" fn CloseThreadpoolWork(pwk: PTP_WORK) {
    if pwk.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return;
    }

    let id = pwk as u64;
    if THREADPOOL_WORKS.lock().remove(&id).is_some() {
        win32_set_last_error(ERROR_SUCCESS);
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
    }
}

/// Internal descriptor for a Win32 threadpool timer object.
pub struct ThreadpoolTimerInner {
    pub id: u64,
    pub pool_id: u64,
    pub cleanup_group_id: u64,
    pub pfn_callback: usize,
    pub context: usize,
    pub due_time: u64,
    pub period_ms: u32,
    pub window_length_ms: u32,
    pub is_set: bool,
    pub fired_count: AtomicU32,
}

pub static THREADPOOL_TIMERS: Spinlock<BTreeMap<u64, ThreadpoolTimerInner>> =
    Spinlock::new(BTreeMap::new());

/// Creates a new timer object for scheduled callbacks in the threadpool.
pub unsafe extern "system" fn CreateThreadpoolTimer(
    pfnti: PTP_TIMER_CALLBACK,
    pv: *mut c_void,
    pcbe: *const TP_CALLBACK_ENVIRON,
) -> PTP_TIMER {
    if pfnti.is_none() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }

    let mut pool_id = 0u64;
    let mut cleanup_group_id = 0u64;

    if !pcbe.is_null() {
        let env = &*pcbe;
        if !env.Pool.is_null() {
            pool_id = env.Pool as u64;
        }
        if !env.CleanupGroup.is_null() {
            cleanup_group_id = env.CleanupGroup as u64;
        }
    }

    let id = NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst);
    let timer = ThreadpoolTimerInner {
        id,
        pool_id,
        cleanup_group_id,
        pfn_callback: core::mem::transmute(pfnti),
        context: pv as usize,
        due_time: 0,
        period_ms: 0,
        window_length_ms: 0,
        is_set: false,
        fired_count: AtomicU32::new(0),
    };

    THREADPOOL_TIMERS.lock().insert(id, timer);
    if cleanup_group_id != 0 {
        let mut cg_lock = CLEANUP_GROUPS.lock();
        if let Some(cg) = cg_lock.get_mut(&cleanup_group_id) {
            cg.timers.push(id);
        }
    }
    win32_set_last_error(ERROR_SUCCESS);
    id as PTP_TIMER
}

/// Sets due time, period, and window length for a threadpool timer.
pub unsafe extern "system" fn SetThreadpoolTimer(
    pti: PTP_TIMER,
    pft_due_time: *const FILETIME,
    ms_period: u32,
    ms_window_length: u32,
) {
    if pti.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return;
    }

    let id = pti as u64;
    let mut lock = THREADPOOL_TIMERS.lock();
    if let Some(t) = lock.get_mut(&id) {
        if pft_due_time.is_null() {
            // Passing NULL for pft_due_time disables the timer.
            t.is_set = false;
        } else {
            t.is_set = true;
            let ft = *pft_due_time;
            t.due_time = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
            t.period_ms = ms_period;
            t.window_length_ms = ms_window_length;
        }
        win32_set_last_error(ERROR_SUCCESS);
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
    }
}

/// Returns whether the threadpool timer is currently scheduled.
pub unsafe extern "system" fn IsThreadpoolTimerSet(pti: PTP_TIMER) -> BOOL {
    if pti.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return FALSE;
    }

    let id = pti as u64;
    let lock = THREADPOOL_TIMERS.lock();
    if let Some(t) = lock.get(&id) {
        win32_set_last_error(ERROR_SUCCESS);
        if t.is_set { TRUE } else { FALSE }
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        FALSE
    }
}

/// Waits for outstanding callbacks of a threadpool timer to finish or cancel.
pub unsafe extern "system" fn WaitForThreadpoolTimerCallbacks(
    pti: PTP_TIMER,
    f_cancel_pending_callbacks: BOOL,
) {
    if pti.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return;
    }

    let id = pti as u64;
    let lock = THREADPOOL_TIMERS.lock();
    if let Some(t) = lock.get(&id) {
        if f_cancel_pending_callbacks != 0 {
            // Cancel pending execution.
        } else if t.is_set && t.pfn_callback != 0 {
            // Dispatch pending timer callback once.
            let cb: unsafe extern "system" fn(
                PTP_CALLBACK_INSTANCE,
                *mut c_void,
                PTP_TIMER,
            ) = core::mem::transmute(t.pfn_callback);
            cb(
                core::ptr::null_mut(),
                t.context as *mut c_void,
                id as PTP_TIMER,
            );
            t.fired_count.fetch_add(1, Ordering::SeqCst);
        }
        win32_set_last_error(ERROR_SUCCESS);
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
    }
}

/// Closes an allocated threadpool timer object.
pub unsafe extern "system" fn CloseThreadpoolTimer(pti: PTP_TIMER) {
    if pti.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return;
    }

    let id = pti as u64;
    if THREADPOOL_TIMERS.lock().remove(&id).is_some() {
        win32_set_last_error(ERROR_SUCCESS);
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
    }
}

/// Internal descriptor for a Win32 threadpool cleanup group.
pub struct ThreadpoolCleanupGroupInner {
    pub id: u64,
    pub works: Vec<u64>,
    pub timers: Vec<u64>,
    pub closed: bool,
}

pub static CLEANUP_GROUPS: Spinlock<BTreeMap<u64, ThreadpoolCleanupGroupInner>> =
    Spinlock::new(BTreeMap::new());

/// Creates a new cleanup group to manage bulk teardown of threadpool objects.
pub unsafe extern "system" fn CreateThreadpoolCleanupGroup() -> PTP_CLEANUP_GROUP {
    let id = NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst);
    let group = ThreadpoolCleanupGroupInner {
        id,
        works: Vec::new(),
        timers: Vec::new(),
        closed: false,
    };

    CLEANUP_GROUPS.lock().insert(id, group);
    win32_set_last_error(ERROR_SUCCESS);
    id as PTP_CLEANUP_GROUP
}

/// Closes all members enrolled in the cleanup group and dispatches cancellations.
pub unsafe extern "system" fn CloseThreadpoolCleanupGroupMembers(
    ptpcg: PTP_CLEANUP_GROUP,
    f_cancel_pending_callbacks: BOOL,
    _pv_cleanup_context: *mut c_void,
) {
    if ptpcg.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return;
    }

    let id = ptpcg as u64;
    let mut cg_lock = CLEANUP_GROUPS.lock();
    let (works_to_clean, timers_to_clean) = match cg_lock.get_mut(&id) {
        Some(cg) => {
            let w = core::mem::take(&mut cg.works);
            let t = core::mem::take(&mut cg.timers);
            (w, t)
        }
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return;
        }
    };
    drop(cg_lock);

    // Teardown all enrolled work items.
    for wid in works_to_clean {
        WaitForThreadpoolWorkCallbacks(wid as PTP_WORK, f_cancel_pending_callbacks);
        CloseThreadpoolWork(wid as PTP_WORK);
    }

    // Teardown all enrolled timer items.
    for tid in timers_to_clean {
        WaitForThreadpoolTimerCallbacks(tid as PTP_TIMER, f_cancel_pending_callbacks);
        CloseThreadpoolTimer(tid as PTP_TIMER);
    }

    win32_set_last_error(ERROR_SUCCESS);
}

/// Closes an empty cleanup group object.
pub unsafe extern "system" fn CloseThreadpoolCleanupGroup(ptpcg: PTP_CLEANUP_GROUP) {
    if ptpcg.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return;
    }

    let id = ptpcg as u64;
    if CLEANUP_GROUPS.lock().remove(&id).is_some() {
        win32_set_last_error(ERROR_SUCCESS);
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
    }
}
