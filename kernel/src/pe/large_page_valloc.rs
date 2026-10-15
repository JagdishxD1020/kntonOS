//! Win32 Memory Management: Large Page (2MB/1GB) Allocations & VirtualAlloc2 Extended Attributes Engine.
//!
//! Implements Windows NT VirtualAlloc2 / NtAllocateVirtualMemoryEx, MEM_LARGE_PAGES (2MB) and 1GB huge page
//! allocation, MEM_EXTENDED_PARAMETER_NUMA_NODE affinity binding, PML2/PML3 page table mapping, graceful
//! fallback to 4KB demand paging, and anti-cheat W^X kernel page protection.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
use crate::pe::ntdll::{
    nt_error, nt_status_to_str, nt_success, NtStatus,
    STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_NO_MEMORY,
    STATUS_SUCCESS, STATUS_UNSUCCESSFUL,
};
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;
pub use crate::pe::virtual_alloc::{
    MEM_COMMIT, MEM_DECOMMIT, MEM_LARGE_PAGES, MEM_RELEASE, MEM_RESERVE,
    PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_GUARD,
    PAGE_NOACCESS, PAGE_NOCACHE, PAGE_READONLY, PAGE_READWRITE,
};

pub const PAGE_SIZE_4KB: usize = 4096;
pub const PAGE_SIZE_2MB: usize = 2 * 1024 * 1024;
pub const PAGE_SIZE_1GB: usize = 1024 * 1024 * 1024;

pub const MEM_4MB_PAGES: u32 = 0x8000_0000;
pub const MEM_RESERVE_PLACEHOLDER: u32 = 0x0004_0000;
pub const MEM_REPLACE_PLACEHOLDER: u32 = 0x0000_4000;

/// Extended parameter types supported by VirtualAlloc2.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemExtendedParameterType {
    InvalidType = 0,
    AddressRequirements = 1,
    NumaNode = 2,
    PartitionHandle = 3,
    UserPhysicalHandle = 4,
    AttributeFlags = 5,
    ImageMachine = 6,
}

/// Address boundary and alignment constraints for VirtualAlloc2.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemAddressRequirements {
    pub lowest_starting_address: usize,
    pub highest_ending_address: usize,
    pub alignment: usize,
}

/// Extended attribute parameter descriptor for VirtualAlloc2 / MapViewOfFile3.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MEM_EXTENDED_PARAMETER {
    pub parameter_type: u64,
    pub arg: u64,
}

impl MEM_EXTENDED_PARAMETER {
    pub fn new(param_type: MemExtendedParameterType, arg: u64) -> Self {
        Self {
            parameter_type: param_type as u64,
            arg,
        }
    }

    pub fn get_type(&self) -> MemExtendedParameterType {
        match self.parameter_type & 0xFF {
            1 => MemExtendedParameterType::AddressRequirements,
            2 => MemExtendedParameterType::NumaNode,
            3 => MemExtendedParameterType::PartitionHandle,
            4 => MemExtendedParameterType::UserPhysicalHandle,
            5 => MemExtendedParameterType::AttributeFlags,
            6 => MemExtendedParameterType::ImageMachine,
            _ => MemExtendedParameterType::InvalidType,
        }
    }

    pub fn numa_node(node_id: u32) -> Self {
        Self {
            parameter_type: 2,
            arg: node_id as u64,
        }
    }
}

/// Physical memory page granularity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageGranularity {
    Standard4KB,
    Huge2MB,
    Gigantic1GB,
}

impl PageGranularity {
    pub fn size_in_bytes(&self) -> usize {
        match self {
            PageGranularity::Standard4KB => PAGE_SIZE_4KB,
            PageGranularity::Huge2MB => PAGE_SIZE_2MB,
            PageGranularity::Gigantic1GB => PAGE_SIZE_1GB,
        }
    }

    pub fn alignment_mask(&self) -> usize {
        self.size_in_bytes() - 1
    }

    pub fn is_aligned(&self, addr: usize) -> bool {
        (addr & self.alignment_mask()) == 0
    }
}


pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITABLE: u64 = 1 << 1;
pub const PTE_USER: u64 = 1 << 2;
pub const PTE_WRITE_THROUGH: u64 = 1 << 3;
pub const PTE_CACHE_DISABLE: u64 = 1 << 4;
pub const PTE_ACCESSED: u64 = 1 << 5;
pub const PTE_DIRTY: u64 = 1 << 6;
pub const PTE_HUGE_PAGE: u64 = 1 << 7;
pub const PTE_GLOBAL: u64 = 1 << 8;
pub const PTE_NO_EXECUTE: u64 = 1 << 63;

