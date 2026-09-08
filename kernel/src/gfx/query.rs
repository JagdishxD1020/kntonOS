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

// Hardware occlusion query evaluation and visibility culler
pub struct OcclusionCullingEngine;

impl OcclusionCullingEngine {
    // Begin occlusion query on proxy bounding geometry
    pub fn begin_query(
        heap: &mut D3D12QueryHeap,
        slot_index: u32,
        binary: bool,
    ) -> Result<(), QueryError> {
        let q_type = if binary {
            D3D12_QUERY_TYPE::BinaryOcclusion
        } else {
            D3D12_QUERY_TYPE::Occlusion
        };
        heap.begin_query(q_type, slot_index)
    }

    // End occlusion query recording rendered sample count or binary visibility
    pub fn end_query(
        heap: &mut D3D12QueryHeap,
        slot_index: u32,
        samples_passed: u64,
        binary: bool,
    ) -> Result<(), QueryError> {
        let q_type = if binary {
            D3D12_QUERY_TYPE::BinaryOcclusion
        } else {
            D3D12_QUERY_TYPE::Occlusion
        };
        let stored_val = if binary {
            if samples_passed > 0 { 1 } else { 0 }
        } else {
            samples_passed
        };
        heap.end_query(q_type, slot_index, stored_val, None)
    }

    // Evaluate visibility status of a resolved occlusion query slot
    pub fn is_visible(heap: &D3D12QueryHeap, slot_index: u32) -> Result<bool, QueryError> {
        let slot = heap.slots.get(slot_index as usize).ok_or(QueryError::InvalidSlotIndex)?;
        if !slot.is_resolved {
            return Err(QueryError::SlotNotResolved);
        }
        Ok(slot.raw_data > 0)
    }

    // Filter array of candidate instance slots, returning only visible indices
    pub fn filter_visible_instances(
        heap: &D3D12QueryHeap,
        candidate_slots: &[u32],
    ) -> Result<Vec<u32>, QueryError> {
        let mut visible = Vec::new();
        for &slot_idx in candidate_slots {
            if Self::is_visible(heap, slot_idx)? {
                visible.push(slot_idx);
            }
        }
        Ok(visible)
    }
}

