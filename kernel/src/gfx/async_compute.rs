//! Direct3D 12 Async Compute & High-Priority Graphics Queue Scheduler.
//! Implements multi-queue submission (Direct, Compute, Copy), cross-queue timeline
//! synchronization, wave saturation tracking, and overlap pacing optimization.

#![allow(dead_code, non_snake_case, non_camel_case_types, unused_imports)]

extern crate alloc;
use alloc::vec::Vec;

// Hardware queue type enumeration matching Direct3D 12 command list types.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum D3D12_COMMAND_LIST_TYPE {
    Direct = 0,
    Bundle = 1,
    Compute = 2,
    Copy = 3,
}

// Hardware command queue priority levels for graphics/compute preemption.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum D3D12_COMMAND_QUEUE_PRIORITY {
    Normal = 0,
    High = 100,
    GlobalRealtime = 10000,
}

// Queue execution flags configuring hardware queue behaviors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_COMMAND_QUEUE_FLAGS(pub u32);

impl D3D12_COMMAND_QUEUE_FLAGS {
    pub const NONE: Self = Self(0);
    pub const DISABLE_GPU_TIMEOUT: Self = Self(1 << 0);
}

// Direct3D 12 command queue creation descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_COMMAND_QUEUE_DESC {
    pub queue_type: D3D12_COMMAND_LIST_TYPE,
    pub priority: D3D12_COMMAND_QUEUE_PRIORITY,
    pub flags: D3D12_COMMAND_QUEUE_FLAGS,
    pub node_mask: u32,
}

impl D3D12_COMMAND_QUEUE_DESC {
    pub const fn new(
        queue_type: D3D12_COMMAND_LIST_TYPE,
        priority: D3D12_COMMAND_QUEUE_PRIORITY,
    ) -> Self {
        Self {
            queue_type,
            priority,
            flags: D3D12_COMMAND_QUEUE_FLAGS::NONE,
            node_mask: 0,
        }
    }
}

// Command batch queued for execution on a hardware queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBatch {
    pub batch_id: u64,
    pub queue_type: D3D12_COMMAND_LIST_TYPE,
    pub priority: D3D12_COMMAND_QUEUE_PRIORITY,
    pub command_count: usize,
    pub estimated_waves: u32,
    pub is_raster_heavy: bool,
}

impl CommandBatch {
    pub const fn new(
        batch_id: u64,
        queue_type: D3D12_COMMAND_LIST_TYPE,
        priority: D3D12_COMMAND_QUEUE_PRIORITY,
        command_count: usize,
        estimated_waves: u32,
        is_raster_heavy: bool,
    ) -> Self {
        Self {
            batch_id,
            queue_type,
            priority,
            command_count,
            estimated_waves,
            is_raster_heavy,
        }
    }
}

// Async compute scheduler error conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncComputeError {
    InvalidQueueType,
    FenceWaitTimeout,
    QueueEmpty,
    Preempted,
    SaturationExceeded,
}

use core::sync::atomic::{AtomicU64, Ordering};

// Monotonic 64-bit timeline semaphore for cross-queue GPU synchronization.
#[derive(Debug)]
pub struct TimelineSemaphore {
    pub semaphore_id: u64,
    pub current_value: AtomicU64,
}

impl TimelineSemaphore {
    // Creates a new timeline semaphore with an initial value.
    pub const fn new(semaphore_id: u64, initial_value: u64) -> Self {
        Self {
            semaphore_id,
            current_value: AtomicU64::new(initial_value),
        }
    }

    // Returns the current completed value of the timeline semaphore.
    #[inline(always)]
    pub fn get_value(&self) -> u64 {
        self.current_value.load(Ordering::Acquire)
    }

    // Monotonically advances the semaphore value to signal completion.
    pub fn signal(&self, value: u64) -> Result<(), AsyncComputeError> {
        let mut current = self.get_value();
        loop {
            if value <= current {
                return Err(AsyncComputeError::Preempted);
            }
            match self.current_value.compare_exchange_weak(
                current,
                value,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => current = actual,
            }
        }
    }
}

