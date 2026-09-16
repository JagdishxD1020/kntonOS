//! Win32 File I/O & VFS DOS Path Translation Subsystem.

#![allow(non_snake_case, non_camel_case_types)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;

use crate::pe::kernel32::INVALID_HANDLE_VALUE;
type HANDLE = u64;

pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const GENERIC_EXECUTE: u32 = 0x2000_0000;
pub const GENERIC_ALL: u32 = 0x1000_0000;

pub const FILE_SHARE_READ: u32 = 0x0000_0001;
pub const FILE_SHARE_WRITE: u32 = 0x0000_0002;
pub const FILE_SHARE_DELETE: u32 = 0x0000_0004;

pub const CREATE_NEW: u32 = 1;
pub const CREATE_ALWAYS: u32 = 2;
pub const OPEN_EXISTING: u32 = 3;
pub const OPEN_ALWAYS: u32 = 4;
pub const TRUNCATE_EXISTING: u32 = 5;

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x0000_0020;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

pub const FILE_BEGIN: u32 = 0;
pub const FILE_CURRENT: u32 = 1;
pub const FILE_END: u32 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FILETIME {
    pub dwLowDateTime: u32,
    pub dwHighDateTime: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct BY_HANDLE_FILE_INFORMATION {
    pub dwFileAttributes: u32,
    pub ftCreationTime: FILETIME,
    pub ftLastAccessTime: FILETIME,
    pub ftLastWriteTime: FILETIME,
    pub dwVolumeSerialNumber: u32,
    pub nFileSizeHigh: u32,
    pub nFileSizeLow: u32,
    pub nNumberOfLinks: u32,
    pub nFileIndexHigh: u32,
    pub nFileIndexLow: u32,
}

/// Translates a DOS/Windows-style file path into a canonical KntonOS VFS path.
pub fn win32_dos_to_vfs_path(dos_path: &str) -> String {
    let trimmed = dos_path.trim();
    if trimmed.is_empty() {
        return String::from("/");
    }

    if trimmed == "CONOUT$" || trimmed == "CONIN$" {
        return String::from(trimmed);
    }

    // 1. Device namespace: \\.\ or //./
    if trimmed.starts_with(r"\\.\") || trimmed.starts_with("//./") {
        let dev_name = &trimmed[4..];
        let mut path = String::from("/dev/");
        for ch in dev_name.chars() {
            if ch == '\\' {
                path.push('/');
            } else {
                path.push(ch);
            }
        }
        return path;
    }

    // Extended-length namespace: \\?\ or //?/
    let trimmed = if trimmed.starts_with(r"\\?\UNC\") || trimmed.starts_with("//?/UNC/") {
        let unc_body = &trimmed[8..];
        return canonicalize_vfs_path(&alloc::format!("/vfs/network/{}", unc_body.replace('\\', "/")));
    } else if trimmed.starts_with(r"\\?\") || trimmed.starts_with("//?/") {
        &trimmed[4..]
    } else {
        trimmed
    };

    // 2. UNC path: \\server\share\...
    if trimmed.starts_with(r"\\") || trimmed.starts_with("//") {
        let unc_body = &trimmed[2..];
        let mut path = String::from("/vfs/network/");
        for ch in unc_body.chars() {
            if ch == '\\' {
                path.push('/');
            } else {
                path.push(ch);
            }
        }
        return canonicalize_vfs_path(&path);
    }

    // 3. Drive letter: C:\... or C:/...
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && (bytes[0].is_ascii_alphabetic()) {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = if bytes.len() >= 3 && (bytes[2] == b'\\' || bytes[2] == b'/') {
            &trimmed[3..]
        } else {
            &trimmed[2..]
        };

        let mut path = alloc::format!("/vfs/drives/{}/", drive);
        for ch in rest.chars() {
            if ch == '\\' {
                path.push('/');
            } else {
                path.push(ch);
            }
        }
        return canonicalize_vfs_path(&path);
    }

    // 4. Absolute VFS path or relative path
    let mut path = String::new();
    if !trimmed.starts_with('/') && !trimmed.starts_with('\\') {
        path.push_str("/vfs/drives/c/");
    } else {
        path.push('/');
    }

    for ch in trimmed.chars() {
        if ch == '\\' {
            path.push('/');
        } else {
            path.push(ch);
        }
    }

    canonicalize_vfs_path(&path)
}

/// Normalizes path segments removing . and .. relative navigations.
pub fn canonicalize_vfs_path(vfs_path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for seg in vfs_path.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        } else if seg == ".." {
            segments.pop();
        } else {
            segments.push(seg);
        }
    }

    let mut result = String::from("/");
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            result.push('/');
        }
        result.push_str(seg);
    }
    result
}

