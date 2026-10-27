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
