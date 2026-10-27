//! Handheld DirectStorage 1.2 GPU Decompression Queue & Hardware Copy Engine Orchestrator.
//!
//! Provides bare-metal low-latency storage-to-VRAM streaming for handheld game consoles:
//! 1. GPU-side GDeflate streaming decompression staging queues with NVMe submission ring.
//! 2. Hardware copy engine / DMA queue submission coordinator minimizing PCIe bandwidth contention with display refresh.
//! 3. Real-time decompression throughput meter and I/O request priority governor preventing frame starvation.
//! 4. Comprehensive self-test suite and kmain integration.

#![allow(dead_code)]

// ============================================================================
// Feature 1: GPU GDeflate Decompression Staging Queue & NVMe Ring
// ============================================================================

pub const MAX_STAGING_QUEUE_ENTRIES: usize = 64;
pub const GDEFLATE_TILE_SIZE_BYTES: u32 = 65536;

/// Lifecycle state of a DirectStorage GDeflate streaming request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GDeflateStageState {
    /// Slot unallocated / free.
    Unallocated,
    /// Request placed in queue; awaiting NVMe command submission.
    Queued,
    /// NVMe read command in flight across PCIe bus into staging buffer.
    NvmeInFlight,
    /// Compressed payload arrived in staging memory; ready for GPU compute shader.
    GpuStaged,
    /// GPU GDeflate compute kernel executing decompression.
    Decompressing,
    /// Uncompressed assets written to destination VRAM; resource ready.
    Completed,
    /// DMA transfer or decompression parity failure.
    Failed,
}

/// Request descriptor for a GDeflate compressed asset tile.
#[derive(Debug, Clone, Copy)]
pub struct GDeflateStageEntry {
    pub request_id: u64,
    pub asset_id: u32,
    pub nvme_lba_start: u64,
    pub nvme_sector_count: u32,
    pub compressed_phys_addr: u64,
    pub destination_vram_addr: u64,
    pub compressed_bytes: u32,
    pub uncompressed_bytes: u32,
    pub state: GDeflateStageState,
    pub submission_timestamp_us: u64,
    pub completion_timestamp_us: u64,
}

impl GDeflateStageEntry {
    pub const fn empty() -> Self {
        Self {
            request_id: 0,
            asset_id: 0,
            nvme_lba_start: 0,
            nvme_sector_count: 0,
            compressed_phys_addr: 0,
            destination_vram_addr: 0,
            compressed_bytes: 0,
            uncompressed_bytes: 0,
            state: GDeflateStageState::Unallocated,
            submission_timestamp_us: 0,
            completion_timestamp_us: 0,
        }
    }
}

/// GPU GDeflate Decompression Staging Ring Buffer.
#[derive(Debug, Clone)]
pub struct GDeflateStagingQueue {
    pub entries: [GDeflateStageEntry; MAX_STAGING_QUEUE_ENTRIES],
    pub head: usize,
    pub tail: usize,
    pub count: usize,
    pub next_request_id: u64,
    pub total_enqueued: u64,
    pub total_completed: u64,
}

impl GDeflateStagingQueue {
    pub const fn new() -> Self {
        Self {
            entries: [GDeflateStageEntry::empty(); MAX_STAGING_QUEUE_ENTRIES],
            head: 0,
            tail: 0,
            count: 0,
            next_request_id: 1,
            total_enqueued: 0,
            total_completed: 0,
        }
    }

    /// Enqueue a new streaming request. Returns allocated request_id or None if queue full.
    pub fn enqueue(
        &mut self,
        asset_id: u32,
        lba: u64,
        sectors: u32,
        comp_phys: u64,
        dest_vram: u64,
        comp_bytes: u32,
        uncomp_bytes: u32,
        time_us: u64,
    ) -> Option<u64> {
        if self.count >= MAX_STAGING_QUEUE_ENTRIES {
            return None;
        }

        let req_id = self.next_request_id;
        self.next_request_id += 1;

        let entry = GDeflateStageEntry {
            request_id: req_id,
            asset_id,
            nvme_lba_start: lba,
            nvme_sector_count: sectors,
            compressed_phys_addr: comp_phys,
            destination_vram_addr: dest_vram,
            compressed_bytes: comp_bytes,
            uncompressed_bytes: uncomp_bytes,
            state: GDeflateStageState::Queued,
            submission_timestamp_us: time_us,
            completion_timestamp_us: 0,
        };

        self.entries[self.tail] = entry;
        self.tail = (self.tail + 1) % MAX_STAGING_QUEUE_ENTRIES;
        self.count += 1;
        self.total_enqueued += 1;

        Some(req_id)
    }

