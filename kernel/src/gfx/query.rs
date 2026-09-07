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

// Recorded query commands for command buffer encoding
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedQueryCommand {
    BeginQuery {
        query_type: D3D12_QUERY_TYPE,
        slot_index: u32,
    },
    EndQuery {
        query_type: D3D12_QUERY_TYPE,
        slot_index: u32,
    },
    ResolveQueryData {
        query_type: D3D12_QUERY_TYPE,
        start_index: u32,
        num_queries: u32,
        destination_offset: usize,
    },
}

impl D3D12QueryHeap {
    // Begin query measurement on a specific slot (invalid for timestamp queries)
    pub fn begin_query(&mut self, query_type: D3D12_QUERY_TYPE, slot_index: u32) -> Result<(), QueryError> {
        self.validate_query_type(query_type)?;
        if query_type == D3D12_QUERY_TYPE::Timestamp {
            return Err(QueryError::InvalidQueryType);
        }

        let slot = self.slots.get_mut(slot_index as usize).ok_or(QueryError::InvalidSlotIndex)?;
        if slot.is_active {
            return Err(QueryError::QueryAlreadyActive);
        }

        slot.is_active = true;
        slot.is_resolved = false;
        Ok(())
    }

    // End query measurement and record raw GPU payload or pipeline statistics
    pub fn end_query(
        &mut self,
        query_type: D3D12_QUERY_TYPE,
        slot_index: u32,
        raw_val: u64,
        stats: Option<D3D12_QUERY_DATA_PIPELINE_STATISTICS>,
    ) -> Result<(), QueryError> {
        self.validate_query_type(query_type)?;
        let slot = self.slots.get_mut(slot_index as usize).ok_or(QueryError::InvalidSlotIndex)?;

        if query_type != D3D12_QUERY_TYPE::Timestamp && !slot.is_active {
            return Err(QueryError::QueryNotActive);
        }

        slot.is_active = false;
        slot.raw_data = raw_val;
        slot.stats = stats;
        slot.is_resolved = true;
        Ok(())
    }

    // Resolve query results into CPU or GPU memory buffer
    pub fn resolve_query_data(
        &self,
        query_type: D3D12_QUERY_TYPE,
        start_index: u32,
        num_queries: u32,
        destination_buffer: &mut [u8],
        dest_offset: usize,
    ) -> Result<usize, QueryError> {
        self.validate_query_type(query_type)?;
        let end_index = (start_index + num_queries) as usize;
        if end_index > self.slots.len() {
            return Err(QueryError::InvalidSlotIndex);
        }

        let stride = match query_type {
            D3D12_QUERY_TYPE::Occlusion | D3D12_QUERY_TYPE::BinaryOcclusion | D3D12_QUERY_TYPE::Timestamp => 8,
            D3D12_QUERY_TYPE::PipelineStatistics => 88,
        };

        let total_bytes = stride * num_queries as usize;
        if dest_offset + total_bytes > destination_buffer.len() {
            return Err(QueryError::ResolveBufferOutOfBounds);
        }

        for i in 0..num_queries as usize {
            let slot = &self.slots[(start_index as usize) + i];
            let out_pos = dest_offset + (i * stride);

            match query_type {
                D3D12_QUERY_TYPE::Occlusion | D3D12_QUERY_TYPE::Timestamp => {
                    destination_buffer[out_pos..out_pos + 8].copy_from_slice(&slot.raw_data.to_le_bytes());
                }
                D3D12_QUERY_TYPE::BinaryOcclusion => {
                    let binary_val: u64 = if slot.raw_data > 0 { 1 } else { 0 };
                    destination_buffer[out_pos..out_pos + 8].copy_from_slice(&binary_val.to_le_bytes());
                }
                D3D12_QUERY_TYPE::PipelineStatistics => {
                    let default_stats = D3D12_QUERY_DATA_PIPELINE_STATISTICS::default();
                    let s = slot.stats.as_ref().unwrap_or(&default_stats);
                    destination_buffer[out_pos..out_pos + 8].copy_from_slice(&s.ia_vertices.to_le_bytes());
                    destination_buffer[out_pos + 8..out_pos + 16].copy_from_slice(&s.ia_primitives.to_le_bytes());
                    destination_buffer[out_pos + 16..out_pos + 24].copy_from_slice(&s.vs_invocations.to_le_bytes());
                    destination_buffer[out_pos + 24..out_pos + 32].copy_from_slice(&s.gs_invocations.to_le_bytes());
                    destination_buffer[out_pos + 32..out_pos + 40].copy_from_slice(&s.gs_primitives.to_le_bytes());
                    destination_buffer[out_pos + 40..out_pos + 48].copy_from_slice(&s.c_invocations.to_le_bytes());
                    destination_buffer[out_pos + 48..out_pos + 56].copy_from_slice(&s.c_primitives.to_le_bytes());
                    destination_buffer[out_pos + 56..out_pos + 64].copy_from_slice(&s.ps_invocations.to_le_bytes());
                    destination_buffer[out_pos + 64..out_pos + 72].copy_from_slice(&s.hs_invocations.to_le_bytes());
                    destination_buffer[out_pos + 72..out_pos + 80].copy_from_slice(&s.ds_invocations.to_le_bytes());
                    destination_buffer[out_pos + 80..out_pos + 88].copy_from_slice(&s.cs_invocations.to_le_bytes());
                }
            }
        }

        Ok(total_bytes)
    }
}

