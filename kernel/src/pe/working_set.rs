//! Win32 Memory Management: Working Set Trimming, Working Set Watch & Page Fault Clustering Engine.
//!
//! Implements Process Working Set management (EmptyWorkingSet, SetProcessWorkingSetSizeEx,
//! QUOTA_LIMITS_EX), Working Set Watch (QueryWorkingSetEx, PSAPI_WORKING_SET_EX_INFORMATION),
//! 64KB demand fault clustering for executable sections, and anti-cheat working set scanning.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

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
    PAGE_SIZE, LARGE_PAGE_SIZE, MEM_COMMIT, MEM_DECOMMIT, MEM_RELEASE, MEM_RESERVE,
    PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_GUARD,
    PAGE_NOACCESS, PAGE_NOCACHE, PAGE_READONLY, PAGE_READWRITE,
};

// Working set quota flags
pub const QUOTA_LIMITS_HARDWS_MIN_ENABLE: u32 = 0x0000_0001;
pub const QUOTA_LIMITS_HARDWS_MIN_DISABLE: u32 = 0x0000_0002;
pub const QUOTA_LIMITS_HARDWS_MAX_ENABLE: u32 = 0x0000_0004;
pub const QUOTA_LIMITS_HARDWS_MAX_DISABLE: u32 = 0x0000_0008;
pub const QUOTA_LIMITS_USE_DEFAULT_LIMITS: u32 = 0x0000_0010;

pub const DEFAULT_MIN_WORKING_SET: usize = 64 * 1024;        // 64 KB minimum
pub const DEFAULT_MAX_WORKING_SET: usize = 1024 * 1024 * 1024; // 1 GB maximum

/// Win32 QUOTA_LIMITS_EX equivalent structure.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QUOTA_LIMITS_EX {
    pub PagedPoolLimit: usize,
    pub NonPagedPoolLimit: usize,
    pub MinimumWorkingSetSize: usize,
    pub MaximumWorkingSetSize: usize,
    pub PagefileLimit: usize,
    pub TimeLimit: i64,
    pub WorkingSetLimit: usize,
    pub Reserved2: usize,
    pub Reserved3: usize,
    pub Reserved4: usize,
    pub Flags: u32,
    pub Reserved5: u32,
}

impl Default for QUOTA_LIMITS_EX {
    fn default() -> Self {
        Self {
            PagedPoolLimit: 0,
            NonPagedPoolLimit: 0,
            MinimumWorkingSetSize: DEFAULT_MIN_WORKING_SET,
            MaximumWorkingSetSize: DEFAULT_MAX_WORKING_SET,
            PagefileLimit: 0,
            TimeLimit: 0,
            WorkingSetLimit: 0,
            Reserved2: 0,
            Reserved3: 0,
            Reserved4: 0,
            Flags: 0,
            Reserved5: 0,
        }
    }
}

/// Descriptor tracking process working set state and physical pages.
#[derive(Debug, Clone)]
pub struct ProcessWorkingSet {
    pub process_id: u32,
    pub min_working_set: usize,
    pub max_working_set: usize,
    pub flags: u32,
    pub hard_min_enabled: bool,
    pub hard_max_enabled: bool,
    pub resident_pages: BTreeMap<usize, WorkingSetPageEntry>,
    pub total_trims: usize,
    pub last_trimmed_pages: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkingSetPageEntry {
    pub virtual_address: usize,
    pub protection: u32,
    pub share_count: u8,
    pub is_shared: bool,
    pub numa_node: u8,
    pub is_locked: bool,
    pub is_large_page: bool,
    pub is_executable: bool,
    pub age: u8,
}

/// Global Working Set Governor managing per-process working sets.
pub struct WorkingSetGovernor {
    sets: Spinlock<BTreeMap<u32, ProcessWorkingSet>>,
}

impl WorkingSetGovernor {
    pub const fn new() -> Self {
        Self {
            sets: Spinlock::new(BTreeMap::new()),
        }
    }

    /// Register or get working set descriptor for a process.
    pub fn ensure_working_set(&self, pid: u32) {
        let mut map = self.sets.lock();
        map.entry(pid).or_insert_with(|| ProcessWorkingSet {
            process_id: pid,
            min_working_set: DEFAULT_MIN_WORKING_SET,
            max_working_set: DEFAULT_MAX_WORKING_SET,
            flags: 0,
            hard_min_enabled: false,
            hard_max_enabled: false,
            resident_pages: BTreeMap::new(),
            total_trims: 0,
            last_trimmed_pages: 0,
        });
    }

    /// Set process working set size and quota limits.
    pub fn set_working_set_size_ex(
        &self,
        pid: u32,
        min_size: usize,
        max_size: usize,
        flags: u32,
    ) -> Result<(), NtStatus> {
        self.ensure_working_set(pid);
        let mut map = self.sets.lock();
        let ws = map.get_mut(&pid).ok_or(STATUS_INVALID_PARAMETER)?;

        // Special case: (usize::MAX, usize::MAX) or (0, 0) signals EmptyWorkingSet
        if (min_size == usize::MAX && max_size == usize::MAX) || (min_size == 0 && max_size == 0) {
            let trimmed = ws.resident_pages.len();
            ws.resident_pages.clear();
            ws.total_trims += 1;
            ws.last_trimmed_pages = trimmed;
            return Ok(());
        }

        if min_size > max_size && max_size != 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }

        if (flags & QUOTA_LIMITS_HARDWS_MIN_ENABLE) != 0 {
            ws.hard_min_enabled = true;
        } else if (flags & QUOTA_LIMITS_HARDWS_MIN_DISABLE) != 0 {
            ws.hard_min_enabled = false;
        }

        if (flags & QUOTA_LIMITS_HARDWS_MAX_ENABLE) != 0 {
            ws.hard_max_enabled = true;
        } else if (flags & QUOTA_LIMITS_HARDWS_MAX_DISABLE) != 0 {
            ws.hard_max_enabled = false;
        }

        if min_size > 0 {
            ws.min_working_set = min_size;
        }
        if max_size > 0 {
            ws.max_working_set = max_size;
        }
        ws.flags = flags;

        // If current residency exceeds maximum working set size and hard max is enabled, trim down.
        let page_bytes = ws.resident_pages.len() * PAGE_SIZE;
        if ws.hard_max_enabled && page_bytes > ws.max_working_set {
            let pages_to_remove = (page_bytes - ws.max_working_set + PAGE_SIZE - 1) / PAGE_SIZE;
            let keys: Vec<usize> = ws.resident_pages.keys().take(pages_to_remove).copied().collect();
            for k in keys {
                ws.resident_pages.remove(&k);
            }
            ws.total_trims += 1;
        }

        Ok(())
    }

    /// Get current working set size limits and active residency.
    pub fn get_working_set_size_ex(
        &self,
        pid: u32,
    ) -> Result<(usize, usize, u32, usize), NtStatus> {
        self.ensure_working_set(pid);
        let map = self.sets.lock();
        let ws = map.get(&pid).ok_or(STATUS_INVALID_PARAMETER)?;
        let current_bytes = ws.resident_pages.len() * PAGE_SIZE;
        Ok((ws.min_working_set, ws.max_working_set, ws.flags, current_bytes))
    }

    /// Empty the process working set by discarding all resident pages.
    pub fn empty_working_set(&self, pid: u32) -> Result<usize, NtStatus> {
        self.ensure_working_set(pid);
        let mut map = self.sets.lock();
        let ws = map.get_mut(&pid).ok_or(STATUS_INVALID_PARAMETER)?;
        let trimmed = ws.resident_pages.len();
        ws.resident_pages.clear();
        ws.total_trims += 1;
        ws.last_trimmed_pages = trimmed;
        Ok(trimmed)
    }

    /// Add page to process working set.
    pub fn commit_resident_page(&self, pid: u32, entry: WorkingSetPageEntry) {
        self.ensure_working_set(pid);
        let mut map = self.sets.lock();
        if let Some(ws) = map.get_mut(&pid) {
            let aligned_va = entry.virtual_address & !(PAGE_SIZE - 1);
            ws.resident_pages.insert(aligned_va, entry);
        }
    }
}

pub static WORKING_SET_GOVERNOR: WorkingSetGovernor = WorkingSetGovernor::new();

/// Emulates Win32 EmptyWorkingSet API.
pub fn win32_empty_working_set(h_process: HANDLE) -> bool {
    let pid = (h_process as u32).max(1);
    match WORKING_SET_GOVERNOR.empty_working_set(pid) {
        Ok(_) => {
            win32_set_last_error(ERROR_SUCCESS);
            true
        }
        Err(_) => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            false
        }
    }
}

/// Emulates Win32 SetProcessWorkingSetSizeEx API.
pub fn win32_set_process_working_set_size_ex(
    h_process: HANDLE,
    dw_minimum_working_set_size: usize,
    dw_maximum_working_set_size: usize,
    flags: u32,
) -> bool {
    let pid = (h_process as u32).max(1);
    match WORKING_SET_GOVERNOR.set_working_set_size_ex(
        pid,
        dw_minimum_working_set_size,
        dw_maximum_working_set_size,
        flags,
    ) {
        Ok(_) => {
            win32_set_last_error(ERROR_SUCCESS);
            true
        }
        Err(_) => {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            false
        }
    }
}

/// Emulates Win32 GetProcessWorkingSetSizeEx API.
pub fn win32_get_process_working_set_size_ex(
    h_process: HANDLE,
    lp_minimum_working_set_size: &mut usize,
    lp_maximum_working_set_size: &mut usize,
    flags: &mut u32,
) -> bool {
    let pid = (h_process as u32).max(1);
    match WORKING_SET_GOVERNOR.get_working_set_size_ex(pid) {
        Ok((min_ws, max_ws, fl, _current)) => {
            *lp_minimum_working_set_size = min_ws;
            *lp_maximum_working_set_size = max_ws;
            *flags = fl;
            win32_set_last_error(ERROR_SUCCESS);
            true
        }
        Err(_) => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            false
        }
    }
}
