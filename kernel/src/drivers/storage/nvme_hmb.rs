//! Handheld Direct NVMe Host Memory Buffer (HMB) Dynamic Allocator & DirectStorage IO Ring.

#![allow(dead_code)]

/// NVMe Feature Identifier for Host Memory Buffer (HMB).
pub const NVME_FEAT_HOST_MEMORY_BUFFER: u32 = 0x0D;

/// Standard NVMe memory page size (4 KiB).
pub const NVME_PAGE_SIZE_BYTES: u64 = 4096;

/// Maximum HMB host allocation (64 MiB).
pub const HMB_MAX_ALLOCATION_BYTES: u64 = 64 * 1024 * 1024;
/// Minimum recommended HMB allocation for DRAM-less SSD FTL mapping (4 MiB).
pub const HMB_MIN_ALLOCATION_BYTES: u64 = 4 * 1024 * 1024;
/// Maximum entries in Host Memory Descriptor List (HMDL).
pub const HMDL_MAX_ENTRIES: usize = 16;

/// Single Host Memory Descriptor List (HMDL) entry per NVMe 1.3+ specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct HostMemoryDescriptor {
    /// Host physical base address of memory buffer chunk (page-aligned).
    pub buffer_address: u64,
    /// Buffer size in memory page (4 KiB) units.
    pub buffer_size_pages: u32,
    /// Reserved per specification.
    pub reserved: u32,
}

impl HostMemoryDescriptor {
    pub const fn empty() -> Self {
        Self {
            buffer_address: 0,
            buffer_size_pages: 0,
            reserved: 0,
        }
    }
}

/// NVMe Set Features (FID 0x0D) Host Memory Buffer command configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HmbFeatureCommand {
    pub feature_id: u32,
    pub enable_hmb: bool,
    pub memory_return: bool,
    pub total_pages: u32,
    pub descriptor_list_addr: u64,
    pub descriptor_entry_count: u32,
}

/// Operational state of the NVMe Host Memory Buffer protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HmbState {
    /// HMB not supported or disabled.
    Disabled,
    /// Negotiation in progress; HMDL mapped.
    Allocating,
    /// HMB active; SSD controller caching FTL mapping tables in host DRAM.
    Active,
    /// Power-down / suspend eviction in progress.
    Evicting,
}

/// Host Memory Buffer (HMB) Allocator and HMDL Generator.
#[derive(Debug, Clone)]
pub struct HmbAllocator {
    pub state: HmbState,
    pub preferred_bytes: u64,
    pub minimum_bytes: u64,
    pub allocated_bytes: u64,
    pub descriptors: [HostMemoryDescriptor; HMDL_MAX_ENTRIES],
    pub descriptor_count: usize,
    pub descriptor_table_phys_addr: u64,
}

impl HmbAllocator {
    pub const fn new(preferred_bytes: u64, minimum_bytes: u64) -> Self {
        Self {
            state: HmbState::Disabled,
            preferred_bytes,
            minimum_bytes,
            allocated_bytes: 0,
            descriptors: [HostMemoryDescriptor::empty(); HMDL_MAX_ENTRIES],
            descriptor_count: 0,
            descriptor_table_phys_addr: 0x0000_0001_8000_0000, // Simulated high-DRAM staging area
        }
    }

    /// Allocates chunks and builds the Host Memory Descriptor List (HMDL).
    pub fn allocate_hmb(&mut self, requested_bytes: u64) -> Result<HmbFeatureCommand, &'static str> {
        if self.state == HmbState::Active {
            return Err("HMB already active");
        }

        let target_bytes = requested_bytes
            .min(self.preferred_bytes)
            .min(HMB_MAX_ALLOCATION_BYTES)
            .max(self.minimum_bytes);

        self.state = HmbState::Allocating;

