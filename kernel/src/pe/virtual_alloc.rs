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

use alloc::collections::BTreeMap;

pub use crate::pe::synch::HANDLE;

pub const HEAP_NO_SERIALIZE: u32 = 0x0000_0001;
pub const HEAP_GENERATE_EXCEPTIONS: u32 = 0x0000_0004;
pub const HEAP_ZERO_MEMORY: u32 = 0x0000_0008;
pub const HEAP_REALLOC_IN_PLACE_ONLY: u32 = 0x0000_0010;

#[derive(Debug, Clone)]
pub struct HeapBlock {
    pub ptr: usize,
    pub size: usize,
    pub user_size: usize,
    pub in_use: bool,
}

#[derive(Debug)]
pub struct Win32Heap {
    pub handle: HANDLE,
    pub flags: u32,
    pub initial_size: usize,
    pub maximum_size: usize,
    pub base_address: usize,
    pub current_offset: usize,
    pub blocks: Vec<HeapBlock>,
    pub buffer: Vec<u8>,
}

static NEXT_HEAP_HANDLE: Spinlock<HANDLE> = Spinlock::new(0x0007_0001);
static HEAP_TABLE: Spinlock<BTreeMap<HANDLE, Win32Heap>> = Spinlock::new(BTreeMap::new());

/// Creates a private heap object that can be used by the calling process.
pub unsafe fn win32_heap_create(
    fl_options: u32,
    dw_initial_size: usize,
    dw_maximum_size: usize,
) -> HANDLE {
    let mut gen = NEXT_HEAP_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let init_size = if dw_initial_size > 0 { dw_initial_size.min(16 * 1024) } else { 16 * 1024 };
    let mut buffer = alloc::vec![0u8; init_size];
    let base = buffer.as_mut_ptr() as usize;

    if base == 0 {
        return 0;
    }

    let heap = Win32Heap {
        handle,
        flags: fl_options,
        initial_size: init_size,
        maximum_size: dw_maximum_size,
        base_address: base,
        current_offset: 0,
        blocks: Vec::new(),
        buffer,
    };

    let mut table = HEAP_TABLE.lock();
    table.insert(handle, heap);
    handle
}

/// Allocates a block of memory from a heap.
pub unsafe fn win32_heap_alloc(
    h_heap: HANDLE,
    dw_flags: u32,
    dw_bytes: usize,
) -> *mut u8 {
    if dw_bytes == 0 {
        return core::ptr::null_mut();
    }

    let mut table = HEAP_TABLE.lock();
    let heap = match table.get_mut(&h_heap) {
        Some(h) => h,
        None => return core::ptr::null_mut(),
    };

    // 16-byte standard aligned allocation
    let align = 16;
    let aligned_size = (dw_bytes + align - 1) & !(align - 1);

    // Look for reusable freed block
    if let Some(block) = heap.blocks.iter_mut().find(|b| !b.in_use && b.size >= aligned_size) {
        block.in_use = true;
        block.user_size = dw_bytes;
        let ptr = block.ptr as *mut u8;
        if (dw_flags & HEAP_ZERO_MEMORY) != 0 || (heap.flags & HEAP_ZERO_MEMORY) != 0 {
            core::ptr::write_bytes(ptr, 0, dw_bytes);
        }
        return ptr;
    }

    if heap.current_offset + aligned_size > heap.initial_size {
        return core::ptr::null_mut();
    }

    let block_addr = heap.base_address + heap.current_offset;
    heap.current_offset += aligned_size;

    let ptr = block_addr as *mut u8;
    if (dw_flags & HEAP_ZERO_MEMORY) != 0 || (heap.flags & HEAP_ZERO_MEMORY) != 0 {
        core::ptr::write_bytes(ptr, 0, dw_bytes);
    }

    heap.blocks.push(HeapBlock {
        ptr: block_addr,
        size: aligned_size,
        user_size: dw_bytes,
        in_use: true,
    });

    ptr
}