// Synchronization operation inserted into hardware queue stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuQueueSyncOp {
    Signal { semaphore_id: u64, value: u64 },
    Wait { semaphore_id: u64, value: u64 },
}

// Central coordinator managing timeline semaphores and cross-queue dependency resolution.
#[derive(Debug, Default)]
pub struct CrossQueueSyncHub {
    pub semaphores: Vec<TimelineSemaphore>,
    pub direct_waits: Vec<(u64, u64)>,
    pub compute_waits: Vec<(u64, u64)>,
    pub copy_waits: Vec<(u64, u64)>,
}

impl CrossQueueSyncHub {
    // Creates a new cross-queue synchronization hub.
    pub fn new() -> Self {
        Self::default()
    }

    // Registers a new timeline semaphore in the hub.
    pub fn register_semaphore(&mut self, semaphore_id: u64, initial_value: u64) {
        self.semaphores.push(TimelineSemaphore::new(semaphore_id, initial_value));
    }

    // Retrieves current signaled value for a given semaphore.
    pub fn get_semaphore_value(&self, semaphore_id: u64) -> Option<u64> {
        self.semaphores
            .iter()
            .find(|s| s.semaphore_id == semaphore_id)
            .map(|s| s.get_value())
    }

    // Direct3D 12 ID3D12CommandQueue::Signal: records a timeline fence advancement.
    pub fn queue_signal(
        &mut self,
        _queue_type: D3D12_COMMAND_LIST_TYPE,
        semaphore_id: u64,
        value: u64,
    ) -> Result<(), AsyncComputeError> {
        let sem = self
            .semaphores
            .iter()
            .find(|s| s.semaphore_id == semaphore_id)
            .ok_or(AsyncComputeError::InvalidQueueType)?;
        sem.signal(value)
    }

    // Direct3D 12 ID3D12CommandQueue::Wait: queues a GPU-side wait on the target queue.
    pub fn queue_wait(
        &mut self,
        queue_type: D3D12_COMMAND_LIST_TYPE,
        semaphore_id: u64,
        value: u64,
    ) {
        match queue_type {
            D3D12_COMMAND_LIST_TYPE::Direct => self.direct_waits.push((semaphore_id, value)),
            D3D12_COMMAND_LIST_TYPE::Compute => self.compute_waits.push((semaphore_id, value)),
            D3D12_COMMAND_LIST_TYPE::Copy => self.copy_waits.push((semaphore_id, value)),
            _ => {}
        }
    }

    // Checks whether all GPU-side waits on the specified queue have been satisfied.
    pub fn can_queue_proceed(&self, queue_type: D3D12_COMMAND_LIST_TYPE) -> bool {
        let wait_list = match queue_type {
            D3D12_COMMAND_LIST_TYPE::Direct => &self.direct_waits,
            D3D12_COMMAND_LIST_TYPE::Compute => &self.compute_waits,
            D3D12_COMMAND_LIST_TYPE::Copy => &self.copy_waits,
            _ => return true,
        };

        for &(sem_id, req_val) in wait_list {
            if let Some(val) = self.get_semaphore_value(sem_id) {
                if val < req_val {
                    return false;
                }
            } else {
                return false;
            }
        }
        true
    }

    // Purges satisfied dependencies for the queue.
    pub fn resolve_completed_waits(&mut self, queue_type: D3D12_COMMAND_LIST_TYPE) {
        let semaphores = &self.semaphores;
        let get_val = |id: u64| -> u64 {
            semaphores
                .iter()
                .find(|s| s.semaphore_id == id)
                .map(|s| s.get_value())
                .unwrap_or(0)
        };

        match queue_type {
            D3D12_COMMAND_LIST_TYPE::Direct => self.direct_waits.retain(|&(id, val)| get_val(id) < val),
            D3D12_COMMAND_LIST_TYPE::Compute => self.compute_waits.retain(|&(id, val)| get_val(id) < val),
            D3D12_COMMAND_LIST_TYPE::Copy => self.copy_waits.retain(|&(id, val)| get_val(id) < val),
            _ => {}
        }
    }
}