        // Partition target buffer into 4 MiB contiguous chunks across HMDL entries
        let chunk_size_bytes = 4 * 1024 * 1024u64; // 4 MiB per descriptor
        let chunk_pages = (chunk_size_bytes / NVME_PAGE_SIZE_BYTES) as u32;
        let num_chunks = ((target_bytes + chunk_size_bytes - 1) / chunk_size_bytes) as usize;

        if num_chunks > HMDL_MAX_ENTRIES {
            return Err("Too many chunks for HMDL table");
        }

        let base_phys = 0x0000_0001_8100_0000u64; // Staging physical memory pool
        for i in 0..num_chunks {
            self.descriptors[i] = HostMemoryDescriptor {
                buffer_address: base_phys + (i as u64 * chunk_size_bytes),
                buffer_size_pages: chunk_pages,
                reserved: 0,
            };
        }

        self.descriptor_count = num_chunks;
        self.allocated_bytes = num_chunks as u64 * chunk_size_bytes;
        self.state = HmbState::Active;

        let total_pages = (self.allocated_bytes / NVME_PAGE_SIZE_BYTES) as u32;

        Ok(HmbFeatureCommand {
            feature_id: NVME_FEAT_HOST_MEMORY_BUFFER,
            enable_hmb: true,
            memory_return: false,
            total_pages,
            descriptor_list_addr: self.descriptor_table_phys_addr,
            descriptor_entry_count: num_chunks as u32,
        })
    }

    /// Disables HMB and frees allocated descriptors.
    pub fn release_hmb(&mut self) -> HmbFeatureCommand {
        self.state = HmbState::Disabled;
        let cmd = HmbFeatureCommand {
            feature_id: NVME_FEAT_HOST_MEMORY_BUFFER,
            enable_hmb: false,
            memory_return: false,
            total_pages: 0,
            descriptor_list_addr: 0,
            descriptor_entry_count: 0,
        };

        self.allocated_bytes = 0;
        self.descriptor_count = 0;
        self.descriptors = [HostMemoryDescriptor::empty(); HMDL_MAX_ENTRIES];
        cmd
    }
}

/// Unit test verifying NVMe HMB protocol negotiation and HMDL descriptor list.
pub fn verify_nvme_hmb_negotiation() -> bool {
    let mut alloc = HmbAllocator::new(64 * 1024 * 1024, 4 * 1024 * 1024);
    if alloc.state != HmbState::Disabled {
        return false;
    }

    // 1. Allocate full 64 MiB HMB for DRAM-less SSD
    let cmd = match alloc.allocate_hmb(64 * 1024 * 1024) {
        Ok(c) => c,
        Err(_) => return false,
    };

    if !cmd.enable_hmb || cmd.feature_id != NVME_FEAT_HOST_MEMORY_BUFFER {
        return false;
    }
    // 64 MiB = 16,384 pages of 4 KiB
    if cmd.total_pages != 16384 || cmd.descriptor_entry_count != 16 {
        return false;
    }
    if alloc.state != HmbState::Active || alloc.allocated_bytes != (64 * 1024 * 1024) {
        return false;
    }

    // Check first and last descriptor entries
    if alloc.descriptors[0].buffer_size_pages != 1024 {
        return false;
    }
    if alloc.descriptors[15].buffer_address != (0x0000_0001_8100_0000 + 15 * 4 * 1024 * 1024) {
        return false;
    }

    // 2. Release HMB
    let release_cmd = alloc.release_hmb();
    if release_cmd.enable_hmb || alloc.state != HmbState::Disabled || alloc.allocated_bytes != 0 {
        return false;
    }

    true
}


// ============================================================================
// DirectStorage NVMe Lock-Free Submission & Completion Bypass Rings
// ============================================================================

/// Maximum capacity of DirectStorage lock-free bypass submission/completion queues.
pub const DIRECT_STORAGE_RING_SIZE: usize = 64;

