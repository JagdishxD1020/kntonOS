//! Win32 Memory-Mapped Files & File Mapping Shared Cache Subsystem.
//!
//! Implements Win32 file mapping objects, named and anonymous paging-file
//! backed mappings, coherent multi-view shared page cache, and copy-on-write isolation.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use core::alloc::Layout;
use crate::memory::spinlock::Spinlock;

use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND,
    ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
    INVALID_HANDLE_VALUE,
};
use crate::pe::virtual_alloc::{PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY};

type HANDLE = u64;

// Win32 MapViewOfFile access flags
pub const FILE_MAP_COPY: u32 = 0x00000001;
pub const FILE_MAP_WRITE: u32 = 0x00000002;
pub const FILE_MAP_READ: u32 = 0x00000004;
pub const FILE_MAP_ALL_ACCESS: u32 = 0x000F001F;
pub const FILE_MAP_EXECUTE: u32 = 0x00000020;

/// Contiguous heap-allocated backing storage for file mapping objects.
pub struct MappingStorage {
    pub ptr: *mut u8,
    pub size: usize,
}

unsafe impl Send for MappingStorage {}
unsafe impl Sync for MappingStorage {}

impl MappingStorage {
    /// Allocates zeroed page-aligned storage of at least `size` bytes.
    pub fn new(size: usize) -> Option<Self> {
        if size == 0 {
            return None;
        }
        let page_size = 4096;
        let aligned_size = (size + page_size - 1) & !(page_size - 1);
        let layout = Layout::from_size_align(aligned_size, page_size).ok()?;
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            None
        } else {
            Some(Self {
                ptr,
                size: aligned_size,
            })
        }
    }
}

impl Drop for MappingStorage {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.size > 0 {
            if let Ok(layout) = Layout::from_size_align(self.size, 4096) {
                unsafe {
                    alloc::alloc::dealloc(self.ptr, layout);
                }
            }
        }
    }
}

/// Win32 File Mapping kernel object.
pub struct FileMappingObject {
    pub name: Option<String>,
    pub h_file: HANDLE,
    pub vfs_path: Option<String>,
    pub protect: u32,
    pub max_size: usize,
    pub storage: Arc<MappingStorage>,
}

pub static NAMED_MAPPINGS: Spinlock<BTreeMap<String, Arc<Spinlock<FileMappingObject>>>> =
    Spinlock::new(BTreeMap::new());

pub static HANDLE_MAPPINGS: Spinlock<BTreeMap<HANDLE, Arc<Spinlock<FileMappingObject>>>> =
    Spinlock::new(BTreeMap::new());

pub static NEXT_MAPPING_HANDLE: Spinlock<u64> = Spinlock::new(0x0010_0001);
