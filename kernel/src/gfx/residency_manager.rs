//! Direct3D 12 Enhanced Memory Pools & Dedicated VRAM Heap Residency Governor.
//! Implements multi-tier memory pools, page properties, priority-ranked LRU eviction, and compaction.

#![allow(dead_code, non_snake_case, non_camel_case_types, unused_imports)]

extern crate alloc;
use alloc::vec::Vec;

// Direct3D 12 physical memory pool classification.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum D3D12_MEMORY_POOL {
    D3D12_MEMORY_POOL_UNKNOWN = 0,
    // Dedicated high-bandwidth local GPU VRAM.
    D3D12_MEMORY_POOL_L0 = 1,
    // Host system RAM accessible via PCIe / unified bus.
    D3D12_MEMORY_POOL_L1 = 2,
}

// CPU page caching properties for GPU memory heaps.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum D3D12_CPU_PAGE_PROPERTY {
    D3D12_CPU_PAGE_PROPERTY_UNKNOWN = 0,
    D3D12_CPU_PAGE_PROPERTY_NOT_AVAILABLE = 1,
    D3D12_CPU_PAGE_PROPERTY_WRITE_COMBINE = 2,
    D3D12_CPU_PAGE_PROPERTY_WRITE_BACK = 3,
}

// Direct3D 12 resource residency priority tiers.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum D3D12_RESIDENCY_PRIORITY {
    D3D12_RESIDENCY_PRIORITY_LOW = 0x5000_0000,
    D3D12_RESIDENCY_PRIORITY_NORMAL = 0x7800_0000,
    D3D12_RESIDENCY_PRIORITY_HIGH = 0xA001_0000,
    D3D12_RESIDENCY_PRIORITY_CRITICAL = 0xC800_0000,
}

// Custom memory heap properties configuring pool and caching behaviors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_HEAP_PROPERTIES {
    pub cpu_page_property: D3D12_CPU_PAGE_PROPERTY,
    pub memory_pool_preference: D3D12_MEMORY_POOL,
    pub creation_node_mask: u32,
    pub visible_node_mask: u32,
}

impl D3D12_HEAP_PROPERTIES {
    // Default properties for dedicated device local GPU VRAM.
    pub const fn default_l0_vram() -> Self {
        Self {
            cpu_page_property: D3D12_CPU_PAGE_PROPERTY::D3D12_CPU_PAGE_PROPERTY_NOT_AVAILABLE,
            memory_pool_preference: D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0,
            creation_node_mask: 0,
            visible_node_mask: 0,
        }
    }

    // Default properties for CPU upload heap in system memory with write-combining.
    pub const fn default_l1_upload() -> Self {
        Self {
            cpu_page_property: D3D12_CPU_PAGE_PROPERTY::D3D12_CPU_PAGE_PROPERTY_WRITE_COMBINE,
            memory_pool_preference: D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L1,
            creation_node_mask: 0,
            visible_node_mask: 0,
        }
    }
}

// Memory heap descriptor used to allocate physical GPU backing store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_HEAP_DESC {
    pub size_in_bytes: u64,
    pub properties: D3D12_HEAP_PROPERTIES,
    pub alignment: u64,
    pub flags: u32,
}

// Managed GPU allocation tracked by the residency governor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedHeapAllocation {
    pub allocation_id: u64,
    pub size_in_bytes: u64,
    pub pool: D3D12_MEMORY_POOL,
    pub priority: D3D12_RESIDENCY_PRIORITY,
    pub is_resident: bool,
    pub is_aliased_transient: bool,
    pub last_used_timestamp: u64,
}

impl ManagedHeapAllocation {
    pub const fn new(
        allocation_id: u64,
        size_in_bytes: u64,
        pool: D3D12_MEMORY_POOL,
        priority: D3D12_RESIDENCY_PRIORITY,
        is_aliased_transient: bool,
    ) -> Self {
        Self {
            allocation_id,
            size_in_bytes,
            pool,
            priority,
            is_resident: false,
            is_aliased_transient,
            last_used_timestamp: 0,
        }
    }
}