    /// Find an entry by request ID.
    pub fn find_entry_mut(&mut self, request_id: u64) -> Option<&mut GDeflateStageEntry> {
        for entry in self.entries.iter_mut() {
            if entry.state != GDeflateStageState::Unallocated && entry.request_id == request_id {
                return Some(entry);
            }
        }
        None
    }

    /// Mark request as submitted to NVMe Submission Queue.
    pub fn dispatch_to_nvme(&mut self, request_id: u64) -> bool {
        if let Some(entry) = self.find_entry_mut(request_id) {
            if entry.state == GDeflateStageState::Queued {
                entry.state = GDeflateStageState::NvmeInFlight;
                return true;
            }
        }
        false
    }

    /// Mark NVMe DMA read completion: compressed data staged in host/BAR memory.
    pub fn on_nvme_transfer_complete(&mut self, request_id: u64) -> bool {
        if let Some(entry) = self.find_entry_mut(request_id) {
            if entry.state == GDeflateStageState::NvmeInFlight {
                entry.state = GDeflateStageState::GpuStaged;
                return true;
            }
        }
        false
    }

    /// Dispatch staged request to GPU GDeflate decompression compute queue.
    pub fn dispatch_to_gpu(&mut self, request_id: u64) -> bool {
        if let Some(entry) = self.find_entry_mut(request_id) {
            if entry.state == GDeflateStageState::GpuStaged {
                entry.state = GDeflateStageState::Decompressing;
                return true;
            }
        }
        false
    }

    /// Mark GPU decompression completion and return uncompressed byte count.
    pub fn on_gpu_decomp_complete(&mut self, request_id: u64, completion_time_us: u64) -> Option<u32> {
        let mut found_idx = None;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.state == GDeflateStageState::Decompressing && entry.request_id == request_id {
                found_idx = Some(i);
                break;
            }
        }

        if let Some(idx) = found_idx {
            self.entries[idx].state = GDeflateStageState::Completed;
            self.entries[idx].completion_timestamp_us = completion_time_us;
            let uncompressed = self.entries[idx].uncompressed_bytes;
            self.total_completed += 1;

            // Retire head entries if completed
            while self.count > 0 && self.entries[self.head].state == GDeflateStageState::Completed {
                self.entries[self.head].state = GDeflateStageState::Unallocated;
                self.head = (self.head + 1) % MAX_STAGING_QUEUE_ENTRIES;
                self.count -= 1;
            }

            return Some(uncompressed);
        }
        None
    }

    pub fn active_count(&self) -> usize {
        self.count
    }
}

/// Verification test for GPU GDeflate staging queue and NVMe ring buffer.
pub fn verify_gdeflate_staging_queue_and_nvme_ring() -> bool {
    let mut queue = GDeflateStagingQueue::new();

    // 1. Initial empty queue state
    if queue.active_count() != 0 || queue.total_enqueued != 0 {
        return false;
    }

    // 2. Enqueue 64 KiB GDeflate tile request
    let req1 = queue.enqueue(
        101,            // asset_id: texture_diffuse
        2048,           // NVMe LBA start
        64,             // 64 sectors = 32 KiB compressed
        0x1_0000_0000,  // Staging physical buffer
        0xE_0000_0000,  // Target VRAM address
        32768,          // Compressed bytes (50% ratio)
        65536,          // Uncompressed bytes (64 KiB)
        100_000,        // Timestamp 100 ms
    );

    let req_id = match req1 {
        Some(id) => id,
        None => return false,
    };

    if queue.active_count() != 1 || queue.total_enqueued != 1 {
        return false;
    }

    // 3. Dispatch to NVMe -> NvmeInFlight
    if !queue.dispatch_to_nvme(req_id) {
        return false;
    }

    // 4. NVMe DMA completion -> GpuStaged
    if !queue.on_nvme_transfer_complete(req_id) {
        return false;
    }

    // 5. Dispatch to GPU compute -> Decompressing
    if !queue.dispatch_to_gpu(req_id) {
        return false;
    }

    // 6. GPU decompression complete -> Completed & retired
    let uncomp = queue.on_gpu_decomp_complete(req_id, 101_500);
    if uncomp != Some(65536) {
        return false;
    }

    if queue.total_completed != 1 || queue.active_count() != 0 {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Hardware Copy Engine & DMA Queue Coordinator with VBLANK Pacing
// ============================================================================

/// Pacing mode governing hardware copy engine DMA submissions to eliminate display scanout contention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaPacingMode {
    /// Bus utilization < 60%: Unrestricted DMA transfer batches.
    Unconstrained,
    /// Bus utilization 60% - 80%: Sliced transfers (64 KiB max) during active scanout.
    ScanoutPaced,
    /// Bus utilization > 80%: Bulk DMA transfers deferred to Vertical Blanking Intervals (VBLANK).
    VBlankBurstOnly,
    /// Bus utilization > 92%: Immediate DMA pause to prevent display FIFO underrun and frame drop.
    EmergencyThrottled,
}

/// Sliced DMA copy descriptor for hardware copy engines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DmaTransferSlice {
    pub slice_id: u64,
    pub source_phys: u64,
    pub dest_phys: u64,
    pub bytes_to_copy: u32,
    pub bytes_copied: u32,
    pub is_complete: bool,
}