/// DirectStorage NVMe read command targeting GPU staging physical memory directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectStorageBypassCommand {
    pub command_id: u16,
    pub nsid: u32,
    pub start_lba: u64,
    pub sector_count: u32,
    pub gpu_staging_phys_addr: u64,
    pub asset_id: u32,
}

impl DirectStorageBypassCommand {
    pub const fn empty() -> Self {
        Self {
            command_id: 0,
            nsid: 0,
            start_lba: 0,
            sector_count: 0,
            gpu_staging_phys_addr: 0,
            asset_id: 0,
        }
    }
}

/// Completion event from NVMe DMA transfer directly into GPU staging buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectStorageBypassCompletion {
    pub command_id: u16,
    pub sq_head: u16,
    pub status: u16,
    pub asset_id: u32,
}

/// Lock-free high-throughput submission and completion rings (SQ/CQ).
#[derive(Debug, Clone)]
pub struct DirectStorageBypassRing {
    pub sq: [DirectStorageBypassCommand; DIRECT_STORAGE_RING_SIZE],
    pub sq_head: u16,
    pub sq_tail: u16,
    pub sq_count: usize,
    pub cq: [DirectStorageBypassCompletion; DIRECT_STORAGE_RING_SIZE],
    pub cq_head: u16,
    pub cq_tail: u16,
    pub cq_count: usize,
    pub next_command_id: u16,
    pub total_submitted: u64,
    pub total_completed: u64,
}

impl DirectStorageBypassRing {
    pub const fn new() -> Self {
        Self {
            sq: [DirectStorageBypassCommand::empty(); DIRECT_STORAGE_RING_SIZE],
            sq_head: 0,
            sq_tail: 0,
            sq_count: 0,
            cq: [DirectStorageBypassCompletion { command_id: 0, sq_head: 0, status: 0, asset_id: 0 }; DIRECT_STORAGE_RING_SIZE],
            cq_head: 0,
            cq_tail: 0,
            cq_count: 0,
            next_command_id: 1,
            total_submitted: 0,
            total_completed: 0,
        }
    }

    /// Submits a raw compressed game asset read bypassing VFS syscall overhead.
    pub fn submit_gpu_read(
        &mut self,
        nsid: u32,
        start_lba: u64,
        sector_count: u32,
        gpu_staging_phys_addr: u64,
        asset_id: u32,
    ) -> Result<u16, &'static str> {
        if self.sq_count >= DIRECT_STORAGE_RING_SIZE {
            return Err("Submission queue full");
        }

        let cid = self.next_command_id;
        self.next_command_id = if self.next_command_id == u16::MAX { 1 } else { self.next_command_id + 1 };

        let cmd = DirectStorageBypassCommand {
            command_id: cid,
            nsid,
            start_lba,
            sector_count,
            gpu_staging_phys_addr,
            asset_id,
        };

        self.sq[self.sq_tail as usize] = cmd;
        self.sq_tail = (self.sq_tail + 1) % (DIRECT_STORAGE_RING_SIZE as u16);
        self.sq_count += 1;
        self.total_submitted += 1;

        Ok(cid)
    }

    /// Records hardware completion from NVMe DMA controller.
    pub fn complete_command(&mut self, command_id: u16, status: u16, asset_id: u32) -> Result<(), &'static str> {
        if self.cq_count >= DIRECT_STORAGE_RING_SIZE {
            return Err("Completion queue full");
        }

        let cpl = DirectStorageBypassCompletion {
            command_id,
            sq_head: self.sq_head,
            status,
            asset_id,
        };

        self.cq[self.cq_tail as usize] = cpl;
        self.cq_tail = (self.cq_tail + 1) % (DIRECT_STORAGE_RING_SIZE as u16);
        self.cq_count += 1;
        self.total_completed += 1;

        // Advance SQ head if matching front command
        if self.sq_count > 0 && self.sq[self.sq_head as usize].command_id == command_id {
            self.sq_head = (self.sq_head + 1) % (DIRECT_STORAGE_RING_SIZE as u16);
            self.sq_count -= 1;
        }

        Ok(())
    }

    /// Polls next completion event without blocking.
    pub fn poll_completion(&mut self) -> Option<DirectStorageBypassCompletion> {
        if self.cq_count == 0 {
            return None;
        }

        let cpl = self.cq[self.cq_head as usize];
        self.cq_head = (self.cq_head + 1) % (DIRECT_STORAGE_RING_SIZE as u16);
        self.cq_count -= 1;
        Some(cpl)
    }
}