// Tracks concurrent GPU wavefront saturation and L2 cache bandwidth pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputeWaveSaturationTracker {
    pub max_hardware_waves: u32,
    pub active_graphics_waves: u32,
    pub active_compute_waves: u32,
    pub l2_bandwidth_pressure_pct: u32,
    pub is_raster_heavy_active: bool,
}

impl ComputeWaveSaturationTracker {
    // Constructs a new wave saturation tracker with given hardware SIMD wave capacity.
    pub const fn new(max_hardware_waves: u32) -> Self {
        Self {
            max_hardware_waves,
            active_graphics_waves: 0,
            active_compute_waves: 0,
            l2_bandwidth_pressure_pct: 0,
            is_raster_heavy_active: false,
        }
    }

    // Updates active graphics rasterization workload state.
    pub fn update_graphics_load(&mut self, waves: u32, is_raster_heavy: bool) {
        self.active_graphics_waves = waves;
        self.is_raster_heavy_active = is_raster_heavy;
        self.recompute_l2_pressure();
    }

    // Updates active async compute workload state.
    pub fn update_compute_load(&mut self, waves: u32) {
        self.active_compute_waves = waves;
        self.recompute_l2_pressure();
    }

    // Recomputes L2 cache pressure estimate based on concurrent workload.
    fn recompute_l2_pressure(&mut self) {
        let total_waves = self.active_graphics_waves.saturating_add(self.active_compute_waves);
        let base_pressure = if self.max_hardware_waves > 0 {
            (total_waves.saturating_mul(100)) / self.max_hardware_waves
        } else {
            0
        };
        self.l2_bandwidth_pressure_pct = if self.is_raster_heavy_active {
            base_pressure.saturating_add(25).min(100)
        } else {
            base_pressure.min(100)
        };
    }

    // Returns total currently active SIMD waves across graphics and compute queues.
    #[inline(always)]
    pub const fn total_active_waves(&self) -> u32 {
        self.active_graphics_waves.saturating_add(self.active_compute_waves)
    }

    // Current GPU SIMD occupancy percentage (0..100%).
    #[inline(always)]
    pub fn occupancy_pct(&self) -> u32 {
        if self.max_hardware_waves > 0 {
            (self.total_active_waves().saturating_mul(100)) / self.max_hardware_waves
        } else {
            0
        }
    }
}

// Scheduling decision for async compute dispatches to prevent cache thrashing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlapDecision {
    // Run at full rate alongside graphics without restriction.
    ExecuteImmediate,
    // Restrict compute dispatch waves to avoid saturating SIMD units.
    ThrottleWaves { max_waves: u32 },
    // Delay compute dispatch until raster-heavy graphics pass completes.
    YieldUntilRasterDone,
}

// Overlap optimizer pacing compute dispatches during heavy rasterization phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsyncComputeOverlapOptimizer {
    pub wave_saturation_threshold_pct: u32,
    pub l2_pressure_threshold_pct: u32,
    pub total_throttled_dispatches: u64,
    pub total_yielded_dispatches: u64,
    pub total_immediate_dispatches: u64,
}

impl Default for AsyncComputeOverlapOptimizer {
    fn default() -> Self {
        Self {
            wave_saturation_threshold_pct: 85,
            l2_pressure_threshold_pct: 80,
            total_throttled_dispatches: 0,
            total_yielded_dispatches: 0,
            total_immediate_dispatches: 0,
        }
    }
}

impl AsyncComputeOverlapOptimizer {
    // Creates a new overlap optimizer with default thresholds.
    pub fn new() -> Self {
        Self::default()
    }