// Residency subsystem error conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidencyError {
    AllocationNotFound,
    AlreadyResident,
    NotResident,
    OutOfMemory,
    BudgetExceeded,
    InvalidPriority,
}

// Residency manager orchestrating VRAM budget tracking and priority LRU eviction.
#[derive(Debug, Clone)]
pub struct ResidencyManager {
    pub allocations: Vec<ManagedHeapAllocation>,
    pub vram_budget_bytes: u64,
    pub current_vram_resident_bytes: u64,
    pub current_system_resident_bytes: u64,
    pub current_timestamp: u64,
    pub total_evictions_count: u64,
    pub total_promotions_count: u64,
}

impl ResidencyManager {
    // Creates a new residency manager with a designated VRAM byte budget.
    pub fn new(vram_budget_bytes: u64) -> Self {
        Self {
            allocations: Vec::new(),
            vram_budget_bytes,
            current_vram_resident_bytes: 0,
            current_system_resident_bytes: 0,
            current_timestamp: 1,
            total_evictions_count: 0,
            total_promotions_count: 0,
        }
    }

    // Registers a newly created heap allocation into the tracked pool.
    pub fn register_allocation(&mut self, alloc: ManagedHeapAllocation) {
        self.allocations.push(alloc);
    }

    // Updates residency priority ranking for a tracked allocation.
    pub fn set_residency_priority(
        &mut self,
        allocation_id: u64,
        priority: D3D12_RESIDENCY_PRIORITY,
    ) -> Result<(), ResidencyError> {
        let alloc = self
            .allocations
            .iter_mut()
            .find(|a| a.allocation_id == allocation_id)
            .ok_or(ResidencyError::AllocationNotFound)?;
        alloc.priority = priority;
        Ok(())
    }

    // Marks an allocation as recently used to update its LRU status.
    pub fn touch_allocation(&mut self, allocation_id: u64) {
        self.current_timestamp = self.current_timestamp.saturating_add(1);
        if let Some(alloc) = self.allocations.iter_mut().find(|a| a.allocation_id == allocation_id) {
            alloc.last_used_timestamp = self.current_timestamp;
        }
    }

    // Finds the best candidate allocation to evict: lowest priority, then oldest LRU timestamp.
    pub fn find_eviction_candidate(&self) -> Option<u64> {
        self.allocations
            .iter()
            .filter(|a| a.is_resident && a.pool == D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0)
            .filter(|a| a.priority < D3D12_RESIDENCY_PRIORITY::D3D12_RESIDENCY_PRIORITY_CRITICAL)
            .min_by(|a, b| {
                match a.priority.cmp(&b.priority) {
                    core::cmp::Ordering::Equal => a.last_used_timestamp.cmp(&b.last_used_timestamp),
                    other => other,
                }
            })
            .map(|a| a.allocation_id)
    }

    // Direct3D 12 ID3D12Device::Evict: evicts allocations from VRAM back to non-resident status.
    pub fn Evict(&mut self, allocation_ids: &[u64]) -> Result<u64, ResidencyError> {
        let mut bytes_evicted: u64 = 0;
        for &id in allocation_ids {
            let alloc = self
                .allocations
                .iter_mut()
                .find(|a| a.allocation_id == id)
                .ok_or(ResidencyError::AllocationNotFound)?;

            if alloc.is_resident {
                alloc.is_resident = false;
                if alloc.pool == D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0 {
                    self.current_vram_resident_bytes = self
                        .current_vram_resident_bytes
                        .saturating_sub(alloc.size_in_bytes);
                } else {
                    self.current_system_resident_bytes = self
                        .current_system_resident_bytes
                        .saturating_sub(alloc.size_in_bytes);
                }
                bytes_evicted = bytes_evicted.saturating_add(alloc.size_in_bytes);
                self.total_evictions_count = self.total_evictions_count.saturating_add(1);
            }
        }
        Ok(bytes_evicted)
    }