/// Page table entry representation for 2MB (PML2) or 1GB (PML3) mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LargePageTableEntry {
    pub virt_addr: usize,
    pub phys_addr: usize,
    pub raw_entry: u64,
    pub level: u8, // 2 = PML2 (2MB), 3 = PML3 (1GB), 1 = PML1 (4KB)
}

impl LargePageTableEntry {
    pub fn is_huge(&self) -> bool {
        (self.raw_entry & PTE_HUGE_PAGE) != 0
    }

    pub fn is_writable(&self) -> bool {
        (self.raw_entry & PTE_WRITABLE) != 0
    }

    pub fn is_executable(&self) -> bool {
        (self.raw_entry & PTE_NO_EXECUTE) == 0
    }
}


pub const LARGE_PAGE_FALLBACK_DEFAULT: u32 = 0x0001;
pub const LARGE_PAGE_STRICT_ALLOCATION: u32 = 0x0002;

/// Standard 4KB fallback frame allocator pool.
#[derive(Debug, Clone)]
pub struct Fallback4KbPool {
    pub base_phys: usize,
    pub next_free_idx: usize,
    pub total_frames: usize,
}

impl Fallback4KbPool {
    pub fn new(base_phys: usize, total_frames: usize) -> Self {
        Self {
            base_phys,
            next_free_idx: 0,
            total_frames,
        }
    }

    pub fn allocate_frame(&mut self) -> Option<usize> {
        if self.next_free_idx < self.total_frames {
            let addr = self.base_phys + self.next_free_idx * PAGE_SIZE_4KB;
            self.next_free_idx += 1;
            Some(addr)
        } else {
            None
        }
    }
}

/// Physical memory pool per NUMA node tracking available contiguous frames.
#[derive(Debug, Clone)]
pub struct NumaNodePool {
    pub node_id: u32,
    pub base_phys: usize,
    pub free_2mb_frames: Vec<usize>,
    pub free_1gb_frames: Vec<usize>,
}

impl NumaNodePool {
    pub fn new(node_id: u32, base_phys: usize, num_2mb: usize, num_1gb: usize) -> Self {
        let mut free_2mb = Vec::with_capacity(num_2mb);
        for i in 0..num_2mb {
            free_2mb.push(base_phys + i * PAGE_SIZE_2MB);
        }

        let mut free_1gb = Vec::with_capacity(num_1gb);
        let base_1gb = base_phys + num_2mb * PAGE_SIZE_2MB;
        for i in 0..num_1gb {
            free_1gb.push(base_1gb + i * PAGE_SIZE_1GB);
        }

        Self {
            node_id,
            base_phys,
            free_2mb_frames: free_2mb,
            free_1gb_frames: free_1gb,
        }
    }

    pub fn allocate_2mb_frame(&mut self) -> Option<usize> {
        self.free_2mb_frames.pop()
    }

    pub fn allocate_1gb_frame(&mut self) -> Option<usize> {
        self.free_1gb_frames.pop()
    }

    pub fn free_2mb_frame(&mut self, frame: usize) {
        self.free_2mb_frames.push(frame);
    }

    pub fn free_1gb_frame(&mut self, frame: usize) {
        self.free_1gb_frames.push(frame);
    }
}


/// Types of anti-cheat memory protection violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntiCheatMemoryViolationKind {
    None,
    RwxAllocationAttempt,
    UnauthorizedPageTransition,
    PageTableIntegrityMismatch,
    StealthWriteProtectionStrip,
}

/// Audit event recording an unauthorized memory allocation or page table tampering attempt.
#[derive(Debug, Clone)]
pub struct MemoryTamperRecord {
    pub timestamp: u64,
    pub process_id: u64,
    pub base_address: usize,
    pub size: usize,
    pub requested_protect: u32,
    pub caller_trust_level: ProcessTrustLevel,
    pub violation_kind: AntiCheatMemoryViolationKind,
}

/// Anti-cheat memory governor enforcing W^X policies and page table walk verification.
pub struct AntiCheatMemoryGovernor {
    pub tamper_records: Vec<MemoryTamperRecord>,
    pub tamper_count: AtomicU32,
    pub wx_enforcement_strict: AtomicBool,
}