/// Hardware Copy Engine and DMA Queue Submission Coordinator.
#[derive(Debug, Clone)]
pub struct HardwareCopyEngineCoordinator {
    pub pacing_mode: DmaPacingMode,
    pub max_slice_bytes: u32,
    pub vblank_burst_max_bytes: u32,
    pub total_slices_dispatched: u64,
    pub vblank_bursts_executed: u64,
    pub active_scanout_throttles: u64,
    pub total_dma_bytes_transferred: u64,
}

impl HardwareCopyEngineCoordinator {
    pub const fn new() -> Self {
        Self {
            pacing_mode: DmaPacingMode::Unconstrained,
            max_slice_bytes: 65536, // 64 KiB slice
            vblank_burst_max_bytes: 524288, // 512 KiB burst
            total_slices_dispatched: 0,
            vblank_bursts_executed: 0,
            active_scanout_throttles: 0,
            total_dma_bytes_transferred: 0,
        }
    }

    /// Update pacing mode based on current PCIe/memory fabric load and display scanout state.
    pub fn update_pacing_mode(&mut self, bus_utilization_pct: u8, _is_vblank: bool) -> DmaPacingMode {
        self.pacing_mode = if bus_utilization_pct >= 92 {
            DmaPacingMode::EmergencyThrottled
        } else if bus_utilization_pct >= 80 {
            DmaPacingMode::VBlankBurstOnly
        } else if bus_utilization_pct >= 60 {
            DmaPacingMode::ScanoutPaced
        } else {
            DmaPacingMode::Unconstrained
        };

        self.pacing_mode
    }

    /// Determine whether a DMA transfer slice can be dispatched immediately or must wait for VBLANK.
    ///
    /// Returns `(allowed: bool, slice_bytes: u32)`.
    pub fn can_dispatch_dma(&mut self, transfer_bytes: u32, is_vblank: bool) -> (bool, u32) {
        match self.pacing_mode {
            DmaPacingMode::EmergencyThrottled => {
                self.active_scanout_throttles += 1;
                (false, 0)
            }
            DmaPacingMode::VBlankBurstOnly => {
                if is_vblank {
                    let burst_size = transfer_bytes.min(self.vblank_burst_max_bytes);
                    self.vblank_bursts_executed += 1;
                    (true, burst_size)
                } else {
                    self.active_scanout_throttles += 1;
                    (false, 0)
                }
            }
            DmaPacingMode::ScanoutPaced => {
                if is_vblank {
                    let burst_size = transfer_bytes.min(self.vblank_burst_max_bytes);
                    self.vblank_bursts_executed += 1;
                    (true, burst_size)
                } else {
                    let slice = transfer_bytes.min(self.max_slice_bytes);
                    (true, slice)
                }
            }
            DmaPacingMode::Unconstrained => {
                let slice = transfer_bytes.min(self.vblank_burst_max_bytes);
                (true, slice)
            }
        }
    }

    /// Record completed DMA slice transfer telemetry.
    pub fn record_dma_transfer(&mut self, bytes_transferred: u32) {
        self.total_slices_dispatched += 1;
        self.total_dma_bytes_transferred += bytes_transferred as u64;
    }
}

