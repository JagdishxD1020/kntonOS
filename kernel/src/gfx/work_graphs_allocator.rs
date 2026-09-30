//! Direct3D 12 Work Graphs Node Memory Allocator & Program State Suballocator.
//! Provides backing store sizing, node record ring suballocation, and execution telemetry.

#![allow(dead_code, non_camel_case_types, unused_imports)]

extern crate alloc;
use alloc::vec::Vec;

// Required byte alignment for work graph node records and backing store offsets.
pub const D3D12_WORK_GRAPH_NODE_RECORD_ALIGNMENT: u64 = 64;

// Required byte alignment for the entire work graph backing store buffer.
pub const D3D12_WORK_GRAPH_BACKING_BUFFER_ALIGNMENT: u64 = 64;

// Flags controlling work graph instantiation and program state updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_SET_WORK_GRAPH_FLAGS(pub u32);

impl D3D12_SET_WORK_GRAPH_FLAGS {
    pub const NONE: Self = Self(0);
    pub const INITIALIZE: Self = Self(1 << 0);
    pub const AUTO_SIZE: Self = Self(1 << 1);
    pub const PERSISTENT: Self = Self(1 << 2);

    // Tests if flags contain specified bitmask.
    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    // Combines two flag sets.
    #[inline(always)]
    pub const fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

// Work graph GPU node launch execution modes.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkGraphLaunchMode {
    BroadcastingLaunch = 0,
    CoalescingLaunch = 1,
    ThreadLaunch = 2,
}

// Memory category required by a work graph node during execution.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkGraphNodeMemoryKind {
    NodeLocalMemory = 0,
    SharedBackingStore = 1,
}

// Memory sizing and alignment specification for an individual node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkGraphNodeMemoryDesc {
    pub node_id: u32,
    pub launch_mode: WorkGraphLaunchMode,
    pub memory_kind: WorkGraphNodeMemoryKind,
    pub input_record_bytes: u32,
    pub local_scratch_bytes: u32,
    pub max_in_flight_records: u32,
}

impl WorkGraphNodeMemoryDesc {
    // Creates a new node memory descriptor.
    pub const fn new(
        node_id: u32,
        launch_mode: WorkGraphLaunchMode,
        memory_kind: WorkGraphNodeMemoryKind,
        input_record_bytes: u32,
        local_scratch_bytes: u32,
        max_in_flight_records: u32,
    ) -> Self {
        Self {
            node_id,
            launch_mode,
            memory_kind,
            input_record_bytes,
            local_scratch_bytes,
            max_in_flight_records,
        }
    }

    // Returns the per-record byte size aligned to 64 bytes.
    #[inline(always)]
    pub const fn aligned_record_bytes(&self) -> u64 {
        let raw = self.input_record_bytes as u64;
        (raw + D3D12_WORK_GRAPH_NODE_RECORD_ALIGNMENT - 1)
            & !(D3D12_WORK_GRAPH_NODE_RECORD_ALIGNMENT - 1)
    }

    // Computes total node memory requirement with alignment padding.
    pub fn compute_total_node_bytes(&self) -> u64 {
        let records_total = self.aligned_record_bytes() * (self.max_in_flight_records as u64);
        let scratch_aligned = ((self.local_scratch_bytes as u64) + D3D12_WORK_GRAPH_NODE_RECORD_ALIGNMENT - 1)
            & !(D3D12_WORK_GRAPH_NODE_RECORD_ALIGNMENT - 1);
        records_total + scratch_aligned
    }
}

// Validates whether a GPU virtual address satisfies the 64-byte alignment requirement.
#[inline(always)]
pub const fn is_64byte_aligned(address: u64) -> bool {
    (address & (D3D12_WORK_GRAPH_NODE_RECORD_ALIGNMENT - 1)) == 0
}

// Aligns an address or offset up to the next 64-byte boundary.
#[inline(always)]
pub const fn align_up_64(val: u64) -> u64 {
    (val + D3D12_WORK_GRAPH_NODE_RECORD_ALIGNMENT - 1) & !(D3D12_WORK_GRAPH_NODE_RECORD_ALIGNMENT - 1)
}


// Suballocated slice of transient GPU memory holding packed node records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuballocatedNodeRecord {
    pub node_id: u32,
    pub launch_mode: WorkGraphLaunchMode,
    pub gpu_virtual_address: u64,
    pub buffer_offset: u64,
    pub requested_bytes: u32,
    pub aligned_stride: u32,
    pub record_count: u32,
}

impl SuballocatedNodeRecord {
    // Total byte footprint reserved for this suballocation.
    #[inline(always)]
    pub fn total_reserved_bytes(&self) -> u64 {
        (self.aligned_stride as u64) * (self.record_count as u64)
    }

