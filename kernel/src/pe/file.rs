//! Win32 File I/O & VFS DOS Path Translation Subsystem.

#![allow(non_snake_case, non_camel_case_types)]

use alloc::string::String;
use alloc::vec::Vec;

pub use crate::pe::synch::{HANDLE, INVALID_HANDLE_VALUE};

/// Translates a DOS/Windows-style file path into a canonical KntonOS VFS path.
pub fn win32_dos_to_vfs_path(dos_path: &str) -> String {
    let trimmed = dos_path.trim();
    if trimmed.is_empty() {
        return String::from("/");
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