impl AntiCheatMemoryGovernor {
    pub fn new() -> Self {
        Self {
            tamper_records: Vec::new(),
            tamper_count: AtomicU32::new(0),
            wx_enforcement_strict: AtomicBool::new(true),
        }
    }

    /// Validates page protection request against W^X security rules.
    pub fn verify_page_protection(
        &mut self,
        caller_pid: u64,
        caller_trust: ProcessTrustLevel,
        base_address: usize,
        size: usize,
        protect: u32,
    ) -> Result<(), NtStatus> {
        // Enforce W^X: PAGE_EXECUTE_READWRITE is forbidden for non-System callers
        let is_rwx = (protect & 0xFF) == PAGE_EXECUTE_READWRITE;
        if is_rwx && self.wx_enforcement_strict.load(Ordering::SeqCst) {
            if caller_trust < ProcessTrustLevel::System {
                self.record_violation(
                    caller_pid,
                    base_address,
                    size,
                    protect,
                    caller_trust,
                    AntiCheatMemoryViolationKind::RwxAllocationAttempt,
                );
                return Err(STATUS_ACCESS_DENIED);
            }
        }

        Ok(())
    }

    /// Records an anti-cheat memory violation event.
    pub fn record_violation(
        &mut self,
        caller_pid: u64,
        base_address: usize,
        size: usize,
        requested_protect: u32,
        caller_trust: ProcessTrustLevel,
        kind: AntiCheatMemoryViolationKind,
    ) {
        self.tamper_count.fetch_add(1, Ordering::SeqCst);
        self.tamper_records.push(MemoryTamperRecord {
            timestamp: 0,
            process_id: caller_pid,
            base_address,
            size,
            requested_protect,
            caller_trust_level: caller_trust,
            violation_kind: kind,
        });
    }
}

/// Descriptor tracking a large page virtual memory allocation.
#[derive(Debug, Clone)]
pub struct LargePageAllocation {
    pub base_address: usize,
    pub size: usize,
    pub granularity: PageGranularity,
    pub state: u32,
    pub protect: u32,
    pub numa_node: u32,
    pub physical_frames: Vec<usize>,
    pub is_downgraded_to_4kb: bool,
}

/// Central Large Page & VirtualAlloc2 Memory Manager Governor.
pub struct LargePageMemoryManager {
    pub allocations: Vec<LargePageAllocation>,
    pub numa_pools: BTreeMap<u32, NumaNodePool>,
    pub fallback_pool: Fallback4KbPool,
    pub page_table_entries: BTreeMap<usize, LargePageTableEntry>,
    pub anti_cheat_gov: AntiCheatMemoryGovernor,
    pub fallback_policy: u32,
    pub downgrade_count: AtomicU32,
    pub next_virt_address: usize,
    pub default_numa_node: u32,
}

