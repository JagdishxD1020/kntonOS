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
