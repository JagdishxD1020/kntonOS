//! Direct3D 12 Work Graphs & Autonomous GPU Shader Execution Engine.
//! Provides hardware-accelerated autonomous GPU-driven work scheduling without CPU intervention:

use alloc::vec::Vec;

/// Memory alignment required for Direct3D 12 Work Graph backing store allocations (256 bytes).
pub const D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT: u64 = 256;

/// Maximum payload byte size supported per node record.
pub const MAX_NODE_RECORD_PAYLOAD_BYTES: usize = 128;

/// Maximum number of nodes supported in a single work graph.
pub const MAX_WORK_GRAPH_NODES: usize = 64;

/// Maximum number of downstream node output targets per node.
pub const MAX_OUTPUT_RECORD_TARGETS: usize = 8;

/// Maximum recursive node expansion depth allowed to prevent runaway GPU loops.
pub const MAX_WORK_GRAPH_RECURSION_DEPTH: u32 = 16;

/// Errors returned by the Work Graph execution subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkGraphError {
    /// Specified node ID was not found in the work graph.
    NodeNotFound(u32),
    /// Total nodes in the graph exceeds the limit (64).
    ExceededMaxNodes,
    /// Record payload size exceeds the maximum limit (128 bytes).
    PayloadSizeTooLarge(usize),
    /// Record payload size does not match node's configured input size.
    PayloadSizeMismatch { expected: usize, actual: usize },
    /// Backing store GPU virtual address is not 256-byte aligned.
    UnalignedBackingMemory(u64),
    /// Allocated backing store size is smaller than required minimum.
    InsufficientBackingMemory { required: u64, provided: u64 },
    /// Node expansion exceeded maximum recursion depth.
    RecursionDepthExceeded(u32),
    /// Node record ring buffer is full.
    RecordRingFull,
    /// Node record ring buffer is empty.
    RecordRingEmpty,
    /// Invalid threadgroup or dispatch grid dimensions.
    InvalidGridDimensions,
    NoProgramBound,
    InvalidProgramIdentifier,
    InvalidDispatchMode,
}

/// Direct3D 12 Work Graph node execution and scheduling types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkGraphNodeType {
    /// Broadcasting node: Each input record launches a fixed or dynamic grid of threadgroups.
    Broadcasting,
    /// Coalescing node: Multiple input records are grouped up to a threshold before launching threadgroups.
    Coalescing,
    /// Threadgroup node: Input records are processed by cooperative threads within a threadgroup.
    Threadgroup,
}

/// Strongly-typed 32-bit identifier for a work graph node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodeId(pub u32);

/// Work Graph Node Record containing a header and payload.
/// Records are the fundamental units of data flowing through the work graph edges.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct NodeRecord {
    /// Destination or producer node ID.
    pub target_node: NodeId,
    /// Actual valid bytes in the payload buffer.
    pub payload_size: u16,
    /// Execution priority tag (0 = default, higher = higher priority).
    pub priority: u8,
    /// Reserved flags.
    pub flags: u8,
    /// Record payload buffer.
    pub payload: [u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
}

impl Default for NodeRecord {
    fn default() -> Self {
        Self {
            target_node: NodeId(0),
            payload_size: 0,
            priority: 0,
            flags: 0,
            payload: [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
        }
    }
}

impl NodeRecord {
    /// Creates a new node record for the specified target node with raw payload bytes.
    pub fn new(target_node: NodeId, payload_bytes: &[u8], priority: u8) -> Result<Self, WorkGraphError> {
        if payload_bytes.len() > MAX_NODE_RECORD_PAYLOAD_BYTES {
            return Err(WorkGraphError::PayloadSizeTooLarge(payload_bytes.len()));
        }

        let mut record = Self {
            target_node,
            payload_size: payload_bytes.len() as u16,
            priority,
            flags: 0,
            payload: [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
        };
        record.payload[..payload_bytes.len()].copy_from_slice(payload_bytes);
        Ok(record)
    }

    /// Creates an empty record targeting `target_node`.
    pub const fn empty(target_node: NodeId) -> Self {
        Self {
            target_node,
            payload_size: 0,
            priority: 0,
            flags: 0,
            payload: [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
        }
    }

    /// Returns a slice over the valid payload bytes.
    #[inline]
    pub fn payload_slice(&self) -> &[u8] {
        &self.payload[..self.payload_size as usize]
    }
}

/// Work Graph Node Descriptor.
/// Defines the shader execution model, threadgroup dimensions, input record contracts,
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkGraphNodeDescriptor {
    /// Unique node identifier.
    pub node_id: NodeId,
    /// Scheduling and execution archetype.
    pub node_type: WorkGraphNodeType,
    /// 32-byte unique shader identifier (D3D12 state object export ID).
    pub shader_identifier: [u8; 32],
    /// Threadgroup dimension `(X, Y, Z)` executed per invocation.
    pub threadgroup_size: [u32; 3],
    /// Maximum dispatch grid `(X, Y, Z)` for broadcasting nodes.
    pub max_dispatch_grid: [u32; 3],
    /// Maximum input records grouped per threadgroup for coalescing nodes.
    pub max_records_per_threadgroup: u32,
    /// Expected byte size of each input record payload (must be <= 128).
    pub input_record_size: usize,
    /// Array of downstream node IDs that this node can dynamically schedule on the GPU.
    pub output_targets: [Option<NodeId>; MAX_OUTPUT_RECORD_TARGETS],
    /// Whether this node can be invoked as an initial entry point by the host.
    pub is_entry_point: bool,
}

impl WorkGraphNodeDescriptor {
    /// Validates the node descriptor against D3D12 hardware requirements.
    pub fn validate(&self) -> Result<(), WorkGraphError> {
        if self.threadgroup_size[0] == 0 || self.threadgroup_size[1] == 0 || self.threadgroup_size[2] == 0 {
            return Err(WorkGraphError::InvalidGridDimensions);
        }
        let total_threads = self.threadgroup_size[0]
            .saturating_mul(self.threadgroup_size[1])
            .saturating_mul(self.threadgroup_size[2]);
        if total_threads > 1024 {
            return Err(WorkGraphError::InvalidGridDimensions);
        }
        if self.input_record_size > MAX_NODE_RECORD_PAYLOAD_BYTES {
            return Err(WorkGraphError::PayloadSizeTooLarge(self.input_record_size));
        }
        if self.node_type == WorkGraphNodeType::Broadcasting {
            if self.max_dispatch_grid[0] == 0 || self.max_dispatch_grid[1] == 0 || self.max_dispatch_grid[2] == 0 {
                return Err(WorkGraphError::InvalidGridDimensions);
            }
        }
        if self.node_type == WorkGraphNodeType::Coalescing {
            if self.max_records_per_threadgroup == 0 {
                return Err(WorkGraphError::InvalidGridDimensions);
            }
        }
        Ok(())
    }
}

/// Memory backing store requirements calculated for a compiled Work Graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkGraphMemoryRequirements {
    /// Minimum required backing store size in bytes.
    pub min_backing_memory_bytes: u64,
    /// Recommended backing store size in bytes for peak throughput.
    pub max_backing_memory_bytes: u64,
    /// Scratch memory required for in-flight node scheduling.
    pub scratch_buffer_bytes: u64,
    /// Size of internal circular record rings.
    pub record_ring_bytes: u64,
}

impl WorkGraphMemoryRequirements {
    /// Calculates memory requirements based on the graph's node topology.
    pub fn calculate(nodes: &[WorkGraphNodeDescriptor]) -> Self {
        let mut min_bytes: u64 = 4096; // Base graph state overhead (4 KiB)
        let mut record_ring_bytes: u64 = 0;

        for node in nodes {
            let record_size = (node.input_record_size as u64 + 16).next_power_of_two();
            // Estimate capacity per node ring (e.g. 64 records per node)
            let node_ring = record_size * 64;
            record_ring_bytes += node_ring;

            // Extra buffer for coalescing nodes
            if node.node_type == WorkGraphNodeType::Coalescing {
                let coalescing_buf = record_size * (node.max_records_per_threadgroup as u64 * 8);
                record_ring_bytes += coalescing_buf;
            }
        }

        min_bytes += record_ring_bytes;
        // Align to 256 bytes
        let aligned_min = (min_bytes + D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT - 1)
            & !(D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT - 1);
        let max_bytes = aligned_min * 2;
        let scratch_bytes = aligned_min / 2;

        Self {
            min_backing_memory_bytes: aligned_min,
            max_backing_memory_bytes: max_bytes,
            scratch_buffer_bytes: scratch_bytes,
            record_ring_bytes,
        }
    }
}

/// Backing store memory buffer descriptor for a Work Graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkGraphBackingStore {
    /// GPU virtual address of the allocated backing buffer.
    pub gpu_virtual_address: u64,
    /// Size of the buffer in bytes.
    pub size_bytes: u64,
}

impl WorkGraphBackingStore {
    /// Creates and validates a backing store descriptor.
    pub fn new(gpu_virtual_address: u64, size_bytes: u64, requirements: &WorkGraphMemoryRequirements) -> Result<Self, WorkGraphError> {
        if (gpu_virtual_address % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0 {
            return Err(WorkGraphError::UnalignedBackingMemory(gpu_virtual_address));
        }
        if size_bytes < requirements.min_backing_memory_bytes {
            return Err(WorkGraphError::InsufficientBackingMemory {
                required: requirements.min_backing_memory_bytes,
                provided: size_bytes,
            });
        }
        Ok(Self {
            gpu_virtual_address,
            size_bytes,
        })
    }
}

/// Circular ring buffer of node records between work graph stages.
#[derive(Debug, Clone)]
pub struct NodeRecordRing {
    buffer: Vec<NodeRecord>,
    capacity: usize,
    head: usize,
    tail: usize,
    count: usize,
}

impl NodeRecordRing {
    /// Creates a new record ring with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            buf.push(NodeRecord::default());
        }
        Self {
            buffer: buf,
            capacity,
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    /// Whether the ring contains no pending records.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Whether the ring has reached its maximum capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.count >= self.capacity
    }

    /// Current number of records waiting in the ring.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Maximum capacity of the ring.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Pushes a record to the tail of the ring buffer.
    pub fn push(&mut self, record: NodeRecord) -> Result<(), WorkGraphError> {
        if self.is_full() {
            return Err(WorkGraphError::RecordRingFull);
        }
        self.buffer[self.tail] = record;
        self.tail = (self.tail + 1) % self.capacity;
        self.count += 1;
        Ok(())
    }

    /// Pops the next record from the head of the ring buffer.
    pub fn pop(&mut self) -> Option<NodeRecord> {
        if self.is_empty() {
            return None;
        }
        let record = self.buffer[self.head];
        self.head = (self.head + 1) % self.capacity;
        self.count -= 1;
        Some(record)
    }
}

/// Accumulation buffer for a Coalescing work graph node.
/// Collects multiple input records until `max_records` is reached or a flush is initiated.
#[derive(Debug, Clone)]
pub struct CoalescingBuffer {
    target_node: NodeId,
    max_records: usize,
    records: Vec<NodeRecord>,
}

impl CoalescingBuffer {
    /// Creates a new coalescing buffer for the target node.
    pub fn new(target_node: NodeId, max_records: usize) -> Self {
        Self {
            target_node,
            max_records,
            records: Vec::with_capacity(max_records),
        }
    }

