//! Direct3D 12 Query Heap, GPU Timestamp & Occlusion Culling Engine.
//! Provides asynchronous hardware query slots, GPU timestamp profiling, and pipeline stats.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use alloc::vec::Vec;

// Direct3D 12 query heap classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_QUERY_HEAP_TYPE {
    Occlusion = 0,
    Timestamp = 1,
    PipelineStatistics = 2,
}

// Direct3D 12 individual query measurement type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_QUERY_TYPE {
    Occlusion = 0,
    BinaryOcclusion = 1,
    Timestamp = 2,
    PipelineStatistics = 3,
}

// Query heap allocation descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_QUERY_HEAP_DESC {
    pub heap_type: D3D12_QUERY_HEAP_TYPE,
    pub count: u32,
    pub node_mask: u32,
}

impl D3D12_QUERY_HEAP_DESC {
    // Construct new query heap descriptor
    pub const fn new(heap_type: D3D12_QUERY_HEAP_TYPE, count: u32) -> Self {
        Self {
            heap_type,
            count,
            node_mask: 0,
        }
    }
}

// Hardware pipeline statistics counter payload
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_QUERY_DATA_PIPELINE_STATISTICS {
    pub ia_vertices: u64,
    pub ia_primitives: u64,
    pub vs_invocations: u64,
    pub gs_invocations: u64,
    pub gs_primitives: u64,
    pub c_invocations: u64,
    pub c_primitives: u64,
    pub ps_invocations: u64,
    pub hs_invocations: u64,
    pub ds_invocations: u64,
    pub cs_invocations: u64,
}

// Internal hardware query slot storage
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuerySlot {
    pub is_active: bool,
    pub is_resolved: bool,
    pub raw_data: u64,
    pub stats: Option<D3D12_QUERY_DATA_PIPELINE_STATISTICS>,
}

impl QuerySlot {
    // Construct default empty query slot
    pub fn new() -> Self {
        Self {
            is_active: false,
            is_resolved: false,
            raw_data: 0,
            stats: None,
        }
    }
}

// Direct3D 12 Query Heap managing an array of hardware measurement slots
pub struct D3D12QueryHeap {
    pub desc: D3D12_QUERY_HEAP_DESC,
    pub slots: Vec<QuerySlot>,
}

impl D3D12QueryHeap {
    // Allocate query heap with requested capacity
    pub fn new(desc: D3D12_QUERY_HEAP_DESC) -> Result<Self, QueryError> {
        if desc.count == 0 {
            return Err(QueryError::InvalidSlotIndex);
        }
        let mut slots = Vec::with_capacity(desc.count as usize);
        for _ in 0..desc.count {
            slots.push(QuerySlot::new());
        }
        Ok(Self { desc, slots })
    }

    // Number of query slots in heap
    pub fn slot_count(&self) -> u32 {
        self.desc.count
    }

    // Heap operational classification type
    pub fn heap_type(&self) -> D3D12_QUERY_HEAP_TYPE {
        self.desc.heap_type
    }

    // Validate if query type is compatible with this heap allocation
    pub fn validate_query_type(&self, query_type: D3D12_QUERY_TYPE) -> Result<(), QueryError> {
        match (self.desc.heap_type, query_type) {
            (D3D12_QUERY_HEAP_TYPE::Occlusion, D3D12_QUERY_TYPE::Occlusion) => Ok(()),
            (D3D12_QUERY_HEAP_TYPE::Occlusion, D3D12_QUERY_TYPE::BinaryOcclusion) => Ok(()),
            (D3D12_QUERY_HEAP_TYPE::Timestamp, D3D12_QUERY_TYPE::Timestamp) => Ok(()),
            (D3D12_QUERY_HEAP_TYPE::PipelineStatistics, D3D12_QUERY_TYPE::PipelineStatistics) => Ok(()),
            _ => Err(QueryError::InvalidQueryType),
        }
    }
}

// Errors produced during query recording and resolution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryError {
    InvalidQueryType,
    InvalidSlotIndex,
    QueryAlreadyActive,
    QueryNotActive,
    BufferTooSmall,
    SlotNotResolved,
    ResolveBufferOutOfBounds,
}