impl LargePageMemoryManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            allocations: Vec::new(),
            numa_pools: BTreeMap::new(),
            fallback_pool: Fallback4KbPool::new(0x3000_0000, 16384), // 64 MiB of 4KB frames
            page_table_entries: BTreeMap::new(),
            anti_cheat_gov: AntiCheatMemoryGovernor::new(),
            fallback_policy: LARGE_PAGE_FALLBACK_DEFAULT,
            downgrade_count: AtomicU32::new(0),
            next_virt_address: 0x0000_0002_0000_0000, // 8 GiB base, 2MB-aligned
            default_numa_node: 0,
        };

        // Initialize NUMA nodes: Node 0 (e.g. 64x 2MB frames, 4x 1GB frames), Node 1 (same)
        mgr.numa_pools.insert(0, NumaNodePool::new(0, 0x1000_0000, 64, 4));
        mgr.numa_pools.insert(1, NumaNodePool::new(1, 0x2000_0000, 64, 4));
        mgr
    }

    /// Aligns an address upward to a given alignment (must be a power of two).
    #[inline(always)]
    pub fn align_up(addr: usize, align: usize) -> usize {
        (addr + align - 1) & !(align - 1)
    }

    /// Allocates virtual memory according to Win32 VirtualAlloc2 / NtAllocateVirtualMemoryEx specifications.
    pub fn allocate_virtual_memory_ex(
        &mut self,
        _process_handle: HANDLE,
        base_address: usize,
        size: usize,
        allocation_type: u32,
        page_protection: u32,
        extended_params: &[MEM_EXTENDED_PARAMETER],
    ) -> Result<usize, NtStatus> {
        if size == 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }

        // Determine requested page granularity and parse NUMA node from extended parameters
        let is_large_page = (allocation_type & MEM_LARGE_PAGES) != 0;
        let mut numa_node = self.default_numa_node;
        let mut custom_alignment = 0usize;

        for param in extended_params {
            match param.get_type() {
                MemExtendedParameterType::NumaNode => {
                    numa_node = param.arg as u32;
                }
                MemExtendedParameterType::AddressRequirements => {
                    let req_ptr = param.arg as *const MemAddressRequirements;
                    if !req_ptr.is_null() {
                        unsafe {
                            custom_alignment = (*req_ptr).alignment;
                        }
                    }
                }
                _ => {}
            }
        }

        let granularity = if is_large_page {
            if (allocation_type & MEM_4MB_PAGES) != 0 || size >= PAGE_SIZE_1GB && (size % PAGE_SIZE_1GB == 0) {
                PageGranularity::Gigantic1GB
            } else {
                PageGranularity::Huge2MB
            }
        } else {
            PageGranularity::Standard4KB
        };

        let req_align = if custom_alignment > 0 {
            custom_alignment
        } else {
            granularity.size_in_bytes()
        };

        // Size must be aligned to the granularity
        let aligned_size = Self::align_up(size, granularity.size_in_bytes());

        // Validate base address alignment if specified by caller
        let target_base = if base_address != 0 {
            if (base_address & (req_align - 1)) != 0 {
                return Err(STATUS_INVALID_PARAMETER);
            }
            base_address
        } else {
            let aligned = Self::align_up(self.next_virt_address, req_align);
            self.next_virt_address = aligned + aligned_size;
            aligned
        };

        // Create initial allocation tracking record
        let record = LargePageAllocation {
            base_address: target_base,
            size: aligned_size,
            granularity,
            state: allocation_type & (MEM_COMMIT | MEM_RESERVE),
            protect: page_protection,
            numa_node,
            physical_frames: Vec::new(),
            is_downgraded_to_4kb: false,
        };

        self.allocations.push(record);
        Ok(target_base)
    }


    /// Converts Win32 page protection constant to x86-64 page table entry attributes.
    pub fn protection_to_pte_flags(protect: u32) -> u64 {
        let flags = PTE_PRESENT | PTE_USER;

        match protect & 0xFF {
            PAGE_NOACCESS => 0,
            PAGE_READONLY => flags | PTE_NO_EXECUTE,
            PAGE_READWRITE => flags | PTE_WRITABLE | PTE_NO_EXECUTE,
            PAGE_EXECUTE => flags,
            PAGE_EXECUTE_READ => flags,
            PAGE_EXECUTE_READWRITE => flags | PTE_WRITABLE,
            _ => flags | PTE_WRITABLE | PTE_NO_EXECUTE,
        }
    }

    /// Backs an allocation with physical memory frames from the designated NUMA node and maps PML2/PML3 entries.
    pub fn commit_large_page_physical_backing(
        &mut self,
        base_address: usize,
    ) -> Result<(), NtStatus> {
        let alloc_idx = self
            .allocations
            .iter()
            .position(|a| a.base_address == base_address)
            .ok_or(STATUS_INVALID_PARAMETER)?;

        let (granularity, size, numa_node, protect) = {
            let a = &self.allocations[alloc_idx];
            (a.granularity, a.size, a.numa_node, a.protect)
        };

        let chunk_size = granularity.size_in_bytes();
        let num_chunks = size / chunk_size;
        let mut allocated_phys = Vec::with_capacity(num_chunks);

        let pte_flags = Self::protection_to_pte_flags(protect);

        // Check if contiguous large frames are available in the requested NUMA pool
        let has_sufficient_large_frames = {
            let pool = self
                .numa_pools
                .get(&numa_node)
                .ok_or(STATUS_INVALID_PARAMETER)?;

            match granularity {
                PageGranularity::Huge2MB => pool.free_2mb_frames.len() >= num_chunks,
                PageGranularity::Gigantic1GB => pool.free_1gb_frames.len() >= num_chunks,
                PageGranularity::Standard4KB => true,
            }
        };

        if !has_sufficient_large_frames {
            // If fallback policy allows, gracefully downgrade to 4KB demand paging frames
            if (self.fallback_policy & LARGE_PAGE_FALLBACK_DEFAULT) != 0 {
                return self.downgrade_allocation_to_4kb(alloc_idx);
            } else {
                return Err(STATUS_NO_MEMORY);
            }
        }

        {
            let pool = self
                .numa_pools
                .get_mut(&numa_node)
                .ok_or(STATUS_INVALID_PARAMETER)?;

            for _ in 0..num_chunks {
                let phys_frame = match granularity {
                    PageGranularity::Huge2MB => pool.allocate_2mb_frame(),
                    PageGranularity::Gigantic1GB => pool.allocate_1gb_frame(),
                    PageGranularity::Standard4KB => None,
                };

                let frame = match phys_frame {
                    Some(f) => f,
                    None => {
                        for f in allocated_phys {
                            if granularity == PageGranularity::Gigantic1GB {
                                pool.free_1gb_frame(f);
                            } else {
                                pool.free_2mb_frame(f);
                            }
                        }
                        return Err(STATUS_NO_MEMORY);
                    }
                };

                allocated_phys.push(frame);
            }
        }

        // Map into page table entries at PML2 or PML3 level with PTE_HUGE_PAGE flag
        let level = match granularity {
            PageGranularity::Huge2MB => 2,
            PageGranularity::Gigantic1GB => 3,
            PageGranularity::Standard4KB => 1,
        };

        for (i, &phys) in allocated_phys.iter().enumerate() {
            let virt = base_address + i * chunk_size;
            let raw_entry = (phys as u64 & 0x000F_FFFF_FFFF_F000)
                | pte_flags
                | if granularity != PageGranularity::Standard4KB {
                    PTE_HUGE_PAGE
                } else {
                    0
                };

            self.page_table_entries.insert(
                virt,
                LargePageTableEntry {
                    virt_addr: virt,
                    phys_addr: phys,
                    raw_entry,
                    level,
                },
            );
        }

        self.allocations[alloc_idx].physical_frames = allocated_phys;
        self.allocations[alloc_idx].state |= MEM_COMMIT;
        Ok(())
    }


    /// Gracefully downgrades an allocation from 2MB/1GB huge pages to standard 4KB demand paging.
    pub fn downgrade_allocation_to_4kb(&mut self, alloc_idx: usize) -> Result<(), NtStatus> {
        let (base_address, size, protect) = {
            let a = &self.allocations[alloc_idx];
            (a.base_address, a.size, a.protect)
        };

        let num_4kb_pages = size / PAGE_SIZE_4KB;
        let mut allocated_4kb = Vec::with_capacity(num_4kb_pages);

        for _ in 0..num_4kb_pages {
            let frame = self.fallback_pool.allocate_frame().ok_or(STATUS_NO_MEMORY)?;
            allocated_4kb.push(frame);
        }

        let pte_flags = Self::protection_to_pte_flags(protect);

        // Map each 4KB frame at PML1 (level = 1, without HUGE_PAGE flag)
        for (i, &phys) in allocated_4kb.iter().enumerate() {
            let virt = base_address + i * PAGE_SIZE_4KB;
            let raw_entry = (phys as u64 & 0x000F_FFFF_FFFF_F000) | pte_flags;

            self.page_table_entries.insert(
                virt,
                LargePageTableEntry {
                    virt_addr: virt,
                    phys_addr: phys,
                    raw_entry,
                    level: 1,
                },
            );
        }

        self.downgrade_count.fetch_add(1, Ordering::SeqCst);
        let a = &mut self.allocations[alloc_idx];
        a.granularity = PageGranularity::Standard4KB;
        a.is_downgraded_to_4kb = true;
        a.physical_frames = allocated_4kb;
        a.state |= MEM_COMMIT;

        Ok(())
    }

    /// Sets the fallback policy (e.g. LARGE_PAGE_FALLBACK_DEFAULT or LARGE_PAGE_STRICT_ALLOCATION).
    pub fn set_fallback_policy(&mut self, policy: u32) {
        self.fallback_policy = policy;
    }

    /// Returns the total count of large page allocations downgraded to 4KB.
    pub fn get_downgrade_count(&self) -> u32 {
        self.downgrade_count.load(Ordering::SeqCst)
    }


    /// Allocates virtual memory enforcing anti-cheat W^X policies and origin checks.
    pub fn allocate_virtual_memory_secure(
        &mut self,
        caller_pid: u64,
        caller_trust: ProcessTrustLevel,
        process_handle: HANDLE,
        base_address: usize,
        size: usize,
        allocation_type: u32,
        page_protection: u32,
        extended_params: &[MEM_EXTENDED_PARAMETER],
    ) -> Result<usize, NtStatus> {
        // Enforce anti-cheat W^X page protection
        self.anti_cheat_gov.verify_page_protection(
            caller_pid,
            caller_trust,
            base_address,
            size,
            page_protection,
        )?;

        self.allocate_virtual_memory_ex(
            process_handle,
            base_address,
            size,
            allocation_type,
            page_protection,
            extended_params,
        )
    }

    /// Performs an integrity walk of page table entries for an allocated range,
    /// verifying physical backing frames and hardware protection flags against kernel descriptors.
    pub fn verify_page_table_walk_integrity(&mut self, base_address: usize) -> Result<bool, NtStatus> {
        let alloc = self
            .find_allocation(base_address)
            .cloned()
            .ok_or(STATUS_INVALID_PARAMETER)?;

        let chunk_size = alloc.granularity.size_in_bytes();
        let num_chunks = alloc.size / chunk_size;

        for i in 0..num_chunks {
            let virt = base_address + i * chunk_size;
            let pte = self.page_table_entries.get(&virt).ok_or(STATUS_UNSUCCESSFUL)?;

            // 1. PTE must be present
            if (pte.raw_entry & PTE_PRESENT) == 0 {
                self.anti_cheat_gov.record_violation(
                    0,
                    virt,
                    chunk_size,
                    alloc.protect,
                    ProcessTrustLevel::System,
                    AntiCheatMemoryViolationKind::PageTableIntegrityMismatch,
                );
                return Err(STATUS_ACCESS_DENIED);
            }

            // 2. Huge page flag verification
            if alloc.granularity != PageGranularity::Standard4KB && !pte.is_huge() {
                self.anti_cheat_gov.record_violation(
                    0,
                    virt,
                    chunk_size,
                    alloc.protect,
                    ProcessTrustLevel::System,
                    AntiCheatMemoryViolationKind::PageTableIntegrityMismatch,
                );
                return Err(STATUS_ACCESS_DENIED);
            }

            // 3. W^X check on hardware PTE: cannot be simultaneously writable and executable
            if pte.is_writable() && pte.is_executable() {
                self.anti_cheat_gov.record_violation(
                    0,
                    virt,
                    chunk_size,
                    alloc.protect,
                    ProcessTrustLevel::System,
                    AntiCheatMemoryViolationKind::StealthWriteProtectionStrip,
                );
                return Err(STATUS_ACCESS_DENIED);
            }

            // 4. Physical frame address consistency check
            if i < alloc.physical_frames.len() && alloc.physical_frames[i] != pte.phys_addr {
                self.anti_cheat_gov.record_violation(
                    0,
                    virt,
                    chunk_size,
                    alloc.protect,
                    ProcessTrustLevel::System,
                    AntiCheatMemoryViolationKind::PageTableIntegrityMismatch,
                );
                return Err(STATUS_ACCESS_DENIED);
            }
        }

        Ok(true)
    }

    /// Queries total anti-cheat memory violation statistics.
    pub fn get_anti_cheat_memory_stats(&self) -> (u32, Option<MemoryTamperRecord>) {
        let count = self.anti_cheat_gov.tamper_count.load(Ordering::SeqCst);
        let last = self.anti_cheat_gov.tamper_records.last().cloned();
        (count, last)
    }

    /// Finds an existing allocation record by base address.
    pub fn find_allocation(&self, base_address: usize) -> Option<&LargePageAllocation> {
        self.allocations.iter().find(|a| a.base_address == base_address)
    }

    /// Finds a mutable allocation record by base address.
    pub fn find_allocation_mut(&mut self, base_address: usize) -> Option<&mut LargePageAllocation> {
        self.allocations.iter_mut().find(|a| a.base_address == base_address)
    }
}

/// Global system-wide instance of LargePageMemoryManager.
pub static GLOBAL_LARGE_PAGE_MANAGER: Spinlock<Option<LargePageMemoryManager>> = Spinlock::new(None);

/// Initializes the Large Page Memory Manager.
pub fn init_large_page_manager() {
    let mut lock = GLOBAL_LARGE_PAGE_MANAGER.lock();
    if lock.is_none() {
        *lock = Some(LargePageMemoryManager::new());
    }
}

/// Accessor for the global Large Page Memory Manager.
pub fn with_large_page_manager<F, R>(f: F) -> R
where
    F: FnOnce(&mut LargePageMemoryManager) -> R,
{
    init_large_page_manager();
    let mut lock = GLOBAL_LARGE_PAGE_MANAGER.lock();
    f(lock.as_mut().unwrap())
}