/// Checks whether a given DOS path is relative.
pub fn win32_path_is_relative(path: &str) -> bool {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return false;
    }
    if path.starts_with(r"\\") || path.starts_with("//") || path.starts_with('/') || path.starts_with('\\') {
        return false;
    }
    true
}

#[derive(Debug, Clone)]
pub struct Win32FileObject {
    pub handle: HANDLE,
    pub vfs_path: String,
    pub desired_access: u32,
    pub share_mode: u32,
    pub creation_disposition: u32,
    pub flags_and_attributes: u32,
    pub cursor: u64,
    pub is_console: bool,
    pub is_directory: bool,
}

static NEXT_FILE_HANDLE: Spinlock<HANDLE> = Spinlock::new(0x000B_0001);
static FILE_HANDLE_TABLE: Spinlock<BTreeMap<HANDLE, Win32FileObject>> = Spinlock::new(BTreeMap::new());

/// Emulates Win32 CreateFile creating or opening a file or device.
pub unsafe fn win32_create_file(
    lp_file_name: &str,
    dw_desired_access: u32,
    dw_share_mode: u32,
    dw_creation_disposition: u32,
    dw_flags_and_attributes: u32,
) -> HANDLE {
    if lp_file_name.is_empty() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return INVALID_HANDLE_VALUE;
    }

    if lp_file_name.starts_with(r#"\\.\pipe\"#) || lp_file_name.starts_with("//./pipe/") {
        return crate::pe::pipe::win32_connect_client_pipe(lp_file_name, dw_desired_access);
    }

    let is_console = lp_file_name == "CONOUT$" || lp_file_name == "CONIN$";
    let vfs_path = win32_dos_to_vfs_path(lp_file_name);

    if !is_console {
        let exists = {
            let vfs = crate::fs::VFS.lock();
            vfs.read_file(&vfs_path).is_ok() || vfs.open(&vfs_path).is_ok()
        };

        match dw_creation_disposition {
            CREATE_NEW => {
                if exists {
                    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_ALREADY_EXISTS);
                    return INVALID_HANDLE_VALUE;
                }
                let mut vfs = crate::fs::VFS.lock();
                vfs.write_file(&vfs_path, &[]);
            }
            CREATE_ALWAYS => {
                let mut vfs = crate::fs::VFS.lock();
                vfs.write_file(&vfs_path, &[]);
            }
            OPEN_EXISTING => {
                if !exists {
                    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_FILE_NOT_FOUND);
                    return INVALID_HANDLE_VALUE;
                }
            }
            OPEN_ALWAYS => {
                if !exists {
                    let mut vfs = crate::fs::VFS.lock();
                    vfs.write_file(&vfs_path, &[]);
                }
            }
            TRUNCATE_EXISTING => {
                if !exists {
                    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_FILE_NOT_FOUND);
                    return INVALID_HANDLE_VALUE;
                }
                let mut vfs = crate::fs::VFS.lock();
                vfs.write_file(&vfs_path, &[]);
            }
            _ => {
                crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
                return INVALID_HANDLE_VALUE;
            }
        }
    }

    let mut gen = NEXT_FILE_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let file_obj = Win32FileObject {
        handle,
        vfs_path,
        desired_access: dw_desired_access,
        share_mode: dw_share_mode,
        creation_disposition: dw_creation_disposition,
        flags_and_attributes: dw_flags_and_attributes,
        cursor: 0,
        is_console,
        is_directory: (dw_flags_and_attributes & FILE_ATTRIBUTE_DIRECTORY) != 0,
    };

    let mut table = FILE_HANDLE_TABLE.lock();
    table.insert(handle, file_obj);

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    handle
}