/// Reallocates a block of memory from a heap.
pub unsafe fn win32_heap_realloc(
    h_heap: HANDLE,
    dw_flags: u32,
    lp_mem: *mut u8,
    dw_bytes: usize,
) -> *mut u8 {
    if lp_mem.is_null() {
        return win32_heap_alloc(h_heap, dw_flags, dw_bytes);
    }
    if dw_bytes == 0 {
        win32_heap_free(h_heap, dw_flags, lp_mem);
        return core::ptr::null_mut();
    }

    let mem_addr = lp_mem as usize;
    let old_user_size = {
        let table = HEAP_TABLE.lock();
        let heap = match table.get(&h_heap) {
            Some(h) => h,
            None => return core::ptr::null_mut(),
        };
        match heap.blocks.iter().find(|b| b.ptr == mem_addr && b.in_use) {
            Some(b) => b.user_size,
            None => return core::ptr::null_mut(),
        }
    };

    let new_ptr = win32_heap_alloc(h_heap, dw_flags, dw_bytes);
    if new_ptr.is_null() {
        return core::ptr::null_mut();
    }

    let copy_bytes = old_user_size.min(dw_bytes);
    core::ptr::copy_nonoverlapping(lp_mem, new_ptr, copy_bytes);

    if dw_bytes > old_user_size && ((dw_flags & HEAP_ZERO_MEMORY) != 0) {
        core::ptr::write_bytes(new_ptr.add(old_user_size), 0, dw_bytes - old_user_size);
    }

    win32_heap_free(h_heap, dw_flags, lp_mem);
    new_ptr
}

/// Frees a memory block allocated from a heap.
pub unsafe fn win32_heap_free(
    h_heap: HANDLE,
    _dw_flags: u32,
    lp_mem: *mut u8,
) -> bool {
    if lp_mem.is_null() {
        return false;
    }

    let mem_addr = lp_mem as usize;
    let mut table = HEAP_TABLE.lock();
    let heap = match table.get_mut(&h_heap) {
        Some(h) => h,
        None => return false,
    };

    if let Some(block) = heap.blocks.iter_mut().find(|b| b.ptr == mem_addr && b.in_use) {
        block.in_use = false;
        true
    } else {
        false
    }
}

/// Destroys the specified heap object.
pub unsafe fn win32_heap_destroy(h_heap: HANDLE) -> bool {
    let mut table = HEAP_TABLE.lock();
    if let Some(heap) = table.remove(&h_heap) {
        win32_virtual_free(heap.base_address as *mut u8, 0, MEM_RELEASE);
        true
    } else {
        false
    }
}

pub const BLOCK_MAGIC_TAG: u32 = 0x4B4E_5448;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BlockHeader {
    pub magic: u32,
    pub size: usize,
    pub alignment: usize,
    pub user_offset: u32,
    pub in_use: bool,
}

/// Allocates an aligned block from the heap for SIMD and cache-line gaming buffers.
pub unsafe fn win32_heap_alloc_aligned(
    h_heap: HANDLE,
    dw_flags: u32,
    dw_bytes: usize,
    alignment: usize,
) -> *mut u8 {
    if dw_bytes == 0 {
        return core::ptr::null_mut();
    }

    let align = if alignment >= 16 && alignment.is_power_of_two() {
        alignment
    } else {
        16
    };

    let header_size = core::mem::size_of::<BlockHeader>();
    let total_alloc = dw_bytes + align + header_size;

    let raw_ptr = win32_heap_alloc(h_heap, dw_flags, total_alloc);
    if raw_ptr.is_null() {
        return core::ptr::null_mut();
    }

    let raw_addr = raw_ptr as usize;
    let min_user = raw_addr + header_size;
    let aligned_user = (min_user + align - 1) & !(align - 1);
    let user_offset = (aligned_user - raw_addr) as u32;

    let header = BlockHeader {
        magic: BLOCK_MAGIC_TAG,
        size: dw_bytes,
        alignment: align,
        user_offset,
        in_use: true,
    };

    *(raw_ptr as *mut BlockHeader) = header;

    aligned_user as *mut u8
}