    // Direct3D 12 ID3D12Device::EnqueueMakeResident: ensures allocations reside in VRAM.
    pub fn EnqueueMakeResident(&mut self, allocation_ids: &[u64]) -> Result<u64, ResidencyError> {
        for &id in allocation_ids {
            self.current_timestamp = self.current_timestamp.saturating_add(1);
            let (size, pool) = {
                let alloc = self
                    .allocations
                    .iter_mut()
                    .find(|a| a.allocation_id == id)
                    .ok_or(ResidencyError::AllocationNotFound)?;

                if alloc.is_resident {
                    alloc.last_used_timestamp = self.current_timestamp;
                    continue;
                }
                (alloc.size_in_bytes, alloc.pool)
            };

            // If local VRAM allocation, evict candidates until budget accommodates new allocation
            if pool == D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0 {
                while self.current_vram_resident_bytes.saturating_add(size) > self.vram_budget_bytes {
                    if let Some(candidate_id) = self.find_eviction_candidate() {
                        self.Evict(&[candidate_id])?;
                    } else {
                        return Err(ResidencyError::BudgetExceeded);
                    }
                }
            }

            let alloc = self
                .allocations
                .iter_mut()
                .find(|a| a.allocation_id == id)
                .ok_or(ResidencyError::AllocationNotFound)?;

            alloc.is_resident = true;
            alloc.last_used_timestamp = self.current_timestamp;
            if pool == D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0 {
                self.current_vram_resident_bytes = self
                    .current_vram_resident_bytes
                    .saturating_add(size);
            } else {
                self.current_system_resident_bytes = self
                    .current_system_resident_bytes
                    .saturating_add(size);
            }
            self.total_promotions_count = self.total_promotions_count.saturating_add(1);
        }

        Ok(self.current_vram_resident_bytes)
    }
}

// Pressure classification for handheld APU shared memory architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApuMemoryPressureLevel {
    Normal,
    Warning,
    Critical,
}

// Background memory governor preventing frame hitching under handheld APU budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApuMemoryBudgetGovernor {
    pub warning_threshold_pct: u32,
    pub critical_threshold_pct: u32,
    pub target_headroom_pct: u32,
    pub total_background_paged_bytes: u64,
}

impl Default for ApuMemoryBudgetGovernor {
    fn default() -> Self {
        Self {
            warning_threshold_pct: 75,
            critical_threshold_pct: 90,
            target_headroom_pct: 15,
            total_background_paged_bytes: 0,
        }
    }
}

impl ApuMemoryBudgetGovernor {
    // Creates a new APU memory budget governor with default handheld thresholds.
    pub fn new() -> Self {
        Self::default()
    }

    // Assesses live memory pressure level against configured budget thresholds.
    pub fn assess_pressure(&self, manager: &ResidencyManager) -> ApuMemoryPressureLevel {
        if manager.vram_budget_bytes == 0 {
            return ApuMemoryPressureLevel::Normal;
        }
        let usage_pct = (manager.current_vram_resident_bytes.saturating_mul(100)) / manager.vram_budget_bytes;
        if usage_pct >= self.critical_threshold_pct as u64 {
            ApuMemoryPressureLevel::Critical
        } else if usage_pct >= self.warning_threshold_pct as u64 {
            ApuMemoryPressureLevel::Warning
        } else {
            ApuMemoryPressureLevel::Normal
        }
    }

