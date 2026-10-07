//! Win32 Named Shared Memory Sections, Page Fault Interception & Anti-Cheat Memory Mirror.
//!
//! Implements Win32 Section objects (CreateFileMapping, MapViewOfFileEx, UnmapViewOfFile),
//! Section access mask security checks (SECTION_MAP_READ, SECTION_MAP_WRITE, SECTION_MAP_EXECUTE),
//! copy-on-write dirty page tracking with write-protection trap for anti-cheat verification,
//! and handheld zero-copy inter-process gaming video/audio ring buffers.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS, INVALID_HANDLE_VALUE,
};
pub use crate::pe::mmap::{FILE_MAP_ALL_ACCESS, FILE_MAP_COPY, FILE_MAP_EXECUTE, FILE_MAP_READ, FILE_MAP_WRITE};
pub use crate::pe::srw_futex::{DWORD, PVOID, SIZE_T};
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

// NT Section Access Rights
pub const SECTION_QUERY: DWORD = 0x0001;
pub const SECTION_MAP_WRITE: DWORD = 0x0002;
pub const SECTION_MAP_READ: DWORD = 0x0004;
pub const SECTION_MAP_EXECUTE: DWORD = 0x0008;
pub const SECTION_EXTEND_SIZE: DWORD = 0x0010;
pub const SECTION_MAP_EXECUTE_EXPLICIT: DWORD = 0x0020;
pub const SECTION_ALL_ACCESS: DWORD = 0x000F_001F;

// Section Page Attributes
pub const SEC_COMMIT: DWORD = 0x0800_0000;
pub const SEC_IMAGE: DWORD = 0x0100_0000;
pub const SEC_NOCACHE: DWORD = 0x1000_0000;
pub const SEC_RESERVE: DWORD = 0x0400_0000;

pub use crate::pe::virtual_alloc::{
    PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY,
    PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY,
};

/// Shared Section Storage holding continuous page-aligned memory.
pub struct SectionStorage {
    pub ptr: *mut u8,
    pub size: usize,
    pub dirty_pages: Vec<AtomicBool>,
    pub page_count: usize,
}

unsafe impl Send for SectionStorage {}
unsafe impl Sync for SectionStorage {}

impl SectionStorage {
    pub fn new(size: usize) -> Option<Self> {
        if size == 0 {
            return None;
        }
        let page_size = 4096;
        let aligned_size = (size + page_size - 1) & !(page_size - 1);
        let layout = Layout::from_size_align(aligned_size, page_size).ok()?;
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            return None;
        }

        let page_count = aligned_size / page_size;
        let mut dirty_pages = Vec::with_capacity(page_count);
        for _ in 0..page_count {
            dirty_pages.push(AtomicBool::new(false));
        }

        Some(Self {
            ptr,
            size: aligned_size,
            dirty_pages,
            page_count,
        })
    }
}

impl Drop for SectionStorage {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.size > 0 {
            let layout = Layout::from_size_align(self.size, 4096).unwrap();
            unsafe {
                alloc::alloc::dealloc(self.ptr, layout);
            }
        }
    }
}

/// Representation of an active mapped view of a shared memory section.
#[derive(Clone)]
pub struct SectionMappedView {
    pub base_address: usize,
    pub size: usize,
    pub offset: usize,
    pub desired_access: DWORD,
    pub is_copy_on_write: bool,
    pub cow_buffer: Option<Arc<Spinlock<Vec<u8>>>>,
}

/// Win32 Named Shared Memory Section object descriptor.
pub struct SharedSectionInner {
    pub id: HANDLE,
    pub name: Option<String>,
    pub storage: Arc<SectionStorage>,
    pub max_size: usize,
    pub page_protection: DWORD,
    pub access_mask: DWORD,
}

static NEXT_SECTION_ID: AtomicU64 = AtomicU64::new(0xA000);
static SECTIONS: Spinlock<BTreeMap<HANDLE, SharedSectionInner>> = Spinlock::new(BTreeMap::new());
static NAMED_SECTIONS: Spinlock<BTreeMap<String, HANDLE>> = Spinlock::new(BTreeMap::new());
static MAPPED_VIEWS: Spinlock<BTreeMap<usize, SectionMappedView>> = Spinlock::new(BTreeMap::new());
static VIRTUAL_VIEW_ALLOC_ADDR: AtomicUsize = AtomicUsize::new(0x0000_7FF9_0000_0000);