/// Emulates Win32 CloseHandle closing an open object handle.
pub unsafe fn win32_close_handle(h_object: HANDLE) -> bool {
    if h_object == INVALID_HANDLE_VALUE || h_object == 0 {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    let mut table = FILE_HANDLE_TABLE.lock();
    if table.remove(&h_object).is_some() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        true
    } else {
        drop(table);
        if crate::pe::pipe::is_pipe_handle(h_object) {
            return crate::pe::pipe::win32_close_pipe(h_object);
        }
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        false
    }
}

/// Emulates Win32 ReadFile reading bytes from an open file or device.
pub unsafe fn win32_read_file(
    h_file: HANDLE,
    lp_buffer: *mut u8,
    n_number_of_bytes_to_read: u32,
    lp_number_of_bytes_read: *mut u32,
) -> bool {
    if h_file == INVALID_HANDLE_VALUE || h_file == 0 || lp_buffer.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    let mut table = FILE_HANDLE_TABLE.lock();
    let file_obj = match table.get_mut(&h_file) {
        Some(f) => f,
        None => {
            drop(table);
            if crate::pe::pipe::is_pipe_handle(h_file) {
                return crate::pe::pipe::win32_read_pipe(
                    h_file,
                    lp_buffer,
                    n_number_of_bytes_to_read,
                    lp_number_of_bytes_read,
                );
            }
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
            return false;
        }
    };

    if file_obj.is_console {
        if !lp_number_of_bytes_read.is_null() {
            *lp_number_of_bytes_read = 0;
        }
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        return true;
    }

    let vfs = crate::fs::VFS.lock();
    let data = match vfs.read_file(&file_obj.vfs_path) {
        Ok(d) => d,
        Err(_) => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_FILE_NOT_FOUND);
            return false;
        }
    };

    let start = file_obj.cursor as usize;
    let available = if start < data.len() { data.len() - start } else { 0 };
    let to_read = (n_number_of_bytes_to_read as usize).min(available);

    if to_read > 0 {
        core::ptr::copy_nonoverlapping(data[start..start + to_read].as_ptr(), lp_buffer, to_read);
        file_obj.cursor += to_read as u64;
    }

    if !lp_number_of_bytes_read.is_null() {
        *lp_number_of_bytes_read = to_read as u32;
    }

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Emulates Win32 WriteFile writing bytes to an open file or device.
pub unsafe fn win32_write_file(
    h_file: HANDLE,
    lp_buffer: *const u8,
    n_number_of_bytes_to_write: u32,
    lp_number_of_bytes_written: *mut u32,
) -> bool {
    if h_file == INVALID_HANDLE_VALUE || h_file == 0 || lp_buffer.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    let mut table = FILE_HANDLE_TABLE.lock();
    let file_obj = match table.get_mut(&h_file) {
        Some(f) => f,
        None => {
            drop(table);
            if crate::pe::pipe::is_pipe_handle(h_file) {
                return crate::pe::pipe::win32_write_pipe(
                    h_file,
                    lp_buffer,
                    n_number_of_bytes_to_write,
                    lp_number_of_bytes_written,
                );
            }
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
            return false;
        }
    };

    let bytes_to_write = n_number_of_bytes_to_write as usize;
    let slice = core::slice::from_raw_parts(lp_buffer, bytes_to_write);

    if file_obj.is_console {
        if let Ok(s) = core::str::from_utf8(slice) {
            crate::serial_write_str(s);
        }
        if !lp_number_of_bytes_written.is_null() {
            *lp_number_of_bytes_written = n_number_of_bytes_to_write;
        }
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        return true;
    }

    let mut vfs = crate::fs::VFS.lock();
    let mut data = vfs.read_file(&file_obj.vfs_path).unwrap_or_default();

    let start = file_obj.cursor as usize;
    if start + bytes_to_write > data.len() {
        data.resize(start + bytes_to_write, 0);
    }
    data[start..start + bytes_to_write].copy_from_slice(slice);

    vfs.write_file(&file_obj.vfs_path, &data);
    file_obj.cursor += bytes_to_write as u64;

    if !lp_number_of_bytes_written.is_null() {
        *lp_number_of_bytes_written = n_number_of_bytes_to_write;
    }

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Emulates Win32 SetFilePointerEx moving the file pointer of an open file.
pub unsafe fn win32_set_file_pointer_ex(
    h_file: u64,
    li_distance_to_move: i64,
    lp_new_file_pointer: *mut i64,
    dw_move_method: u32,
) -> bool {
    if h_file == INVALID_HANDLE_VALUE || h_file == 0 {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    let mut table = FILE_HANDLE_TABLE.lock();
    let file_obj = match table.get_mut(&h_file) {
        Some(obj) => obj,
        None => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
            return false;
        }
    };

    if file_obj.is_console {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    let file_len = {
        let vfs = crate::fs::VFS.lock();
        vfs.read_file(&file_obj.vfs_path).map(|d| d.len()).unwrap_or(0) as i64
    };

    let target_offset = match dw_move_method {
        FILE_BEGIN => li_distance_to_move,
        FILE_CURRENT => (file_obj.cursor as i64).saturating_add(li_distance_to_move),
        FILE_END => file_len.saturating_add(li_distance_to_move),
        _ => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
            return false;
        }
    };

    if target_offset < 0 {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    file_obj.cursor = target_offset as u64;

    if !lp_new_file_pointer.is_null() {
        *lp_new_file_pointer = target_offset;
    }

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Emulates Win32 GetFileSizeEx retrieving the size of the specified file.
pub unsafe fn win32_get_file_size_ex(h_file: u64, lp_file_size: *mut i64) -> bool {
    if h_file == INVALID_HANDLE_VALUE || h_file == 0 || lp_file_size.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    let table = FILE_HANDLE_TABLE.lock();
    let file_obj = match table.get(&h_file) {
        Some(obj) => obj,
        None => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
            return false;
        }
    };

    if file_obj.is_console {
        *lp_file_size = 0;
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        return true;
    }

    let vfs = crate::fs::VFS.lock();
    let size = vfs.read_file(&file_obj.vfs_path).map(|d| d.len()).unwrap_or(0) as i64;
    *lp_file_size = size;

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Emulates Win32 GetFileInformationByHandle retrieving file geometry and attributes.
pub unsafe fn win32_get_file_information_by_handle(
    h_file: u64,
    lp_file_information: *mut BY_HANDLE_FILE_INFORMATION,
) -> bool {
    if h_file == INVALID_HANDLE_VALUE || h_file == 0 || lp_file_information.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    let table = FILE_HANDLE_TABLE.lock();
    let file_obj = match table.get(&h_file) {
        Some(obj) => obj,
        None => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
            return false;
        }
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if file_obj.is_console {
        info.dwFileAttributes = FILE_ATTRIBUTE_NORMAL;
    } else {
        let vfs = crate::fs::VFS.lock();
        let size = vfs.read_file(&file_obj.vfs_path).map(|d| d.len()).unwrap_or(0) as u64;
        info.dwFileAttributes = file_obj.flags_and_attributes;
        info.nFileSizeLow = (size & 0xFFFF_FFFF) as u32;
        info.nFileSizeHigh = (size >> 32) as u32;
        info.nNumberOfLinks = 1;
        info.dwVolumeSerialNumber = 0x1A2B_3C4D;
        info.nFileIndexLow = file_obj.handle as u32;
        info.nFileIndexHigh = (file_obj.handle >> 32) as u32;
        info.ftCreationTime = FILETIME { dwLowDateTime: 0x1234_5678, dwHighDateTime: 0x01D9_0000 };
        info.ftLastAccessTime = FILETIME { dwLowDateTime: 0x1234_5678, dwHighDateTime: 0x01D9_0000 };
        info.ftLastWriteTime = FILETIME { dwLowDateTime: 0x1234_5678, dwHighDateTime: 0x01D9_0000 };
    }

    *lp_file_information = info;
    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Emulates Win32 FlushFileBuffers flushing written buffers to disk.
pub unsafe fn win32_flush_file_buffers(h_file: u64) -> bool {
    if h_file == INVALID_HANDLE_VALUE || h_file == 0 {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    let table = FILE_HANDLE_TABLE.lock();
    if !table.contains_key(&h_file) {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}


pub const ERROR_NO_MORE_FILES: u32 = 18;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WIN32_FIND_DATAW {
    pub dwFileAttributes: u32,
    pub ftCreationTime: FILETIME,
    pub ftLastAccessTime: FILETIME,
    pub ftLastWriteTime: FILETIME,
    pub nFileSizeHigh: u32,
    pub nFileSizeLow: u32,
    pub dwReserved0: u32,
    pub dwReserved1: u32,
    pub cFileName: [u16; 260],
    pub cAlternateFileName: [u16; 14],
}

impl Default for WIN32_FIND_DATAW {
    fn default() -> Self {
        Self {
            dwFileAttributes: 0,
            ftCreationTime: FILETIME::default(),
            ftLastAccessTime: FILETIME::default(),
            ftLastWriteTime: FILETIME::default(),
            nFileSizeHigh: 0,
            nFileSizeLow: 0,
            dwReserved0: 0,
            dwReserved1: 0,
            cFileName: [0; 260],
            cAlternateFileName: [0; 14],
        }
    }
}

#[derive(Debug, Clone)]
pub struct FindEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct FindFileContext {
    pub entries: Vec<FindEntry>,
    pub current_idx: usize,
}

static NEXT_FIND_HANDLE: Spinlock<u64> = Spinlock::new(0x000F_0001);
static FIND_HANDLE_TABLE: Spinlock<BTreeMap<u64, FindFileContext>> = Spinlock::new(BTreeMap::new());

fn matches_pattern(name: &str, pattern: &str) -> bool {
    if pattern == "*" || pattern == "*.*" || pattern.is_empty() {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        if let Some(rest) = suffix.strip_prefix('.') {
            return name.to_ascii_lowercase().ends_with(&alloc::format!(".{}", rest.to_ascii_lowercase()));
        }
        return name.to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase());
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase());
    }
    name.eq_ignore_ascii_case(pattern)
}

fn copy_find_entry_to_dataw(entry: &FindEntry, data: &mut WIN32_FIND_DATAW) {
    *data = WIN32_FIND_DATAW::default();
    data.dwFileAttributes = if entry.is_dir {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    };
    data.nFileSizeLow = (entry.size & 0xFFFF_FFFF) as u32;
    data.nFileSizeHigh = (entry.size >> 32) as u32;
    data.ftCreationTime = FILETIME { dwLowDateTime: 0x1234_5678, dwHighDateTime: 0x01D9_0000 };
    data.ftLastAccessTime = FILETIME { dwLowDateTime: 0x1234_5678, dwHighDateTime: 0x01D9_0000 };
    data.ftLastWriteTime = FILETIME { dwLowDateTime: 0x1234_5678, dwHighDateTime: 0x01D9_0000 };

    let mut i = 0;
    for c in entry.name.encode_utf16() {
        if i >= 259 {
            break;
        }
        data.cFileName[i] = c;
        i += 1;
    }
    data.cFileName[i] = 0;
}

/// Emulates Win32 FindFirstFile searching a directory for a file or subdirectory.
pub unsafe fn win32_find_first_file(
    lp_file_name: &str,
    lp_find_file_data: *mut WIN32_FIND_DATAW,
) -> u64 {
    if lp_file_name.is_empty() || lp_find_file_data.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return INVALID_HANDLE_VALUE;
    }

    let norm = lp_file_name.replace('\\', "/");
    let (dir_part, pattern) = match norm.rsplit_once('/') {
        Some((d, p)) => (if d.is_empty() { "/" } else { d }, p),
        None => (".", norm.as_str()),
    };

    let vfs_dir = win32_dos_to_vfs_path(dir_part);
    let vfs = crate::fs::VFS.lock();

    let dir_entries = match vfs.list_directory(&vfs_dir) {
        Ok(entries) => entries,
        Err(_) => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_FILE_NOT_FOUND);
            return INVALID_HANDLE_VALUE;
        }
    };

    let mut matched_entries = Vec::new();
    for entry in dir_entries {
        if matches_pattern(&entry.name, pattern) {
            let full_vfs = if vfs_dir.ends_with('/') {
                alloc::format!("{}{}", vfs_dir, entry.name)
            } else {
                alloc::format!("{}/{}", vfs_dir, entry.name)
            };
            let size = vfs.read_file(&full_vfs).map(|d| d.len() as u64).unwrap_or(0);
            matched_entries.push(FindEntry {
                name: entry.name,
                is_dir: entry.file_type == crate::fs::FileType::Directory,
                size,
            });
        }
    }

    if matched_entries.is_empty() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_FILE_NOT_FOUND);
        return INVALID_HANDLE_VALUE;
    }

    copy_find_entry_to_dataw(&matched_entries[0], &mut *lp_find_file_data);

    let mut gen = NEXT_FIND_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let ctx = FindFileContext {
        entries: matched_entries,
        current_idx: 1,
    };

    let mut table = FIND_HANDLE_TABLE.lock();
    table.insert(handle, ctx);

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    handle
}

