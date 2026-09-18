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

/// Checks if an opaque handle is registered as a file mapping object.
pub fn is_mapping_handle(h_object: HANDLE) -> bool {
    HANDLE_MAPPINGS.lock().contains_key(&h_object)
}

/// Closes a file mapping object handle and decrements reference count.
pub fn win32_close_mapping(h_object: HANDLE) -> bool {
    let mut table = HANDLE_MAPPINGS.lock();
    if let Some(obj_arc) = table.remove(&h_object) {
        let obj = obj_arc.lock();
        if let Some(ref name) = obj.name {
            let mut named = NAMED_MAPPINGS.lock();
            if let Some(existing) = named.get(name) {
                if Arc::ptr_eq(existing, &obj_arc) && Arc::strong_count(&obj_arc) <= 2 {
                    named.remove(name);
                }
            }
        }
        drop(obj);
        win32_set_last_error(ERROR_SUCCESS);
        true
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        false
    }
}

unsafe fn parse_pcstr(ptr: *const u8) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    while *ptr.add(len) != 0 && len < 260 {
        len += 1;
    }
    if len == 0 {
        return None;
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    core::str::from_utf8(slice).ok().map(String::from)
}

unsafe fn parse_pcwstr(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    while *ptr.add(len) != 0 && len < 260 {
        len += 1;
    }
    if len == 0 {
        return None;
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    let mut s = String::with_capacity(len);
    for c in core::char::decode_utf16(slice.iter().cloned()) {
        s.push(c.unwrap_or('?'));
    }
    Some(s)
}

unsafe fn create_file_mapping_internal(
    h_file: HANDLE,
    fl_protect: u32,
    max_size: usize,
    name: Option<String>,
) -> HANDLE {
    // Check if named object already exists
    if let Some(ref n) = name {
        let named_table = NAMED_MAPPINGS.lock();
        if let Some(existing_obj) = named_table.get(n) {
            let mut h_lock = NEXT_MAPPING_HANDLE.lock();
            let new_h = *h_lock;
            *h_lock += 1;
            drop(h_lock);

            HANDLE_MAPPINGS.lock().insert(new_h, Arc::clone(existing_obj));
            win32_set_last_error(ERROR_ALREADY_EXISTS);
            return new_h;
        }
    }

    // Validate protection
    let valid_prot = (fl_protect & (PAGE_READONLY | PAGE_READWRITE | PAGE_WRITECOPY)) != 0;
    if !valid_prot {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }

    // Resolve backing storage
    let (vfs_path, actual_size, storage): (Option<String>, usize, MappingStorage) = if h_file == INVALID_HANDLE_VALUE || h_file == 0 {
        if max_size == 0 {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        let st = match MappingStorage::new(max_size) {
            Some(s) => s,
            None => {
                win32_set_last_error(ERROR_NOT_ENOUGH_MEMORY);
                return 0;
            }
        };
        (None, max_size, st)
    } else {
        let path = match crate::pe::file::win32_get_file_path_for_mapping(h_file) {
            Some(p) => p,
            None => {
                win32_set_last_error(ERROR_INVALID_HANDLE);
                return 0;
            }
        };
        let file_bytes = crate::fs::VFS.lock().read_file(&path).unwrap_or_default();
        let target_size = if max_size == 0 {
            if file_bytes.is_empty() {
                win32_set_last_error(ERROR_INVALID_PARAMETER);
                return 0;
            }
            file_bytes.len()
        } else {
            max_size
        };

        let st = match MappingStorage::new(target_size) {
            Some(s) => s,
            None => {
                win32_set_last_error(ERROR_NOT_ENOUGH_MEMORY);
                return 0;
            }
        };

        let copy_len = core::cmp::min(file_bytes.len(), target_size);
        if copy_len > 0 {
            core::ptr::copy_nonoverlapping(file_bytes.as_ptr(), st.ptr, copy_len);
        }
        (Some(path), target_size, st)
    };

    let obj = Arc::new(Spinlock::new(FileMappingObject {
        name: name.clone(),
        h_file,
        vfs_path,
        protect: fl_protect,
        max_size: actual_size,
        storage: Arc::new(storage),
    }));

    if let Some(n) = name {
        NAMED_MAPPINGS.lock().insert(n, Arc::clone(&obj));
    }

    let mut h_lock = NEXT_MAPPING_HANDLE.lock();
    let handle = *h_lock;
    *h_lock += 1;
    drop(h_lock);

    HANDLE_MAPPINGS.lock().insert(handle, obj);
    win32_set_last_error(ERROR_SUCCESS);
    handle
}

/// Creates or opens a named or unnamed file mapping object (ANSI).
pub unsafe fn CreateFileMappingA(
    h_file: HANDLE,
    _lp_attributes: usize,
    fl_protect: u32,
    dw_maximum_size_high: u32,
    dw_maximum_size_low: u32,
    lp_name: *const u8,
) -> HANDLE {
    let max_size = (((dw_maximum_size_high as u64) << 32) | (dw_maximum_size_low as u64)) as usize;
    let name = parse_pcstr(lp_name);
    create_file_mapping_internal(h_file, fl_protect, max_size, name)
}

/// Creates or opens a named or unnamed file mapping object (Unicode).
pub unsafe fn CreateFileMappingW(
    h_file: HANDLE,
    _lp_attributes: usize,
    fl_protect: u32,
    dw_maximum_size_high: u32,
    dw_maximum_size_low: u32,
    lp_name: *const u16,
) -> HANDLE {
    let max_size = (((dw_maximum_size_high as u64) << 32) | (dw_maximum_size_low as u64)) as usize;
    let name = parse_pcwstr(lp_name);
    create_file_mapping_internal(h_file, fl_protect, max_size, name)
}

unsafe fn open_file_mapping_internal(
    dw_desired_access: u32,
    name: Option<String>,
) -> HANDLE {
    let n = match name {
        Some(s) => s,
        None => {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
    };

    let table = NAMED_MAPPINGS.lock();
    let obj_arc = match table.get(&n) {
        Some(arc) => Arc::clone(arc),
        None => {
            win32_set_last_error(ERROR_FILE_NOT_FOUND);
            return 0;
        }
    };
    drop(table);

    let obj = obj_arc.lock();
    if (obj.protect & PAGE_READONLY) != 0 && (dw_desired_access & (FILE_MAP_WRITE | FILE_MAP_ALL_ACCESS)) != 0 {
        win32_set_last_error(ERROR_ACCESS_DENIED);
        return 0;
    }
    drop(obj);

    let mut h_lock = NEXT_MAPPING_HANDLE.lock();
    let handle = *h_lock;
    *h_lock += 1;
    drop(h_lock);

    HANDLE_MAPPINGS.lock().insert(handle, obj_arc);
    win32_set_last_error(ERROR_SUCCESS);
    handle
}

/// Opens an existing named file mapping object (ANSI).
pub unsafe fn OpenFileMappingA(
    dw_desired_access: u32,
    _b_inherit_handle: i32,
    lp_name: *const u8,
) -> HANDLE {
    let name = parse_pcstr(lp_name);
    open_file_mapping_internal(dw_desired_access, name)
}

/// Opens an existing named file mapping object (Unicode).
pub unsafe fn OpenFileMappingW(
    dw_desired_access: u32,
    _b_inherit_handle: i32,
    lp_name: *const u16,
) -> HANDLE {
    let name = parse_pcwstr(lp_name);
    open_file_mapping_internal(dw_desired_access, name)
}