    // Evaluates optimal dispatch strategy based on live saturation metrics.
    pub fn evaluate_dispatch(
        &mut self,
        tracker: &ComputeWaveSaturationTracker,
        requested_waves: u32,
    ) -> OverlapDecision {
        if tracker.is_raster_heavy_active && tracker.l2_bandwidth_pressure_pct >= self.l2_pressure_threshold_pct {
            self.total_yielded_dispatches = self.total_yielded_dispatches.saturating_add(1);
            return OverlapDecision::YieldUntilRasterDone;
        }

        let prospective_total = tracker.total_active_waves().saturating_add(requested_waves);
        let prospective_occupancy = if tracker.max_hardware_waves > 0 {
            (prospective_total.saturating_mul(100)) / tracker.max_hardware_waves
        } else {
            0
        };

        if prospective_occupancy > self.wave_saturation_threshold_pct {
            let max_allowed = (tracker.max_hardware_waves.saturating_mul(self.wave_saturation_threshold_pct) / 100)
                .saturating_sub(tracker.active_graphics_waves);
            self.total_throttled_dispatches = self.total_throttled_dispatches.saturating_add(1);
            OverlapDecision::ThrottleWaves { max_waves: max_allowed.max(32) }
        } else {
            self.total_immediate_dispatches = self.total_immediate_dispatches.saturating_add(1);
            OverlapDecision::ExecuteImmediate
        }
    }
}

// Execution state for an individual hardware queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareQueueState {
    pub desc: D3D12_COMMAND_QUEUE_DESC,
    pub pending_batches: Vec<CommandBatch>,
    pub in_flight_batches: Vec<CommandBatch>,
    pub total_executed_batches: u64,
}

impl HardwareQueueState {
    pub const fn new(desc: D3D12_COMMAND_QUEUE_DESC) -> Self {
        Self {
            desc,
            pending_batches: Vec::new(),
            in_flight_batches: Vec::new(),
            total_executed_batches: 0,
        }
    }

    // Pushes a new command batch to the queue's pending list.
    pub fn push_batch(&mut self, batch: CommandBatch) {
        self.pending_batches.push(batch);
    }
}

// Multi-queue scheduler orchestrating Direct, Compute, and Copy queues with priority preemption.
#[derive(Debug)]
pub struct MultiQueueScheduler {
    pub direct_queue: HardwareQueueState,
    pub compute_queue: HardwareQueueState,
    pub copy_queue: HardwareQueueState,
    pub sync_hub: CrossQueueSyncHub,
    pub saturation_tracker: ComputeWaveSaturationTracker,
    pub overlap_optimizer: AsyncComputeOverlapOptimizer,
    pub preemption_events_count: u64,
}

impl MultiQueueScheduler {
    // Creates a new multi-queue scheduler with standard hardware queue configurations.
    pub fn new(max_hardware_waves: u32) -> Self {
        Self {
            direct_queue: HardwareQueueState::new(D3D12_COMMAND_QUEUE_DESC::new(
                D3D12_COMMAND_LIST_TYPE::Direct,
                D3D12_COMMAND_QUEUE_PRIORITY::Normal,
            )),
            compute_queue: HardwareQueueState::new(D3D12_COMMAND_QUEUE_DESC::new(
                D3D12_COMMAND_LIST_TYPE::Compute,
                D3D12_COMMAND_QUEUE_PRIORITY::Normal,
            )),
            copy_queue: HardwareQueueState::new(D3D12_COMMAND_QUEUE_DESC::new(
                D3D12_COMMAND_LIST_TYPE::Copy,
                D3D12_COMMAND_QUEUE_PRIORITY::Normal,
            )),
            sync_hub: CrossQueueSyncHub::new(),
            saturation_tracker: ComputeWaveSaturationTracker::new(max_hardware_waves),
            overlap_optimizer: AsyncComputeOverlapOptimizer::new(),
            preemption_events_count: 0,
        }
    }