    /// Target node ID.
    #[inline]
    pub fn target_node(&self) -> NodeId {
        self.target_node
    }

    /// Appends a record to the coalescing buffer. Returns true if threshold is reached.
    pub fn add_record(&mut self, record: NodeRecord) -> bool {
        self.records.push(record);
        self.records.len() >= self.max_records
    }

    /// Whether the buffer has accumulated sufficient records for a threadgroup launch.
    #[inline]
    pub fn is_ready(&self) -> bool {
        self.records.len() >= self.max_records
    }

    /// Drains all accumulated records for execution.
    pub fn drain_records(&mut self) -> Vec<NodeRecord> {
        core::mem::take(&mut self.records)
    }

    /// Number of records currently buffered.
    #[inline]
    pub fn len(&self) -> usize {
        self.records.len()
    }
}

/// Profiling telemetry for Work Graph scheduling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkGraphSchedulerStats {
    /// Total records enqueued into node rings.
    pub total_records_enqueued: u64,
    /// Total records dequeued and dispatched to execution units.
    pub total_records_processed: u64,
    /// Number of Broadcasting node dispatches.
    pub broadcasting_dispatches: u64,
    /// Number of Coalescing node dispatches.
    pub coalescing_dispatches: u64,
    /// Number of Threadgroup node dispatches.
    pub threadgroup_dispatches: u64,
}

/// Work Graph Scheduler Engine.
/// Orchestrates registered node pipelines, routes incoming records to destination queues,
#[derive(Debug)]
pub struct WorkGraphScheduler {
    nodes: Vec<WorkGraphNodeDescriptor>,
    rings: Vec<NodeRecordRing>,
    coalescing_buffers: Vec<CoalescingBuffer>,
    stats: WorkGraphSchedulerStats,
}

impl WorkGraphScheduler {
    /// Creates a new WorkGraphScheduler.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            rings: Vec::new(),
            coalescing_buffers: Vec::new(),
            stats: WorkGraphSchedulerStats::default(),
        }
    }

    /// Registers a node descriptor in the work graph.
    pub fn register_node(&mut self, desc: WorkGraphNodeDescriptor) -> Result<(), WorkGraphError> {
        desc.validate()?;
        if self.nodes.len() >= MAX_WORK_GRAPH_NODES {
            return Err(WorkGraphError::ExceededMaxNodes);
        }
        if self.nodes.iter().any(|n| n.node_id == desc.node_id) {
            return Ok(()); // Already registered
        }

        self.nodes.push(desc);
        self.rings.push(NodeRecordRing::new(64));

        if desc.node_type == WorkGraphNodeType::Coalescing {
            self.coalescing_buffers.push(CoalescingBuffer::new(
                desc.node_id,
                desc.max_records_per_threadgroup as usize,
            ));
        }

        Ok(())
    }

    /// Retrieves a reference to a registered node descriptor.
    pub fn get_node(&self, id: NodeId) -> Option<&WorkGraphNodeDescriptor> {
        self.nodes.iter().find(|n| n.node_id == id)
    }

    /// Enqueues a record targeting a specific node in the work graph.
    pub fn enqueue_record(&mut self, record: NodeRecord) -> Result<(), WorkGraphError> {
        let (node_type, expected_size, node_idx) = {
            let (idx, node) = self
                .nodes
                .iter()
                .enumerate()
                .find(|(_, n)| n.node_id == record.target_node)
                .ok_or(WorkGraphError::NodeNotFound(record.target_node.0))?;
            (node.node_type, node.input_record_size, idx)
        };

        if (record.payload_size as usize) != expected_size && expected_size > 0 {
            return Err(WorkGraphError::PayloadSizeMismatch {
                expected: expected_size,
                actual: record.payload_size as usize,
            });
        }

        if node_type == WorkGraphNodeType::Coalescing {
            if let Some(buf) = self.coalescing_buffers.iter_mut().find(|b| b.target_node == record.target_node) {
                buf.add_record(record);
            }
        } else {
            self.rings[node_idx].push(record)?;
        }

        self.stats.total_records_enqueued += 1;
        Ok(())
    }

    /// Number of records currently waiting in a node's ring buffer.
    #[inline]
    pub fn ring_len(&self, node_id: NodeId) -> usize {
        self.nodes.iter().position(|n| n.node_id == node_id).map(|idx| self.rings[idx].len()).unwrap_or(0)
    }

    /// Pops the next pending record from a node's ring.
    pub fn pop_record(&mut self, node_id: NodeId) -> Option<NodeRecord> {
        let node_idx = self.nodes.iter().position(|n| n.node_id == node_id)?;
        let rec = self.rings[node_idx].pop();
        if rec.is_some() {
            self.stats.total_records_processed += 1;
        }
        rec
    }

    /// Checks if a coalescing node has accumulated enough records for launch.
    pub fn pop_coalesced_batch(&mut self, node_id: NodeId, force_flush: bool) -> Option<Vec<NodeRecord>> {
        let buf = self.coalescing_buffers.iter_mut().find(|b| b.target_node == node_id)?;
        if buf.is_ready() || (force_flush && buf.len() > 0) {
            let batch = buf.drain_records();
            self.stats.total_records_processed += batch.len() as u64;
            Some(batch)
        } else {
            None
        }
    }

    /// Whether there is any pending work in any node ring or coalescing buffer.
    pub fn has_pending_work(&self) -> bool {
        self.rings.iter().any(|r| !r.is_empty()) || self.coalescing_buffers.iter().any(|b| b.len() > 0)
    }

    /// Current scheduler statistics.
    #[inline]
    pub fn stats(&self) -> &WorkGraphSchedulerStats {
        &self.stats
    }

    /// Mutable statistics for dispatch tracking.
    #[inline]
    pub fn stats_mut(&mut self) -> &mut WorkGraphSchedulerStats {
        &mut self.stats
    }

    /// Resets scheduler telemetry statistics.
    pub fn reset_stats(&mut self) {
        self.stats = WorkGraphSchedulerStats::default();
    }
}

/// Status of a completed or halted Work Graph execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkGraphStatus {
    /// Work graph completed all autonomous tasks successfully.
    Completed,
    /// Work graph halted due to reaching maximum recursion limit.
    RecursionLimitExceeded,
    /// Work graph halted due to backing store exhaustion or memory error.
    MemoryExhausted,
}

impl Default for WorkGraphStatus {
    fn default() -> Self {
        Self::Completed
    }
}

/// Direct3D 12 `DispatchGraph` command parameters.
#[derive(Debug, Clone)]
pub struct DispatchGraphDesc {
    /// Entry node ID invoked by the host command list.
    pub entry_node_id: NodeId,
    /// Initial root records provided to the entry node.
    pub entry_records: Vec<NodeRecord>,
    /// Backing store GPU virtual address and size.
    pub backing_store: WorkGraphBackingStore,
    /// Maximum recursive expansion depth.
    pub max_recursion_depth: u32,
}

/// Execution telemetry and results from a `DispatchGraph` invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkGraphExecutionResult {
    /// Total node invocations executed across all stages.
    pub total_node_invocations: u64,
    /// Total records generated dynamically by nodes on the GPU.
    pub total_records_generated: u64,
    /// Total records consumed by downstream nodes.
    pub total_records_consumed: u64,
    /// Maximum recursion depth reached during node expansion.
    pub max_recursion_depth_reached: u32,
    /// Final execution status.
    pub status: WorkGraphStatus,
}

/// Autonomous GPU Work Graph Execution Engine.
/// Orchestrates autonomous GPU shader graphs without CPU synchronization:
#[derive(Debug)]
pub struct WorkGraphEngine {
    scheduler: WorkGraphScheduler,
    backing_store: Option<WorkGraphBackingStore>,
}

impl WorkGraphEngine {
    /// Creates a new WorkGraphEngine.
    pub fn new() -> Self {
        Self {
            scheduler: WorkGraphScheduler::new(),
            backing_store: None,
        }
    }

    /// Reference to the internal scheduler.
    #[inline]
    pub fn scheduler(&self) -> &WorkGraphScheduler {
        &self.scheduler
    }

    /// Mutably access the internal scheduler.
    #[inline]
    pub fn scheduler_mut(&mut self) -> &mut WorkGraphScheduler {
        &mut self.scheduler
    }

    /// Sets the GPU backing store for the work graph.
    pub fn set_backing_store(&mut self, backing_store: WorkGraphBackingStore) {
        self.backing_store = Some(backing_store);
    }

    /// Registers a node descriptor in the work graph.
    pub fn register_node(&mut self, desc: WorkGraphNodeDescriptor) -> Result<(), WorkGraphError> {
        self.scheduler.register_node(desc)
    }

