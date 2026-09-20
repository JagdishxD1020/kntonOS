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

/// Represents an active mapped view of a file mapping.
pub struct ActiveViewEntry {
    pub base_ptr: usize,
    pub size: usize,
    pub offset: usize,
    pub access: u32,
    pub is_cow: bool,
    pub cow_buffer: Option<*mut u8>,
    pub mapping_obj: Arc<Spinlock<FileMappingObject>>,
    pub storage: Arc<MappingStorage>,
}

unsafe impl Send for ActiveViewEntry {}
unsafe impl Sync for ActiveViewEntry {}

pub static ACTIVE_VIEWS: Spinlock<alloc::vec::Vec<ActiveViewEntry>> =
    Spinlock::new(alloc::vec::Vec::new());

unsafe fn map_view_internal(
    h_file_mapping: HANDLE,
    dw_desired_access: u32,
    dw_file_offset_high: u32,
    dw_file_offset_low: u32,
    dw_number_of_bytes_to_map: usize,
    _lp_base_address: *mut u8,
) -> *mut u8 {
    let mapping_table = HANDLE_MAPPINGS.lock();
    let obj_arc = match mapping_table.get(&h_file_mapping) {
        Some(arc) => Arc::clone(arc),
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return core::ptr::null_mut();
        }
    };
    drop(mapping_table);

    let obj = obj_arc.lock();
    if (obj.protect & PAGE_READONLY) != 0 && (dw_desired_access & (FILE_MAP_WRITE | FILE_MAP_ALL_ACCESS)) != 0 {
        win32_set_last_error(ERROR_ACCESS_DENIED);
        return core::ptr::null_mut();
    }

    let offset = (((dw_file_offset_high as u64) << 32) | (dw_file_offset_low as u64)) as usize;
    if offset >= obj.max_size {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }

    let bytes_to_map = if dw_number_of_bytes_to_map == 0 {
        obj.max_size - offset
    } else {
        if offset + dw_number_of_bytes_to_map > obj.max_size {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return core::ptr::null_mut();
        }
        dw_number_of_bytes_to_map
    };

    let is_cow = dw_desired_access == FILE_MAP_COPY;
    let (view_ptr, cow_buf) = if is_cow {
        let layout = match Layout::from_size_align(bytes_to_map, 4096) {
            Ok(l) => l,
            Err(_) => {
                win32_set_last_error(ERROR_NOT_ENOUGH_MEMORY);
                return core::ptr::null_mut();
            }
        };
        let replica = alloc::alloc::alloc_zeroed(layout);
        if replica.is_null() {
            win32_set_last_error(ERROR_NOT_ENOUGH_MEMORY);
            return core::ptr::null_mut();
        }
        core::ptr::copy_nonoverlapping(obj.storage.ptr.add(offset), replica, bytes_to_map);
        (replica, Some(replica))
    } else {
        (obj.storage.ptr.add(offset), None)
    };

    let base_key = view_ptr as usize;
    let entry = ActiveViewEntry {
        base_ptr: base_key,
        size: bytes_to_map,
        offset,
        access: dw_desired_access,
        is_cow,
        cow_buffer: cow_buf,
        mapping_obj: Arc::clone(&obj_arc),
        storage: Arc::clone(&obj.storage),
    };
    drop(obj);

    ACTIVE_VIEWS.lock().push(entry);
    win32_set_last_error(ERROR_SUCCESS);
    view_ptr
}

/// Maps a view of a file mapping into the address space of a calling process.
pub unsafe fn MapViewOfFile(
    h_file_mapping: HANDLE,
    dw_desired_access: u32,
    dw_file_offset_high: u32,
    dw_file_offset_low: u32,
    dw_number_of_bytes_to_map: usize,
) -> *mut u8 {
    map_view_internal(
        h_file_mapping,
        dw_desired_access,
        dw_file_offset_high,
        dw_file_offset_low,
        dw_number_of_bytes_to_map,
        core::ptr::null_mut(),
    )
}

/// Maps a view of a file mapping into the address space with an optional base address.
pub unsafe fn MapViewOfFileEx(
    h_file_mapping: HANDLE,
    dw_desired_access: u32,
    dw_file_offset_high: u32,
    dw_file_offset_low: u32,
    dw_number_of_bytes_to_map: usize,
    lp_base_address: *mut u8,
) -> *mut u8 {
    map_view_internal(
        h_file_mapping,
        dw_desired_access,
        dw_file_offset_high,
        dw_file_offset_low,
        dw_number_of_bytes_to_map,
        lp_base_address,
    )
}