// Command stream query recorder mimicking command list interface
pub struct QueryCommandRecorder {
    pub commands: Vec<RecordedQueryCommand>,
}

impl QueryCommandRecorder {
    // Construct new empty query command recorder
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    // Record BeginQuery command
    pub fn record_begin_query(&mut self, query_type: D3D12_QUERY_TYPE, slot_index: u32) {
        self.commands.push(RecordedQueryCommand::BeginQuery {
            query_type,
            slot_index,
        });
    }

    // Record EndQuery command
    pub fn record_end_query(&mut self, query_type: D3D12_QUERY_TYPE, slot_index: u32) {
        self.commands.push(RecordedQueryCommand::EndQuery {
            query_type,
            slot_index,
        });
    }

    // Record ResolveQueryData command
    pub fn record_resolve_query_data(
        &mut self,
        query_type: D3D12_QUERY_TYPE,
        start_index: u32,
        num_queries: u32,
        destination_offset: usize,
    ) {
        self.commands.push(RecordedQueryCommand::ResolveQueryData {
            query_type,
            start_index,
            num_queries,
            destination_offset,
        });
    }
}

// Measured GPU timeline execution span in microsecond units
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuExecutionSpan {
    pub start_ticks: u64,
    pub end_ticks: u64,
    pub duration_us: u64,
    pub label: [u8; 16],
}

// GPU hardware timestamp clock frequency profiler and calibrator
pub struct GpuTimestampProfiler {
    pub frequency_hz: u64,
}

impl GpuTimestampProfiler {
    // Construct new GPU timestamp profiler calibrated with hardware timer frequency
    pub fn new(frequency_hz: u64) -> Self {
        Self {
            frequency_hz: if frequency_hz == 0 { 1_000_000_000 } else { frequency_hz },
        }
    }

    // Retrieve active GPU timer clock tick frequency in Hertz
    pub fn get_timestamp_frequency(&self) -> u64 {
        self.frequency_hz
    }

    // Convert raw GPU timer ticks to elapsed nanoseconds
    pub fn ticks_to_nanoseconds(&self, ticks: u64) -> u64 {
        ((ticks as u128 * 1_000_000_000) / self.frequency_hz as u128) as u64
    }

    // Convert raw GPU timer ticks to elapsed microseconds
    pub fn ticks_to_microseconds(&self, ticks: u64) -> u64 {
        ((ticks as u128 * 1_000_000) / self.frequency_hz as u128) as u64
    }

    // Calculate execution duration in microseconds between two timestamp ticks
    pub fn compute_duration_microseconds(&self, start_ticks: u64, end_ticks: u64) -> u64 {
        let delta = end_ticks.saturating_sub(start_ticks);
        self.ticks_to_microseconds(delta)
    }

    // Build execution span descriptor from start and end GPU ticks
    pub fn calculate_execution_span(&self, start_ticks: u64, end_ticks: u64, label: &[u8]) -> GpuExecutionSpan {
        let mut name = [0u8; 16];
        let copy_len = label.len().min(15);
        name[..copy_len].copy_from_slice(&label[..copy_len]);
        GpuExecutionSpan {
            start_ticks,
            end_ticks,
            duration_us: self.compute_duration_microseconds(start_ticks, end_ticks),
            label: name,
        }
    }

