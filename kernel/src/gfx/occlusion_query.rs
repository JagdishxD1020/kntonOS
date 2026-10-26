//! Direct3D 12 Agility Dynamic Query Heap Recycling & Asynchronous Occlusion Query Streamer.
//! Implements circular ring buffer query heap allocation for standard and binary occlusion queries,
//! GPU timeline fence synchronization, non-blocking result readback, and batched query resolving.

#![allow(dead_code, non_camel_case_types)]

extern crate alloc;
use alloc::vec::Vec;

// Classification of occlusion query types.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcclusionQueryType {
    Standard = 0,
    Binary = 1,
}

// State tracking for individual query slots.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuerySlotState {
    Free = 0,
    Active = 1,
    Resolved = 2,
    Retired = 3,
}

// Slot descriptor within the circular occlusion query heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcclusionQuerySlot {
    pub slot_index: u32,
    pub query_type: OcclusionQueryType,
    pub state: QuerySlotState,
    pub fence_value: u64,
    pub samples_passed: u64,
}

// Circular ring buffer query heap allocator.
#[derive(Debug)]
pub struct CircularQueryHeapAllocator {
    pub capacity: u32,
    pub head: u32,
    pub tail: u32,
    pub active_count: u32,
    pub slots: Vec<OcclusionQuerySlot>,
    pub total_allocations: u64,
    pub total_wraparounds: u64,
}

impl CircularQueryHeapAllocator {
    pub fn new(capacity: u32) -> Self {
        let mut slots = Vec::with_capacity(capacity as usize);
        for i in 0..capacity {
            slots.push(OcclusionQuerySlot {
                slot_index: i,
                query_type: OcclusionQueryType::Standard,
                state: QuerySlotState::Free,
                fence_value: 0,
                samples_passed: 0,
            });
        }
        Self {
            capacity,
            head: 0,
            tail: 0,
            active_count: 0,
            slots,
            total_allocations: 0,
            total_wraparounds: 0,
        }
    }

    // Suballocates next available query slot using circular FIFO semantics.
    pub fn allocate_query(&mut self, query_type: OcclusionQueryType, fence_tag: u64) -> Result<u32, &'static str> {
        if self.active_count >= self.capacity {
            return Err("Query heap ring buffer full");
        }

        let slot_idx = self.head;
        let slot = &mut self.slots[slot_idx as usize];
        if slot.state == QuerySlotState::Active {
            return Err("Head slot still in active flight");
        }

        slot.query_type = query_type;
        slot.state = QuerySlotState::Active;
        slot.fence_value = fence_tag;
        slot.samples_passed = 0;

        self.head = (self.head + 1) % self.capacity;
        if self.head == 0 {
            self.total_wraparounds += 1;
        }
        self.active_count += 1;
        self.total_allocations += 1;

        Ok(slot_idx)
    }

    // Recycles slots up to tail whose timeline fence is completed.
    pub fn advance_tail_with_fence(&mut self, completed_fence: u64) -> u32 {
        let mut freed = 0;
        while self.active_count > 0 {
            let tail_idx = self.tail as usize;
            if self.slots[tail_idx].state != QuerySlotState::Free && self.slots[tail_idx].fence_value <= completed_fence {
                self.slots[tail_idx].state = QuerySlotState::Free;
                self.tail = (self.tail + 1) % self.capacity;
                self.active_count -= 1;
                freed += 1;
            } else {
                break;
            }
        }
        freed
    }

    #[inline(always)]
    pub fn available_slots(&self) -> u32 {
        self.capacity - self.active_count
    }
}


// Result of an asynchronous occlusion query readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryReadbackStatus {
    Ready(u64),
    Pending(u64),
    NotAllocated,
}

// GPU timeline fence synchronization and non-blocking query result readback governor.
#[derive(Debug)]
pub struct NonBlockingQueryReadbackGovernor {
    pub current_timeline_fence: u64,
    pub total_readback_queries: u64,
    pub immediate_hits: u64,
    pub pending_stalls_avoided: u64,
}

impl NonBlockingQueryReadbackGovernor {
    pub fn new() -> Self {
        Self {
            current_timeline_fence: 0,
            total_readback_queries: 0,
            immediate_hits: 0,
            pending_stalls_avoided: 0,
        }
    }

    // Updates latest signal value from GPU timeline fence.
    #[inline(always)]
    pub fn signal_fence(&mut self, fence_val: u64) {
        if fence_val > self.current_timeline_fence {
            self.current_timeline_fence = fence_val;
        }
    }

    // Checks whether query slot is ready without blocking CPU thread.
    pub fn poll_query_result(&mut self, slot: &OcclusionQuerySlot) -> QueryReadbackStatus {
        self.total_readback_queries += 1;
        if slot.state == QuerySlotState::Free {
            return QueryReadbackStatus::NotAllocated;
        }

        if slot.fence_value <= self.current_timeline_fence && slot.state == QuerySlotState::Resolved {
            self.immediate_hits += 1;
            QueryReadbackStatus::Ready(slot.samples_passed)
        } else {
            self.pending_stalls_avoided += 1;
            QueryReadbackStatus::Pending(slot.fence_value)
        }
    }

    #[inline(always)]
    pub fn hit_ratio_percentage(&self) -> u32 {
        if self.total_readback_queries == 0 {
            0
        } else {
            ((self.immediate_hits * 100) / self.total_readback_queries) as u32
        }
    }
}