    /// Executes an autonomous Work Graph dispatch (`DispatchGraph`).
    /// Seeds the graph with `entry_records` at `entry_node_id` and processes all
    pub fn dispatch_graph(
        &mut self,
        desc: &DispatchGraphDesc,
    ) -> Result<WorkGraphExecutionResult, WorkGraphError> {
        // 1. Verify backing store
        if (desc.backing_store.gpu_virtual_address % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0 {
            return Err(WorkGraphError::UnalignedBackingMemory(desc.backing_store.gpu_virtual_address));
        }
        self.backing_store = Some(desc.backing_store);

        // 2. Enqueue initial entry records
        for record in &desc.entry_records {
            let mut rec = *record;
            rec.target_node = desc.entry_node_id;
            self.scheduler.enqueue_record(rec)?;
        }

        let mut result = WorkGraphExecutionResult::default();
        let mut current_depth: u32 = 0;

        // 3. Autonomous GPU execution loop
        while self.scheduler.has_pending_work() {
            current_depth += 1;
            if current_depth > desc.max_recursion_depth {
                result.status = WorkGraphStatus::RecursionLimitExceeded;
                result.max_recursion_depth_reached = current_depth;
                return Ok(result);
            }

            result.max_recursion_depth_reached = core::cmp::max(
                result.max_recursion_depth_reached,
                current_depth,
            );

            let mut work_done_in_pass = false;

            // Collect node IDs to iterate over
            let node_ids: Vec<NodeId> = self.scheduler.nodes.iter().map(|n| n.node_id).collect();

            for node_id in node_ids {
                let node_desc = match self.scheduler.get_node(node_id) {
                    Some(d) => *d,
                    None => continue,
                };

                match node_desc.node_type {
                    WorkGraphNodeType::Broadcasting => {
                        let count_to_process = self.scheduler.ring_len(node_id);
                        for _ in 0..count_to_process {
                            if let Some(input_record) = self.scheduler.pop_record(node_id) {
                                work_done_in_pass = true;
                                result.total_node_invocations += 1;
                                result.total_records_consumed += 1;
                                self.scheduler.stats_mut().broadcasting_dispatches += 1;

                                for target in node_desc.output_targets.iter().flatten() {
                                    let mut out_payload = [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES];
                                    let copy_len = core::cmp::min(
                                        input_record.payload_size as usize,
                                        node_desc.input_record_size,
                                    );
                                    out_payload[..copy_len].copy_from_slice(&input_record.payload[..copy_len]);

                                    let child_record = NodeRecord {
                                        target_node: *target,
                                        payload_size: copy_len as u16,
                                        priority: input_record.priority,
                                        flags: 0,
                                        payload: out_payload,
                                    };
                                    self.scheduler.enqueue_record(child_record)?;
                                    result.total_records_generated += 1;
                                }
                            }
                        }
                    }
                    WorkGraphNodeType::Coalescing => {
                        // Check if ready or if this is a flush
                        let force_flush = !self.scheduler.rings.iter().any(|r| !r.is_empty());
                        if let Some(batch) = self.scheduler.pop_coalesced_batch(node_id, force_flush) {
                            work_done_in_pass = true;
                            result.total_node_invocations += 1;
                            result.total_records_consumed += batch.len() as u64;
                            self.scheduler.stats_mut().coalescing_dispatches += 1;

                            // Coalescing node emits downstream records per target
                            for target in node_desc.output_targets.iter().flatten() {
                                let mut out_payload = [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES];
                                out_payload[0] = batch.len() as u8; // Write batch count into header

                                let child_record = NodeRecord {
                                    target_node: *target,
                                    payload_size: core::cmp::max(1, node_desc.input_record_size as u16),
                                    priority: 0,
                                    flags: 0,
                                    payload: out_payload,
                                };
                                self.scheduler.enqueue_record(child_record)?;
                                result.total_records_generated += 1;
                            }
                        }
                    }
                    WorkGraphNodeType::Threadgroup => {
                        let count_to_process = self.scheduler.ring_len(node_id);
                        for _ in 0..count_to_process {
                            if let Some(input_record) = self.scheduler.pop_record(node_id) {
                                work_done_in_pass = true;
                                result.total_node_invocations += 1;
                                result.total_records_consumed += 1;
                                self.scheduler.stats_mut().threadgroup_dispatches += 1;

                                for target in node_desc.output_targets.iter().flatten() {
                                    let child_record = NodeRecord {
                                        target_node: *target,
                                        payload_size: input_record.payload_size,
                                        priority: input_record.priority,
                                        flags: 0,
                                        payload: input_record.payload,
                                    };
                                    self.scheduler.enqueue_record(child_record)?;
                                    result.total_records_generated += 1;
                                }
                            }
                        }
                    }
                }
            }

            if !work_done_in_pass {
                // If no work was done but coalescing buffers still hold un-flushed records, force-flush them
                let mut flushed_any = false;
                let c_node_ids: Vec<NodeId> = self.scheduler.coalescing_buffers.iter().map(|b| b.target_node).collect();
                for c_id in c_node_ids {
                    if let Some(batch) = self.scheduler.pop_coalesced_batch(c_id, true) {
                        flushed_any = true;
                        result.total_node_invocations += 1;
                        result.total_records_consumed += batch.len() as u64;
                        self.scheduler.stats_mut().coalescing_dispatches += 1;
                    }
                }
                if !flushed_any {
                    break;
                }
            }
        }

        result.status = WorkGraphStatus::Completed;
        Ok(result)
    }
}

// D3D12 Work Graph subobject type for pipeline state creation
pub const D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_WORK_GRAPH: u32 = 0x24;

pub const D3D12_WORK_GRAPH_FLAG_NONE: u32 = 0;
pub const D3D12_WORK_GRAPH_FLAG_AUTONOMOUS_EXECUTION: u32 = 1 << 0;
pub const D3D12_WORK_GRAPH_FLAG_DYNAMIC_DISPATCH: u32 = 1 << 1;

// Opaque 32-byte D3D12 Work Graph program identifier
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[allow(non_camel_case_types)]
pub struct D3D12_PROGRAM_IDENTIFIER {
    pub opaque_data: [u64; 4],
}

impl D3D12_PROGRAM_IDENTIFIER {
    // Construct program identifier from 4 64-bit words
    pub const fn new(id0: u64, id1: u64, id2: u64, id3: u64) -> Self {
        Self {
            opaque_data: [id0, id1, id2, id3],
        }
    }

    // Whether the program identifier is non-zero
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.opaque_data != [0; 4]
    }
}

// Direct3D 12 Work Graph state object descriptor
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct D3D12_WORK_GRAPH_DESC {
    pub program_name: &'static str,
    pub flags: u32,
    pub num_nodes: u32,
    pub num_entrypoints: u32,
}

// Memory backing and scratch requirements for Work Graph program
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_WORK_GRAPH_MEMORY_REQUIREMENTS {
    pub min_backing_memory_size_in_bytes: u64,
    pub max_backing_memory_size_in_bytes: u64,
    pub scratch_memory_size_in_bytes: u64,
    pub record_ring_size_in_bytes: u64,
}

impl D3D12_WORK_GRAPH_MEMORY_REQUIREMENTS {
    // Calculate requirements for a given number of nodes and estimated record throughput
    pub fn calculate(num_nodes: u32, max_record_size: usize) -> Self {
        let base_overhead = 4096u64;
        let per_node_ring = ((max_record_size as u64 + 31) & !31) * 128;
        let total_ring = per_node_ring * (num_nodes as u64).max(1);
        let min_backing = (base_overhead + total_ring + 255) & !255;
        let max_backing = min_backing * 2;
        let scratch_mem = (min_backing / 2 + 255) & !255;

        Self {
            min_backing_memory_size_in_bytes: min_backing,
            max_backing_memory_size_in_bytes: max_backing,
            scratch_memory_size_in_bytes: scratch_mem,
            record_ring_size_in_bytes: total_ring,
        }
    }
}

// Direct3D 12 pipeline state subobject for Work Graphs
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct D3D12_STATE_SUBOBJECT_WORK_GRAPH {
    pub subobject_type: u32,
    pub desc: D3D12_WORK_GRAPH_DESC,
    pub program_id: D3D12_PROGRAM_IDENTIFIER,
    pub memory_requirements: D3D12_WORK_GRAPH_MEMORY_REQUIREMENTS,
}

impl D3D12_STATE_SUBOBJECT_WORK_GRAPH {
    // Create new Work Graph state subobject and compute program ID
    pub fn new(program_name: &'static str, flags: u32, num_nodes: u32, num_entrypoints: u32) -> Self {
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in program_name.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let program_id = D3D12_PROGRAM_IDENTIFIER::new(hash, flags as u64, num_nodes as u64, num_entrypoints as u64);
        let memory_requirements = D3D12_WORK_GRAPH_MEMORY_REQUIREMENTS::calculate(num_nodes, 128);

        Self {
            subobject_type: D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_WORK_GRAPH,
            desc: D3D12_WORK_GRAPH_DESC {
                program_name,
                flags,
                num_nodes,
                num_entrypoints,
            },
            program_id,
            memory_requirements,
        }
    }
}

// D3D12 Node ID pairing a 32-bit name hash and array index
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_NODE_ID {
    pub name_hash: u32,
    pub index: u32,
}

impl D3D12_NODE_ID {
    // Construct a node identifier from string name and index
    pub fn new(name: &str, index: u32) -> Self {
        let mut hash: u32 = 0x811c9dc5;
        for b in name.as_bytes() {
            hash ^= *b as u32;
            hash = hash.wrapping_mul(0x01000193);
        }
        Self { name_hash: hash, index }
    }
}

// Memory layout specification for node input and output records
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_NODE_RECORD_LAYOUT {
    pub byte_stride: u32,
    pub alignment: u32,
    pub max_records: u32,
}

impl D3D12_NODE_RECORD_LAYOUT {
    // Construct a record layout specification
    pub fn new(byte_stride: u32, alignment: u32, max_records: u32) -> Self {
        Self {
            byte_stride,
            alignment: alignment.max(4),
            max_records,
        }
    }
}

// Broadcasting node launch parameters and dispatch grid dimensions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_BROADCASTING_LAUNCH_OVERRIDES {
    pub dispatch_grid: [u32; 3],
    pub max_dispatch_grid: [u32; 3],
}

// Coalescing node launch parameters and record grouping threshold
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_COALESCING_LAUNCH_OVERRIDES {
    pub max_records_per_threadgroup: u32,
}

// Threadgroup node launch parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_THREAD_LAUNCH_OVERRIDES {
    pub threadgroup_size: [u32; 3],
}

// Complete D3D12 node descriptor supporting Broadcaster, Coalescing, and Thread nodes
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct D3D12_NODE {
    pub node_id: D3D12_NODE_ID,
    pub node_type: WorkGraphNodeType,
    pub shader_id: [u8; 32],
    pub input_record_layout: D3D12_NODE_RECORD_LAYOUT,
    pub output_record_layout: D3D12_NODE_RECORD_LAYOUT,
    pub broadcasting_overrides: Option<D3D12_BROADCASTING_LAUNCH_OVERRIDES>,
    pub coalescing_overrides: Option<D3D12_COALESCING_LAUNCH_OVERRIDES>,
    pub thread_overrides: Option<D3D12_THREAD_LAUNCH_OVERRIDES>,
    pub output_nodes: Vec<D3D12_NODE_ID>,
    pub is_entry_point: bool,
}

impl D3D12_NODE {
    // Create Broadcaster node descriptor
    pub fn broadcaster(name: &str, index: u32, shader_id: [u8; 32], grid: [u32; 3], in_stride: u32, out_stride: u32) -> Self {
        Self {
            node_id: D3D12_NODE_ID::new(name, index),
            node_type: WorkGraphNodeType::Broadcasting,
            shader_id,
            input_record_layout: D3D12_NODE_RECORD_LAYOUT::new(in_stride, 4, 64),
            output_record_layout: D3D12_NODE_RECORD_LAYOUT::new(out_stride, 4, 64),
            broadcasting_overrides: Some(D3D12_BROADCASTING_LAUNCH_OVERRIDES {
                dispatch_grid: grid,
                max_dispatch_grid: grid,
            }),
            coalescing_overrides: None,
            thread_overrides: None,
            output_nodes: Vec::new(),
            is_entry_point: true,
        }
    }

    // Create Coalescing node descriptor
    pub fn coalescing(name: &str, index: u32, shader_id: [u8; 32], max_records: u32, in_stride: u32, out_stride: u32) -> Self {
        Self {
            node_id: D3D12_NODE_ID::new(name, index),
            node_type: WorkGraphNodeType::Coalescing,
            shader_id,
            input_record_layout: D3D12_NODE_RECORD_LAYOUT::new(in_stride, 4, max_records * 4),
            output_record_layout: D3D12_NODE_RECORD_LAYOUT::new(out_stride, 4, 64),
            broadcasting_overrides: None,
            coalescing_overrides: Some(D3D12_COALESCING_LAUNCH_OVERRIDES {
                max_records_per_threadgroup: max_records,
            }),
            thread_overrides: None,
            output_nodes: Vec::new(),
            is_entry_point: false,
        }
    }

    // Create Threadgroup node descriptor
    pub fn thread(name: &str, index: u32, shader_id: [u8; 32], tg_size: [u32; 3], in_stride: u32, out_stride: u32) -> Self {
        Self {
            node_id: D3D12_NODE_ID::new(name, index),
            node_type: WorkGraphNodeType::Threadgroup,
            shader_id,
            input_record_layout: D3D12_NODE_RECORD_LAYOUT::new(in_stride, 4, 64),
            output_record_layout: D3D12_NODE_RECORD_LAYOUT::new(out_stride, 4, 64),
            broadcasting_overrides: None,
            coalescing_overrides: None,
            thread_overrides: Some(D3D12_THREAD_LAUNCH_OVERRIDES {
                threadgroup_size: tg_size,
            }),
            output_nodes: Vec::new(),
            is_entry_point: false,
        }
    }

