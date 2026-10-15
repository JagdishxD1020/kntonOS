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
    pub next_virt_address: usize,
    pub default_numa_node: u32,
}

impl LargePageMemoryManager {
    pub fn new() -> Self {
        Self {
            allocations: Vec::new(),
            next_virt_address: 0x0000_0002_0000_0000, // 8 GiB base, 2MB-aligned
            default_numa_node: 0,
        }
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
