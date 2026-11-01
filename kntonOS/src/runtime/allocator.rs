//! User-mode buddy/slab hybrid allocator over virtual memory blocks.

pub const MIN_CHUNK_SIZE: usize = 32;
pub const MAX_SLAB_SIZE: usize = 2048;
pub const PAGE_SIZE: usize = 4096;
pub const NUM_SLAB_CLASSES: usize = 7;
pub const MAX_PAGES: usize = 64; // 256 KiB user arena

pub const SLAB_SIZES: [usize; NUM_SLAB_CLASSES] = [32, 64, 128, 256, 512, 1024, 2048];

/// Single slab page managing fixed-size chunks using bitmap words.
#[derive(Debug, Clone, Copy)]
pub struct SlabPage {
    pub page_addr: usize,
    pub chunk_size: usize,
    pub total_chunks: usize,
    pub free_chunks: usize,
    pub bitmap: [u64; 2],
}

impl SlabPage {
    pub const fn empty() -> Self {
        Self {
            page_addr: 0,
            chunk_size: 0,
            total_chunks: 0,
            free_chunks: 0,
            bitmap: [0; 2],
        }
    }

    pub fn init(&mut self, page_addr: usize, chunk_size: usize) {
        self.page_addr = page_addr;
        self.chunk_size = chunk_size;
        let total = PAGE_SIZE / chunk_size;
        self.total_chunks = if total > 128 { 128 } else { total };
        self.free_chunks = self.total_chunks;
        self.bitmap = [0; 2];
    }

    pub fn allocate_slot(&mut self) -> Option<*mut u8> {
        if self.free_chunks == 0 {
            return None;
        }
        for word_idx in 0..2 {
            let limit = if word_idx == 0 {
                if self.total_chunks > 64 { 64 } else { self.total_chunks }
            } else if self.total_chunks > 64 {
                self.total_chunks - 64
            } else {
                0
            };
            if limit == 0 {
                continue;
            }
            let mask = if limit == 64 { !0u64 } else { (1u64 << limit) - 1 };
            let inverted = !self.bitmap[word_idx] & mask;
            if inverted != 0 {
                let bit = inverted.trailing_zeros() as usize;
                self.bitmap[word_idx] |= 1u64 << bit;
                self.free_chunks -= 1;
                let chunk_idx = word_idx * 64 + bit;
                let addr = self.page_addr + chunk_idx * self.chunk_size;
                return Some(addr as *mut u8);
            }
        }
        None
    }

    pub fn free_slot(&mut self, ptr: *mut u8) -> bool {
        let addr = ptr as usize;
        if addr < self.page_addr || addr >= self.page_addr + PAGE_SIZE {
            return false;
        }
        let offset = addr - self.page_addr;
        let chunk_idx = offset / self.chunk_size;
        if chunk_idx >= self.total_chunks {
            return false;
        }
        let word_idx = chunk_idx / 64;
        let bit = chunk_idx % 64;
        if (self.bitmap[word_idx] & (1u64 << bit)) != 0 {
            self.bitmap[word_idx] &= !(1u64 << bit);
            self.free_chunks += 1;
            return true;
        }
        false
    }
}

/// User-mode Buddy/Slab Allocator managing contiguous virtual memory blocks.
pub struct UserBuddySlabAllocator {
    pub base_address: usize,
    pub total_bytes: usize,
    pub total_pages: usize,
    pub page_bitmap: u64, // 64 pages * 4096 = 256 KiB
    pub slabs: [SlabPage; 16],
    pub active_slabs: usize,
    pub allocated_bytes: usize,
}

impl UserBuddySlabAllocator {
    pub const fn new() -> Self {
        Self {
            base_address: 0,
            total_bytes: 0,
            total_pages: 0,
            page_bitmap: 0,
            slabs: [const { SlabPage::empty() }; 16],
            active_slabs: 0,
            allocated_bytes: 0,
        }
    }