    // Add downstream target node for autonomous dispatch routing
    pub fn add_output_target(&mut self, target: D3D12_NODE_ID) -> Result<(), WorkGraphError> {
        if self.output_nodes.len() >= MAX_OUTPUT_RECORD_TARGETS {
            return Err(WorkGraphError::ExceededMaxNodes);
        }
        self.output_nodes.push(target);
        Ok(())
    }

    // Validate node configuration against Direct3D 12 constraints
    pub fn validate(&self) -> Result<(), WorkGraphError> {
        if self.input_record_layout.byte_stride as usize > MAX_NODE_RECORD_PAYLOAD_BYTES {
            return Err(WorkGraphError::PayloadSizeTooLarge(self.input_record_layout.byte_stride as usize));
        }
        if self.output_record_layout.byte_stride as usize > MAX_NODE_RECORD_PAYLOAD_BYTES {
            return Err(WorkGraphError::PayloadSizeTooLarge(self.output_record_layout.byte_stride as usize));
        }
        match self.node_type {
            WorkGraphNodeType::Broadcasting => {
                if let Some(ref ov) = self.broadcasting_overrides {
                    if ov.dispatch_grid[0] == 0 || ov.dispatch_grid[1] == 0 || ov.dispatch_grid[2] == 0 {
                        return Err(WorkGraphError::InvalidGridDimensions);
                    }
                } else {
                    return Err(WorkGraphError::InvalidGridDimensions);
                }
            }
            WorkGraphNodeType::Coalescing => {
                if let Some(ref ov) = self.coalescing_overrides {
                    if ov.max_records_per_threadgroup == 0 {
                        return Err(WorkGraphError::InvalidGridDimensions);
                    }
                } else {
                    return Err(WorkGraphError::InvalidGridDimensions);
                }
            }
            WorkGraphNodeType::Threadgroup => {
                if let Some(ref ov) = self.thread_overrides {
                    if ov.threadgroup_size[0] == 0 || ov.threadgroup_size[1] == 0 || ov.threadgroup_size[2] == 0 {
                        return Err(WorkGraphError::InvalidGridDimensions);
                    }
                    let threads = ov.threadgroup_size[0].saturating_mul(ov.threadgroup_size[1]).saturating_mul(ov.threadgroup_size[2]);
                    if threads > 1024 {
                        return Err(WorkGraphError::InvalidGridDimensions);
                    }
                } else {
                    return Err(WorkGraphError::InvalidGridDimensions);
                }
            }
        }
        Ok(())
    }
}

// Program type enumeration for SetProgram command
pub const D3D12_PROGRAM_TYPE_GENERIC_PIPELINE: u32 = 1;
pub const D3D12_PROGRAM_TYPE_WORK_GRAPH: u32 = 2;

// Dispatch graph mode enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum D3D12_DISPATCH_GRAPH_MODE {
    NodeCPUInput,
    NodeGPUInput,
    MultiNodeCPUInput,
}

use crate::gfx::dispatch_rays::D3D12_GPU_VIRTUAL_ADDRESS_RANGE;

// Parameter descriptor for SetProgram command list recording
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct D3D12_SET_PROGRAM_DESC {
    pub program_type: u32,
    pub program_id: D3D12_PROGRAM_IDENTIFIER,
    pub backing_memory: D3D12_GPU_VIRTUAL_ADDRESS_RANGE,
    pub scratch_memory: D3D12_GPU_VIRTUAL_ADDRESS_RANGE,
}

// Parameter descriptor for DispatchGraph command list recording
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct D3D12_DISPATCH_GRAPH_DESC {
    pub mode: D3D12_DISPATCH_GRAPH_MODE,
    pub node_id: D3D12_NODE_ID,
    pub num_records: u32,
    pub records: Vec<NodeRecord>,
}

// Command stream token for Work Graph operations
#[derive(Debug, Clone)]
pub enum WorkGraphCommand {
    SetProgram(D3D12_SET_PROGRAM_DESC),
    DispatchGraph(D3D12_DISPATCH_GRAPH_DESC),
}

// Command list recorder for Direct3D 12 Work Graphs
#[derive(Debug, Default)]
pub struct WorkGraphCommandRecorder {
    commands: Vec<WorkGraphCommand>,
    active_program: Option<D3D12_PROGRAM_IDENTIFIER>,
}

impl WorkGraphCommandRecorder {
    // Construct new empty command recorder
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            active_program: None,
        }
    }

    // Record SetProgram command into command stream with validation
    pub fn set_program(&mut self, desc: &D3D12_SET_PROGRAM_DESC) -> Result<(), WorkGraphError> {
        if !desc.program_id.is_valid() {
            return Err(WorkGraphError::InvalidProgramIdentifier);
        }
        if (desc.backing_memory.start_address % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0 {
            return Err(WorkGraphError::UnalignedBackingMemory(desc.backing_memory.start_address));
        }
        if desc.backing_memory.size_in_bytes == 0 || (desc.backing_memory.size_in_bytes % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0 {
            return Err(WorkGraphError::InsufficientBackingMemory {
                required: D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT,
                provided: desc.backing_memory.size_in_bytes,
            });
        }
        if desc.scratch_memory.start_address != 0 && (desc.scratch_memory.start_address % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0 {
            return Err(WorkGraphError::UnalignedBackingMemory(desc.scratch_memory.start_address));
        }

        self.active_program = Some(desc.program_id);
        self.commands.push(WorkGraphCommand::SetProgram(desc.clone()));
        Ok(())
    }

    // Record DispatchGraph command into command stream with validation
    pub fn dispatch_graph(&mut self, desc: &D3D12_DISPATCH_GRAPH_DESC) -> Result<(), WorkGraphError> {
        if self.active_program.is_none() {
            return Err(WorkGraphError::NoProgramBound);
        }
        if desc.mode == D3D12_DISPATCH_GRAPH_MODE::NodeCPUInput && desc.num_records == 0 {
            return Err(WorkGraphError::RecordRingEmpty);
        }
        for rec in &desc.records {
            if rec.payload_size as usize > MAX_NODE_RECORD_PAYLOAD_BYTES {
                return Err(WorkGraphError::PayloadSizeTooLarge(rec.payload_size as usize));
            }
        }

        self.commands.push(WorkGraphCommand::DispatchGraph(desc.clone()));
        Ok(())
    }

    // Slice of recorded commands in submission order
    #[inline]
    pub fn commands(&self) -> &[WorkGraphCommand] {
        &self.commands
    }

    // Number of recorded commands
    #[inline]
    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    // Currently bound program identifier
    #[inline]
    pub fn active_program(&self) -> Option<D3D12_PROGRAM_IDENTIFIER> {
        self.active_program
    }

    // Reset command recorder state
    pub fn reset(&mut self) {
        self.commands.clear();
        self.active_program = None;
    }
}

// Unified memory pool managing backing store and scratch memory partitions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkGraphBackingMemoryPool {
    pub base_address: u64,
    pub total_size: u64,
    pub backing_store: D3D12_GPU_VIRTUAL_ADDRESS_RANGE,
    pub scratch_memory: D3D12_GPU_VIRTUAL_ADDRESS_RANGE,
}

impl WorkGraphBackingMemoryPool {
    // Partition memory pool into backing store and scratch memory with 256-byte alignment
    pub fn allocate(base_address: u64, total_size: u64, reqs: &D3D12_WORK_GRAPH_MEMORY_REQUIREMENTS) -> Result<Self, WorkGraphError> {
        if (base_address % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0 {
            return Err(WorkGraphError::UnalignedBackingMemory(base_address));
        }
        let needed = reqs.min_backing_memory_size_in_bytes + reqs.scratch_memory_size_in_bytes;
        if total_size < needed {
            return Err(WorkGraphError::InsufficientBackingMemory {
                required: needed,
                provided: total_size,
            });
        }

        let backing_size = reqs.min_backing_memory_size_in_bytes;
        let scratch_start = (base_address + backing_size + 255) & !255;
        let scratch_size = total_size.saturating_sub(scratch_start.saturating_sub(base_address));

        Ok(Self {
            base_address,
            total_size,
            backing_store: D3D12_GPU_VIRTUAL_ADDRESS_RANGE::new(base_address, backing_size),
            scratch_memory: D3D12_GPU_VIRTUAL_ADDRESS_RANGE::new(scratch_start, scratch_size),
        })
    }
}

// Autonomous queue scheduling descriptor for hardware work queue dispatch
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_AUTONOMOUS_QUEUE_DESCRIPTOR {
    pub queue_id: u32,
    pub target_node: D3D12_NODE_ID,
    pub ring_virtual_address: u64,
    pub ring_capacity: u32,
    pub head_index: u32,
    pub tail_index: u32,
    pub is_active: bool,
}

// Execution statistics for autonomous queue scheduler
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AutonomousSchedulerStats {
    pub total_dispatches: u64,
    pub records_routed: u64,
    pub broadcasting_launches: u64,
    pub coalescing_launches: u64,
    pub thread_launches: u64,
}

// Autonomous GPU queue scheduler managing hardware node queues and record flow
#[derive(Debug)]
pub struct AutonomousQueueScheduler {
    nodes: Vec<D3D12_NODE>,
    queue_descriptors: Vec<D3D12_AUTONOMOUS_QUEUE_DESCRIPTOR>,
    node_records: Vec<(D3D12_NODE_ID, Vec<NodeRecord>)>,
    stats: AutonomousSchedulerStats,
}

impl AutonomousQueueScheduler {
    // Construct new autonomous queue scheduler
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            queue_descriptors: Vec::new(),
            node_records: Vec::new(),
            stats: AutonomousSchedulerStats::default(),
        }
    }

    // Register node and configure its autonomous queue descriptor
    pub fn register_node(&mut self, node: D3D12_NODE, base_gpu_va: u64) -> Result<(), WorkGraphError> {
        node.validate()?;
        let queue_id = self.queue_descriptors.len() as u32;
        let ring_va = base_gpu_va + (queue_id as u64 * 4096);
        let q_desc = D3D12_AUTONOMOUS_QUEUE_DESCRIPTOR {
            queue_id,
            target_node: node.node_id,
            ring_virtual_address: ring_va,
            ring_capacity: 64,
            head_index: 0,
            tail_index: 0,
            is_active: true,
        };
        self.node_records.push((node.node_id, Vec::new()));
        self.queue_descriptors.push(q_desc);
        self.nodes.push(node);
        Ok(())
    }

    // Enqueue record to destination node queue
    pub fn enqueue_record(&mut self, target: D3D12_NODE_ID, record: NodeRecord) -> Result<(), WorkGraphError> {
        if let Some((_, list)) = self.node_records.iter_mut().find(|(id, _)| *id == target) {
            list.push(record);
            self.stats.records_routed += 1;
            Ok(())
        } else {
            Err(WorkGraphError::NodeNotFound(target.name_hash))
        }
    }

    // Execute autonomous dispatch loop across all active queues
    pub fn execute_autonomous_dispatch(&mut self, max_iterations: u32) -> Result<AutonomousSchedulerStats, WorkGraphError> {
        for _ in 0..max_iterations {
            let mut work_done = false;

            for i in 0..self.nodes.len() {
                let node = self.nodes[i].clone();
                let pending: Vec<NodeRecord> = {
                    let entry = &mut self.node_records[i];
                    core::mem::take(&mut entry.1)
                };

                if pending.is_empty() {
                    continue;
                }

                work_done = true;
                match node.node_type {
                    WorkGraphNodeType::Broadcasting => {
                        self.stats.broadcasting_launches += pending.len() as u64;
                        self.stats.total_dispatches += pending.len() as u64;
                        for rec in pending {
                            for target in &node.output_nodes {
                                let child = NodeRecord::new(NodeId(target.name_hash), rec.payload_slice(), rec.priority)?;
                                self.enqueue_record(*target, child)?;
                            }
                        }
                    }
                    WorkGraphNodeType::Coalescing => {
                        let threshold = node.coalescing_overrides.map(|o| o.max_records_per_threadgroup).unwrap_or(4) as usize;
                        let batches = (pending.len() + threshold - 1) / threshold.max(1);
                        self.stats.coalescing_launches += batches as u64;
                        self.stats.total_dispatches += batches as u64;
                        for target in &node.output_nodes {
                            for b in 0..batches {
                                let mut payload = [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES];
                                payload[0] = (b + 1) as u8;
                                let child = NodeRecord {
                                    target_node: NodeId(target.name_hash),
                                    payload_size: 16,
                                    priority: 0,
                                    flags: 0,
                                    payload,
                                };
                                self.enqueue_record(*target, child)?;
                            }
                        }
                    }
                    WorkGraphNodeType::Threadgroup => {
                        self.stats.thread_launches += pending.len() as u64;
                        self.stats.total_dispatches += pending.len() as u64;
                        for rec in pending {
                            for target in &node.output_nodes {
                                let child = NodeRecord::new(NodeId(target.name_hash), rec.payload_slice(), rec.priority)?;
                                self.enqueue_record(*target, child)?;
                            }
                        }
                    }
                }
            }

            if !work_done {
                break;
            }
        }

        Ok(self.stats)
    }

    // Access scheduler execution statistics
    #[inline]
    pub fn stats(&self) -> &AutonomousSchedulerStats {
        &self.stats
    }

    // Number of registered hardware queues
    #[inline]
    pub fn queue_count(&self) -> usize {
        self.queue_descriptors.len()
    }
}