    // Submits a command batch to the specified hardware queue.
    pub fn submit_batch(&mut self, batch: CommandBatch) {
        match batch.queue_type {
            D3D12_COMMAND_LIST_TYPE::Direct => self.direct_queue.push_batch(batch),
            D3D12_COMMAND_LIST_TYPE::Compute => self.compute_queue.push_batch(batch),
            D3D12_COMMAND_LIST_TYPE::Copy => self.copy_queue.push_batch(batch),
            _ => {}
        }
    }

    // Sets execution priority for a specific hardware queue.
    pub fn set_queue_priority(
        &mut self,
        queue_type: D3D12_COMMAND_LIST_TYPE,
        priority: D3D12_COMMAND_QUEUE_PRIORITY,
    ) {
        match queue_type {
            D3D12_COMMAND_LIST_TYPE::Direct => self.direct_queue.desc.priority = priority,
            D3D12_COMMAND_LIST_TYPE::Compute => self.compute_queue.desc.priority = priority,
            D3D12_COMMAND_LIST_TYPE::Copy => self.copy_queue.desc.priority = priority,
            _ => {}
        }
    }

    // Selects and schedules the next eligible command batch based on priority and overlap rules.
    pub fn schedule_next(&mut self) -> Option<(D3D12_COMMAND_LIST_TYPE, CommandBatch, OverlapDecision)> {
        let direct_ready = !self.direct_queue.pending_batches.is_empty()
            && self.sync_hub.can_queue_proceed(D3D12_COMMAND_LIST_TYPE::Direct);
        let compute_ready = !self.compute_queue.pending_batches.is_empty()
            && self.sync_hub.can_queue_proceed(D3D12_COMMAND_LIST_TYPE::Compute);
        let copy_ready = !self.copy_queue.pending_batches.is_empty()
            && self.sync_hub.can_queue_proceed(D3D12_COMMAND_LIST_TYPE::Copy);

        let mut candidates = Vec::new();
        if direct_ready {
            candidates.push((self.direct_queue.desc.priority, D3D12_COMMAND_LIST_TYPE::Direct));
        }
        if compute_ready {
            candidates.push((self.compute_queue.desc.priority, D3D12_COMMAND_LIST_TYPE::Compute));
        }
        if copy_ready {
            candidates.push((self.copy_queue.desc.priority, D3D12_COMMAND_LIST_TYPE::Copy));
        }

        candidates.sort_by(|a, b| b.0.cmp(&a.0));

        for &(_prio, q_type) in &candidates {
            match q_type {
                D3D12_COMMAND_LIST_TYPE::Direct => {
                    let batch = self.direct_queue.pending_batches.remove(0);
                    self.saturation_tracker.update_graphics_load(batch.estimated_waves, batch.is_raster_heavy);
                    self.direct_queue.in_flight_batches.push(batch.clone());
                    return Some((D3D12_COMMAND_LIST_TYPE::Direct, batch, OverlapDecision::ExecuteImmediate));
                }
                D3D12_COMMAND_LIST_TYPE::Compute => {
                    let batch = &self.compute_queue.pending_batches[0];
                    let decision = self.overlap_optimizer.evaluate_dispatch(&self.saturation_tracker, batch.estimated_waves);
                    if decision == OverlapDecision::YieldUntilRasterDone {
                        continue;
                    }
                    let batch = self.compute_queue.pending_batches.remove(0);
                    let waves = match decision {
                        OverlapDecision::ThrottleWaves { max_waves } => max_waves.min(batch.estimated_waves),
                        _ => batch.estimated_waves,
                    };
                    self.saturation_tracker.update_compute_load(waves);
                    self.compute_queue.in_flight_batches.push(batch.clone());
                    return Some((D3D12_COMMAND_LIST_TYPE::Compute, batch, decision));
                }
                D3D12_COMMAND_LIST_TYPE::Copy => {
                    let batch = self.copy_queue.pending_batches.remove(0);
                    self.copy_queue.in_flight_batches.push(batch.clone());
                    return Some((D3D12_COMMAND_LIST_TYPE::Copy, batch, OverlapDecision::ExecuteImmediate));
                }
                _ => {}
            }
        }

        None
    }