    /// Initializes allocator with virtual memory arena.
    pub unsafe fn init(&mut self, base_addr: *mut u8, total_bytes: usize) {
        self.base_address = base_addr as usize;
        self.total_bytes = total_bytes;
        let pages = total_bytes / PAGE_SIZE;
        self.total_pages = if pages > MAX_PAGES { MAX_PAGES } else { pages };
        self.page_bitmap = 0;
        self.active_slabs = 0;
        self.allocated_bytes = 0;
    }

    /// Finds chunk size matching requested allocation.
    pub fn class_for_size(size: usize) -> Option<usize> {
        for &sz in &SLAB_SIZES {
            if size <= sz {
                return Some(sz);
            }
        }
        None
    }

    /// Allocates contiguous pages from page bitmap.
    pub fn allocate_pages(&mut self, count: usize) -> Option<*mut u8> {
        if count == 0 || count > self.total_pages {
            return None;
        }
        let mut consecutive = 0;
        let mut start_idx = 0;

        for i in 0..self.total_pages {
            if (self.page_bitmap & (1u64 << i)) == 0 {
                if consecutive == 0 {
                    start_idx = i;
                }
                consecutive += 1;
                if consecutive == count {
                    for bit in start_idx..(start_idx + count) {
                        self.page_bitmap |= 1u64 << bit;
                    }
                    self.allocated_bytes += count * PAGE_SIZE;
                    let addr = self.base_address + start_idx * PAGE_SIZE;
                    return Some(addr as *mut u8);
                }
            } else {
                consecutive = 0;
            }
        }
        None
    }

    /// Frees previously allocated contiguous pages.
    pub fn free_pages(&mut self, ptr: *mut u8, count: usize) -> bool {
        let addr = ptr as usize;
        if addr < self.base_address || count == 0 {
            return false;
        }
        let offset = addr - self.base_address;
        if (offset % PAGE_SIZE) != 0 {
            return false;
        }
        let start_idx = offset / PAGE_SIZE;
        if start_idx + count > self.total_pages {
            return false;
        }
        for bit in start_idx..(start_idx + count) {
            self.page_bitmap &= !(1u64 << bit);
        }
        self.allocated_bytes = self.allocated_bytes.saturating_sub(count * PAGE_SIZE);
        true
    }

    /// Userland memory allocation entry point.
    pub unsafe fn allocate(&mut self, size: usize, _align: usize) -> Option<*mut u8> {
        if size == 0 {
            return None;
        }
        // Micro/Slab allocation
        if let Some(chunk_size) = Self::class_for_size(size) {
            for i in 0..self.active_slabs {
                if self.slabs[i].chunk_size == chunk_size && self.slabs[i].free_chunks > 0 {
                    if let Some(ptr) = self.slabs[i].allocate_slot() {
                        self.allocated_bytes += chunk_size;
                        return Some(ptr);
                    }
                }
            }
            // Need a new slab page
            if self.active_slabs < self.slabs.len() {
                if let Some(page_ptr) = self.allocate_pages(1) {
                    let idx = self.active_slabs;
                    self.slabs[idx].init(page_ptr as usize, chunk_size);
                    self.active_slabs += 1;
                    if let Some(ptr) = self.slabs[idx].allocate_slot() {
                        return Some(ptr);
                    }
                }
            }
        }

        // Multi-page allocation
        let pages_needed = (size + PAGE_SIZE - 1) / PAGE_SIZE;
        self.allocate_pages(pages_needed)
    }

    /// Userland memory deallocation entry point.
    pub unsafe fn deallocate(&mut self, ptr: *mut u8, size: usize, _align: usize) {
        if ptr.is_null() || size == 0 {
            return;
        }
        if let Some(chunk_size) = Self::class_for_size(size) {
            for i in 0..self.active_slabs {
                if self.slabs[i].free_slot(ptr) {
                    self.allocated_bytes = self.allocated_bytes.saturating_sub(chunk_size);
                    return;
                }
            }
        }
        let pages_needed = (size + PAGE_SIZE - 1) / PAGE_SIZE;
        self.free_pages(ptr, pages_needed);
    }
}
