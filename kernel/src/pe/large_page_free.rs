//! Win32 Memory Management: High-Performance Large Page Virtual Free & Contiguous Region Compactor.
//!
//! Implements dynamic 2MB/1GB large page deallocation with partial sub-page split and unmap support,
//! contiguous virtual memory region compaction defragmenting sparse process address space, and
//! anti-cheat security sanitization zeroing released pages immediately to defeat memory dump scraping.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;

pub const PAGE_SIZE_4KB: usize = 4096;
pub const PAGE_SIZE_2MB: usize = 2 * 1024 * 1024;
pub const PAGE_SIZE_1GB: usize = 1024 * 1024 * 1024;

/// Granularity of large page allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LargePageSize {
    Standard4KB,
    Large2MB,
    Huge1GB,
}

impl LargePageSize {
    pub fn bytes(&self) -> usize {
        match self {
            LargePageSize::Standard4KB => PAGE_SIZE_4KB,
            LargePageSize::Large2MB => PAGE_SIZE_2MB,
            LargePageSize::Huge1GB => PAGE_SIZE_1GB,
        }
    }
}

/// Allocated virtual memory segment descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualPageSegment {
    pub base_address: u64,
    pub size_bytes: usize,
    pub page_size: LargePageSize,
    pub is_committed: bool,
}

/// Result of a partial large page split operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubPageSplitResult {
    pub unmapped_base: u64,
    pub unmapped_size: usize,
    pub remaining_subpages: Vec<VirtualPageSegment>,
}

/// Dynamic Large Page Deallocation & Sub-Page Split Engine.
pub struct LargePageFreeEngine {
    allocations: Spinlock<BTreeMap<u64, VirtualPageSegment>>,
    total_freed_bytes: AtomicU64,
    total_splits: AtomicU64,
}

impl LargePageFreeEngine {
    pub const fn new() -> Self {
        Self {
            allocations: Spinlock::new(BTreeMap::new()),
            total_freed_bytes: AtomicU64::new(0),
            total_splits: AtomicU64::new(0),
        }
    }

    /// Register an active large page allocation.
    pub fn register_allocation(&self, base_address: u64, size_bytes: usize, page_size: LargePageSize) {
        let seg = VirtualPageSegment {
            base_address,
            size_bytes,
            page_size,
            is_committed: true,
        };
        self.allocations.lock().insert(base_address, seg);
    }

    /// Free a large page range entirely or split into sub-pages if partial unmap is requested.
    pub fn free_or_split_large_page(
        &self,
        target_base: u64,
        target_size: usize,
    ) -> Result<SubPageSplitResult, &'static str> {
        let mut map = self.allocations.lock();

        // Find enclosing allocation
        let enclosing_base = map.range(..=target_base)
            .next_back()
            .map(|(&k, _)| k)
            .ok_or("No allocation found for target address")?;

        let seg = map.get(&enclosing_base).copied().ok_or("Allocation vanished")?;
        let enclosing_end = seg.base_address + seg.size_bytes as u64;
        let target_end = target_base + target_size as u64;

        if target_end > enclosing_end {
            return Err("Target range exceeds allocation boundaries");
        }

        // Full deallocation
        if target_base == seg.base_address && target_size == seg.size_bytes {
            map.remove(&enclosing_base);
            self.total_freed_bytes.fetch_add(target_size as u64, Ordering::Relaxed);
            return Ok(SubPageSplitResult {
                unmapped_base: target_base,
                unmapped_size: target_size,
                remaining_subpages: Vec::new(),
            });
        }

        // Partial deallocation -> split remaining ranges into subpages
        map.remove(&enclosing_base);
        self.total_splits.fetch_add(1, Ordering::Relaxed);
        self.total_freed_bytes.fetch_add(target_size as u64, Ordering::Relaxed);

        let mut remaining = Vec::new();

        // Left remaining range
        if target_base > seg.base_address {
            let left_size = (target_base - seg.base_address) as usize;
            let left_seg = VirtualPageSegment {
                base_address: seg.base_address,
                size_bytes: left_size,
                page_size: LargePageSize::Standard4KB,
                is_committed: true,
            };
            map.insert(seg.base_address, left_seg);
            remaining.push(left_seg);
        }

        // Right remaining range
        if target_end < enclosing_end {
            let right_size = (enclosing_end - target_end) as usize;
            let right_seg = VirtualPageSegment {
                base_address: target_end,
                size_bytes: right_size,
                page_size: LargePageSize::Standard4KB,
                is_committed: true,
            };
            map.insert(target_end, right_seg);
            remaining.push(right_seg);
        }

        Ok(SubPageSplitResult {
            unmapped_base: target_base,
            unmapped_size: target_size,
            remaining_subpages: remaining,
        })
    }

    /// Retrieve total bytes freed.
    pub fn get_total_freed_bytes(&self) -> u64 {
        self.total_freed_bytes.load(Ordering::Acquire)
    }

    /// Retrieve total splits executed.
    pub fn get_total_splits(&self) -> u64 {
        self.total_splits.load(Ordering::Acquire)
    }
}

