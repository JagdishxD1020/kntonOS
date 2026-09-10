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