    // Record timestamp query into query heap slot
    pub fn record_timestamp(
        &self,
        heap: &mut D3D12QueryHeap,
        slot_index: u32,
        current_gpu_ticks: u64,
    ) -> Result<(), QueryError> {
        heap.end_query(D3D12_QUERY_TYPE::Timestamp, slot_index, current_gpu_ticks, None)
    }
}

// Hardware pipeline statistics counter accumulator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PipelineStatisticsAccumulator {
    pub current: D3D12_QUERY_DATA_PIPELINE_STATISTICS,
    pub baseline: D3D12_QUERY_DATA_PIPELINE_STATISTICS,
}

impl PipelineStatisticsAccumulator {
    // Construct new empty pipeline statistics accumulator
    pub fn new() -> Self {
        Self::default()
    }

    // Reset accumulated statistics counters to zero
    pub fn reset(&mut self) {
        self.current = D3D12_QUERY_DATA_PIPELINE_STATISTICS::default();
        self.baseline = D3D12_QUERY_DATA_PIPELINE_STATISTICS::default();
    }

    // Accumulate draw call geometry and rasterizer pipeline metrics
    pub fn record_draw(&mut self, vertex_count: u64, primitive_count: u64, estimated_pixels: u64) {
        self.current.ia_vertices += vertex_count;
        self.current.ia_primitives += primitive_count;
        self.current.vs_invocations += vertex_count;
        self.current.c_invocations += primitive_count;
        self.current.c_primitives += primitive_count;
        self.current.ps_invocations += estimated_pixels;
    }

    // Accumulate compute shader dispatch execution metrics
    pub fn record_dispatch(&mut self, groups_x: u64, groups_y: u64, groups_z: u64, threads_per_group: u64) {
        let total_groups = groups_x * groups_y * groups_z;
        self.current.cs_invocations += total_groups * threads_per_group;
    }

    // Snapshot active pipeline statistics counters
    pub fn snapshot(&self) -> D3D12_QUERY_DATA_PIPELINE_STATISTICS {
        self.current
    }

    // Compute differential statistics relative to query begin baseline
    pub fn delta_from_baseline(&self) -> D3D12_QUERY_DATA_PIPELINE_STATISTICS {
        D3D12_QUERY_DATA_PIPELINE_STATISTICS {
            ia_vertices: self.current.ia_vertices.saturating_sub(self.baseline.ia_vertices),
            ia_primitives: self.current.ia_primitives.saturating_sub(self.baseline.ia_primitives),
            vs_invocations: self.current.vs_invocations.saturating_sub(self.baseline.vs_invocations),
            gs_invocations: self.current.gs_invocations.saturating_sub(self.baseline.gs_invocations),
            gs_primitives: self.current.gs_primitives.saturating_sub(self.baseline.gs_primitives),
            c_invocations: self.current.c_invocations.saturating_sub(self.baseline.c_invocations),
            c_primitives: self.current.c_primitives.saturating_sub(self.baseline.c_primitives),
            ps_invocations: self.current.ps_invocations.saturating_sub(self.baseline.ps_invocations),
            hs_invocations: self.current.hs_invocations.saturating_sub(self.baseline.hs_invocations),
            ds_invocations: self.current.ds_invocations.saturating_sub(self.baseline.ds_invocations),
            cs_invocations: self.current.cs_invocations.saturating_sub(self.baseline.cs_invocations),
        }
    }

    // Begin pipeline statistics measurement recording baseline
    pub fn begin_query(&mut self, heap: &mut D3D12QueryHeap, slot_index: u32) -> Result<(), QueryError> {
        heap.begin_query(D3D12_QUERY_TYPE::PipelineStatistics, slot_index)?;
        self.baseline = self.current;
        Ok(())
    }

    // End pipeline statistics measurement recording delta into query slot
    pub fn end_query(&mut self, heap: &mut D3D12QueryHeap, slot_index: u32) -> Result<(), QueryError> {
        let delta = self.delta_from_baseline();
        heap.end_query(D3D12_QUERY_TYPE::PipelineStatistics, slot_index, 0, Some(delta))
    }
}