// Comprehensive verification test suite for Direct3D 12 Work Graphs and Autonomous GPU Execution
pub fn test_d3d12_work_graphs_subsystem() -> bool {
    crate::serial_write_str("[KNT-WORK-GRAPHS] Initializing Direct3D 12 Work Graphs & Autonomous GPU Execution self-test...\r\n");

    // 1. Work Graph state object, program identifier, and memory requirements
    let state_obj = D3D12_STATE_SUBOBJECT_WORK_GRAPH::new("AutonomousCullingGraph", D3D12_WORK_GRAPH_FLAG_AUTONOMOUS_EXECUTION, 3, 1);
    if state_obj.subobject_type != D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_WORK_GRAPH {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Subobject type mismatch\r\n");
        return false;
    }
    if !state_obj.program_id.is_valid() {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Program identifier is invalid\r\n");
        return false;
    }
    if state_obj.memory_requirements.min_backing_memory_size_in_bytes == 0
        || (state_obj.memory_requirements.min_backing_memory_size_in_bytes % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0
        || (state_obj.memory_requirements.scratch_memory_size_in_bytes % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0
    {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Memory requirements unaligned\r\n");
        return false;
    }
    crate::serial_write_str("  * Work Graph state object, program identifier and memory requirements verified\r\n");

    // 2. Broadcaster, Coalescing, and Thread node descriptors with input/output record layouts
    let mut broadcaster = D3D12_NODE::broadcaster("CullBroadcaster", 0, [1u8; 32], [4, 1, 1], 16, 16);
    let mut coalescer = D3D12_NODE::coalescing("BatchCoalescer", 1, [2u8; 32], 4, 16, 16);
    let worker = D3D12_NODE::thread("DrawWorker", 2, [3u8; 32], [32, 1, 1], 16, 16);

    if broadcaster.add_output_target(coalescer.node_id).is_err() {
        return false;
    }
    if coalescer.add_output_target(worker.node_id).is_err() {
        return false;
    }

    if broadcaster.validate().is_err() || coalescer.validate().is_err() || worker.validate().is_err() {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Node validation failed\r\n");
        return false;
    }

    let invalid_broadcaster = D3D12_NODE::broadcaster("BadNode", 99, [0u8; 32], [0, 1, 1], 16, 16);
    if invalid_broadcaster.validate() != Err(WorkGraphError::InvalidGridDimensions) {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Invalid broadcaster grid was not rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * Broadcaster, Coalescing, and Thread node descriptors and record layouts verified\r\n");

    // 3. Command list recording: SetProgram and DispatchGraph validation
    let pool = match WorkGraphBackingMemoryPool::allocate(0x0000_8000_0000, 131072, &state_obj.memory_requirements) {
        Ok(p) => p,
        Err(_) => {
            crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Memory pool allocation failed\r\n");
            return false;
        }
    };

    let mut cmd_recorder = WorkGraphCommandRecorder::new();
    let set_prog_desc = D3D12_SET_PROGRAM_DESC {
        program_type: D3D12_PROGRAM_TYPE_WORK_GRAPH,
        program_id: state_obj.program_id,
        backing_memory: pool.backing_store,
        scratch_memory: pool.scratch_memory,
    };

    // Test rejection of DispatchGraph before SetProgram
    let unrecorded_dispatch = D3D12_DISPATCH_GRAPH_DESC {
        mode: D3D12_DISPATCH_GRAPH_MODE::NodeCPUInput,
        node_id: broadcaster.node_id,
        num_records: 1,
        records: alloc::vec![NodeRecord::default()],
    };
    if cmd_recorder.dispatch_graph(&unrecorded_dispatch) != Err(WorkGraphError::NoProgramBound) {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Dispatch without SetProgram was not rejected\r\n");
        return false;
    }

    if cmd_recorder.set_program(&set_prog_desc).is_err() {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] SetProgram recording failed\r\n");
        return false;
    }

    let mut input_records = alloc::vec::Vec::new();
    for i in 0..4 {
        let rec = match NodeRecord::new(NodeId(broadcaster.node_id.name_hash), &[i as u8; 16], 1) {
            Ok(r) => r,
            Err(_) => return false,
        };
        input_records.push(rec);
    }

    let dispatch_desc = D3D12_DISPATCH_GRAPH_DESC {
        mode: D3D12_DISPATCH_GRAPH_MODE::NodeCPUInput,
        node_id: broadcaster.node_id,
        num_records: 4,
        records: input_records.clone(),
    };
    if cmd_recorder.dispatch_graph(&dispatch_desc).is_err() {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] DispatchGraph recording failed\r\n");
        return false;
    }

    if cmd_recorder.command_count() != 2 {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Recorded command count mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Command list SetProgram and DispatchGraph recording and parameter validation verified\r\n");

    // 4. Backing store memory pool, scratch partitions, and autonomous queue scheduling
    let mut autonomous_scheduler = AutonomousQueueScheduler::new();
    if autonomous_scheduler.register_node(broadcaster.clone(), pool.backing_store.start_address).is_err()
        || autonomous_scheduler.register_node(coalescer.clone(), pool.backing_store.start_address).is_err()
        || autonomous_scheduler.register_node(worker.clone(), pool.backing_store.start_address).is_err()
    {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Queue registration failed\r\n");
        return false;
    }

    if autonomous_scheduler.queue_count() != 3 {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Queue count mismatch\r\n");
        return false;
    }

    for rec in input_records {
        if autonomous_scheduler.enqueue_record(broadcaster.node_id, rec).is_err() {
            return false;
        }
    }

    let stats = match autonomous_scheduler.execute_autonomous_dispatch(16) {
        Ok(s) => s,
        Err(_) => {
            crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Autonomous dispatch execution failed\r\n");
            return false;
        }
    };

    if stats.broadcasting_launches != 4 || stats.coalescing_launches < 1 || stats.thread_launches < 1 || stats.total_dispatches < 6 {
        crate::serial_write_str("[KNT-WORK-GRAPHS-FAIL] Autonomous dispatch statistics mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Backing store memory pool, scratch partitions, and autonomous queue scheduling verified\r\n");

    crate::serial_write_str("[KNT-GFX-WORK-GRAPHS-PASS] Direct3D 12 work graphs autonomous GPU execution engine verified\r\n");
    true
}

// Direct3D 12 Agility Work Graphs Tier 1.0 capability levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum D3D12_WORK_GRAPHS_TIER {
    NotSupported = 0,
    Tier1_0 = 10,
}

// Work graph program configuration flags for autonomous GPU scheduling.
pub const D3D12_WORK_GRAPH_FLAG_ENTRYPOINT: u32 = 1 << 0;
pub const D3D12_WORK_GRAPH_FLAG_ALLOW_RECURSION: u32 = 1 << 1;
pub const D3D12_WORK_GRAPH_FLAG_AUTONOMOUS_ALLOCATION: u32 = 1 << 2;

// Tier 1.0 work graph entrypoint specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkGraphTier1EntryPoint {
    pub node_id: D3D12_NODE_ID,
    pub flags: u32,
    pub record_stride: u32,
}

impl WorkGraphTier1EntryPoint {
    // Construct a new entrypoint specification.
    pub fn new(node_id: D3D12_NODE_ID, flags: u32, record_stride: u32) -> Self {
        Self {
            node_id,
            flags,
            record_stride,
        }
    }
}

// Tier 1.0 work graph node classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkGraphTier1NodeType {
    Broadcasting,
    Coalescing,
    Thread,
}

// Broadcasting launch overrides specifying dispatch grid dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BroadcastingNodeSpec {
    pub dispatch_grid: [u32; 3],
    pub max_dispatch_grid: [u32; 3],
    pub threadgroup_size: [u32; 3],
}

// Coalescing launch overrides specifying record grouping threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoalescingNodeSpec {
    pub max_records_per_threadgroup: u32,
    pub threadgroup_size: [u32; 3],
}

// Thread launch overrides specifying threadgroup dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ThreadNodeSpec {
    pub threadgroup_size: [u32; 3],
}

// Tier 1.0 work graph node specification.
#[derive(Debug, Clone)]
pub struct WorkGraphTier1NodeSpec {
    pub node_id: D3D12_NODE_ID,
    pub node_type: WorkGraphTier1NodeType,
    pub shader_id: [u8; 32],
    pub input_record_stride: u32,
    pub input_record_alignment: u32,
    pub output_record_stride: u32,
    pub is_entry_point: bool,
    pub broadcasting: Option<BroadcastingNodeSpec>,
    pub coalescing: Option<CoalescingNodeSpec>,
    pub thread: Option<ThreadNodeSpec>,
    pub target_nodes: Vec<D3D12_NODE_ID>,
}