/// Unmaps a mapped view of a file from the calling process\'s address space.
pub unsafe fn UnmapViewOfFile(lp_base_address: *const u8) -> i32 {
    if lp_base_address.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }

    let mut views = ACTIVE_VIEWS.lock();
    let key = lp_base_address as usize;
    let pos = views.iter().rposition(|v| v.base_ptr == key);
    if let Some(idx) = pos {
        let view = views.remove(idx);
        if view.is_cow {
            if let Some(buf) = view.cow_buffer {
                if let Ok(layout) = Layout::from_size_align(view.size, 4096) {
                    alloc::alloc::dealloc(buf, layout);
                }
            }
        }
        win32_set_last_error(ERROR_SUCCESS);
        1
    } else {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        0
    }
}

/// Flushes a range within a mapped view of a file to disk.
pub unsafe fn FlushViewOfFile(
    lp_base_address: *const u8,
    dw_number_of_bytes_to_flush: usize,
) -> i32 {
    if lp_base_address.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }

    let key = lp_base_address as usize;
    let views = ACTIVE_VIEWS.lock();
    let mut target = None;
    for view in views.iter() {
        let base = view.base_ptr;
        if key >= base && key < base + view.size {
            let rel_offset = key - base;
            let max_flush = view.size - rel_offset;
            let flush_len = if dw_number_of_bytes_to_flush == 0 {
                max_flush
            } else {
                core::cmp::min(dw_number_of_bytes_to_flush, max_flush)
            };
            target = Some((
                view.is_cow,
                view.offset + rel_offset,
                flush_len,
                Arc::clone(&view.mapping_obj),
                view.storage.ptr,
            ));
            break;
        }
    }
    drop(views);

    let (is_cow, abs_offset, flush_len, mapping_arc, storage_ptr) = match target {
        Some(t) => t,
        None => {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
    };

    // Copy-on-write views do not flush modifications back to the file
    if is_cow {
        win32_set_last_error(ERROR_SUCCESS);
        return 1;
    }

    let obj = mapping_arc.lock();
    if let Some(ref path) = obj.vfs_path {
        let mut vfs = crate::fs::VFS.lock();
        let mut file_data = vfs.read_file(path).unwrap_or_default();
        if abs_offset + flush_len > file_data.len() {
            file_data.resize(abs_offset + flush_len, 0);
        }
        let src_slice = core::slice::from_raw_parts(storage_ptr.add(abs_offset), flush_len);
        file_data[abs_offset..abs_offset + flush_len].copy_from_slice(src_slice);
        vfs.write_file(path, &file_data);
    }
    drop(obj);

    win32_set_last_error(ERROR_SUCCESS);
    1
}