/// Unit test verifying DirectStorage NVMe lock-free bypass ring.
pub fn verify_directstorage_bypass_ring() -> bool {
    let mut ring = DirectStorageBypassRing::new();

    // 1. Submit 3 GPU-direct asset reads (e.g. 4K texture, mesh, audio bank)
    let cid1 = ring.submit_gpu_read(1, 100_000, 2048, 0x0000_3800_1000_0000, 501);
    let cid2 = ring.submit_gpu_read(1, 102_048, 1024, 0x0000_3800_1040_0000, 502);
    let cid3 = ring.submit_gpu_read(1, 103_072, 4096, 0x0000_3800_1060_0000, 503);

    if cid1 != Ok(1) || cid2 != Ok(2) || cid3 != Ok(3) {
        return false;
    }
    if ring.sq_count != 3 || ring.total_submitted != 3 {
        return false;
    }

    // 2. Complete first two commands
    if ring.complete_command(1, 0, 501).is_err() {
        return false;
    }
    if ring.complete_command(2, 0, 502).is_err() {
        return false;
    }

    // 3. Poll completions
    let cpl1 = match ring.poll_completion() { Some(c) => c, None => return false };
    let cpl2 = match ring.poll_completion() { Some(c) => c, None => return false };

    if cpl1.command_id != 1 || cpl1.asset_id != 501 || cpl1.status != 0 {
        return false;
    }
    if cpl2.command_id != 2 || cpl2.asset_id != 502 || cpl2.status != 0 {
        return false;
    }

    if ring.cq_count != 0 || ring.total_completed != 2 {
        return false;
    }

    true
}


// ============================================================================
// HMB Power-State Eviction & Standby Flushing Governor
// ============================================================================

/// Handheld storage subsystem power state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandheldStoragePowerState {
    /// Normal operational state; HMB active and DirectStorage rings open.
    AwakeActive,
    /// Fast flush and HMB deallocation in progress before D3hot/D3cold sleep.
    EnteringSuspend,
    /// Fully suspended; PCIe link in L1/L2 and host buffers isolated.
    Suspended,
    /// Restoring PCIe link and re-establishing HMB contract after wake.
    Resuming,
}

/// Governor coordinating NVMe cache flushing and safe HMB eviction during suspend.
#[derive(Debug, Clone)]
pub struct HmbPowerGovernor {
    pub power_state: HandheldStoragePowerState,
    pub flush_completed: bool,
    pub hmb_evicted: bool,
    pub eviction_count: u32,
    pub restore_count: u32,
}

impl HmbPowerGovernor {
    pub const fn new() -> Self {
        Self {
            power_state: HandheldStoragePowerState::AwakeActive,
            flush_completed: false,
            hmb_evicted: false,
            eviction_count: 0,
            restore_count: 0,
        }
    }

    /// Prepares NVMe controller for handheld instant suspend.
    pub fn prepare_for_suspend(&mut self, allocator: &mut HmbAllocator) -> Result<HmbFeatureCommand, &'static str> {
        if self.power_state != HandheldStoragePowerState::AwakeActive {
            return Err("Not in active state");
        }

        self.power_state = HandheldStoragePowerState::EnteringSuspend;
        // 1. Mark volatile write caches flushed to persistent NAND
        self.flush_completed = true;

        // 2. Command SSD controller to flush FTL and release Host Memory Buffer
        let disable_cmd = allocator.release_hmb();
        self.hmb_evicted = true;
        self.eviction_count += 1;