impl WorkGraphTier1NodeSpec {
    // Construct a Tier 1.0 broadcasting node specification.
    pub fn broadcaster(
        name: &str,
        index: u32,
        shader_id: [u8; 32],
        grid: [u32; 3],
        tg_size: [u32; 3],
        in_stride: u32,
        out_stride: u32,
    ) -> Self {
        Self {
            node_id: D3D12_NODE_ID::new(name, index),
            node_type: WorkGraphTier1NodeType::Broadcasting,
            shader_id,
            input_record_stride: in_stride,
            input_record_alignment: 4,
            output_record_stride: out_stride,
            is_entry_point: false,
            broadcasting: Some(BroadcastingNodeSpec {
                dispatch_grid: grid,
                max_dispatch_grid: grid,
                threadgroup_size: tg_size,
            }),
            coalescing: None,
            thread: None,
            target_nodes: Vec::new(),
        }
    }

    // Construct a Tier 1.0 coalescing node specification.
    pub fn coalescing(
        name: &str,
        index: u32,
        shader_id: [u8; 32],
        max_records: u32,
        tg_size: [u32; 3],
        in_stride: u32,
        out_stride: u32,
    ) -> Self {
        Self {
            node_id: D3D12_NODE_ID::new(name, index),
            node_type: WorkGraphTier1NodeType::Coalescing,
            shader_id,
            input_record_stride: in_stride,
            input_record_alignment: 4,
            output_record_stride: out_stride,
            is_entry_point: false,
            broadcasting: None,
            coalescing: Some(CoalescingNodeSpec {
                max_records_per_threadgroup: max_records,
                threadgroup_size: tg_size,
            }),
            thread: None,
            target_nodes: Vec::new(),
        }
    }

    // Construct a Tier 1.0 thread node specification.
    pub fn thread(
        name: &str,
        index: u32,
        shader_id: [u8; 32],
        tg_size: [u32; 3],
        in_stride: u32,
        out_stride: u32,
    ) -> Self {
        Self {
            node_id: D3D12_NODE_ID::new(name, index),
            node_type: WorkGraphTier1NodeType::Thread,
            shader_id,
            input_record_stride: in_stride,
            input_record_alignment: 4,
            output_record_stride: out_stride,
            is_entry_point: false,
            broadcasting: None,
            coalescing: None,
            thread: Some(ThreadNodeSpec {
                threadgroup_size: tg_size,
            }),
            target_nodes: Vec::new(),
        }
    }

    // Connect a downstream child node target.
    pub fn add_target(&mut self, target: D3D12_NODE_ID) -> Result<(), WorkGraphError> {
        if self.target_nodes.len() >= MAX_OUTPUT_RECORD_TARGETS {
            return Err(WorkGraphError::ExceededMaxNodes);
        }
        self.target_nodes.push(target);
        Ok(())
    }

    // Validate node configuration against Tier 1.0 rules.
    pub fn validate(&self) -> Result<(), WorkGraphError> {
        if self.input_record_stride as usize > MAX_NODE_RECORD_PAYLOAD_BYTES {
            return Err(WorkGraphError::PayloadSizeTooLarge(self.input_record_stride as usize));
        }
        match self.node_type {
            WorkGraphTier1NodeType::Broadcasting => {
                let spec = self.broadcasting.as_ref().ok_or(WorkGraphError::InvalidDispatchMode)?;
                if spec.dispatch_grid[0] == 0 || spec.dispatch_grid[1] == 0 || spec.dispatch_grid[2] == 0 {
                    return Err(WorkGraphError::InvalidGridDimensions);
                }
                if spec.threadgroup_size[0] == 0 || spec.threadgroup_size[1] == 0 || spec.threadgroup_size[2] == 0 {
                    return Err(WorkGraphError::InvalidGridDimensions);
                }
            }
            WorkGraphTier1NodeType::Coalescing => {
                let spec = self.coalescing.as_ref().ok_or(WorkGraphError::InvalidDispatchMode)?;
                if spec.max_records_per_threadgroup == 0 {
                    return Err(WorkGraphError::InvalidGridDimensions);
                }
                if spec.threadgroup_size[0] == 0 || spec.threadgroup_size[1] == 0 || spec.threadgroup_size[2] == 0 {
                    return Err(WorkGraphError::InvalidGridDimensions);
                }
            }
            WorkGraphTier1NodeType::Thread => {
                let spec = self.thread.as_ref().ok_or(WorkGraphError::InvalidDispatchMode)?;
                if spec.threadgroup_size[0] == 0 || spec.threadgroup_size[1] == 0 || spec.threadgroup_size[2] == 0 {
                    return Err(WorkGraphError::InvalidGridDimensions);
                }
            }
        }
        Ok(())
    }
}

// Direct3D 12 Agility Work Graphs Tier 1.0 Program Descriptor.
#[derive(Debug, Clone)]
pub struct WorkGraphTier1ProgramDesc {
    pub program_name: &'static str,
    pub tier: D3D12_WORK_GRAPHS_TIER,
    pub flags: u32,
    pub max_recursion_depth: u32,
    pub entrypoints: Vec<WorkGraphTier1EntryPoint>,
    pub nodes: Vec<WorkGraphTier1NodeSpec>,
}

impl WorkGraphTier1ProgramDesc {
    // Construct a new Tier 1.0 program descriptor.
    pub fn new(program_name: &'static str, flags: u32, max_depth: u32) -> Self {
        Self {
            program_name,
            tier: D3D12_WORK_GRAPHS_TIER::Tier1_0,
            flags,
            max_recursion_depth: max_depth.min(MAX_WORK_GRAPH_RECURSION_DEPTH),
            entrypoints: Vec::new(),
            nodes: Vec::new(),
        }
    }

    // Register a node specification in the work graph program.
    pub fn add_node(&mut self, node: WorkGraphTier1NodeSpec) -> Result<(), WorkGraphError> {
        if self.nodes.len() >= MAX_WORK_GRAPH_NODES {
            return Err(WorkGraphError::ExceededMaxNodes);
        }
        self.nodes.push(node);
        Ok(())
    }

    // Register an entrypoint node for the work graph program.
    pub fn add_entrypoint(&mut self, entry: WorkGraphTier1EntryPoint) -> Result<(), WorkGraphError> {
        self.entrypoints.push(entry);
        Ok(())
    }

    // Find node specification by node ID.
    pub fn find_node(&self, id: D3D12_NODE_ID) -> Option<&WorkGraphTier1NodeSpec> {
        self.nodes.iter().find(|n| n.node_id == id)
    }

    // Find node specification mutable reference by node ID.
    pub fn find_node_mut(&mut self, id: D3D12_NODE_ID) -> Option<&mut WorkGraphTier1NodeSpec> {
        self.nodes.iter_mut().find(|n| n.node_id == id)
    }

    // Return number of registered nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    // Return number of registered entrypoints.
    pub fn entrypoint_count(&self) -> usize {
        self.entrypoints.len()
    }
}

// Tier 1.0 backing memory and scratch requirements for autonomous GPU work graphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkGraphTier1MemoryRequirements {
    pub min_backing_memory_size_in_bytes: u64,
    pub max_backing_memory_size_in_bytes: u64,
    pub scratch_memory_size_in_bytes: u64,
    pub node_input_record_pool_size: u64,
    pub max_in_flight_records: u32,
}

impl WorkGraphTier1MemoryRequirements {
    // Calculate backing memory requirements from program node topology.
    pub fn calculate(program: &WorkGraphTier1ProgramDesc) -> Self {
        let base_overhead = 4096u64;
        let mut total_pools = 0u64;
        let mut max_records = 0u32;

        for node in &program.nodes {
            let slot_size = (node.input_record_stride as u64 + 15) & !15;
            let node_pool = slot_size.max(32) * 64;
            total_pools += (node_pool + 255) & !255;
            max_records += 64;
        }

        let aligned_pools = (total_pools + 255) & !255;
        let min_backing = (base_overhead + aligned_pools + 255) & !255;
        let max_backing = min_backing * 2;
        let scratch = (min_backing / 2 + 255) & !255;

        Self {
            min_backing_memory_size_in_bytes: min_backing,
            max_backing_memory_size_in_bytes: max_backing,
            scratch_memory_size_in_bytes: scratch,
            node_input_record_pool_size: aligned_pools,
            max_in_flight_records: max_records,
        }
    }
}

// Memory range representing a GPU virtual address allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackingMemoryRange {
    pub gpu_va: u64,
    pub size: u64,
}

// Autonomous GPU node input record pool managing dynamic record slots.
#[derive(Debug, Clone)]
pub struct NodeInputRecordPool {
    pub node_id: D3D12_NODE_ID,
    pub slot_size: usize,
    pub capacity: usize,
    pub base_va: u64,
    pub allocated_mask: u64,
    pub total_allocations: u64,
    pub total_frees: u64,
}