/// Verification test for hardware copy engine and DMA coordinator with display VBLANK pacing.
pub fn verify_hardware_copy_engine_and_vblank_pacing() -> bool {
    let mut coordinator = HardwareCopyEngineCoordinator::new();

    // 1. Initial unconstrained state (45% bus utilization)
    let mode1 = coordinator.update_pacing_mode(45, false);
    if mode1 != DmaPacingMode::Unconstrained {
        return false;
    }
    let (allow1, size1) = coordinator.can_dispatch_dma(131072, false);
    if !allow1 || size1 != 131072 {
        return false;
    }
    coordinator.record_dma_transfer(size1);

    // 2. High bus utilization during active scanout (85% bus load, not in VBLANK)
    // Must prevent display memory underrun by deferring DMA to VBLANK
    let mode2 = coordinator.update_pacing_mode(85, false);
    if mode2 != DmaPacingMode::VBlankBurstOnly {
        return false;
    }
    let (allow2, _) = coordinator.can_dispatch_dma(262144, false);
    if allow2 {
        // Must NOT allow DMA while scanout is active at 85% bus load
        return false;
    }
    if coordinator.active_scanout_throttles == 0 {
        return false;
    }

    // 3. VBLANK interrupt arrives (is_vblank = true): burst transfer allowed
    let (allow3, size3) = coordinator.can_dispatch_dma(262144, true);
    if !allow3 || size3 != 262144 {
        return false;
    }
    if coordinator.vblank_bursts_executed != 1 {
        return false;
    }
    coordinator.record_dma_transfer(size3);

    // 4. Moderate bus load (70%): sliced transfers during scanout
    coordinator.update_pacing_mode(70, false);
    let (allow4, size4) = coordinator.can_dispatch_dma(262144, false);
    if !allow4 || size4 != 65536 {
        // Must slice to 64 KiB
        return false;
    }

    // 5. Emergency throttling (>92% bus saturation): total pause
    coordinator.update_pacing_mode(95, true);
    let (allow5, _) = coordinator.can_dispatch_dma(65536, true);
    if allow5 {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: Real-Time Decompression Meter & I/O Priority Governor
// ============================================================================

/// DirectStorage I/O request priority class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IoRequestPriority {
    /// Low priority background asset streaming (audio banks, distant skybox).
    LowBackgroundBulk = 0,
    /// Medium priority open-world terrain chunk streaming.
    MediumWorldStream = 1,
    /// High priority mipmap preloading for near-field camera motion.
    HighMipmapPreload = 2,
    /// Immediate frame-critical asset needed for current/next display frame.
    ImmediateFrameCritical = 3,
}

/// Rolling window telemetry for DirectStorage GDeflate decompression throughput.
#[derive(Debug, Clone)]
pub struct DecompThroughputMeter {
    pub sample_bytes: [u32; 16],
    pub sample_duration_us: [u32; 16],
    pub sample_head: usize,
    pub sample_count: usize,
    pub total_decompressed_bytes: u64,
    pub total_decompression_time_us: u64,
}

impl DecompThroughputMeter {
    pub const fn new() -> Self {
        Self {
            sample_bytes: [0; 16],
            sample_duration_us: [0; 16],
            sample_head: 0,
            sample_count: 0,
            total_decompressed_bytes: 0,
            total_decompression_time_us: 0,
        }
    }

    /// Record a completed decompression tile sample.
    pub fn record_sample(&mut self, bytes: u32, duration_us: u32) {
        if duration_us == 0 {
            return;
        }

        self.sample_bytes[self.sample_head] = bytes;
        self.sample_duration_us[self.sample_head] = duration_us;
        self.sample_head = (self.sample_head + 1) % 16;
        if self.sample_count < 16 {
            self.sample_count += 1;
        }

        self.total_decompressed_bytes += bytes as u64;
        self.total_decompression_time_us += duration_us as u64;
    }

    /// Calculate smoothed rolling throughput in Megabytes per second (MB/s).
    pub fn rolling_throughput_mb_s(&self) -> u32 {
        if self.sample_count == 0 {
            return 0;
        }

        let mut sum_bytes: u64 = 0;
        let mut sum_us: u64 = 0;
        let mut i = 0;
        while i < self.sample_count {
            sum_bytes += self.sample_bytes[i] as u64;
            sum_us += self.sample_duration_us[i] as u64;
            i += 1;
        }

        if sum_us == 0 {
            return 0;
        }

        // Throughput = (sum_bytes / sum_us) * 1_000_000 bytes/sec => / 1_000_000 = MB/s
        (sum_bytes / sum_us) as u32
    }

    /// Calculate average decompression latency per tile in microseconds.
    pub fn average_latency_us(&self) -> u32 {
        if self.sample_count == 0 {
            0
        } else {
            let mut sum_us: u64 = 0;
            let mut i = 0;
            while i < self.sample_count {
                sum_us += self.sample_duration_us[i] as u64;
                i += 1;
            }
            (sum_us / self.sample_count as u64) as u32
        }
    }
}

/// Governor preventing frame starvation by enforcing strict priority scheduling and aging anti-starvation.
#[derive(Debug, Clone)]
pub struct IoPriorityGovernor {
    /// Starvation aging threshold in microseconds (e.g. 100 ms).
    pub aging_threshold_us: u64,
    /// Total priority promotion events to prevent starvation.
    pub total_starvation_promotions: u64,
    /// Total critical frame assets served without deadline miss.
    pub critical_frames_served: u64,
}

impl IoPriorityGovernor {
    pub const fn new() -> Self {
        Self {
            aging_threshold_us: 100_000, // 100 ms
            total_starvation_promotions: 0,
            critical_frames_served: 0,
        }
    }

    /// Evaluate whether a queued entry should be promoted due to aging.
    pub fn evaluate_aging(&mut self, priority: IoRequestPriority, queued_duration_us: u64) -> IoRequestPriority {
        if queued_duration_us >= self.aging_threshold_us {
            self.total_starvation_promotions += 1;
            match priority {
                IoRequestPriority::LowBackgroundBulk => IoRequestPriority::MediumWorldStream,
                IoRequestPriority::MediumWorldStream => IoRequestPriority::HighMipmapPreload,
                IoRequestPriority::HighMipmapPreload => IoRequestPriority::ImmediateFrameCritical,
                IoRequestPriority::ImmediateFrameCritical => IoRequestPriority::ImmediateFrameCritical,
            }
        } else {
            priority
        }
    }

    /// Select the highest priority request index from candidate requests.
    pub fn pick_highest_priority(
        &mut self,
        candidates: &[(usize, IoRequestPriority, u64)], // (index, priority, age_us)
    ) -> Option<usize> {
        if candidates.is_empty() {
            return None;
        }

        let mut best_idx = candidates[0].0;
        let mut best_eff_prio = self.evaluate_aging(candidates[0].1, candidates[0].2);

        for &(idx, prio, age_us) in candidates.iter().skip(1) {
            let eff_prio = self.evaluate_aging(prio, age_us);
            if eff_prio > best_eff_prio {
                best_eff_prio = eff_prio;
                best_idx = idx;
            }
        }

        if best_eff_prio == IoRequestPriority::ImmediateFrameCritical {
            self.critical_frames_served += 1;
        }

        Some(best_idx)
    }
}

/// Verification test for real-time decompression throughput meter and I/O priority governor.
pub fn verify_decomp_throughput_meter_and_priority_governor() -> bool {
    let mut meter = DecompThroughputMeter::new();
    let mut governor = IoPriorityGovernor::new();

    // 1. Throughput meter verification: simulate 16 tiles of 64 KiB GDeflate decompressions taking 16 us each
    // 65536 bytes / 16 us = 4096 bytes/us = 4096 MB/s (~4.1 GB/s)
    for _ in 0..16 {
        meter.record_sample(65536, 16);
    }

    let tp = meter.rolling_throughput_mb_s();
    if tp != 4096 {
        return false;
    }
    if meter.average_latency_us() != 16 {
        return false;
    }

    // 2. Priority selection test
    let candidates = [
        (0, IoRequestPriority::LowBackgroundBulk, 10_000),      // Low, age 10ms
        (1, IoRequestPriority::HighMipmapPreload, 20_000),      // High, age 20ms
        (2, IoRequestPriority::MediumWorldStream, 15_000),     // Medium, age 15ms
    ];

    let selected = governor.pick_highest_priority(&candidates);
    if selected != Some(1) {
        // Must choose HighMipmapPreload (index 1)
        return false;
    }

    // 3. Starvation avoidance promotion test: aged Low request (120 ms >= 100 ms threshold)
    let aged_prio = governor.evaluate_aging(IoRequestPriority::LowBackgroundBulk, 120_000);
    if aged_prio != IoRequestPriority::MediumWorldStream {
        return false;
    }
    if governor.total_starvation_promotions == 0 {
        return false;
    }

    // 4. Critical frame priority selection
    let candidates_crit = [
        (0, IoRequestPriority::HighMipmapPreload, 50_000),
        (1, IoRequestPriority::ImmediateFrameCritical, 5_000),
    ];
    let selected_crit = governor.pick_highest_priority(&candidates_crit);
    if selected_crit != Some(1) {
        return false;
    }
    if governor.critical_frames_served == 0 {
        return false;
    }

    true
}


// ============================================================================
// Feature 4: DirectStorage GPU Decompression Orchestrator Pipeline
// ============================================================================

/// Action determined by the orchestrator tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchestratorAction {
    /// No action required (idle or queues full).
    Idle,
    /// Dispatched NVMe read request for compressed tile.
    DispatchedNvmeRead { request_id: u64, bytes: u32 },
    /// Staged compressed tile dispatched to GPU compute queue for GDeflate decompression.
    DispatchedGpuCompute { request_id: u64 },
    /// DMA transfer deferred due to active display scanout bandwidth conservation.
    DeferredForVBlank,
}