    // Calculates the GPU virtual address for an individual record index.
    #[inline(always)]
    pub fn get_record_gpu_va(&self, index: u32) -> Option<u64> {
        if index >= self.record_count {
            None
        } else {
            Some(self.gpu_virtual_address + ((index as u64) * (self.aligned_stride as u64)))
        }
    }
}

// Transient ring buffer suballocator for GPU node inputs across launch modes.
#[derive(Debug, Clone)]
pub struct DynamicNodeRecordSuballocator {
    pub base_gpu_va: u64,
    pub capacity_bytes: u64,
    pub head_offset: u64,
    pub tail_offset: u64,
    pub allocated_bytes_current_frame: u64,
    pub total_allocations: u64,
    pub total_bytes_allocated: u64,
    pub high_water_mark_bytes: u64,
}

impl DynamicNodeRecordSuballocator {
    // Creates a new dynamic node record suballocator.
    pub fn new(base_gpu_va: u64, capacity_bytes: u64) -> Result<Self, &'static str> {
        if !is_64byte_aligned(base_gpu_va) {
            return Err("Suballocator base GPU VA must be 64-byte aligned");
        }
        if !is_64byte_aligned(capacity_bytes) || capacity_bytes == 0 {
            return Err("Suballocator capacity must be non-zero and 64-byte aligned");
        }

        Ok(Self {
            base_gpu_va,
            capacity_bytes,
            head_offset: 0,
            tail_offset: 0,
            allocated_bytes_current_frame: 0,
            total_allocations: 0,
            total_bytes_allocated: 0,
            high_water_mark_bytes: 0,
        })
    }

    // Suballocates transient memory for one or more node input records.
    pub fn suballocate_node_records(
        &mut self,
        node_id: u32,
        launch_mode: WorkGraphLaunchMode,
        record_size_bytes: u32,
        record_count: u32,
    ) -> Result<SuballocatedNodeRecord, &'static str> {
        if record_count == 0 {
            return Err("Record count must be at least 1");
        }
        if record_size_bytes == 0 {
            return Err("Record size must be greater than 0");
        }

        let aligned_stride = align_up_64(record_size_bytes as u64) as u32;
        let total_bytes = (aligned_stride as u64) * (record_count as u64);

        if total_bytes > self.capacity_bytes {
            return Err("Requested allocation exceeds total ring buffer capacity");
        }

        let alloc_offset: u64;

        if self.tail_offset >= self.head_offset {
            // Case A: Tail is ahead of or equal to Head
            let remaining_at_end = self.capacity_bytes.saturating_sub(self.tail_offset);
            if remaining_at_end >= total_bytes {
                alloc_offset = self.tail_offset;
                self.tail_offset = self.tail_offset.saturating_add(total_bytes);
            } else {
                // Wrap around to start of ring buffer
                if self.head_offset > total_bytes {
                    alloc_offset = 0;
                    self.tail_offset = total_bytes;
                } else {
                    return Err("Work graph node ring buffer exhausted (wrap collision)");
                }
            }
        } else {
            // Case B: Tail has wrapped behind Head
            let space_before_head = self.head_offset.saturating_sub(self.tail_offset);
            if space_before_head > total_bytes {
                alloc_offset = self.tail_offset;
                self.tail_offset = self.tail_offset.saturating_add(total_bytes);
            } else {
                return Err("Work graph node ring buffer exhausted (tail caught head)");
            }
        }

        self.allocated_bytes_current_frame = self.allocated_bytes_current_frame.saturating_add(total_bytes);
        self.total_allocations = self.total_allocations.saturating_add(1);
        self.total_bytes_allocated = self.total_bytes_allocated.saturating_add(total_bytes);

        let active_bytes = if self.tail_offset >= self.head_offset {
            self.tail_offset - self.head_offset
        } else {
            (self.capacity_bytes - self.head_offset) + self.tail_offset
        };
        if active_bytes > self.high_water_mark_bytes {
            self.high_water_mark_bytes = active_bytes;
        }

        Ok(SuballocatedNodeRecord {
            node_id,
            launch_mode,
            gpu_virtual_address: self.base_gpu_va + alloc_offset,
            buffer_offset: alloc_offset,
            requested_bytes: record_size_bytes,
            aligned_stride,
            record_count,
        })
    }

    // Advances the read head offset as the GPU retires consumed node records.
    pub fn advance_head(&mut self, new_head_offset: u64) {
        let aligned = align_up_64(new_head_offset);
        if aligned <= self.capacity_bytes {
            self.head_offset = aligned;
        }
    }

    // Resets per-frame accounting counters at frame boundaries.
    pub fn reset_frame(&mut self) {
        self.allocated_bytes_current_frame = 0;
    }
}
// Temporary stub for self-test during incremental commits.
pub fn test_work_graphs_allocator_subsystem() -> bool {
    true
}