        Ok(disable_cmd)
    }

    /// Completes suspend once PCIe link enters D3hot / L1.2.
    pub fn complete_suspend(&mut self) -> Result<(), &'static str> {
        if self.power_state != HandheldStoragePowerState::EnteringSuspend {
            return Err("Suspend not prepared");
        }
        self.power_state = HandheldStoragePowerState::Suspended;
        Ok(())
    }

    /// Re-allocates HMB and restores high-speed DRAM-less SSD performance upon resume.
    pub fn resume_from_suspend(
        &mut self,
        allocator: &mut HmbAllocator,
        requested_bytes: u64,
    ) -> Result<HmbFeatureCommand, &'static str> {
        if self.power_state != HandheldStoragePowerState::Suspended {
            return Err("Not in suspended state");
        }

        self.power_state = HandheldStoragePowerState::Resuming;

        // Re-establish HMDL mapping in host DRAM
        let enable_cmd = allocator.allocate_hmb(requested_bytes)?;

        self.hmb_evicted = false;
        self.flush_completed = false;
        self.power_state = HandheldStoragePowerState::AwakeActive;
        self.restore_count += 1;

        Ok(enable_cmd)
    }
}

/// Unit test verifying HMB power-state eviction and standby cache flushing.
pub fn verify_hmb_power_state_eviction() -> bool {
    let mut alloc = HmbAllocator::new(64 * 1024 * 1024, 4 * 1024 * 1024);
    let mut gov = HmbPowerGovernor::new();

    // 1. Initial active allocation
    if alloc.allocate_hmb(64 * 1024 * 1024).is_err() || alloc.state != HmbState::Active {
        return false;
    }

    // 2. Enter suspend: flush and evict HMB
    let dis_cmd = match gov.prepare_for_suspend(&mut alloc) {
        Ok(c) => c,
        Err(_) => return false,
    };

    if dis_cmd.enable_hmb || alloc.state != HmbState::Disabled || !gov.flush_completed || !gov.hmb_evicted {
        return false;
    }

    // 3. Complete suspend
    if gov.complete_suspend().is_err() || gov.power_state != HandheldStoragePowerState::Suspended {
        return false;
    }

    // 4. Resume from suspend: re-allocate HMB
    let en_cmd = match gov.resume_from_suspend(&mut alloc, 64 * 1024 * 1024) {
        Ok(c) => c,
        Err(_) => return false,
    };

    if !en_cmd.enable_hmb || alloc.state != HmbState::Active || alloc.allocated_bytes != (64 * 1024 * 1024) {
        return false;
    }
    if gov.power_state != HandheldStoragePowerState::AwakeActive || gov.hmb_evicted {
        return false;
    }
    if gov.eviction_count != 1 || gov.restore_count != 1 {
        return false;
    }

    true
}


// ============================================================================
// Integrated NVMe HMB Allocator & DirectStorage IO Coordinator
// ============================================================================

/// Integrated Handheld Direct NVMe Host Memory Buffer & DirectStorage IO Coordinator.
#[derive(Debug, Clone)]
pub struct NvmeHmbManager {
    pub allocator: HmbAllocator,
    pub io_ring: DirectStorageBypassRing,
    pub power_governor: HmbPowerGovernor,
}

impl NvmeHmbManager {
    pub const fn new() -> Self {
        Self {
            allocator: HmbAllocator::new(64 * 1024 * 1024, 4 * 1024 * 1024),
            io_ring: DirectStorageBypassRing::new(),
            power_governor: HmbPowerGovernor::new(),
        }
    }