/// Emulates Win32 FindNextFile continuing a file search from a previous FindFirstFile call.
pub unsafe fn win32_find_next_file(
    h_find_file: u64,
    lp_find_file_data: *mut WIN32_FIND_DATAW,
) -> bool {
    if h_find_file == INVALID_HANDLE_VALUE || h_find_file == 0 || lp_find_file_data.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    let mut table = FIND_HANDLE_TABLE.lock();
    let ctx = match table.get_mut(&h_find_file) {
        Some(c) => c,
        None => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
            return false;
        }
    };

    if ctx.current_idx >= ctx.entries.len() {
        crate::pe::kernel32::win32_set_last_error(ERROR_NO_MORE_FILES);
        return false;
    }

    copy_find_entry_to_dataw(&ctx.entries[ctx.current_idx], &mut *lp_find_file_data);
    ctx.current_idx += 1;

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Emulates Win32 FindClose closing a search handle.
pub unsafe fn win32_find_close(h_find_file: u64) -> bool {
    if h_find_file == INVALID_HANDLE_VALUE || h_find_file == 0 {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    let mut table = FIND_HANDLE_TABLE.lock();
    if table.remove(&h_find_file).is_some() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        true
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        false
    }
}


/// Comprehensive self-test suite for Win32 File I/O & DOS path translation subsystem.
pub fn test_win32_file_io_subsystem() -> bool {
    // 1. Path translation & DOS backward-slash normalization
    if win32_dos_to_vfs_path(r"C:\Games\Doom\doom.exe") != "/vfs/drives/c/Games/Doom/doom.exe" {
        return false;
    }
    if win32_dos_to_vfs_path("D:/Data/save.dat") != "/vfs/drives/d/Data/save.dat" {
        return false;
    }
    if win32_dos_to_vfs_path(r"\\?\C:\Games\Doom") != "/vfs/drives/c/Games/Doom" {
        return false;
    }
    if win32_dos_to_vfs_path(r"\\.\PhysicalDrive0") != "/dev/PhysicalDrive0" {
        return false;
    }
    if win32_dos_to_vfs_path(r"\\server\share\file.txt") != "/vfs/network/server/share/file.txt" {
        return false;
    }
    if !win32_path_is_relative(r"foo\bar.txt") || win32_path_is_relative(r"C:\foo\bar.txt") {
        return false;
    }

    unsafe {
        // 2. File creation, writing, geometry, and buffers
        let file_path = r"C:\knton_test\payload.bin";
        let h_file = win32_create_file(
            file_path,
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
        );
        if h_file == INVALID_HANDLE_VALUE || h_file == 0 {
            return false;
        }

        let test_payload = b"KntonOS Win32 File I/O Engine Verification Payload 2026!";
        let mut bytes_written = 0u32;
        let w_ok = win32_write_file(
            h_file,
            test_payload.as_ptr(),
            test_payload.len() as u32,
            &mut bytes_written as *mut u32,
        );
        if !w_ok || bytes_written != test_payload.len() as u32 {
            return false;
        }

        if !win32_flush_file_buffers(h_file) {
            return false;
        }

        let mut f_size = 0i64;
        if !win32_get_file_size_ex(h_file, &mut f_size as *mut i64) || f_size != test_payload.len() as i64 {
            return false;
        }

        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if !win32_get_file_information_by_handle(h_file, &mut info as *mut BY_HANDLE_FILE_INFORMATION) {
            return false;
        }
        if info.nFileSizeLow != test_payload.len() as u32 {
            return false;
        }

        if !win32_close_handle(h_file) {
            return false;
        }

        // 3. Re-open, Seeking, and Reading
        let h_read = win32_create_file(
            file_path,
            GENERIC_READ,
            FILE_SHARE_READ,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
        );
        if h_read == INVALID_HANDLE_VALUE {
            return false;
        }

        let mut new_ptr = 0i64;
        let seek_ok = win32_set_file_pointer_ex(h_read, 8, &mut new_ptr as *mut i64, FILE_BEGIN);
        if !seek_ok || new_ptr != 8 {
            return false;
        }

        let mut read_buf = [0u8; 5];
        let mut bytes_read = 0u32;
        let r_ok = win32_read_file(
            h_read,
            read_buf.as_mut_ptr(),
            5,
            &mut bytes_read as *mut u32,
        );
        if !r_ok || bytes_read != 5 || &read_buf != b"Win32" {
            return false;
        }

        let seek_begin = win32_set_file_pointer_ex(h_read, 0, &mut new_ptr as *mut i64, FILE_BEGIN);
        if !seek_begin || new_ptr != 0 {
            return false;
        }

        let mut full_buf = [0u8; 64];
        let r_full = win32_read_file(
            h_read,
            full_buf.as_mut_ptr(),
            test_payload.len() as u32,
            &mut bytes_read as *mut u32,
        );
        if !r_full || bytes_read != test_payload.len() as u32 || &full_buf[..bytes_read as usize] != test_payload {
            return false;
        }

        if !win32_close_handle(h_read) {
            return false;
        }

        // 4. Directory enumeration & FindFirstFile iterator
        let file_path2 = r"C:\knton_test\another.txt";
        let h_f2 = win32_create_file(
            file_path2,
            GENERIC_WRITE,
            0,
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
        );
        if h_f2 == INVALID_HANDLE_VALUE {
            return false;
        }
        win32_close_handle(h_f2);

        let mut find_data = WIN32_FIND_DATAW::default();
        let h_find = win32_find_first_file(r"C:\knton_test\*.*", &mut find_data as *mut WIN32_FIND_DATAW);
        if h_find == INVALID_HANDLE_VALUE {
            return false;
        }

        let mut found_count = 0;
        let mut has_payload = false;
        let mut has_another = false;

        let mut check_entry = |d: &WIN32_FIND_DATAW| {
            found_count += 1;
            let len = d.cFileName.iter().position(|&c| c == 0).unwrap_or(d.cFileName.len());
            let name_chars: alloc::vec::Vec<char> = d.cFileName[..len].iter().map(|&c| c as u8 as char).collect();
            let name: alloc::string::String = name_chars.into_iter().collect();
            if name == "payload.bin" {
                has_payload = true;
            } else if name == "another.txt" {
                has_another = true;
            }
        };

        check_entry(&find_data);

        while win32_find_next_file(h_find, &mut find_data as *mut WIN32_FIND_DATAW) {
            check_entry(&find_data);
        }

        if !win32_find_close(h_find) {
            return false;
        }

        if found_count < 2 || !has_payload || !has_another {
            return false;
        }
    }

    crate::serial_write_str("[KNT-PE-FILE-IO-PASS] Win32 file I/O and VFS path translation verified\r\n");
    true
}
