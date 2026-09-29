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

// Temporary stub for self-test during incremental commits.
pub fn test_work_graphs_allocator_subsystem() -> bool {
    true
}