    // Completes an in-flight batch and updates synchronization dependencies.
    pub fn complete_batch(
        &mut self,
        queue_type: D3D12_COMMAND_LIST_TYPE,
        batch_id: u64,
        signal_op: Option<(u64, u64)>,
    ) -> bool {
        let queue = match queue_type {
            D3D12_COMMAND_LIST_TYPE::Direct => &mut self.direct_queue,
            D3D12_COMMAND_LIST_TYPE::Compute => &mut self.compute_queue,
            D3D12_COMMAND_LIST_TYPE::Copy => &mut self.copy_queue,
            _ => return false,
        };

        let pos = queue.in_flight_batches.iter().position(|b| b.batch_id == batch_id);
        if let Some(idx) = pos {
            queue.in_flight_batches.remove(idx);
            queue.total_executed_batches = queue.total_executed_batches.saturating_add(1);

            if queue_type == D3D12_COMMAND_LIST_TYPE::Direct {
                self.saturation_tracker.update_graphics_load(0, false);
            } else if queue_type == D3D12_COMMAND_LIST_TYPE::Compute {
                self.saturation_tracker.update_compute_load(0);
            }

            if let Some((sem_id, val)) = signal_op {
                let _ = self.sync_hub.queue_signal(queue_type, sem_id, val);
                self.sync_hub.resolve_completed_waits(D3D12_COMMAND_LIST_TYPE::Direct);
                self.sync_hub.resolve_completed_waits(D3D12_COMMAND_LIST_TYPE::Compute);
                self.sync_hub.resolve_completed_waits(D3D12_COMMAND_LIST_TYPE::Copy);
            }
            true
        } else {
            false
        }
    }
}

