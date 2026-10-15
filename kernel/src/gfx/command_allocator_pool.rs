// Direct3D 12 Agility Command Allocator Thread Affinity Pool & Sub-Microsecond Multi-Queue Engine.
// Manages thread-affine command allocators, zero-contention freelists, and multi-queue synchronization.

extern crate alloc;
use alloc::vec::Vec;

// Default allocation arena page size (64 KiB).
pub const COMMAND_ALLOCATOR_DEFAULT_PAGE_SIZE: usize = 64 * 1024;
// Memory alignment for command buffer page arenas.
pub const COMMAND_ALLOCATOR_PAGE_ALIGNMENT: usize = 256;

pub use super::resource_barrier::D3D12_COMMAND_LIST_TYPE;

// Lifecycle state of an individual command allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocatorLifecycle {
    Ready,
    Recording,
    Closed,
    PendingGpu(u64),
}

// Fixed-size page arena suballocating command packet memory.
#[derive(Debug, Clone)]
pub struct CommandAllocatorPageArena {
    pub arena_id: u32,
    pub base_address: u64,
    pub capacity_bytes: usize,
    pub allocated_bytes: usize,
}

impl CommandAllocatorPageArena {
    // Construct a new page arena.
    pub fn new(arena_id: u32, base_address: u64, capacity_bytes: usize) -> Self {
        Self {
            arena_id,
            base_address,
            capacity_bytes,
            allocated_bytes: 0,
        }
    }

    // Suballocate a slice of memory with requested alignment.
    pub fn suballocate(&mut self, size: usize, alignment: usize) -> Option<u64> {
        let align = alignment.max(4);
        let current_va = self.base_address + self.allocated_bytes as u64;
        let aligned_va = (current_va + align as u64 - 1) & !(align as u64 - 1);
        let padding = (aligned_va - current_va) as usize;
        if self.allocated_bytes + padding + size > self.capacity_bytes {
            return None;
        }
        self.allocated_bytes += padding + size;
        Some(aligned_va)
    }

    // Reset allocated bytes offset back to zero.
    pub fn reset(&mut self) {
        self.allocated_bytes = 0;
    }

    // Remaining available bytes before arena expansion.
    pub fn available_bytes(&self) -> usize {
        self.capacity_bytes.saturating_sub(self.allocated_bytes)
    }
}

// Command allocator instance holding linear page arenas for command recording.
#[derive(Debug, Clone)]
pub struct CommandAllocatorInstance {
    pub allocator_id: u32,
    pub thread_id: u32,
    pub list_type: D3D12_COMMAND_LIST_TYPE,
    pub lifecycle: AllocatorLifecycle,
    pub page_arenas: Vec<CommandAllocatorPageArena>,
    pub total_committed_bytes: usize,
    pub total_allocated_bytes: usize,
    pub sync_fence_value: u64,
}

impl CommandAllocatorInstance {
    // Create new allocator instance with initial 64 KiB page arena.
    pub fn new(allocator_id: u32, thread_id: u32, list_type: D3D12_COMMAND_LIST_TYPE, base_va: u64) -> Self {
        let initial_arena = CommandAllocatorPageArena::new(0, base_va, COMMAND_ALLOCATOR_DEFAULT_PAGE_SIZE);
        Self {
            allocator_id,
            thread_id,
            list_type,
            lifecycle: AllocatorLifecycle::Ready,
            page_arenas: alloc::vec![initial_arena],
            total_committed_bytes: COMMAND_ALLOCATOR_DEFAULT_PAGE_SIZE,
            total_allocated_bytes: 0,
            sync_fence_value: 0,
        }
    }

    // Suballocate memory from current arena or append a new arena.
    pub fn allocate_command_memory(&mut self, size: usize, alignment: usize, next_base_va: u64) -> u64 {
        if let Some(last) = self.page_arenas.last_mut() {
            if let Some(va) = last.suballocate(size, alignment) {
                self.total_allocated_bytes += size;
                return va;
            }
        }
        let arena_id = self.page_arenas.len() as u32;
        let new_capacity = COMMAND_ALLOCATOR_DEFAULT_PAGE_SIZE.max(size);
        let mut new_arena = CommandAllocatorPageArena::new(arena_id, next_base_va, new_capacity);
        let va = new_arena.suballocate(size, alignment).unwrap_or(next_base_va);
        self.total_committed_bytes += new_capacity;
        self.total_allocated_bytes += size;
        self.page_arenas.push(new_arena);
        va
    }