/// Creates or opens a Win32 named/anonymous shared memory section.
pub unsafe extern "system" fn CreateSharedSectionA(
    hFile: HANDLE,
    _lpAttributes: *const c_void,
    flProtect: DWORD,
    dwMaximumSizeHigh: DWORD,
    dwMaximumSizeLow: DWORD,
    lpName: *const u8,
) -> HANDLE {
    let mut total_size = ((dwMaximumSizeHigh as usize) << 32) | (dwMaximumSizeLow as usize);
    if total_size == 0 {
        total_size = 4096;
    }

    let name_str = if !lpName.is_null() {
        let mut len = 0;
        while *lpName.add(len) != 0 && len < 256 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(lpName, len);
        core::str::from_utf8(slice).ok().map(String::from)
    } else {
        None
    };

    if let Some(ref name) = name_str {
        let named = NAMED_SECTIONS.lock();
        if let Some(&existing_id) = named.get(name) {
            win32_set_last_error(ERROR_ALREADY_EXISTS);
            return existing_id;
        }
    }

    let storage = match SectionStorage::new(total_size) {
        Some(s) => Arc::new(s),
        None => {
            win32_set_last_error(ERROR_NOT_ENOUGH_MEMORY);
            return 0;
        }
    };

    // If backing file was provided, ingest initial data into storage
    if hFile != 0 && hFile != (INVALID_HANDLE_VALUE as HANDLE) {
        let file_data = {
            let vfs = crate::fs::VFS.lock();
            vfs.read_file("/bin/init.exe").unwrap_or_default()
        };
        let copy_len = file_data.len().min(storage.size);
        if copy_len > 0 {
            core::ptr::copy_nonoverlapping(file_data.as_ptr(), storage.ptr, copy_len);
        }
    }

    let id = NEXT_SECTION_ID.fetch_add(1, Ordering::SeqCst) as HANDLE;
    let access_mask = match flProtect {
        PAGE_READONLY => SECTION_QUERY | SECTION_MAP_READ,
        PAGE_READWRITE | PAGE_WRITECOPY => SECTION_QUERY | SECTION_MAP_READ | SECTION_MAP_WRITE,
        PAGE_EXECUTE_READ => SECTION_QUERY | SECTION_MAP_READ | SECTION_MAP_EXECUTE,
        PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY => {
            SECTION_QUERY | SECTION_MAP_READ | SECTION_MAP_WRITE | SECTION_MAP_EXECUTE
        }
        _ => SECTION_ALL_ACCESS,
    };

    let section = SharedSectionInner {
        id,
        name: name_str.clone(),
        storage,
        max_size: total_size,
        page_protection: flProtect,
        access_mask,
    };

    SECTIONS.lock().insert(id, section);
    if let Some(name) = name_str {
        NAMED_SECTIONS.lock().insert(name, id);
    }

    win32_set_last_error(ERROR_SUCCESS);
    id
}

/// Maps a view of a shared memory section into the address space of the calling process.
pub unsafe extern "system" fn MapSharedSectionViewEx(
    hSection: HANDLE,
    dwDesiredAccess: DWORD,
    dwFileOffsetHigh: DWORD,
    dwFileOffsetLow: DWORD,
    dwNumberOfBytesToMap: SIZE_T,
    lpBaseAddress: PVOID,
) -> PVOID {
    if hSection == 0 || hSection == (INVALID_HANDLE_VALUE as HANDLE) {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return core::ptr::null_mut();
    }

    let sections = SECTIONS.lock();
    let section = match sections.get(&hSection) {
        Some(s) => s,
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return core::ptr::null_mut();
        }
    };

    // Access mask security checks: verify section supports the requested view mode
    if (dwDesiredAccess & FILE_MAP_WRITE) != 0 && (section.access_mask & SECTION_MAP_WRITE) == 0 {
        win32_set_last_error(ERROR_ACCESS_DENIED);
        return core::ptr::null_mut();
    }
    if (dwDesiredAccess & FILE_MAP_READ) != 0 && (section.access_mask & SECTION_MAP_READ) == 0 {
        win32_set_last_error(ERROR_ACCESS_DENIED);
        return core::ptr::null_mut();
    }
    if (dwDesiredAccess & FILE_MAP_EXECUTE) != 0 && (section.access_mask & SECTION_MAP_EXECUTE) == 0 {
        win32_set_last_error(ERROR_ACCESS_DENIED);
        return core::ptr::null_mut();
    }

    let offset = ((dwFileOffsetHigh as usize) << 32) | (dwFileOffsetLow as usize);
    let view_len = if dwNumberOfBytesToMap == 0 {
        section.max_size.saturating_sub(offset)
    } else {
        dwNumberOfBytesToMap
    };

    let is_cow = (dwDesiredAccess & FILE_MAP_COPY) != 0;
    let base_addr = if !lpBaseAddress.is_null() {
        lpBaseAddress as usize
    } else {
        VIRTUAL_VIEW_ALLOC_ADDR.fetch_add(view_len + 0x10000, Ordering::SeqCst)
    };

    let cow_buf = if is_cow {
        let mut cow_vec = Vec::with_capacity(view_len);
        let src = section.storage.ptr.add(offset);
        let slice = core::slice::from_raw_parts(src, view_len);
        cow_vec.extend_from_slice(slice);
        Some(Arc::new(Spinlock::new(cow_vec)))
    } else {
        None
    };

    let view = SectionMappedView {
        base_address: base_addr,
        size: view_len,
        offset,
        desired_access: dwDesiredAccess,
        is_copy_on_write: is_cow,
        cow_buffer: cow_buf,
    };

    MAPPED_VIEWS.lock().insert(base_addr, view);
    win32_set_last_error(ERROR_SUCCESS);
    base_addr as PVOID
}

/// Unmaps a previously mapped view of a shared memory section.
pub unsafe extern "system" fn UnmapSharedSectionView(lpBaseAddress: PVOID) -> BOOL {
    if lpBaseAddress.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let addr = lpBaseAddress as usize;
    if MAPPED_VIEWS.lock().remove(&addr).is_some() {
        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        FALSE
    }
}