/// Comprehensive self-test for Win32 memory-mapped files and shared cache.
pub unsafe fn test_win32_mmap_subsystem() -> bool {
    // 1. Anonymous named mapping & multi-view coherence
    let map_name = b"Local\\KntonMmapTestShared\0";
    let h_map1 = CreateFileMappingA(
        INVALID_HANDLE_VALUE,
        0,
        PAGE_READWRITE,
        0,
        8192,
        map_name.as_ptr(),
    );
    if h_map1 == 0 {
        return false;
    }
    let view1 = MapViewOfFile(h_map1, FILE_MAP_WRITE, 0, 0, 4096);
    if view1.is_null() {
        win32_close_mapping(h_map1);
        return false;
    }
    let h_map2 = OpenFileMappingA(FILE_MAP_READ, 0, map_name.as_ptr());
    if h_map2 == 0 {
        UnmapViewOfFile(view1);
        win32_close_mapping(h_map1);
        return false;
    }
    let view2 = MapViewOfFile(h_map2, FILE_MAP_READ, 0, 0, 4096);
    if view2.is_null() {
        win32_close_mapping(h_map2);
        UnmapViewOfFile(view1);
        win32_close_mapping(h_map1);
        return false;
    }
    let test_token = b"KNTON_MMAP_COHERENCE_TOKEN_1234";
    core::ptr::copy_nonoverlapping(test_token.as_ptr(), view1, test_token.len());

    let slice2 = core::slice::from_raw_parts(view2, test_token.len());
    if slice2 != test_token {
        UnmapViewOfFile(view2);
        win32_close_mapping(h_map2);
        UnmapViewOfFile(view1);
        win32_close_mapping(h_map1);
        return false;
    }

    // 2. Copy-On-Write (COW) View Isolation
    let view_cow = MapViewOfFile(h_map1, FILE_MAP_COPY, 0, 0, 4096);
    if view_cow.is_null() {
        UnmapViewOfFile(view2);
        win32_close_mapping(h_map2);
        UnmapViewOfFile(view1);
        win32_close_mapping(h_map1);
        return false;
    }
    let cow_slice_init = core::slice::from_raw_parts(view_cow, test_token.len());
    if cow_slice_init != test_token {
        UnmapViewOfFile(view_cow);
        UnmapViewOfFile(view2);
        win32_close_mapping(h_map2);
        UnmapViewOfFile(view1);
        win32_close_mapping(h_map1);
        return false;
    }
    let cow_mut_token = b"OVERWRITTEN_IN_COW_VIEW_5678";
    core::ptr::copy_nonoverlapping(cow_mut_token.as_ptr(), view_cow, cow_mut_token.len());

    let slice1 = core::slice::from_raw_parts(view1, test_token.len());
    if slice1 != test_token || slice2 != test_token {
        UnmapViewOfFile(view_cow);
        UnmapViewOfFile(view2);
        win32_close_mapping(h_map2);
        UnmapViewOfFile(view1);
        win32_close_mapping(h_map1);
        return false;
    }
    if UnmapViewOfFile(view_cow) == 0 {
        return false;
    }
    if UnmapViewOfFile(view2) == 0 {
        return false;
    }
    if UnmapViewOfFile(view1) == 0 {
        return false;
    }
    win32_close_mapping(h_map2);
    win32_close_mapping(h_map1);

    // 3. File-backed mapping and FlushViewOfFile
    let test_file_path = r"C:\knton_test\mmap_disk.dat";
    let h_file = crate::pe::file::win32_create_file(
        test_file_path,
        crate::pe::file::GENERIC_READ | crate::pe::file::GENERIC_WRITE,
        crate::pe::file::FILE_SHARE_READ | crate::pe::file::FILE_SHARE_WRITE,
        crate::pe::file::CREATE_ALWAYS,
        crate::pe::file::FILE_ATTRIBUTE_NORMAL,
    );
    if h_file == INVALID_HANDLE_VALUE || h_file == 0 {
        return false;
    }
    let init_file_data = b"INITIAL_VFS_FILE_DATA_BLOCK";
    let mut written = 0u32;
    crate::pe::file::win32_write_file(
        h_file,
        init_file_data.as_ptr(),
        init_file_data.len() as u32,
        &mut written,
    );
    let h_file_map = CreateFileMappingA(
        h_file,
        0,
        PAGE_READWRITE,
        0,
        4096,
        core::ptr::null(),
    );
    if h_file_map == 0 {
        crate::pe::file::win32_close_handle(h_file);
        return false;
    }
    let file_view = MapViewOfFile(h_file_map, FILE_MAP_ALL_ACCESS, 0, 0, 4096);
    if file_view.is_null() {
        win32_close_mapping(h_file_map);
        crate::pe::file::win32_close_handle(h_file);
        return false;
    }

    let init_view_slice = core::slice::from_raw_parts(file_view, init_file_data.len());
    if init_view_slice != init_file_data {
        UnmapViewOfFile(file_view);
        win32_close_mapping(h_file_map);
        crate::pe::file::win32_close_handle(h_file);
        return false;
    }
    let flushed_data = b"MODIFIED_MMAP_DISK_CACHE_FLUSH";
    core::ptr::copy_nonoverlapping(flushed_data.as_ptr(), file_view, flushed_data.len());

    if FlushViewOfFile(file_view, flushed_data.len()) == 0 {
        UnmapViewOfFile(file_view);
        win32_close_mapping(h_file_map);
        crate::pe::file::win32_close_handle(h_file);
        return false;
    }
    let vfs_path = crate::pe::file::win32_dos_to_vfs_path(test_file_path);
    let disk_bytes = crate::fs::VFS.lock().read_file(&vfs_path).unwrap_or_default();
    if disk_bytes.len() < flushed_data.len() || &disk_bytes[..flushed_data.len()] != flushed_data {
        UnmapViewOfFile(file_view);
        win32_close_mapping(h_file_map);
        crate::pe::file::win32_close_handle(h_file);
        return false;
    }
    if UnmapViewOfFile(file_view) == 0 {
        return false;
    }
    if !crate::pe::file::win32_close_handle(h_file_map) {
        return false;
    }
    if !crate::pe::file::win32_close_handle(h_file) {
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-MMAP-OK] Win32 memory-mapped files and shared cache verified\r\n");
    true
}