/// Frees an aligned block allocated by win32_heap_alloc_aligned.
pub unsafe fn win32_heap_free_aligned(
    h_heap: HANDLE,
    _dw_flags: u32,
    lp_mem: *mut u8,
) -> bool {
    if lp_mem.is_null() {
        return false;
    }

    let user_addr = lp_mem as usize;
    let mut table = HEAP_TABLE.lock();
    let heap = match table.get_mut(&h_heap) {
        Some(h) => h,
        None => return false,
    };

    for block in heap.blocks.iter_mut() {
        if block.in_use && user_addr >= block.ptr && user_addr < block.ptr + block.size {
            let header = &mut *(block.ptr as *mut BlockHeader);
            if header.magic == BLOCK_MAGIC_TAG {
                header.in_use = false;
                block.in_use = false;
                return true;
            }
        }
    }

    false
}

/// Validates the integrity of a heap or specific heap block.
pub unsafe fn win32_heap_validate(
    h_heap: HANDLE,
    _dw_flags: u32,
    lp_mem: *const u8,
) -> bool {
    let table = HEAP_TABLE.lock();
    let heap = match table.get(&h_heap) {
        Some(h) => h,
        None => return false,
    };

    if lp_mem.is_null() {
        true
    } else {
        let mem_addr = lp_mem as usize;
        heap.blocks.iter().any(|b| b.in_use && (b.ptr == mem_addr || (mem_addr >= b.ptr && mem_addr < b.ptr + b.size)))
    }
}

static PROCESS_HEAP: Spinlock<HANDLE> = Spinlock::new(0);

/// Retrieves a handle to the default process heap.
pub unsafe fn win32_get_process_heap() -> HANDLE {
    let mut ph = PROCESS_HEAP.lock();
    if *ph == 0 {
        let handle = win32_heap_create(0, 0x10000, 0);
        *ph = handle;
    }
    *ph
}

/// Returns the number of active heaps and optionally copies heap handles into the buffer.
pub unsafe fn win32_get_process_heaps(number_of_heaps: u32, process_heaps: *mut HANDLE) -> u32 {
    let table = HEAP_TABLE.lock();
    let total = table.len() as u32;

    if number_of_heaps > 0 && !process_heaps.is_null() {
        let count = (number_of_heaps as usize).min(table.len());
        for (i, &handle) in table.keys().take(count).enumerate() {
            *process_heaps.add(i) = handle;
        }
    }

    total
}

/// Retrieves the size of a memory block allocated from a heap.
pub unsafe fn win32_heap_size(h_heap: HANDLE, _dw_flags: u32, lp_mem: *const u8) -> usize {
    if lp_mem.is_null() {
        return usize::MAX;
    }

    let table = HEAP_TABLE.lock();
    let heap = match table.get(&h_heap) {
        Some(h) => h,
        None => return usize::MAX,
    };

    let mem_addr = lp_mem as usize;
    for block in heap.blocks.iter() {
        if block.in_use {
            if block.ptr == mem_addr {
                return block.size;
            }
            if mem_addr > block.ptr && mem_addr < block.ptr + block.size {
                let header = &*(block.ptr as *const BlockHeader);
                if header.magic == BLOCK_MAGIC_TAG {
                    return header.size;
                }
            }
        }
    }

    usize::MAX
}