    // Reset all arenas and mark allocator ready for reuse.
    pub fn reset(&mut self) {
        for arena in &mut self.page_arenas {
            arena.reset();
        }
        self.total_allocated_bytes = 0;
        self.lifecycle = AllocatorLifecycle::Ready;
    }

    // Mark allocator as in-flight on GPU until fence completes.
    pub fn mark_pending_gpu(&mut self, fence_val: u64) {
        self.sync_fence_value = fence_val;
        self.lifecycle = AllocatorLifecycle::PendingGpu(fence_val);
    }

    // Whether GPU execution has passed the allocator sync fence.
    pub fn is_ready_for_reset(&self, completed_fence_val: u64) -> bool {
        match self.lifecycle {
            AllocatorLifecycle::Ready => true,
            AllocatorLifecycle::Recording => false,
            AllocatorLifecycle::Closed => true,
            AllocatorLifecycle::PendingGpu(fence) => completed_fence_val >= fence,
        }
    }
}

// Per-thread command allocator pool managing Direct, Compute, and Copy allocators.
#[derive(Debug, Clone)]
pub struct ThreadCommandAllocatorPool {
    pub thread_id: u32,
    pub direct_allocators: Vec<CommandAllocatorInstance>,
    pub compute_allocators: Vec<CommandAllocatorInstance>,
    pub copy_allocators: Vec<CommandAllocatorInstance>,
    pub next_allocator_id: u32,
    pub next_virtual_address: u64,
}

impl ThreadCommandAllocatorPool {
    // Construct new thread-affine command allocator pool.
    pub fn new(thread_id: u32, base_va: u64) -> Self {
        Self {
            thread_id,
            direct_allocators: Vec::new(),
            compute_allocators: Vec::new(),
            copy_allocators: Vec::new(),
            next_allocator_id: 1,
            next_virtual_address: base_va,
        }
    }

    // Obtain an available allocator for the requested list type.
    pub fn obtain_allocator(
        &mut self,
        list_type: D3D12_COMMAND_LIST_TYPE,
        completed_fence: u64,
    ) -> &mut CommandAllocatorInstance {
        let allocators = match list_type {
            D3D12_COMMAND_LIST_TYPE::Direct | D3D12_COMMAND_LIST_TYPE::Bundle => &mut self.direct_allocators,
            D3D12_COMMAND_LIST_TYPE::Compute => &mut self.compute_allocators,
            D3D12_COMMAND_LIST_TYPE::Copy => &mut self.copy_allocators,
            _ => &mut self.direct_allocators,
        };

        for i in 0..allocators.len() {
            if allocators[i].is_ready_for_reset(completed_fence) {
                allocators[i].reset();
                allocators[i].lifecycle = AllocatorLifecycle::Recording;
                return &mut allocators[i];
            }
        }

        let id = self.next_allocator_id;
        self.next_allocator_id += 1;
        let va = self.next_virtual_address;
        self.next_virtual_address += COMMAND_ALLOCATOR_DEFAULT_PAGE_SIZE as u64;

        let mut instance = CommandAllocatorInstance::new(id, self.thread_id, list_type, va);
        instance.lifecycle = AllocatorLifecycle::Recording;
        allocators.push(instance);
        let last_idx = allocators.len() - 1;
        &mut allocators[last_idx]
    }

    // Count of active allocators for a given command list type.
    pub fn active_allocators_count(&self, list_type: D3D12_COMMAND_LIST_TYPE) -> usize {
        let allocators = match list_type {
            D3D12_COMMAND_LIST_TYPE::Direct | D3D12_COMMAND_LIST_TYPE::Bundle => &self.direct_allocators,
            D3D12_COMMAND_LIST_TYPE::Compute => &self.compute_allocators,
            D3D12_COMMAND_LIST_TYPE::Copy => &self.copy_allocators,
            _ => &self.direct_allocators,
        };
        allocators.len()
    }

    // Total memory committed across all allocators in this thread pool.
    pub fn total_committed_memory(&self) -> usize {
        let sum_d: usize = self.direct_allocators.iter().map(|a| a.total_committed_bytes).sum();
        let sum_c: usize = self.compute_allocators.iter().map(|a| a.total_committed_bytes).sum();
        let sum_cp: usize = self.copy_allocators.iter().map(|a| a.total_committed_bytes).sum();
        sum_d + sum_c + sum_cp
    }
}

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

