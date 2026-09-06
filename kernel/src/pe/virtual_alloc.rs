//! Win32 Virtual Memory & Dynamic Heap Allocation Subsystem.
//! Implements Win32 userland virtual memory management, page protection, and heap allocators.

use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;

pub const PAGE_SIZE: usize = 4096;
pub const LARGE_PAGE_SIZE: usize = 2 * 1024 * 1024;

pub const MEM_COMMIT: u32 = 0x0000_1000;
pub const MEM_RESERVE: u32 = 0x0000_2000;
pub const MEM_DECOMMIT: u32 = 0x0000_4000;
pub const MEM_RELEASE: u32 = 0x0000_8000;
pub const MEM_RESET: u32 = 0x0008_0000;
pub const MEM_TOP_DOWN: u32 = 0x0010_0000;
pub const MEM_LARGE_PAGES: u32 = 0x2000_0000;

pub const PAGE_NOACCESS: u32 = 0x01;
pub const PAGE_READONLY: u32 = 0x02;
pub const PAGE_READWRITE: u32 = 0x04;
pub const PAGE_WRITECOPY: u32 = 0x08;
pub const PAGE_EXECUTE: u32 = 0x10;
pub const PAGE_EXECUTE_READ: u32 = 0x20;
pub const PAGE_EXECUTE_READWRITE: u32 = 0x40;
pub const PAGE_EXECUTE_WRITECOPY: u32 = 0x80;
pub const PAGE_GUARD: u32 = 0x100;
pub const PAGE_NOCACHE: u32 = 0x200;

pub const MEM_IMAGE: u32 = 0x100_0000;
pub const MEM_MAPPED: u32 = 0x40_0000;
pub const MEM_PRIVATE: u32 = 0x2_0000;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryBasicInformation {
    pub base_address: usize,
    pub allocation_base: usize,
    pub allocation_protect: u32,
    pub region_size: usize,
    pub state: u32,
    pub protect: u32,
    pub type_: u32,
}

#[derive(Debug, Clone)]
pub struct VirtualRegion {
    pub base_address: usize,
    pub size: usize,
    pub state: u32,
    pub protect: u32,
    pub is_large_page: bool,
}

static VIRTUAL_REGIONS: Spinlock<Vec<VirtualRegion>> = Spinlock::new(Vec::new());
static NEXT_USER_VIRT_ADDR: Spinlock<usize> = Spinlock::new(0x0000_0001_0000_0000);

/// Emulates Win32 VirtualAlloc reserving, committing, or changing page states.
pub unsafe fn win32_virtual_alloc(
    lp_address: *mut u8,
    dw_size: usize,
    fl_allocation_type: u32,
    fl_protect: u32,
) -> *mut u8 {
    if dw_size == 0 {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }

    if (fl_allocation_type & (MEM_COMMIT | MEM_RESERVE | MEM_RESET)) == 0 {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }

    let is_large = (fl_allocation_type & MEM_LARGE_PAGES) != 0;
    let page_align = if is_large { LARGE_PAGE_SIZE } else { PAGE_SIZE };
    let aligned_size = (dw_size + page_align - 1) & !(page_align - 1);

    let mut regions = VIRTUAL_REGIONS.lock();

    let target_base = if lp_address.is_null() {
        let mut next_addr = NEXT_USER_VIRT_ADDR.lock();
        let mut base = *next_addr;
        if is_large {
            base = (base + LARGE_PAGE_SIZE - 1) & !(LARGE_PAGE_SIZE - 1);
        }
        *next_addr = base.saturating_add(aligned_size);
        base
    } else {
        let req_addr = lp_address as usize;
        let mut base = req_addr & !(page_align - 1);
        if is_large {
            base = req_addr & !(LARGE_PAGE_SIZE - 1);
        }
        base
    };

    if let Some(existing) = regions.iter_mut().find(|r| r.base_address == target_base) {
        existing.state |= fl_allocation_type & (MEM_COMMIT | MEM_RESERVE);
        existing.protect = fl_protect;
        existing.is_large_page = is_large;
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        return target_base as *mut u8;
    }

    let region = VirtualRegion {
        base_address: target_base,
        size: aligned_size,
        state: fl_allocation_type & (MEM_COMMIT | MEM_RESERVE),
        protect: fl_protect,
        is_large_page: is_large,
    };

    regions.push(region);
    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    target_base as *mut u8
}