// Comprehensive self-test suite for Direct3D 12 Async Compute & High-Priority Graphics Queue Scheduler.
pub fn test_d3d12_async_compute_subsystem() -> bool {
    // 1. Verify priority preemption scheduling
    let mut scheduler = MultiQueueScheduler::new(1024);

    scheduler.set_queue_priority(
        D3D12_COMMAND_LIST_TYPE::Direct,
        D3D12_COMMAND_QUEUE_PRIORITY::Normal,
    );
    scheduler.set_queue_priority(
        D3D12_COMMAND_LIST_TYPE::Compute,
        D3D12_COMMAND_QUEUE_PRIORITY::High,
    );

    // Direct batch submitted first
    scheduler.submit_batch(CommandBatch::new(
        1,
        D3D12_COMMAND_LIST_TYPE::Direct,
        D3D12_COMMAND_QUEUE_PRIORITY::Normal,
        100,
        256,
        false,
    ));

    // High priority compute batch submitted second
    scheduler.submit_batch(CommandBatch::new(
        2,
        D3D12_COMMAND_LIST_TYPE::Compute,
        D3D12_COMMAND_QUEUE_PRIORITY::High,
        50,
        256,
        false,
    ));

    // High priority compute batch must preempt normal priority direct batch
    let scheduled_first = scheduler.schedule_next();
    match scheduled_first {
        Some((q_type, batch, _)) => {
            if q_type != D3D12_COMMAND_LIST_TYPE::Compute || batch.batch_id != 2 {
                return false;
            }
        }
        None => return false,
    }

    // Complete the compute batch
    if !scheduler.complete_batch(D3D12_COMMAND_LIST_TYPE::Compute, 2, None) {
        return false;
    }

    // Now Direct batch can be scheduled
    let scheduled_second = scheduler.schedule_next();
    match scheduled_second {
        Some((q_type, batch, _)) => {
            if q_type != D3D12_COMMAND_LIST_TYPE::Direct || batch.batch_id != 1 {
                return false;
            }
        }
        None => return false,
    }

    if !scheduler.complete_batch(D3D12_COMMAND_LIST_TYPE::Direct, 1, None) {
        return false;
    }

    // 2. Verify cross-queue timeline semaphore synchronization
    let sem_id = 42;
    scheduler.sync_hub.register_semaphore(sem_id, 0);

    // Direct queue waits on semaphore 42 reaching value 1
    scheduler.sync_hub.queue_wait(D3D12_COMMAND_LIST_TYPE::Direct, sem_id, 1);

    scheduler.submit_batch(CommandBatch::new(
        10,
        D3D12_COMMAND_LIST_TYPE::Direct,
        D3D12_COMMAND_QUEUE_PRIORITY::Normal,
        100,
        256,
        false,
    ));

    // Direct queue is stalled by unsatisfied timeline wait
    if scheduler.sync_hub.can_queue_proceed(D3D12_COMMAND_LIST_TYPE::Direct) {
        return false;
    }

    // Submit compute batch 11 that produces data and signals semaphore 42
    scheduler.submit_batch(CommandBatch::new(
        11,
        D3D12_COMMAND_LIST_TYPE::Compute,
        D3D12_COMMAND_QUEUE_PRIORITY::Normal,
        80,
        256,
        false,
    ));

    // Direct cannot proceed, so Compute batch 11 must be scheduled
    let scheduled_compute = scheduler.schedule_next();
    match scheduled_compute {
        Some((q_type, batch, _)) => {
            if q_type != D3D12_COMMAND_LIST_TYPE::Compute || batch.batch_id != 11 {
                return false;
            }
        }
        None => return false,
    }

    // Complete compute batch 11 and signal semaphore 42 with value 1
    if !scheduler.complete_batch(D3D12_COMMAND_LIST_TYPE::Compute, 11, Some((sem_id, 1))) {
        return false;
    }

    // Now Direct queue dependency is satisfied
    if !scheduler.sync_hub.can_queue_proceed(D3D12_COMMAND_LIST_TYPE::Direct) {
        return false;
    }

    let scheduled_direct = scheduler.schedule_next();
    match scheduled_direct {
        Some((q_type, batch, _)) => {
            if q_type != D3D12_COMMAND_LIST_TYPE::Direct || batch.batch_id != 10 {
                return false;
            }
        }
        None => return false,
    }

    if !scheduler.complete_batch(D3D12_COMMAND_LIST_TYPE::Direct, 10, None) {
        return false;
    }

    // 3. Verify compute overlap pacing & saturation optimization
    let mut tracker = ComputeWaveSaturationTracker::new(1024);
    let mut optimizer = AsyncComputeOverlapOptimizer::new();

    // Heavy rasterization pass (700 waves, raster-heavy flag = true)
    tracker.update_graphics_load(700, true);
    if tracker.l2_bandwidth_pressure_pct < 80 {
        return false;
    }

    // High L2 pressure during raster-heavy pass triggers YieldUntilRasterDone
    let yield_decision = optimizer.evaluate_dispatch(&tracker, 512);
    if yield_decision != OverlapDecision::YieldUntilRasterDone {
        return false;
    }

    // Moderate non-raster graphics pass (200 waves, raster-heavy flag = false)
    tracker.update_graphics_load(200, false);
    let immediate_decision = optimizer.evaluate_dispatch(&tracker, 512);
    if immediate_decision != OverlapDecision::ExecuteImmediate {
        return false;
    }

    // Saturated condition (600 graphics waves + 400 compute waves = 1000/1024 > 85%)
    tracker.update_graphics_load(600, false);
    let throttle_decision = optimizer.evaluate_dispatch(&tracker, 400);
    match throttle_decision {
        OverlapDecision::ThrottleWaves { max_waves } => {
            if max_waves > 400 {
                return false;
            }
        }
        _ => return false,
    }

    crate::serial_write_str("[KNT-D3D12-ASYNC-COMPUTE-PASS] Direct3D 12 async compute and high-priority graphics queue scheduler verified\r\n");
    true
}