    // Runs a background proactive paging pass to reclaim headroom before allocations fail.
    pub fn run_background_paging_worker(&mut self, manager: &mut ResidencyManager) -> u64 {
        let pressure = self.assess_pressure(manager);
        if pressure == ApuMemoryPressureLevel::Normal {
            return 0;
        }

        let target_bytes = manager.vram_budget_bytes
            .saturating_sub((manager.vram_budget_bytes * self.target_headroom_pct as u64) / 100);

        let mut paged_out: u64 = 0;
        while manager.current_vram_resident_bytes > target_bytes {
            if let Some(candidate_id) = manager.find_eviction_candidate() {
                if let Ok(evicted) = manager.Evict(&[candidate_id]) {
                    paged_out = paged_out.saturating_add(evicted);
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        self.total_background_paged_bytes = self.total_background_paged_bytes.saturating_add(paged_out);
        paged_out
    }

    // Queries budget metrics: total budget, resident VRAM, and percentage utilized.
    pub fn query_budget_statistics(&self, manager: &ResidencyManager) -> (u64, u64, u32) {
        let usage_pct = if manager.vram_budget_bytes > 0 {
            (manager.current_vram_resident_bytes.saturating_mul(100)) / manager.vram_budget_bytes
        } else {
            0
        } as u32;
        (manager.vram_budget_bytes, manager.current_vram_resident_bytes, usage_pct)
    }
}

// Heap compaction engine trimming stale aliased render targets and consolidating memory gaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeapCompactionEngine {
    pub total_compaction_passes: u64,
    pub total_bytes_reclaimed: u64,
}

impl HeapCompactionEngine {
    // Creates a new compaction engine instance.
    pub fn new() -> Self {
        Self::default()
    }

    // Trims stale aliased transient render targets that have not been touched recently.
    pub fn trim_stale_aliased_targets(
        &mut self,
        manager: &mut ResidencyManager,
        stale_threshold_ticks: u64,
    ) -> u64 {
        let current_time = manager.current_timestamp;
        let mut stale_ids = Vec::new();

        for alloc in &manager.allocations {
            if alloc.is_aliased_transient
                && alloc.is_resident
                && current_time.saturating_sub(alloc.last_used_timestamp) >= stale_threshold_ticks
            {
                stale_ids.push(alloc.allocation_id);
            }
        }

        let reclaimed = manager.Evict(&stale_ids).unwrap_or(0);
        if reclaimed > 0 {
            self.total_compaction_passes = self.total_compaction_passes.saturating_add(1);
            self.total_bytes_reclaimed = self.total_bytes_reclaimed.saturating_add(reclaimed);
        }
        reclaimed
    }

    // Unregisters completely stale evicted transient allocations to free tracking metadata.
    pub fn purge_evicted_transients(&mut self, manager: &mut ResidencyManager) -> usize {
        let before_count = manager.allocations.len();
        manager.allocations.retain(|a| !(a.is_aliased_transient && !a.is_resident));
        before_count.saturating_sub(manager.allocations.len())
    }

    // Performs full heap compaction: trims stale transients and purges evicted slots.
    pub fn execute_full_compaction(
        &mut self,
        manager: &mut ResidencyManager,
        stale_threshold_ticks: u64,
    ) -> (u64, usize) {
        let bytes_trimmed = self.trim_stale_aliased_targets(manager, stale_threshold_ticks);
        let purged_count = self.purge_evicted_transients(manager);
        (bytes_trimmed, purged_count)
    }
}

// Comprehensive self-test suite for Direct3D 12 Enhanced Memory Pools & Residency Governor.
pub fn test_residency_manager_subsystem() -> bool {
    // 1. Verify memory pool and page property descriptors
    let l0_props = D3D12_HEAP_PROPERTIES::default_l0_vram();
    if l0_props.memory_pool_preference != D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0
        || l0_props.cpu_page_property != D3D12_CPU_PAGE_PROPERTY::D3D12_CPU_PAGE_PROPERTY_NOT_AVAILABLE
    {
        return false;
    }

    let l1_props = D3D12_HEAP_PROPERTIES::default_l1_upload();
    if l1_props.memory_pool_preference != D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L1
        || l1_props.cpu_page_property != D3D12_CPU_PAGE_PROPERTY::D3D12_CPU_PAGE_PROPERTY_WRITE_COMBINE
    {
        return false;
    }

    // 2. Verify priority-based LRU eviction and priority protection
    let mb: u64 = 1024 * 1024;
    let mut manager = ResidencyManager::new(100 * mb);

    // Alloc 1: 40MB Low priority
    manager.register_allocation(ManagedHeapAllocation::new(
        1,
        40 * mb,
        D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0,
        D3D12_RESIDENCY_PRIORITY::D3D12_RESIDENCY_PRIORITY_LOW,
        false,
    ));

    // Alloc 2: 40MB Normal priority
    manager.register_allocation(ManagedHeapAllocation::new(
        2,
        40 * mb,
        D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0,
        D3D12_RESIDENCY_PRIORITY::D3D12_RESIDENCY_PRIORITY_NORMAL,
        false,
    ));

    // Alloc 3: 15MB Critical priority (e.g. active swapchain backbuffer)
    manager.register_allocation(ManagedHeapAllocation::new(
        3,
        15 * mb,
        D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0,
        D3D12_RESIDENCY_PRIORITY::D3D12_RESIDENCY_PRIORITY_CRITICAL,
        false,
    ));

    // Make allocations 1, 2, 3 resident (total 95MB <= 100MB budget)
    if manager.EnqueueMakeResident(&[1, 2, 3]).is_err() {
        return false;
    }
    if manager.current_vram_resident_bytes != 95 * mb {
        return false;
    }

    // Alloc 4: 20MB High priority. Demands eviction of 15MB to fit in 100MB budget.
    manager.register_allocation(ManagedHeapAllocation::new(
        4,
        20 * mb,
        D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0,
        D3D12_RESIDENCY_PRIORITY::D3D12_RESIDENCY_PRIORITY_HIGH,
        false,
    ));

    if manager.EnqueueMakeResident(&[4]).is_err() {
        return false;
    }

    // Alloc 1 (Low) must have been evicted. Alloc 2 (Normal), 3 (Critical), 4 (High) must remain resident.
    let a1 = manager.allocations.iter().find(|a| a.allocation_id == 1).unwrap();
    let a2 = manager.allocations.iter().find(|a| a.allocation_id == 2).unwrap();
    let a3 = manager.allocations.iter().find(|a| a.allocation_id == 3).unwrap();
    let a4 = manager.allocations.iter().find(|a| a.allocation_id == 4).unwrap();

    if a1.is_resident || !a2.is_resident || !a3.is_resident || !a4.is_resident {
        return false;
    }
    // Resident: 40MB + 15MB + 20MB = 75MB
    if manager.current_vram_resident_bytes != 75 * mb {
        return false;
    }

    // 3. Verify APU memory budget governor and background proactive paging
    let mut governor = ApuMemoryBudgetGovernor::new();
    // 75MB / 100MB = 75% -> Warning level
    if governor.assess_pressure(&manager) != ApuMemoryPressureLevel::Warning {
        return false;
    }

    // Register and make resident Alloc 5 (18MB Normal) -> 75 + 18 = 93MB (93% > 90% Critical)
    manager.register_allocation(ManagedHeapAllocation::new(
        5,
        18 * mb,
        D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0,
        D3D12_RESIDENCY_PRIORITY::D3D12_RESIDENCY_PRIORITY_NORMAL,
        false,
    ));
    if manager.EnqueueMakeResident(&[5]).is_err() {
        return false;
    }
    if governor.assess_pressure(&manager) != ApuMemoryPressureLevel::Critical {
        return false;
    }

    // Background paging worker must trim down to target headroom (<= 85MB)
    let paged = governor.run_background_paging_worker(&mut manager);
    if paged == 0 || manager.current_vram_resident_bytes > 85 * mb {
        return false;
    }
    if governor.assess_pressure(&manager) != ApuMemoryPressureLevel::Normal {
        return false;
    }

    // 4. Verify memory compaction of stale aliased transient render targets
    manager.register_allocation(ManagedHeapAllocation::new(
        6,
        10 * mb,
        D3D12_MEMORY_POOL::D3D12_MEMORY_POOL_L0,
        D3D12_RESIDENCY_PRIORITY::D3D12_RESIDENCY_PRIORITY_NORMAL,
        true,
    ));
    if manager.EnqueueMakeResident(&[6]).is_err() {
        return false;
    }

    // Advance timestamp to simulate stale render target
    manager.current_timestamp = manager.current_timestamp.saturating_add(25);

    let mut compactor = HeapCompactionEngine::new();
    let (trimmed_bytes, purged) = compactor.execute_full_compaction(&mut manager, 10);
    if trimmed_bytes != 10 * mb || purged != 1 {
        return false;
    }

    crate::serial_write_str("[KNT-D3D12-RESIDENCY-PASS] Direct3D 12 enhanced memory pools & VRAM residency governor verified\r\n");
    true
}
