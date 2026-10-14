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