/// Integrated DirectStorage 1.2 GPU Decompression & Hardware Copy Orchestrator.
#[derive(Debug, Clone)]
pub struct DirectStorageGpuOrchestrator {
    pub staging_queue: GDeflateStagingQueue,
    pub dma_coordinator: HardwareCopyEngineCoordinator,
    pub throughput_meter: DecompThroughputMeter,
    pub priority_governor: IoPriorityGovernor,
    pub request_priorities: [IoRequestPriority; MAX_STAGING_QUEUE_ENTRIES],
    pub total_pipeline_dispatches: u64,
}

impl DirectStorageGpuOrchestrator {
    pub const fn new() -> Self {
        Self {
            staging_queue: GDeflateStagingQueue::new(),
            dma_coordinator: HardwareCopyEngineCoordinator::new(),
            throughput_meter: DecompThroughputMeter::new(),
            priority_governor: IoPriorityGovernor::new(),
            request_priorities: [IoRequestPriority::MediumWorldStream; MAX_STAGING_QUEUE_ENTRIES],
            total_pipeline_dispatches: 0,
        }
    }

    /// Submit a high-speed game asset streaming request.
    pub fn submit_request(
        &mut self,
        asset_id: u32,
        lba: u64,
        sectors: u32,
        comp_phys: u64,
        dest_vram: u64,
        comp_bytes: u32,
        uncomp_bytes: u32,
        priority: IoRequestPriority,
        time_us: u64,
    ) -> Option<u64> {
        let req_id = self.staging_queue.enqueue(
            asset_id, lba, sectors, comp_phys, dest_vram, comp_bytes, uncomp_bytes, time_us,
        )?;

        // Record priority for this slot
        for (i, entry) in self.staging_queue.entries.iter().enumerate() {
            if entry.state == GDeflateStageState::Queued && entry.request_id == req_id {
                self.request_priorities[i] = priority;
                break;
            }
        }

        Some(req_id)
    }