/// Self-test suite verifying Win32 virtual memory and dynamic heap subsystem.
pub fn test_virtual_alloc_emulation_subsystem() -> bool {
    crate::serial_write_str("[KNT-VIRT-MEM-TEST] Executing Win32 virtual memory and dynamic heap verification...\r\n");

    // 1. Verify virtual reservation, committing, and protection transitions
    let res = unsafe { win32_virtual_alloc(core::ptr::null_mut(), 0x10000, MEM_RESERVE, PAGE_READWRITE) };
    if res.is_null() {
        return false;
    }

    let comm = unsafe { win32_virtual_alloc(res, 0x1000, MEM_COMMIT, PAGE_READWRITE) };
    if comm != res {
        return false;
    }

    let mut old_protect = 0u32;
    if unsafe { !win32_virtual_protect(res, 0x1000, PAGE_EXECUTE_READWRITE, &mut old_protect) } || old_protect != PAGE_READWRITE {
        return false;
    }

    // 2. Verify virtual query
    let mut mbi = MemoryBasicInformation::default();
    let query_sz = unsafe { win32_virtual_query(res, &mut mbi, core::mem::size_of::<MemoryBasicInformation>()) };
    if query_sz != core::mem::size_of::<MemoryBasicInformation>() || mbi.protect != PAGE_EXECUTE_READWRITE || (mbi.state & MEM_COMMIT) == 0 {
        return false;
    }

    // 3. Verify large page reservation and alignment
    let large = unsafe { win32_virtual_alloc_large_pages(LARGE_PAGE_SIZE, PAGE_READWRITE) };
    if large.is_null() || (large as usize % LARGE_PAGE_SIZE != 0) {
        return false;
    }

    // 4. Verify decommit and release lifecycle
    if unsafe { !win32_virtual_free(res, 0x1000, MEM_DECOMMIT) } {
        return false;
    }
    if unsafe { !win32_virtual_free(res, 0, MEM_RELEASE) } {
        return false;
    }
    if unsafe { !win32_virtual_free(large, 0, MEM_RELEASE) } {
        return false;
    }

    // 5. Verify dynamic heap creation, allocation, zeroing, reallocation, and destroy
    let heap = unsafe { win32_heap_create(0, 4096, 0) };
    if heap == 0 {
        return false;
    }

    let p_block = unsafe { win32_heap_alloc(heap, HEAP_ZERO_MEMORY, 64) };
    if p_block.is_null() {
        return false;
    }
    for i in 0..64 {
        if unsafe { *p_block.add(i) } != 0 {
            return false;
        }
    }
    unsafe { core::ptr::write_bytes(p_block, 0xEE, 64); }

    let p_realloc = unsafe { win32_heap_realloc(heap, 0, p_block, 128) };
    if p_realloc.is_null() {
        return false;
    }
    if unsafe { *p_realloc } != 0xEE {
        return false;
    }

    // 6. Verify cache-line aligned gaming sub-allocator
    let p_aligned64 = unsafe { win32_heap_alloc_aligned(heap, 0, 128, 64) };
    if p_aligned64.is_null() || (p_aligned64 as usize % 64 != 0) {
        return false;
    }
    if unsafe { !win32_heap_validate(heap, 0, p_aligned64) } {
        return false;
    }
    let sz = unsafe { win32_heap_size(heap, 0, p_aligned64) };
    if sz != 128 {
        return false;
    }
    if unsafe { !win32_heap_free_aligned(heap, 0, p_aligned64) } {
        return false;
    }

    if unsafe { !win32_heap_free(heap, 0, p_realloc) } {
        return false;
    }
    if unsafe { !win32_heap_destroy(heap) } {
        return false;
    }

    // 7. Verify default process heap query
    let proc_heap = unsafe { win32_get_process_heap() };
    if proc_heap == 0 {
        return false;
    }
    let mut heaps = [0usize; 4];
    let num_heaps = unsafe { win32_get_process_heaps(4, heaps.as_mut_ptr()) };
    if num_heaps == 0 || heaps[0] != proc_heap {
        return false;
    }

    crate::serial_write_str("[KNT-VIRT-MEM-PASS] Win32 virtual memory and heap subsystem verified\r\n");
    true
}