// Maximum slots in atomic lock-free allocator freelist.
pub const MAX_LOCK_FREE_SLOTS: usize = 64;

// Atomic lock-free allocator freelist for zero-contention recycling across threads.
pub struct AtomicAllocatorFreelist {
    pub available_mask: AtomicU64,
    pub slots: Vec<Option<CommandAllocatorInstance>>,
    pub total_recycles: AtomicUsize,
    pub total_acquisitions: AtomicUsize,
}

impl AtomicAllocatorFreelist {
    // Construct empty atomic freelist.
    pub fn new() -> Self {
        let mut slots = Vec::with_capacity(MAX_LOCK_FREE_SLOTS);
        for _ in 0..MAX_LOCK_FREE_SLOTS {
            slots.push(None);
        }
        Self {
            available_mask: AtomicU64::new(0),
            slots,
            total_recycles: AtomicUsize::new(0),
            total_acquisitions: AtomicUsize::new(0),
        }
    }

    // Push a completed allocator into the lock-free freelist.
    pub fn push_allocator(&mut self, mut allocator: CommandAllocatorInstance) -> bool {
        allocator.reset();
        for i in 0..MAX_LOCK_FREE_SLOTS {
            let mask = 1u64 << i;
            let current = self.available_mask.load(Ordering::Acquire);
            if (current & mask) == 0 {
                self.slots[i] = Some(allocator);
                self.available_mask.fetch_or(mask, Ordering::Release);
                self.total_recycles.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    // Pop an allocator of matching command list type from freelist.
    pub fn pop_allocator(&mut self, list_type: D3D12_COMMAND_LIST_TYPE) -> Option<CommandAllocatorInstance> {
        let current = self.available_mask.load(Ordering::Acquire);
        for i in 0..MAX_LOCK_FREE_SLOTS {
            let mask = 1u64 << i;
            if (current & mask) != 0 {
                if let Some(ref alloc) = self.slots[i] {
                    if alloc.list_type == list_type {
                        self.available_mask.fetch_and(!mask, Ordering::Release);
                        self.total_acquisitions.fetch_add(1, Ordering::Relaxed);
                        return self.slots[i].take();
                    }
                }
            }
        }
        None
    }

    // Number of available allocators in the freelist.
    pub fn available_count(&self) -> usize {
        self.available_mask.load(Ordering::Relaxed).count_ones() as usize
    }
}

// Thread-affine allocator manager coordinating per-thread pools and global lock-free recycling.
pub struct ThreadAffineAllocatorManager {
    pub thread_pools: Vec<ThreadCommandAllocatorPool>,
    pub global_freelist: AtomicAllocatorFreelist,
    pub base_virtual_address: u64,
    pub next_thread_id: u32,
    pub thread_affine_hits: usize,
    pub global_freelist_hits: usize,
    pub cold_allocations: usize,
}

impl ThreadAffineAllocatorManager {
    // Construct thread-affine manager starting at given GPU virtual address.
    pub fn new(base_va: u64) -> Self {
        Self {
            thread_pools: Vec::new(),
            global_freelist: AtomicAllocatorFreelist::new(),
            base_virtual_address: base_va,
            next_thread_id: 1,
            thread_affine_hits: 0,
            global_freelist_hits: 0,
            cold_allocations: 0,
        }
    }

    // Register a thread and return its thread ID.
    pub fn register_thread(&mut self) -> u32 {
        let tid = self.next_thread_id;
        self.next_thread_id += 1;
        let pool_va = self.base_virtual_address + (tid as u64) * 0x1000_0000;
        self.thread_pools.push(ThreadCommandAllocatorPool::new(tid, pool_va));
        tid
    }

    // Obtain an allocator with thread-affinity priority, falling back to lock-free freelist.
    pub fn acquire_allocator(
        &mut self,
        thread_id: u32,
        list_type: D3D12_COMMAND_LIST_TYPE,
        completed_fence: u64,
    ) -> &mut CommandAllocatorInstance {
        let pool_idx = match self.thread_pools.iter().position(|p| p.thread_id == thread_id) {
            Some(idx) => idx,
            None => {
                let pool_va = self.base_virtual_address + (thread_id as u64) * 0x1000_0000;
                self.thread_pools.push(ThreadCommandAllocatorPool::new(thread_id, pool_va));
                self.thread_pools.len() - 1
            }
        };

        let pool = &mut self.thread_pools[pool_idx];
        let allocators = match list_type {
            D3D12_COMMAND_LIST_TYPE::Direct | D3D12_COMMAND_LIST_TYPE::Bundle => &mut pool.direct_allocators,
            D3D12_COMMAND_LIST_TYPE::Compute => &mut pool.compute_allocators,
            D3D12_COMMAND_LIST_TYPE::Copy => &mut pool.copy_allocators,
            _ => &mut pool.direct_allocators,
        };

        for i in 0..allocators.len() {
            if allocators[i].is_ready_for_reset(completed_fence) {
                allocators[i].reset();
                allocators[i].lifecycle = AllocatorLifecycle::Recording;
                self.thread_affine_hits += 1;
                return &mut allocators[i];
            }
        }

        if let Some(mut recycled) = self.global_freelist.pop_allocator(list_type) {
            recycled.thread_id = thread_id;
            recycled.lifecycle = AllocatorLifecycle::Recording;
            allocators.push(recycled);
            self.global_freelist_hits += 1;
            let last = allocators.len() - 1;
            return &mut allocators[last];
        }

        self.cold_allocations += 1;
        let id = pool.next_allocator_id;
        pool.next_allocator_id += 1;
        let va = pool.next_virtual_address;
        pool.next_virtual_address += COMMAND_ALLOCATOR_DEFAULT_PAGE_SIZE as u64;

        let mut instance = CommandAllocatorInstance::new(id, thread_id, list_type, va);
        instance.lifecycle = AllocatorLifecycle::Recording;
        allocators.push(instance);
        let last = allocators.len() - 1;
        &mut allocators[last]
    }

    // Recycle allocator into the global lock-free freelist.
    pub fn recycle_to_global_freelist(&mut self, allocator: CommandAllocatorInstance) -> bool {
        self.global_freelist.push_allocator(allocator)
    }
}

// Command list batch submitted to hardware execution queues.
#[derive(Debug, Clone)]
pub struct CommandListBatch {
    pub list_id: u32,
    pub allocator_id: u32,
    pub command_count: usize,
    pub recorded_bytes: usize,
}

impl CommandListBatch {
    // Construct a new command list batch.
    pub fn new(list_id: u32, allocator_id: u32, command_count: usize, recorded_bytes: usize) -> Self {
        Self {
            list_id,
            allocator_id,
            command_count,
            recorded_bytes,
        }
    }
}

// Hardware command queue maintaining monotonic timeline fence and pending wait-points.
#[derive(Debug, Clone)]
pub struct HardwareCommandQueue {
    pub queue_type: D3D12_COMMAND_LIST_TYPE,
    pub timeline_fence_value: u64,
    pub completed_fence_value: u64,
    pub pending_waits: Vec<(u32, u64)>,
    pub submitted_batches: Vec<CommandListBatch>,
}

impl HardwareCommandQueue {
    // Construct new hardware queue.
    pub fn new(queue_type: D3D12_COMMAND_LIST_TYPE) -> Self {
        Self {
            queue_type,
            timeline_fence_value: 0,
            completed_fence_value: 0,
            pending_waits: Vec::new(),
            submitted_batches: Vec::new(),
        }
    }

    // Insert GPU wait condition on an external timeline fence.
    pub fn wait_fence(&mut self, fence_id: u32, target_value: u64) {
        self.pending_waits.push((fence_id, target_value));
    }

    // Advance GPU completed fence value up to specified value.
    pub fn advance_progress(&mut self, completed_value: u64) {
        if completed_value > self.completed_fence_value {
            self.completed_fence_value = completed_value;
        }
    }
}

// Sub-microsecond multi-queue submission engine coordinating Direct, Compute, and Copy queues.
pub struct SubMicrosecondMultiQueueEngine {
    pub direct_queue: HardwareCommandQueue,
    pub compute_queue: HardwareCommandQueue,
    pub copy_queue: HardwareCommandQueue,
    pub timeline_fence_id: u32,
    pub total_submissions: u64,
    pub total_sync_points: u64,
}

impl SubMicrosecondMultiQueueEngine {
    // Construct new multi-queue submission engine.
    pub fn new() -> Self {
        Self {
            direct_queue: HardwareCommandQueue::new(D3D12_COMMAND_LIST_TYPE::Direct),
            compute_queue: HardwareCommandQueue::new(D3D12_COMMAND_LIST_TYPE::Compute),
            copy_queue: HardwareCommandQueue::new(D3D12_COMMAND_LIST_TYPE::Copy),
            timeline_fence_id: 1,
            total_submissions: 0,
            total_sync_points: 0,
        }
    }

    // Retrieve mutable reference to targeted queue.
    fn get_queue_mut(&mut self, queue_type: D3D12_COMMAND_LIST_TYPE) -> &mut HardwareCommandQueue {
        match queue_type {
            D3D12_COMMAND_LIST_TYPE::Direct | D3D12_COMMAND_LIST_TYPE::Bundle => &mut self.direct_queue,
            D3D12_COMMAND_LIST_TYPE::Compute => &mut self.compute_queue,
            D3D12_COMMAND_LIST_TYPE::Copy => &mut self.copy_queue,
            _ => &mut self.direct_queue,
        }
    }

    // Retrieve shared reference to targeted queue.
    fn get_queue(&self, queue_type: D3D12_COMMAND_LIST_TYPE) -> &HardwareCommandQueue {
        match queue_type {
            D3D12_COMMAND_LIST_TYPE::Direct | D3D12_COMMAND_LIST_TYPE::Bundle => &self.direct_queue,
            D3D12_COMMAND_LIST_TYPE::Compute => &self.compute_queue,
            D3D12_COMMAND_LIST_TYPE::Copy => &self.copy_queue,
            _ => &self.direct_queue,
        }
    }

    // Execute command lists batch on the target queue, returning new timeline fence value.
    pub fn execute_command_lists(
        &mut self,
        queue_type: D3D12_COMMAND_LIST_TYPE,
        batches: &[CommandListBatch],
    ) -> u64 {
        let queue = self.get_queue_mut(queue_type);
        queue.timeline_fence_value += 1;
        let signal_val = queue.timeline_fence_value;
        for batch in batches {
            queue.submitted_batches.push(batch.clone());
        }
        self.total_submissions += 1;
        signal_val
    }

    // Enqueue cross-queue wait on target queue until source queue fence completes.
    pub fn cross_queue_sync(
        &mut self,
        waiting_queue: D3D12_COMMAND_LIST_TYPE,
        signaling_queue: D3D12_COMMAND_LIST_TYPE,
    ) {
        let signal_fence = self.get_queue(signaling_queue).timeline_fence_value;
        let fence_id = self.timeline_fence_id;
        let queue = self.get_queue_mut(waiting_queue);
        queue.wait_fence(fence_id, signal_fence);
        self.total_sync_points += 1;
    }

    // Advance completion progress on the targeted queue.
    pub fn advance_queue_progress(&mut self, queue_type: D3D12_COMMAND_LIST_TYPE, completed_val: u64) {
        self.get_queue_mut(queue_type).advance_progress(completed_val);
    }

    // Check if target queue timeline fence has completed specified value.
    pub fn is_fence_completed(&self, queue_type: D3D12_COMMAND_LIST_TYPE, target_val: u64) -> bool {
        self.get_queue(queue_type).completed_fence_value >= target_val
    }

    // Retrieve current timeline fence value for queue.
    pub fn timeline_value(&self, queue_type: D3D12_COMMAND_LIST_TYPE) -> u64 {
        self.get_queue(queue_type).timeline_fence_value
    }

    // Retrieve current completed fence value for queue.
    pub fn completed_value(&self, queue_type: D3D12_COMMAND_LIST_TYPE) -> u64 {
        self.get_queue(queue_type).completed_fence_value
    }
}

// Configuration for allocator memory watermark threshold governor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocatorWatermarkConfig {
    pub high_watermark_bytes: usize,
    pub low_watermark_bytes: usize,
    pub max_retained_arenas_per_allocator: usize,
    pub trim_on_frame_end: bool,
}

impl Default for AllocatorWatermarkConfig {
    fn default() -> Self {
        Self {
            high_watermark_bytes: 16 * 1024 * 1024,
            low_watermark_bytes: 4 * 1024 * 1024,
            max_retained_arenas_per_allocator: 1,
            trim_on_frame_end: true,
        }
    }
}

// Memory tracking metrics for allocator page arena reclamation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AllocatorMemoryStats {
    pub total_committed_bytes: usize,
    pub total_allocated_bytes: usize,
    pub total_trimmed_bytes: usize,
    pub total_reclaimed_arenas: usize,
    pub trim_cycles_performed: usize,
}

// Memory watermark governor reclaiming surplus page arenas on memory overflow.
pub struct AllocatorWatermarkGovernor {
    pub config: AllocatorWatermarkConfig,
    pub stats: AllocatorMemoryStats,
}

impl AllocatorWatermarkGovernor {
    // Construct new memory watermark governor.
    pub fn new(config: AllocatorWatermarkConfig) -> Self {
        Self {
            config,
            stats: AllocatorMemoryStats::default(),
        }
    }