// Comprehensive verification self-test suite for D3D12 Query Heap and Profiler Engine
pub fn test_query_subsystem() -> bool {
    crate::serial_write_str("[KNT-GFX] Testing Direct3D 12 Query Heap, GPU Timestamps & Occlusion Engine...\r\n");

    // 1. Verify query heap allocation, slot bounds, and query type validation
    let occ_desc = D3D12_QUERY_HEAP_DESC::new(D3D12_QUERY_HEAP_TYPE::Occlusion, 4);
    let mut occ_heap = match D3D12QueryHeap::new(occ_desc) {
        Ok(h) => h,
        Err(_) => {
            crate::serial_write_str("[KNT-QUERY-FAIL] Failed to allocate occlusion query heap\r\n");
            return false;
        }
    };

    let ts_desc = D3D12_QUERY_HEAP_DESC::new(D3D12_QUERY_HEAP_TYPE::Timestamp, 8);
    let mut ts_heap = match D3D12QueryHeap::new(ts_desc) {
        Ok(h) => h,
        Err(_) => {
            crate::serial_write_str("[KNT-QUERY-FAIL] Failed to allocate timestamp query heap\r\n");
            return false;
        }
    };

    let stats_desc = D3D12_QUERY_HEAP_DESC::new(D3D12_QUERY_HEAP_TYPE::PipelineStatistics, 4);
    let mut stats_heap = match D3D12QueryHeap::new(stats_desc) {
        Ok(h) => h,
        Err(_) => {
            crate::serial_write_str("[KNT-QUERY-FAIL] Failed to allocate pipeline statistics query heap\r\n");
            return false;
        }
    };

    if occ_heap.validate_query_type(D3D12_QUERY_TYPE::Timestamp).is_ok() {
        crate::serial_write_str("[KNT-QUERY-FAIL] Incompatible query type must be rejected\r\n");
        return false;
    }
    if ts_heap.begin_query(D3D12_QUERY_TYPE::Timestamp, 0).is_ok() {
        crate::serial_write_str("[KNT-QUERY-FAIL] BeginQuery on timestamp query must be rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * Query heap descriptor validation and slot allocation verified\r\n");

    // 2. Verify GPU timestamp frequency profiler and execution span calculation
    let profiler = GpuTimestampProfiler::new(100_000_000); // 100 MHz clock
    if profiler.get_timestamp_frequency() != 100_000_000 {
        crate::serial_write_str("[KNT-QUERY-FAIL] Profiler frequency mismatch\r\n");
        return false;
    }

    let start_ticks = 10_000_000;
    let end_ticks = 10_500_000; // 500,000 ticks = 5 ms = 5,000 us
    if profiler.record_timestamp(&mut ts_heap, 0, start_ticks).is_err()
        || profiler.record_timestamp(&mut ts_heap, 1, end_ticks).is_err()
    {
        crate::serial_write_str("[KNT-QUERY-FAIL] Failed to record timestamp queries\r\n");
        return false;
    }

    let span = profiler.calculate_execution_span(start_ticks, end_ticks, b"MainRenderPass");
    if span.duration_us != 5_000 {
        crate::serial_write_str("[KNT-QUERY-FAIL] Timestamp microsecond duration mismatch\r\n");
        return false;
    }

    let mut ts_resolve_buf = [0u8; 16];
    if ts_heap.resolve_query_data(D3D12_QUERY_TYPE::Timestamp, 0, 2, &mut ts_resolve_buf, 0).is_err() {
        crate::serial_write_str("[KNT-QUERY-FAIL] Failed to resolve timestamp queries\r\n");
        return false;
    }
    let resolved_t0 = u64::from_le_bytes(ts_resolve_buf[0..8].try_into().unwrap());
    let resolved_t1 = u64::from_le_bytes(ts_resolve_buf[8..16].try_into().unwrap());
    if resolved_t0 != start_ticks || resolved_t1 != end_ticks {
        crate::serial_write_str("[KNT-QUERY-FAIL] Resolved timestamp payload mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * GPU timestamp timer frequency profiling and span duration verified\r\n");

    // 3. Verify pipeline statistics counter accumulation and delta recording
    let mut accumulator = PipelineStatisticsAccumulator::new();
    if accumulator.begin_query(&mut stats_heap, 0).is_err() {
        crate::serial_write_str("[KNT-QUERY-FAIL] Failed to begin pipeline statistics query\r\n");
        return false;
    }

    accumulator.record_draw(36, 12, 500);
    accumulator.record_dispatch(4, 4, 1, 64); // 16 groups * 64 threads = 1024 CS invocations

    if accumulator.end_query(&mut stats_heap, 0).is_err() {
        crate::serial_write_str("[KNT-QUERY-FAIL] Failed to end pipeline statistics query\r\n");
        return false;
    }

    let mut stats_resolve_buf = [0u8; 88];
    if stats_heap.resolve_query_data(D3D12_QUERY_TYPE::PipelineStatistics, 0, 1, &mut stats_resolve_buf, 0).is_err() {
        crate::serial_write_str("[KNT-QUERY-FAIL] Failed to resolve pipeline statistics query\r\n");
        return false;
    }
    let ia_verts = u64::from_le_bytes(stats_resolve_buf[0..8].try_into().unwrap());
    let ia_prims = u64::from_le_bytes(stats_resolve_buf[8..16].try_into().unwrap());
    let vs_invs = u64::from_le_bytes(stats_resolve_buf[16..24].try_into().unwrap());
    let ps_invs = u64::from_le_bytes(stats_resolve_buf[56..64].try_into().unwrap());
    let cs_invs = u64::from_le_bytes(stats_resolve_buf[80..88].try_into().unwrap());

    if ia_verts != 36 || ia_prims != 12 || vs_invs != 36 || ps_invs != 500 || cs_invs != 1024 {
        crate::serial_write_str("[KNT-QUERY-FAIL] Pipeline statistics counter mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Pipeline statistics accumulator and differential delta reporting verified\r\n");

    // 4. Verify occlusion and binary visibility culling
    if OcclusionCullingEngine::begin_query(&mut occ_heap, 0, false).is_err()
        || OcclusionCullingEngine::end_query(&mut occ_heap, 0, 150, false).is_err()
    {
        crate::serial_write_str("[KNT-QUERY-FAIL] Standard occlusion query failed\r\n");
        return false;
    }

    if OcclusionCullingEngine::begin_query(&mut occ_heap, 1, true).is_err()
        || OcclusionCullingEngine::end_query(&mut occ_heap, 1, 0, true).is_err()
    {
        crate::serial_write_str("[KNT-QUERY-FAIL] Binary occlusion query failed\r\n");
        return false;
    }

    if OcclusionCullingEngine::is_visible(&occ_heap, 0) != Ok(true)
        || OcclusionCullingEngine::is_visible(&occ_heap, 1) != Ok(false)
    {
        crate::serial_write_str("[KNT-QUERY-FAIL] Occlusion visibility evaluation mismatch\r\n");
        return false;
    }

    let candidates = [0, 1];
    let visible_instances = match OcclusionCullingEngine::filter_visible_instances(&occ_heap, &candidates) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if visible_instances != alloc::vec![0] {
        crate::serial_write_str("[KNT-QUERY-FAIL] Occlusion culling candidate filtering mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Binary occlusion culling and visibility candidate filtering verified\r\n");

    crate::serial_write_str("[KNT-D3D12-QUERY-PASS] Direct3D 12 query heap and GPU timestamp engine verified\r\n");
    true
}