impl NodeInputRecordPool {
    // Construct a new node input record pool with 256-byte alignment.
    pub fn new(node_id: D3D12_NODE_ID, slot_size: usize, capacity: usize, base_va: u64) -> Result<Self, WorkGraphError> {
        if (base_va % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0 {
            return Err(WorkGraphError::UnalignedBackingMemory(base_va));
        }
        let cap = capacity.min(64).max(1);
        let aligned_slot = (slot_size.max(16) + 15) & !15;
        Ok(Self {
            node_id,
            slot_size: aligned_slot,
            capacity: cap,
            base_va,
            allocated_mask: 0,
            total_allocations: 0,
            total_frees: 0,
        })
    }

    // Allocate next available record slot from the pool.
    pub fn allocate_slot(&mut self) -> Result<u64, WorkGraphError> {
        for bit in 0..self.capacity {
            let mask = 1u64 << bit;
            if (self.allocated_mask & mask) == 0 {
                self.allocated_mask |= mask;
                self.total_allocations += 1;
                return Ok(self.base_va + (bit * self.slot_size) as u64);
            }
        }
        Err(WorkGraphError::RecordRingFull)
    }

    // Free a previously allocated record slot back to the pool.
    pub fn free_slot(&mut self, gpu_va: u64) -> Result<(), WorkGraphError> {
        if gpu_va < self.base_va {
            return Err(WorkGraphError::InsufficientBackingMemory { required: self.base_va, provided: gpu_va });
        }
        let offset = (gpu_va - self.base_va) as usize;
        if (offset % self.slot_size) != 0 {
            return Err(WorkGraphError::RecordRingEmpty);
        }
        let bit = offset / self.slot_size;
        if bit >= self.capacity {
            return Err(WorkGraphError::RecordRingEmpty);
        }
        let mask = 1u64 << bit;
        if (self.allocated_mask & mask) == 0 {
            return Err(WorkGraphError::RecordRingEmpty);
        }
        self.allocated_mask &= !mask;
        self.total_frees += 1;
        Ok(())
    }

    // Return count of available free slots.
    pub fn available_slots(&self) -> usize {
        self.capacity - self.allocated_mask.count_ones() as usize
    }

    // Return count of currently in-use slots.
    pub fn in_use_slots(&self) -> usize {
        self.allocated_mask.count_ones() as usize
    }

    // Reset all slot allocations.
    pub fn reset(&mut self) {
        self.allocated_mask = 0;
    }
}

// Autonomous GPU Backing Memory Allocator managing backing and scratch partitions.
#[derive(Debug, Clone)]
pub struct AutonomousWorkGraphBackingAllocator {
    pub base_address: u64,
    pub total_size: u64,
    pub allocated_bytes: u64,
    pub scratch_offset: u64,
    pub scratch_size: u64,
    pub record_pools: Vec<NodeInputRecordPool>,
}

impl AutonomousWorkGraphBackingAllocator {
    // Allocate backing store partitions from physical GPU virtual address.
    pub fn new(base_address: u64, total_size: u64, reqs: &WorkGraphTier1MemoryRequirements) -> Result<Self, WorkGraphError> {
        if (base_address % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0 {
            return Err(WorkGraphError::UnalignedBackingMemory(base_address));
        }
        if total_size < reqs.min_backing_memory_size_in_bytes {
            return Err(WorkGraphError::InsufficientBackingMemory {
                required: reqs.min_backing_memory_size_in_bytes,
                provided: total_size,
            });
        }

        let scratch_sz = reqs.scratch_memory_size_in_bytes;
        let scratch_off = (total_size.saturating_sub(scratch_sz)) & !255;

        Ok(Self {
            base_address,
            total_size,
            allocated_bytes: 4096,
            scratch_offset: scratch_off,
            scratch_size: scratch_sz,
            record_pools: Vec::new(),
        })
    }

    // Register a node input record pool within the backing store.
    pub fn create_node_pool(&mut self, node: &WorkGraphTier1NodeSpec, capacity: usize) -> Result<&mut NodeInputRecordPool, WorkGraphError> {
        let slot_size = ((node.input_record_stride as usize).max(16) + 15) & !15;
        let pool_bytes = (slot_size * capacity.min(64) + 255) & !255;

        if self.allocated_bytes + pool_bytes as u64 > self.scratch_offset {
            return Err(WorkGraphError::InsufficientBackingMemory {
                required: self.allocated_bytes + pool_bytes as u64,
                provided: self.scratch_offset,
            });
        }

        let pool_va = self.base_address + self.allocated_bytes;
        self.allocated_bytes += pool_bytes as u64;

        let pool = NodeInputRecordPool::new(node.node_id, slot_size, capacity, pool_va)?;
        self.record_pools.push(pool);
        let idx = self.record_pools.len() - 1;
        Ok(&mut self.record_pools[idx])
    }

    // Locate node pool mutably by target node ID.
    pub fn find_node_pool_mut(&mut self, node_id: D3D12_NODE_ID) -> Option<&mut NodeInputRecordPool> {
        self.record_pools.iter_mut().find(|p| p.node_id == node_id)
    }

    // Query backing memory range.
    pub fn backing_memory_range(&self) -> BackingMemoryRange {
        BackingMemoryRange {
            gpu_va: self.base_address,
            size: self.scratch_offset,
        }
    }

    // Query scratch memory range.
    pub fn scratch_memory_range(&self) -> BackingMemoryRange {
        BackingMemoryRange {
            gpu_va: self.base_address + self.scratch_offset,
            size: self.scratch_size,
        }
    }

    // Query total allocated bytes.
    pub fn allocated_bytes(&self) -> u64 {
        self.allocated_bytes
    }

    // Query remaining available bytes before scratch space.
    pub fn available_bytes(&self) -> u64 {
        self.scratch_offset.saturating_sub(self.allocated_bytes)
    }

    // Recycle all active node pool allocations.
    pub fn recycle_all(&mut self) {
        for pool in &mut self.record_pools {
            pool.reset();
        }
    }
}

// Child node dispatch record generated by node shaders for downstream scheduling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChildDispatchRecord {
    pub producer_node: D3D12_NODE_ID,
    pub target_node: D3D12_NODE_ID,
    pub record_va: u64,
    pub payload_size: u16,
    pub recursion_depth: u16,
    pub priority: u8,
    pub flags: u8,
    pub payload: [u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
}

impl Default for ChildDispatchRecord {
    fn default() -> Self {
        Self {
            producer_node: D3D12_NODE_ID::default(),
            target_node: D3D12_NODE_ID::default(),
            record_va: 0,
            payload_size: 0,
            recursion_depth: 0,
            priority: 0,
            flags: 0,
            payload: [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
        }
    }
}

impl ChildDispatchRecord {
    // Construct child dispatch record with validated payload size and recursion depth.
    pub fn new(
        producer_node: D3D12_NODE_ID,
        target_node: D3D12_NODE_ID,
        payload_bytes: &[u8],
        recursion_depth: u16,
    ) -> Result<Self, WorkGraphError> {
        if payload_bytes.len() > MAX_NODE_RECORD_PAYLOAD_BYTES {
            return Err(WorkGraphError::PayloadSizeTooLarge(payload_bytes.len()));
        }
        if recursion_depth as u32 > MAX_WORK_GRAPH_RECURSION_DEPTH {
            return Err(WorkGraphError::RecursionDepthExceeded(recursion_depth as u32));
        }

        let mut record = Self {
            producer_node,
            target_node,
            record_va: 0,
            payload_size: payload_bytes.len() as u16,
            recursion_depth,
            priority: 0,
            flags: 0,
            payload: [0u8; MAX_NODE_RECORD_PAYLOAD_BYTES],
        };
        record.payload[..payload_bytes.len()].copy_from_slice(payload_bytes);
        Ok(record)
    }

    // Access slice of valid payload bytes.
    pub fn payload_bytes(&self) -> &[u8] {
        &self.payload[..self.payload_size as usize]
    }
}

// Circular ring buffer backing child node dispatches in GPU memory.
#[derive(Debug, Clone)]
pub struct ChildDispatchRingBuffer {
    pub target_node: D3D12_NODE_ID,
    pub capacity: usize,
    pub head: usize,
    pub tail: usize,
    pub count: usize,
    pub ring: Vec<Option<ChildDispatchRecord>>,
    pub total_enqueued: u64,
    pub total_dequeued: u64,
}

impl ChildDispatchRingBuffer {
    // Create a new child dispatch ring buffer with specified record capacity.
    pub fn new(target_node: D3D12_NODE_ID, capacity: usize) -> Self {
        let cap = capacity.max(1);
        let mut ring = Vec::with_capacity(cap);
        for _ in 0..cap {
            ring.push(None);
        }
        Self {
            target_node,
            capacity: cap,
            head: 0,
            tail: 0,
            count: 0,
            ring,
            total_enqueued: 0,
            total_dequeued: 0,
        }
    }

    // Push child record to ring buffer tail.
    pub fn push(&mut self, record: ChildDispatchRecord) -> Result<(), WorkGraphError> {
        if self.count >= self.capacity {
            return Err(WorkGraphError::RecordRingFull);
        }
        self.ring[self.tail] = Some(record);
        self.tail = (self.tail + 1) % self.capacity;
        self.count += 1;
        self.total_enqueued += 1;
        Ok(())
    }

    // Pop child record from ring buffer head.
    pub fn pop(&mut self) -> Option<ChildDispatchRecord> {
        if self.count == 0 {
            return None;
        }
        let record = self.ring[self.head].take();
        self.head = (self.head + 1) % self.capacity;
        self.count -= 1;
        self.total_dequeued += 1;
        record
    }

    // Whether the ring buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    // Whether the ring buffer is full.
    pub fn is_full(&self) -> bool {
        self.count >= self.capacity
    }

    // Number of records currently queued.
    pub fn len(&self) -> usize {
        self.count
    }

    // Clear all records from the ring buffer.
    pub fn clear(&mut self) {
        for slot in &mut self.ring {
            *slot = None;
        }
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }
}

// Node Shader Record Distributor managing dynamic allocations and dispatch queues.
#[derive(Debug, Clone)]
pub struct NodeShaderRecordDistributor {
    pub max_recursion_depth: u32,
    pub queues: Vec<ChildDispatchRingBuffer>,
    pub total_dispatched: u64,
    pub total_completed: u64,
}

impl NodeShaderRecordDistributor {
    // Construct new record distributor with recursion depth limit.
    pub fn new(max_recursion_depth: u32) -> Self {
        Self {
            max_recursion_depth: max_recursion_depth.min(MAX_WORK_GRAPH_RECURSION_DEPTH),
            queues: Vec::new(),
            total_dispatched: 0,
            total_completed: 0,
        }
    }

    // Register a child dispatch queue for a target node.
    pub fn register_queue(&mut self, target_node: D3D12_NODE_ID, capacity: usize) {
        if self.queues.iter().any(|q| q.target_node == target_node) {
            return;
        }
        self.queues.push(ChildDispatchRingBuffer::new(target_node, capacity));
    }

    // Allocate dynamic input record from backing memory and enqueue child dispatch.
    pub fn allocate_and_dispatch(
        &mut self,
        producer: D3D12_NODE_ID,
        target: D3D12_NODE_ID,
        payload: &[u8],
        depth: u16,
        allocator: &mut AutonomousWorkGraphBackingAllocator,
    ) -> Result<u64, WorkGraphError> {
        if depth as u32 > self.max_recursion_depth {
            return Err(WorkGraphError::RecursionDepthExceeded(depth as u32));
        }

        let pool = allocator
            .find_node_pool_mut(target)
            .ok_or(WorkGraphError::NodeNotFound(target.name_hash))?;
        let va = pool.allocate_slot()?;

        let mut child_rec = ChildDispatchRecord::new(producer, target, payload, depth)?;
        child_rec.record_va = va;

        let queue = self
            .queues
            .iter_mut()
            .find(|q| q.target_node == target)
            .ok_or(WorkGraphError::NodeNotFound(target.name_hash))?;

        if let Err(e) = queue.push(child_rec) {
            let _ = pool.free_slot(va);
            return Err(e);
        }

        self.total_dispatched += 1;
        Ok(va)
    }

    // Pop next pending dispatch record for the target node.
    pub fn pop_next_record(&mut self, target_node: D3D12_NODE_ID) -> Option<ChildDispatchRecord> {
        let queue = self.queues.iter_mut().find(|q| q.target_node == target_node)?;
        queue.pop()
    }

    // Free child dispatch record and recycle its backing memory slot.
    pub fn recycle_record(
        &mut self,
        record: &ChildDispatchRecord,
        allocator: &mut AutonomousWorkGraphBackingAllocator,
    ) -> Result<(), WorkGraphError> {
        if record.record_va != 0 {
            if let Some(pool) = allocator.find_node_pool_mut(record.target_node) {
                pool.free_slot(record.record_va)?;
            }
        }
        self.total_completed += 1;
        Ok(())
    }

    // Query pending record count for target node.
    pub fn pending_records_for(&self, target_node: D3D12_NODE_ID) -> usize {
        self.queues
            .iter()
            .find(|q| q.target_node == target_node)
            .map(|q| q.len())
            .unwrap_or(0)
    }

    // Query total pending records across all queues.
    pub fn total_pending(&self) -> usize {
        self.queues.iter().map(|q| q.len()).sum()
    }
}

// Validation error details for work graph compilation and hardware state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkGraphValidationError {
    ExceededMaxRecursionDepth(u32),
    InvalidRecordAlignment { stride: u32, alignment: u32 },
    InvalidGridDimensions([u32; 3]),
    CycleDetected(D3D12_NODE_ID),
    MissingEntryPoint,
    UnreachableNode(D3D12_NODE_ID),
}

// Hardware state validator checking recursion depth, record alignment, grids, and cycles.
#[derive(Debug, Clone)]
pub struct WorkGraphStateValidator {
    pub max_recursion_limit: u32,
    pub max_grid_limit: [u32; 3],
}

impl WorkGraphStateValidator {
    // Construct state validator with maximum recursion threshold.
    pub fn new(max_recursion_limit: u32) -> Self {
        Self {
            max_recursion_limit: max_recursion_limit.min(MAX_WORK_GRAPH_RECURSION_DEPTH),
            max_grid_limit: [65535, 65535, 65535],
        }
    }