    /// Initializes HMB allocation and configures FTL host cache.
    pub fn initialize(&mut self, requested_bytes: u64) -> Result<HmbFeatureCommand, &'static str> {
        self.allocator.allocate_hmb(requested_bytes)
    }

    /// Queues a zero-copy DirectStorage asset read directly into GPU VRAM/GTT memory.
    pub fn submit_asset_stream(
        &mut self,
        nsid: u32,
        start_lba: u64,
        sector_count: u32,
        gpu_staging_phys_addr: u64,
        asset_id: u32,
    ) -> Result<u16, &'static str> {
        if self.power_governor.power_state != HandheldStoragePowerState::AwakeActive {
            return Err("Storage device suspended");
        }
        self.io_ring.submit_gpu_read(nsid, start_lba, sector_count, gpu_staging_phys_addr, asset_id)
    }

    /// Handles NVMe DMA completion for an asset stream.
    pub fn process_completion(&mut self, command_id: u16, asset_id: u32) -> Result<(), &'static str> {
        self.io_ring.complete_command(command_id, 0, asset_id)
    }

    /// Executes safe instant suspend sequence.
    pub fn suspend(&mut self) -> Result<HmbFeatureCommand, &'static str> {
        let cmd = self.power_governor.prepare_for_suspend(&mut self.allocator)?;
        self.power_governor.complete_suspend()?;
        Ok(cmd)
    }

    /// Restores HMB upon wake from suspend.
    pub fn resume(&mut self, requested_bytes: u64) -> Result<HmbFeatureCommand, &'static str> {
        self.power_governor.resume_from_suspend(&mut self.allocator, requested_bytes)
    }
}

/// Unit test verifying integrated NVMe HMB allocator and DirectStorage coordinator.
pub fn verify_nvme_hmb_manager() -> bool {
    let mut mgr = NvmeHmbManager::new();

    // 1. Initialize HMB with 64 MiB
    let init_cmd = match mgr.initialize(64 * 1024 * 1024) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if !init_cmd.enable_hmb || mgr.allocator.state != HmbState::Active {
        return false;
    }

    // 2. Stream game texture asset (ID 777)
    let cid = match mgr.submit_asset_stream(1, 500_000, 4096, 0x0000_3800_2000_0000, 777) {
        Ok(id) => id,
        Err(_) => return false,
    };
    if cid != 1 || mgr.io_ring.sq_count != 1 {
        return false;
    }

    // 3. Complete transfer and poll completion
    if mgr.process_completion(cid, 777).is_err() {
        return false;
    }
    let cpl = match mgr.io_ring.poll_completion() {
        Some(c) => c,
        None => return false,
    };
    if cpl.command_id != 1 || cpl.asset_id != 777 {
        return false;
    }

    // 4. Suspend system: HMB evicted
    if mgr.suspend().is_err() || mgr.allocator.state != HmbState::Disabled {
        return false;
    }
    // Submitting reads while suspended must fail
    if mgr.submit_asset_stream(1, 500_000, 4096, 0x0000_3800_2000_0000, 778).is_ok() {
        return false;
    }

    // 5. Resume system: HMB restored
    let res_cmd = match mgr.resume(64 * 1024 * 1024) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if !res_cmd.enable_hmb || mgr.allocator.state != HmbState::Active {
        return false;
    }

    true
}


/// Comprehensive verification self-test suite for Handheld Direct NVMe Host Memory Buffer (HMB) Dynamic Allocator & DirectStorage IO Ring.
pub fn test_nvme_hmb_subsystem() -> bool {
    crate::serial_write_str("[KNT-STORAGE] Testing Direct NVMe Host Memory Buffer & DirectStorage IO Ring...\r\n");
    if !verify_nvme_hmb_negotiation() {
        return false;
    }
    if !verify_directstorage_bypass_ring() {
        return false;
    }
    if !verify_hmb_power_state_eviction() {
        return false;
    }
    if !verify_nvme_hmb_manager() {
        return false;
    }
    crate::serial_write_str("[KNT-STORAGE-HMB-PASS] Handheld direct NVMe host memory buffer and DirectStorage IO ring verified\r\n");
    true
}