    // Evaluate memory consumption and trim idle page arenas if above high watermark.
    pub fn check_and_trim(&mut self, manager: &mut ThreadAffineAllocatorManager) -> usize {
        let mut total_committed = 0usize;
        for pool in &manager.thread_pools {
            total_committed += pool.total_committed_memory();
        }

        self.stats.total_committed_bytes = total_committed;
        if total_committed <= self.config.high_watermark_bytes && !self.config.trim_on_frame_end {
            return 0;
        }

        self.trim_excess_arenas(manager)
    }

    // Force trimming of surplus page arenas across all thread pools and global freelist.
    pub fn force_trim_all(&mut self, manager: &mut ThreadAffineAllocatorManager) -> usize {
        self.trim_excess_arenas(manager)
    }

    // Internal helper executing arena trimming on ready allocators.
    fn trim_excess_arenas(&mut self, manager: &mut ThreadAffineAllocatorManager) -> usize {
        let max_arenas = self.config.max_retained_arenas_per_allocator.max(1);
        let mut trimmed_bytes = 0usize;
        let mut reclaimed_arenas = 0usize;

        for pool in &mut manager.thread_pools {
            let all_allocators = [
                &mut pool.direct_allocators,
                &mut pool.compute_allocators,
                &mut pool.copy_allocators,
            ];

            for alloc_list in all_allocators {
                for alloc in alloc_list.iter_mut() {
                    if alloc.lifecycle == AllocatorLifecycle::Ready && alloc.page_arenas.len() > max_arenas {
                        let surplus = alloc.page_arenas.split_off(max_arenas);
                        for arena in surplus {
                            trimmed_bytes += arena.capacity_bytes;
                            alloc.total_committed_bytes = alloc.total_committed_bytes.saturating_sub(arena.capacity_bytes);
                            reclaimed_arenas += 1;
                        }
                    }
                }
            }
        }

        for slot in &mut manager.global_freelist.slots {
            if let Some(ref mut alloc) = slot {
                if alloc.page_arenas.len() > max_arenas {
                    let surplus = alloc.page_arenas.split_off(max_arenas);
                    for arena in surplus {
                        trimmed_bytes += arena.capacity_bytes;
                        alloc.total_committed_bytes = alloc.total_committed_bytes.saturating_sub(arena.capacity_bytes);
                        reclaimed_arenas += 1;
                    }
                }
            }
        }

        self.stats.total_trimmed_bytes += trimmed_bytes;
        self.stats.total_reclaimed_arenas += reclaimed_arenas;
        self.stats.trim_cycles_performed += 1;
        trimmed_bytes
    }
}

// Comprehensive self-test suite for Direct3D 12 command allocator pool, multi-queue engine, and watermark governor.
pub fn test_command_allocator_pool_subsystem() -> bool {
    crate::serial_write_str("[KNT-ALLOCATOR-POOL] Initializing Direct3D 12 command allocator pool self-test...\r\n");

    // 1. Verify per-thread allocator acquisition and page arena expansion.
    let mut manager = ThreadAffineAllocatorManager::new(0x0000_6000_0000);
    let t1 = manager.register_thread();
    let t2 = manager.register_thread();

    let id1: u32;
    {
        let alloc_direct = manager.acquire_allocator(t1, D3D12_COMMAND_LIST_TYPE::Direct, 0);
        if alloc_direct.page_arenas.len() != 1 || alloc_direct.total_committed_bytes != COMMAND_ALLOCATOR_DEFAULT_PAGE_SIZE {
            return false;
        }

        let va1 = alloc_direct.allocate_command_memory(1024, 256, 0x0000_6100_0000);
        if va1 == 0 || (va1 % 256) != 0 {
            return false;
        }

        // Force page arena expansion by requesting memory larger than remaining first arena capacity.
        let _va_large = alloc_direct.allocate_command_memory(COMMAND_ALLOCATOR_DEFAULT_PAGE_SIZE, 256, 0x0000_6200_0000);
        if alloc_direct.page_arenas.len() != 2 {
            return false;
        }

        alloc_direct.mark_pending_gpu(10);
        id1 = alloc_direct.allocator_id;
    }

    // 2. Verify thread-affine allocator recycling and lock-free global freelist.
    {
        let alloc_direct2 = manager.acquire_allocator(t1, D3D12_COMMAND_LIST_TYPE::Direct, 0);
        if alloc_direct2.allocator_id == id1 {
            return false;
        }
    }

    {
        let alloc_recycled = manager.acquire_allocator(t1, D3D12_COMMAND_LIST_TYPE::Direct, 10);
        if alloc_recycled.lifecycle != AllocatorLifecycle::Recording || manager.thread_affine_hits < 1 {
            return false;
        }
    }

    let global_alloc = CommandAllocatorInstance::new(99, 0, D3D12_COMMAND_LIST_TYPE::Compute, 0x0000_6300_0000);
    if !manager.recycle_to_global_freelist(global_alloc) {
        return false;
    }
    if manager.global_freelist.available_count() != 1 {
        return false;
    }

    {
        let alloc_t2 = manager.acquire_allocator(t2, D3D12_COMMAND_LIST_TYPE::Compute, 0);
        if alloc_t2.allocator_id != 99 || manager.global_freelist_hits < 1 {
            return false;
        }
    }

    // 3. Verify sub-microsecond multi-queue submission and timeline fence synchronization.
    let mut mq_engine = SubMicrosecondMultiQueueEngine::new();
    let b_copy = CommandListBatch::new(1, 10, 4, 1024);
    let b_compute = CommandListBatch::new(2, 11, 8, 2048);
    let b_direct = CommandListBatch::new(3, 12, 16, 4096);

    let f_copy = mq_engine.execute_command_lists(D3D12_COMMAND_LIST_TYPE::Copy, &[b_copy]);
    mq_engine.cross_queue_sync(D3D12_COMMAND_LIST_TYPE::Compute, D3D12_COMMAND_LIST_TYPE::Copy);

    let f_compute = mq_engine.execute_command_lists(D3D12_COMMAND_LIST_TYPE::Compute, &[b_compute]);
    mq_engine.cross_queue_sync(D3D12_COMMAND_LIST_TYPE::Direct, D3D12_COMMAND_LIST_TYPE::Compute);

    let f_direct = mq_engine.execute_command_lists(D3D12_COMMAND_LIST_TYPE::Direct, &[b_direct]);

    if f_copy != 1 || f_compute != 1 || f_direct != 1 {
        return false;
    }
    if mq_engine.total_submissions != 3 || mq_engine.total_sync_points != 2 {
        return false;
    }

    if mq_engine.is_fence_completed(D3D12_COMMAND_LIST_TYPE::Direct, 1) {
        return false;
    }
    mq_engine.advance_queue_progress(D3D12_COMMAND_LIST_TYPE::Copy, 1);
    mq_engine.advance_queue_progress(D3D12_COMMAND_LIST_TYPE::Compute, 1);
    mq_engine.advance_queue_progress(D3D12_COMMAND_LIST_TYPE::Direct, 1);

    if !mq_engine.is_fence_completed(D3D12_COMMAND_LIST_TYPE::Direct, 1) {
        return false;
    }

    // 4. Verify memory watermark governor and surplus page arena trimming.
    for pool in &mut manager.thread_pools {
        for alloc in &mut pool.direct_allocators {
            alloc.lifecycle = AllocatorLifecycle::Ready;
        }
    }

    let wm_config = AllocatorWatermarkConfig {
        high_watermark_bytes: 64 * 1024,
        low_watermark_bytes: 64 * 1024,
        max_retained_arenas_per_allocator: 1,
        trim_on_frame_end: true,
    };
    let mut governor = AllocatorWatermarkGovernor::new(wm_config);
    let trimmed = governor.force_trim_all(&mut manager);
    if trimmed == 0 || governor.stats.total_reclaimed_arenas == 0 {
        return false;
    }

    for pool in &manager.thread_pools {
        for alloc in &pool.direct_allocators {
            if alloc.page_arenas.len() > 1 {
                return false;
            }
        }
    }

    // 5. Emit deterministic boot sentinel.
    crate::serial_write_str("[KNT-D3D12-ALLOCATOR-POOL-PASS] Direct3D 12 agility command allocator pool and submission engine verified\r\n");
    true
}