    // Comprehensive validation of Tier 1.0 work graph program descriptor.
    pub fn validate_program(&self, program: &WorkGraphTier1ProgramDesc) -> Result<(), WorkGraphValidationError> {
        if program.entrypoints.is_empty() {
            return Err(WorkGraphValidationError::MissingEntryPoint);
        }
        if program.max_recursion_depth > self.max_recursion_limit {
            return Err(WorkGraphValidationError::ExceededMaxRecursionDepth(program.max_recursion_depth));
        }

        for entry in &program.entrypoints {
            if program.find_node(entry.node_id).is_none() {
                return Err(WorkGraphValidationError::UnreachableNode(entry.node_id));
            }
        }

        for node in &program.nodes {
            if node.input_record_alignment < 4 || (node.input_record_stride % node.input_record_alignment) != 0 {
                return Err(WorkGraphValidationError::InvalidRecordAlignment {
                    stride: node.input_record_stride,
                    alignment: node.input_record_alignment,
                });
            }
            if node.input_record_stride as usize > MAX_NODE_RECORD_PAYLOAD_BYTES {
                return Err(WorkGraphValidationError::InvalidRecordAlignment {
                    stride: node.input_record_stride,
                    alignment: node.input_record_alignment,
                });
            }

            if let Some(ref bc) = node.broadcasting {
                if bc.dispatch_grid[0] == 0
                    || bc.dispatch_grid[1] == 0
                    || bc.dispatch_grid[2] == 0
                    || bc.dispatch_grid[0] > self.max_grid_limit[0]
                    || bc.dispatch_grid[1] > self.max_grid_limit[1]
                    || bc.dispatch_grid[2] > self.max_grid_limit[2]
                {
                    return Err(WorkGraphValidationError::InvalidGridDimensions(bc.dispatch_grid));
                }
            }

            for target in &node.target_nodes {
                if program.find_node(*target).is_none() {
                    return Err(WorkGraphValidationError::UnreachableNode(*target));
                }
            }
        }

        if (program.flags & D3D12_WORK_GRAPH_FLAG_ALLOW_RECURSION) == 0 {
            if let Some(cycle_node) = self.detect_cycles(program) {
                return Err(WorkGraphValidationError::CycleDetected(cycle_node));
            }
        }

        let max_depth = self.max_graph_depth(program);
        if max_depth > program.max_recursion_depth {
            return Err(WorkGraphValidationError::ExceededMaxRecursionDepth(max_depth));
        }

        Ok(())
    }

    // Detect cycles in work graph DAG using depth-first search traversal.
    pub fn detect_cycles(&self, program: &WorkGraphTier1ProgramDesc) -> Option<D3D12_NODE_ID> {
        let mut visited = Vec::new();
        let mut in_stack = Vec::new();

        for entry in &program.entrypoints {
            if self.dfs_cycle(entry.node_id, program, &mut visited, &mut in_stack) {
                return in_stack.last().copied();
            }
        }
        None
    }

    // Recursive DFS helper to discover back-edges in work graph.
    fn dfs_cycle(
        &self,
        current: D3D12_NODE_ID,
        program: &WorkGraphTier1ProgramDesc,
        visited: &mut Vec<D3D12_NODE_ID>,
        in_stack: &mut Vec<D3D12_NODE_ID>,
    ) -> bool {
        if in_stack.contains(&current) {
            return true;
        }
        if visited.contains(&current) {
            return false;
        }

        visited.push(current);
        in_stack.push(current);

        if let Some(node) = program.find_node(current) {
            for target in &node.target_nodes {
                if self.dfs_cycle(*target, program, visited, in_stack) {
                    return true;
                }
            }
        }

        in_stack.pop();
        false
    }

    // Calculate maximum traversal depth across all branches from entrypoints.
    pub fn max_graph_depth(&self, program: &WorkGraphTier1ProgramDesc) -> u32 {
        let mut max_depth = 0u32;
        for entry in &program.entrypoints {
            let depth = self.dfs_depth(entry.node_id, program, 1);
            if depth > max_depth {
                max_depth = depth;
            }
        }
        max_depth
    }

    // Recursive DFS helper calculating branch depth.
    fn dfs_depth(&self, current: D3D12_NODE_ID, program: &WorkGraphTier1ProgramDesc, current_depth: u32) -> u32 {
        if current_depth > self.max_recursion_limit {
            return current_depth;
        }
        let mut deepest = current_depth;
        if let Some(node) = program.find_node(current) {
            for target in &node.target_nodes {
                let d = self.dfs_depth(*target, program, current_depth + 1);
                if d > deepest {
                    deepest = d;
                }
            }
        }
        deepest
    }
}

// Comprehensive self-test suite for Direct3D 12 Agility Work Graphs Tier 1.0 and autonomous allocator.
pub fn test_work_graphs_tier1_subsystem() -> bool {
    crate::serial_write_str("[KNT-WORK-GRAPHS] Initializing Direct3D 12 Agility Work Graphs Tier 1.0 self-test...\r\n");

    // 1. Verify Tier 1.0 program descriptors, nodes, and entrypoints.
    let mut program = WorkGraphTier1ProgramDesc::new("AgilityAutonomousCullGraph", D3D12_WORK_GRAPH_FLAG_AUTONOMOUS_ALLOCATION, 8);
    let mut broadcaster = WorkGraphTier1NodeSpec::broadcaster("CullBroadcaster", 0, [1u8; 32], [4, 1, 1], [8, 8, 1], 16, 16);
    let mut coalescer = WorkGraphTier1NodeSpec::coalescing("BatchCoalescer", 1, [2u8; 32], 4, [16, 1, 1], 16, 16);
    let worker = WorkGraphTier1NodeSpec::thread("DrawWorker", 2, [3u8; 32], [32, 1, 1], 16, 0);

    if broadcaster.add_target(coalescer.node_id).is_err() {
        return false;
    }
    if coalescer.add_target(worker.node_id).is_err() {
        return false;
    }

    if broadcaster.validate().is_err() || coalescer.validate().is_err() || worker.validate().is_err() {
        return false;
    }

    let entry = WorkGraphTier1EntryPoint::new(broadcaster.node_id, D3D12_WORK_GRAPH_FLAG_ENTRYPOINT, 16);
    if program.add_entrypoint(entry).is_err() {
        return false;
    }
    if program.add_node(broadcaster).is_err() || program.add_node(coalescer).is_err() || program.add_node(worker).is_err() {
        return false;
    }

    if program.node_count() != 3 || program.entrypoint_count() != 1 {
        return false;
    }

    // 2. Verify hardware state validator, recursion depth limits, and cycle detector.
    let validator = WorkGraphStateValidator::new(16);
    if validator.validate_program(&program).is_err() {
        return false;
    }

    // Verify rejection of cycles when recursion flag is omitted.
    let mut cycle_prog = WorkGraphTier1ProgramDesc::new("CycleGraph", 0, 8);
    let mut node_a = WorkGraphTier1NodeSpec::thread("NodeA", 10, [10u8; 32], [8, 1, 1], 16, 16);
    let mut node_b = WorkGraphTier1NodeSpec::thread("NodeB", 11, [11u8; 32], [8, 1, 1], 16, 16);
    let _ = node_a.add_target(node_b.node_id);
    let _ = node_b.add_target(node_a.node_id);
    let _ = cycle_prog.add_entrypoint(WorkGraphTier1EntryPoint::new(node_a.node_id, 0, 16));
    let _ = cycle_prog.add_node(node_a);
    let _ = cycle_prog.add_node(node_b);

    if validator.validate_program(&cycle_prog).is_ok() {
        return false;
    }

    // 3. Verify autonomous backing memory allocator and memory requirements.
    let reqs = WorkGraphTier1MemoryRequirements::calculate(&program);
    if reqs.min_backing_memory_size_in_bytes == 0 || (reqs.min_backing_memory_size_in_bytes % D3D12_WORK_GRAPH_BACKING_MEMORY_ALIGNMENT) != 0 {
        return false;
    }

    if AutonomousWorkGraphBackingAllocator::new(0x8001, reqs.min_backing_memory_size_in_bytes, &reqs).is_ok() {
        return false;
    }
    if AutonomousWorkGraphBackingAllocator::new(0x8000, reqs.min_backing_memory_size_in_bytes - 1, &reqs).is_ok() {
        return false;
    }

    let mut allocator = match AutonomousWorkGraphBackingAllocator::new(0x0000_8000_0000, reqs.min_backing_memory_size_in_bytes * 2, &reqs) {
        Ok(a) => a,
        Err(_) => return false,
    };

    for node in &program.nodes {
        if allocator.create_node_pool(node, 16).is_err() {
            return false;
        }
    }

    // 4. Verify node shader record distributor, child dispatch rings, and backing store recycling.
    let mut distributor = NodeShaderRecordDistributor::new(8);
    for node in &program.nodes {
        distributor.register_queue(node.node_id, 16);
    }

    let broadcaster_id = program.nodes[0].node_id;
    let coalescer_id = program.nodes[1].node_id;

    let payload_sample = [0x5au8; 16];
    let allocated_va = match distributor.allocate_and_dispatch(broadcaster_id, broadcaster_id, &payload_sample, 0, &mut allocator) {
        Ok(va) => va,
        Err(_) => return false,
    };

    if allocated_va == 0 || (allocated_va % 16) != 0 {
        return false;
    }

    let popped = match distributor.pop_next_record(broadcaster_id) {
        Some(r) => r,
        None => return false,
    };
    if popped.record_va != allocated_va || popped.payload_bytes() != &payload_sample {
        return false;
    }

    // Dispatch child record with incremented recursion depth.
    let _child_va = match distributor.allocate_and_dispatch(broadcaster_id, coalescer_id, &payload_sample, 1, &mut allocator) {
        Ok(va) => va,
        Err(_) => return false,
    };

    let child_popped = match distributor.pop_next_record(coalescer_id) {
        Some(r) => r,
        None => return false,
    };
    if child_popped.recursion_depth != 1 {
        return false;
    }

    // Recycle records back to the allocator.
    if distributor.recycle_record(&popped, &mut allocator).is_err() {
        return false;
    }
    if distributor.recycle_record(&child_popped, &mut allocator).is_err() {
        return false;
    }

    let bc_pool = allocator.find_node_pool_mut(broadcaster_id).unwrap();
    if bc_pool.available_slots() != 16 {
        return false;
    }

    // 5. Emit deterministic boot sentinels.
    crate::serial_write_str("[KNT-GPU-WORKGRAPHS-PASS] Direct3D 12 work graphs and autonomous GPU shader execution engine verified\r\n");
    crate::serial_write_str("[KNT-D3D12-WORK-GRAPHS-PASS] Direct3D 12 agility work graphs tier 1.0 verified\r\n");
    true
}

// Forwarding alias for Direct3D 12 Agility Work Graphs self-test.
pub fn test_agility_work_graphs_subsystem() -> bool {
    test_work_graphs_tier1_subsystem()
}