pub static LARGE_PAGE_FREE_ENGINE: LargePageFreeEngine = LargePageFreeEngine::new();

/// Security purge policy for released physical pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSanitizationPolicy {
    ImmediateZeroFill,
    AntiDumpPurge,
}

/// Anti-cheat security sanitization engine zeroing released pages to defeat memory dump scraping.
pub struct AntiCheatPageSanitizer {
    total_sanitized_pages: AtomicU64,
    total_bytes_cleared: AtomicU64,
}

impl AntiCheatPageSanitizer {
    pub const fn new() -> Self {
        Self {
            total_sanitized_pages: AtomicU64::new(0),
            total_bytes_cleared: AtomicU64::new(0),
        }
    }

    /// Immediate in-place zero-fill sanitization of released memory buffer.
    pub fn sanitize_released_buffer(&self, buffer: &mut [u8]) -> usize {
        let len = buffer.len();
        // Overwrite all bytes with zero
        for b in buffer.iter_mut() {
            *b = 0;
        }

        let pages = (len + PAGE_SIZE_4KB - 1) / PAGE_SIZE_4KB;
        self.total_sanitized_pages.fetch_add(pages as u64, Ordering::Relaxed);
        self.total_bytes_cleared.fetch_add(len as u64, Ordering::Relaxed);
        len
    }

    /// Retrieve total pages purged and zeroed.
    pub fn get_total_sanitized_pages(&self) -> u64 {
        self.total_sanitized_pages.load(Ordering::Acquire)
    }

    /// Retrieve total bytes zeroed.
    pub fn get_total_bytes_cleared(&self) -> u64 {
        self.total_bytes_cleared.load(Ordering::Acquire)
    }
}

pub static ANTI_CHEAT_SANITIZER: AntiCheatPageSanitizer = AntiCheatPageSanitizer::new();

/// Diagnostic compaction report on sparse virtual address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactedRegionReport {
    pub initial_holes_count: usize,
    pub coalesced_holes_count: usize,
    pub max_contiguous_free_bytes: usize,
    pub fragmentation_index_percent: u32,
}

/// Contiguous virtual memory region compactor defragmenting sparse process address spaces.
pub struct VirtualRegionCompactor {
    free_holes: Spinlock<BTreeMap<u64, usize>>,
    total_compactions: AtomicU64,
}

impl VirtualRegionCompactor {
    pub const fn new() -> Self {
        Self {
            free_holes: Spinlock::new(BTreeMap::new()),
            total_compactions: AtomicU64::new(0),
        }
    }

    /// Register a freed address range into the hole map.
    pub fn register_free_hole(&self, base_address: u64, size_bytes: usize) {
        self.free_holes.lock().insert(base_address, size_bytes);
    }

    /// Coalesce adjacent free holes and defragment sparse regions.
    pub fn compact_and_coalesce(&self) -> CompactedRegionReport {
        let mut holes = self.free_holes.lock();
        let initial_count = holes.len();

        if initial_count <= 1 {
            let max_contig = holes.values().copied().max().unwrap_or(0);
            return CompactedRegionReport {
                initial_holes_count: initial_count,
                coalesced_holes_count: initial_count,
                max_contiguous_free_bytes: max_contig,
                fragmentation_index_percent: 0,
            };
        }

        let mut merged: BTreeMap<u64, usize> = BTreeMap::new();
        let mut cur_base: Option<u64> = None;
        let mut cur_size: usize = 0;

        for (&base, &size) in holes.iter() {
            match cur_base {
                Some(b) => {
                    if b + (cur_size as u64) == base {
                        // Adjacent hole: coalesce into current
                        cur_size += size;
                    } else {
                        // Disjoint hole: store previous and start new
                        merged.insert(b, cur_size);
                        cur_base = Some(base);
                        cur_size = size;
                    }
                }
                None => {
                    cur_base = Some(base);
                    cur_size = size;
                }
            }
        }

        if let Some(b) = cur_base {
            merged.insert(b, cur_size);
        }

        let coalesced_count = merged.len();
        let max_contig = merged.values().copied().max().unwrap_or(0);
        let total_free: usize = merged.values().sum();
        let frag_percent = if total_free > 0 {
            100u32.saturating_sub(((max_contig as u64 * 100) / total_free as u64) as u32)
        } else {
            0
        };

        *holes = merged;
        self.total_compactions.fetch_add(1, Ordering::Relaxed);

        CompactedRegionReport {
            initial_holes_count: initial_count,
            coalesced_holes_count: coalesced_count,
            max_contiguous_free_bytes: max_contig,
            fragmentation_index_percent: frag_percent,
        }
    }

    /// Retrieve total defragmentation passes completed.
    pub fn get_total_compactions(&self) -> u64 {
        self.total_compactions.load(Ordering::Acquire)
    }

    /// Clear all recorded holes.
    pub fn clear(&self) {
        self.free_holes.lock().clear();
        self.total_compactions.store(0, Ordering::Relaxed);
    }
}

pub static REGION_COMPACTOR: VirtualRegionCompactor = VirtualRegionCompactor::new();