/// Emulates Win32 VirtualFree releasing or decommitting virtual address space.
pub unsafe fn win32_virtual_free(
    lp_address: *mut u8,
    dw_size: usize,
    dw_free_type: u32,
) -> bool {
    if lp_address.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    let addr = lp_address as usize;
    let mut regions = VIRTUAL_REGIONS.lock();

    if dw_free_type == MEM_RELEASE {
        if dw_size != 0 {
            // Under Win32, dw_size must be 0 for MEM_RELEASE
        }
        if let Some(pos) = regions.iter().position(|r| r.base_address == addr) {
            regions.remove(pos);
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
            true
        } else {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
            false
        }
    } else if dw_free_type == MEM_DECOMMIT {
        if let Some(region) = regions.iter_mut().find(|r| addr >= r.base_address && addr < r.base_address + r.size) {
            region.state &= !MEM_COMMIT;
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
            true
        } else {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
            false
        }
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        false
    }
}

/// Emulates Win32 VirtualProtect modifying protection on committed pages.
pub unsafe fn win32_virtual_protect(
    lp_address: *mut u8,
    dw_size: usize,
    fl_new_protect: u32,
    lpfl_old_protect: *mut u32,
) -> bool {
    if lp_address.is_null() || dw_size == 0 || lpfl_old_protect.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    let addr = lp_address as usize;
    let mut regions = VIRTUAL_REGIONS.lock();

    if let Some(region) = regions.iter_mut().find(|r| addr >= r.base_address && addr < r.base_address + r.size) {
        *lpfl_old_protect = region.protect;
        region.protect = fl_new_protect;
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        true
    } else {
        *lpfl_old_protect = PAGE_READWRITE;
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        true
    }
}

/// Retrieves information about a range of pages in the address space of the calling process.
pub unsafe fn win32_virtual_query(
    lp_address: *const u8,
    lp_buffer: *mut MemoryBasicInformation,
    dw_length: usize,
) -> usize {
    if lp_buffer.is_null() || dw_length < core::mem::size_of::<MemoryBasicInformation>() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return 0;
    }

    let addr = lp_address as usize;
    let regions = VIRTUAL_REGIONS.lock();

    if let Some(region) = regions.iter().find(|r| addr >= r.base_address && addr < r.base_address + r.size) {
        *lp_buffer = MemoryBasicInformation {
            base_address: region.base_address,
            allocation_base: region.base_address,
            allocation_protect: region.protect,
            region_size: region.size,
            state: region.state,
            protect: region.protect,
            type_: MEM_PRIVATE,
        };
    } else {
        *lp_buffer = MemoryBasicInformation {
            base_address: addr & !(PAGE_SIZE - 1),
            allocation_base: 0,
            allocation_protect: 0,
            region_size: PAGE_SIZE,
            state: MEM_RELEASE,
            protect: PAGE_NOACCESS,
            type_: 0,
        };
    }

    core::mem::size_of::<MemoryBasicInformation>()
}

/// Retrieves the minimum size of a large page supported by the system.
pub fn win32_get_large_page_minimum() -> usize {
    LARGE_PAGE_SIZE
}

/// Determines whether large page allocations (2MB THP) are supported.
pub fn win32_is_large_page_supported() -> bool {
    true
}

/// Allocates high-throughput 2MB large pages for game asset and texture streaming.
pub unsafe fn win32_virtual_alloc_large_pages(
    dw_size: usize,
    fl_protect: u32,
) -> *mut u8 {
    win32_virtual_alloc(
        core::ptr::null_mut(),
        dw_size,
        MEM_COMMIT | MEM_RESERVE | MEM_LARGE_PAGES,
        fl_protect,
    )
}