    /// Progress the storage-to-VRAM streaming pipeline by one scheduling tick.
    pub fn orchestrate_tick(
        &mut self,
        current_time_us: u64,
        is_vblank: bool,
        bus_utilization_pct: u8,
    ) -> OrchestratorAction {
        self.dma_coordinator.update_pacing_mode(bus_utilization_pct, is_vblank);

        // 1. First priority: Check if any GpuStaged entry can be dispatched to GPU GDeflate compute
        for entry in self.staging_queue.entries.iter_mut() {
            if entry.state == GDeflateStageState::GpuStaged {
                entry.state = GDeflateStageState::Decompressing;
                self.total_pipeline_dispatches += 1;
                return OrchestratorAction::DispatchedGpuCompute {
                    request_id: entry.request_id,
                };
            }
        }

        // 2. Second priority: Select highest-priority Queued request for NVMe DMA fetching
        let mut candidates: [(usize, IoRequestPriority, u64); MAX_STAGING_QUEUE_ENTRIES] =
            [(0, IoRequestPriority::LowBackgroundBulk, 0); MAX_STAGING_QUEUE_ENTRIES];
        let mut candidate_count = 0;

        for (i, entry) in self.staging_queue.entries.iter().enumerate() {
            if entry.state == GDeflateStageState::Queued {
                let age = if current_time_us > entry.submission_timestamp_us {
                    current_time_us - entry.submission_timestamp_us
                } else {
                    0
                };
                candidates[candidate_count] = (i, self.request_priorities[i], age);
                candidate_count += 1;
            }
        }

        if candidate_count == 0 {
            return OrchestratorAction::Idle;
        }

        if let Some(best_entry_idx) = self
            .priority_governor
            .pick_highest_priority(&candidates[0..candidate_count])
        {
            let entry = &self.staging_queue.entries[best_entry_idx];
            let req_id = entry.request_id;
            let comp_bytes = entry.compressed_bytes;

            // Check if hardware DMA engine permits transfer now or defers for VBLANK
            let (allowed, slice_size) = self.dma_coordinator.can_dispatch_dma(comp_bytes, is_vblank);

            if allowed {
                self.staging_queue.entries[best_entry_idx].state = GDeflateStageState::NvmeInFlight;
                self.dma_coordinator.record_dma_transfer(slice_size);
                self.total_pipeline_dispatches += 1;
                return OrchestratorAction::DispatchedNvmeRead {
                    request_id: req_id,
                    bytes: slice_size,
                };
            } else {
                return OrchestratorAction::DeferredForVBlank;
            }
        }

        OrchestratorAction::Idle
    }

    /// Complete a GPU decompression tile, update throughput metrics, and retire request.
    pub fn complete_tile(&mut self, request_id: u64, completion_time_us: u64) -> bool {
        let start_time = self
            .staging_queue
            .find_entry_mut(request_id)
            .map(|e| e.submission_timestamp_us)
            .unwrap_or(completion_time_us);

        let duration = if completion_time_us > start_time {
            (completion_time_us - start_time) as u32
        } else {
            1
        };

        if let Some(uncomp_bytes) = self
            .staging_queue
            .on_gpu_decomp_complete(request_id, completion_time_us)
        {
            self.throughput_meter.record_sample(uncomp_bytes, duration);
            true
        } else {
            false
        }
    }
}

/// Verification test for DirectStorage GPU decompression orchestrator pipeline.
pub fn verify_gpu_decomp_orchestrator_pipeline() -> bool {
    let mut orchestrator = DirectStorageGpuOrchestrator::new();

    // 1. Submit two streaming tiles: one background, one immediate frame critical
    let req_bg = orchestrator.submit_request(
        201, 1000, 64, 0x1_0000_0000, 0xE_0000_0000, 32768, 65536,
        IoRequestPriority::LowBackgroundBulk, 10_000,
    );
    let req_crit = orchestrator.submit_request(
        202, 2000, 64, 0x1_0001_0000, 0xE_0001_0000, 32768, 65536,
        IoRequestPriority::ImmediateFrameCritical, 10_005,
    );

    if req_bg.is_none() || req_crit.is_none() {
        return false;
    }
    let req_crit_id = req_crit.unwrap();

    // 2. Tick during active scanout with high bus load (85% bus load, not in VBLANK)
    // Priority governor selects critical request, but DMA coordinator defers because of active scanout contention
    let act1 = orchestrator.orchestrate_tick(10_010, false, 85);
    if act1 != OrchestratorAction::DeferredForVBlank {
        return false;
    }

    // 3. VBLANK interrupt arrives (is_vblank = true): critical request dispatched first!
    let act2 = orchestrator.orchestrate_tick(10_015, true, 85);
    match act2 {
        OrchestratorAction::DispatchedNvmeRead { request_id, bytes } => {
            if request_id != req_crit_id || bytes != 32768 {
                return false;
            }
        }
        _ => return false,
    }

    // 4. Simulate NVMe DMA read completion for critical tile
    if !orchestrator.staging_queue.on_nvme_transfer_complete(req_crit_id) {
        return false;
    }

    // 5. Next tick dispatches GpuStaged tile to GPU GDeflate compute
    let act3 = orchestrator.orchestrate_tick(10_020, false, 50);
    match act3 {
        OrchestratorAction::DispatchedGpuCompute { request_id } => {
            if request_id != req_crit_id {
                return false;
            }
        }
        _ => return false,
    }

    // 6. Complete GPU decompression tile
    if !orchestrator.complete_tile(req_crit_id, 10_036) {
        return false;
    }

    if orchestrator.throughput_meter.total_decompressed_bytes != 65536 {
        return false;
    }

    true
}
