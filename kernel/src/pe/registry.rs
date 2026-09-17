//! Win32 Windows Registry Subsystem & Game Configuration Hive.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;

pub type HKEY = usize;

// Predefined root registry keys
pub const HKEY_CLASSES_ROOT: HKEY = 0x8000_0000;
pub const HKEY_CURRENT_USER: HKEY = 0x8000_0001;
pub const HKEY_LOCAL_MACHINE: HKEY = 0x8000_0002;
pub const HKEY_USERS: HKEY = 0x8000_0003;
pub const HKEY_PERFORMANCE_DATA: HKEY = 0x8000_0004;
pub const HKEY_CURRENT_CONFIG: HKEY = 0x8000_0005;

// Registry value data types
pub const REG_NONE: u32 = 0;
pub const REG_SZ: u32 = 1;
pub const REG_EXPAND_SZ: u32 = 2;
pub const REG_BINARY: u32 = 3;
pub const REG_DWORD: u32 = 4;
pub const REG_DWORD_LITTLE_ENDIAN: u32 = 4;
pub const REG_DWORD_BIG_ENDIAN: u32 = 5;
pub const REG_LINK: u32 = 6;
pub const REG_MULTI_SZ: u32 = 7;
pub const REG_QWORD: u32 = 11;
pub const REG_QWORD_LITTLE_ENDIAN: u32 = 11;

// Win32 registry status codes
#[allow(unused_imports)]
use crate::pe::kernel32::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER,
    ERROR_SUCCESS,
};
#[allow(unused_imports)]
use crate::pe::pipe::ERROR_MORE_DATA;
pub const ERROR_NO_MORE_ITEMS: u32 = 259;

// Registry access masks & disposition
pub const KEY_QUERY_VALUE: u32 = 0x0001;
pub const KEY_SET_VALUE: u32 = 0x0002;
pub const KEY_CREATE_SUB_KEY: u32 = 0x0004;
pub const KEY_ENUMERATE_SUB_KEYS: u32 = 0x0008;
pub const KEY_READ: u32 = 0x20019;
pub const KEY_WRITE: u32 = 0x20006;
pub const KEY_ALL_ACCESS: u32 = 0xF003F;

pub const REG_CREATED_NEW_KEY: u32 = 0x00000001;
pub const REG_OPENED_EXISTING_KEY: u32 = 0x00000002;

/// In-memory representation of a registry value entry.
#[derive(Debug, Clone)]
pub struct RegistryValue {
    pub name: String,
    pub val_type: u32,
    pub data: Vec<u8>,
}

/// Node in the hierarchical registry key tree.
#[derive(Debug, Clone)]
pub struct RegistryKeyNode {
    pub name: String,
    pub subkeys: BTreeMap<String, RegistryKeyNode>,
    pub values: BTreeMap<String, RegistryValue>,
}

impl RegistryKeyNode {
    /// Creates a new empty registry key node.
    pub fn new(name: &str) -> Self {
        Self {
            name: String::from(name),
            subkeys: BTreeMap::new(),
            values: BTreeMap::new(),
        }
    }

    /// Finds an existing subkey matching case-insensitively.
    pub fn find_subkey(&self, name: &str) -> Option<&RegistryKeyNode> {
        self.subkeys.get(&name.to_lowercase())
    }

    /// Finds a mutable subkey matching case-insensitively.
    pub fn find_subkey_mut(&mut self, name: &str) -> Option<&mut RegistryKeyNode> {
        self.subkeys.get_mut(&name.to_lowercase())
    }

    /// Returns or creates a subkey by case-insensitive name.
    pub fn get_or_create_subkey(&mut self, name: &str) -> &mut RegistryKeyNode {
        let key = name.to_lowercase();
        self.subkeys.entry(key).or_insert_with(|| RegistryKeyNode::new(name))
    }

    /// Stores or overwrites a named value.
    pub fn set_value(&mut self, name: &str, val_type: u32, data: &[u8]) {
        let key = name.to_lowercase();
        let val = RegistryValue {
            name: String::from(name),
            val_type,
            data: data.to_vec(),
        };
        self.values.insert(key, val);
    }

    /// Retrieves a value by case-insensitive name.
    pub fn get_value(&self, name: &str) -> Option<&RegistryValue> {
        self.values.get(&name.to_lowercase())
    }

    /// Deletes a value by case-insensitive name.
    pub fn delete_value(&mut self, name: &str) -> bool {
        self.values.remove(&name.to_lowercase()).is_some()
    }

    /// Deletes a subkey by case-insensitive name.
    pub fn delete_subkey(&mut self, name: &str) -> bool {
        self.subkeys.remove(&name.to_lowercase()).is_some()
    }

    /// Resolves a nested subkey path.
    pub fn resolve_path(&self, parts: &[&str]) -> Option<&RegistryKeyNode> {
        let mut curr = self;
        for part in parts {
            if part.is_empty() {
                continue;
            }
            curr = curr.find_subkey(part)?;
        }
        Some(curr)
    }

    /// Resolves a mutable nested subkey path.
    pub fn resolve_path_mut(&mut self, parts: &[&str]) -> Option<&mut RegistryKeyNode> {
        let mut curr = self;
        for part in parts {
            if part.is_empty() {
                continue;
            }
            curr = curr.find_subkey_mut(part)?;
        }
        Some(curr)
    }

    /// Traverses and creates all intermediate subkeys along the path.
    pub fn create_path(&mut self, parts: &[&str]) -> &mut RegistryKeyNode {
        let mut curr = self;
        for part in parts {
            if part.is_empty() {
                continue;
            }
            curr = curr.get_or_create_subkey(part);
        }
        curr
    }
}

/// Metadata describing an open dynamic registry key handle.
#[derive(Debug, Clone)]
pub struct OpenKeyHandle {
    pub root_hkey: HKEY,
    pub path: Vec<String>,
}

#[allow(dead_code)]
static REGISTRY_HIVES: Spinlock<BTreeMap<HKEY, RegistryKeyNode>> = Spinlock::new(BTreeMap::new());
#[allow(dead_code)]
static OPEN_KEYS: Spinlock<BTreeMap<HKEY, OpenKeyHandle>> = Spinlock::new(BTreeMap::new());
#[allow(dead_code)]
static NEXT_HKEY: Spinlock<HKEY> = Spinlock::new(0x000F_0001);

static REGISTRY_INITIALIZED: Spinlock<bool> = Spinlock::new(false);

/// Populates default system and gaming configuration hives.
pub fn ensure_registry_initialized() {
    let mut init = REGISTRY_INITIALIZED.lock();
    if *init {
        return;
    }

    let mut hives = REGISTRY_HIVES.lock();
    hives.insert(HKEY_CLASSES_ROOT, RegistryKeyNode::new("HKEY_CLASSES_ROOT"));
    hives.insert(HKEY_CURRENT_USER, RegistryKeyNode::new("HKEY_CURRENT_USER"));
    hives.insert(HKEY_USERS, RegistryKeyNode::new("HKEY_USERS"));
    hives.insert(HKEY_CURRENT_CONFIG, RegistryKeyNode::new("HKEY_CURRENT_CONFIG"));

    let mut hklm = RegistryKeyNode::new("HKEY_LOCAL_MACHINE");

    // 1. DirectX gaming installation keys
    let directx = hklm.create_path(&["SOFTWARE", "Microsoft", "DirectX"]);
    directx.set_value("Version", REG_SZ, b"4.09.00.0904\0");
    directx.set_value(
        "InstalledVersion",
        REG_BINARY,
        &[0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x04, 0x09],
    );

    // 2. Windows NT version strings
    let winnt = hklm.create_path(&["SOFTWARE", "Microsoft", "Windows NT", "CurrentVersion"]);
    winnt.set_value("ProductName", REG_SZ, b"Windows 10 Pro\0");
    winnt.set_value("CurrentMajorVersionNumber", REG_DWORD, &10u32.to_le_bytes());
    winnt.set_value("CurrentMinorVersionNumber", REG_DWORD, &0u32.to_le_bytes());
    winnt.set_value("CurrentBuildNumber", REG_SZ, b"19045\0");
    winnt.set_value("ReleaseId", REG_SZ, b"22H2\0");
    winnt.set_value("CurrentVersion", REG_SZ, b"6.3\0");

    // 3. Display video device map
    let video = hklm.create_path(&["HARDWARE", "DEVICEMAP", "VIDEO"]);
    video.set_value(
        "\\Device\\Video0",
        REG_SZ,
        b"\\Registry\\Machine\\System\\CurrentControlSet\\Control\\Video\\{12345678-ABCD-EF01-2345-6789ABCDEF01}\\0000\0",
    );
    video.set_value("MaxObjectNumber", REG_DWORD, &1u32.to_le_bytes());

    hives.insert(HKEY_LOCAL_MACHINE, hklm);
    *init = true;
}

/// Parses a null-terminated ANSI C-string up to 260 bytes.
pub unsafe fn parse_cstr_a(ptr: *const u8) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    while *ptr.add(len) != 0 && len < 260 {
        len += 1;
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    core::str::from_utf8(slice).ok().map(String::from)
}

/// Parses a null-terminated UTF-16 C-string up to 260 code units.
pub unsafe fn parse_cstr_w(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    while *ptr.add(len) != 0 && len < 260 {
        len += 1;
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    let chars: Vec<char> = slice.iter().map(|&c| c as u8 as char).collect();
    Some(chars.into_iter().collect())
}

/// Splits a registry key path by backslash or forward slash separators.
pub fn split_registry_path(path: &str) -> Vec<String> {
    path.split(|c| c == '\\' || c == '/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Resolves an open HKEY handle to its root hive and path components.
fn resolve_hkey_context(h_key: HKEY) -> Option<(HKEY, Vec<String>)> {
    match h_key {
        HKEY_CLASSES_ROOT
        | HKEY_CURRENT_USER
        | HKEY_LOCAL_MACHINE
        | HKEY_USERS
        | HKEY_PERFORMANCE_DATA
        | HKEY_CURRENT_CONFIG => Some((h_key, Vec::new())),
        _ => {
            let open_keys = OPEN_KEYS.lock();
            open_keys.get(&h_key).map(|entry| (entry.root_hkey, entry.path.clone()))
        }
    }
}

/// Emulates Win32 RegOpenKeyExA opening a subkey (ANSI).
pub unsafe fn RegOpenKeyExA(
    h_key: HKEY,
    lp_sub_key: *const u8,
    _ul_options: u32,
    _sam_desired: u32,
    phk_result: *mut HKEY,
) -> u32 {
    if phk_result.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let subkey_str = match parse_cstr_a(lp_sub_key) {
        Some(s) => s,
        None => String::new(),
    };
    reg_open_key_internal(h_key, &subkey_str, phk_result)
}

/// Emulates Win32 RegOpenKeyExW opening a subkey (UTF-16).
pub unsafe fn RegOpenKeyExW(
    h_key: HKEY,
    lp_sub_key: *const u16,
    _ul_options: u32,
    _sam_desired: u32,
    phk_result: *mut HKEY,
) -> u32 {
    if phk_result.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let subkey_str = match parse_cstr_w(lp_sub_key) {
        Some(s) => s,
        None => String::new(),
    };
    reg_open_key_internal(h_key, &subkey_str, phk_result)
}

fn reg_open_key_internal(h_key: HKEY, subkey_str: &str, phk_result: *mut HKEY) -> u32 {
    ensure_registry_initialized();
    let (root_hkey, mut path) = match resolve_hkey_context(h_key) {
        Some(ctx) => ctx,
        None => return ERROR_INVALID_HANDLE,
    };

    let sub_parts = split_registry_path(subkey_str);
    path.extend(sub_parts);

    let hives = REGISTRY_HIVES.lock();
    let root = match hives.get(&root_hkey) {
        Some(r) => r,
        None => return ERROR_FILE_NOT_FOUND,
    };

    let str_parts: Vec<&str> = path.iter().map(|s| s.as_str()).collect();
    if root.resolve_path(&str_parts).is_none() {
        return ERROR_FILE_NOT_FOUND;
    }
    drop(hives);

    let mut gen = NEXT_HKEY.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let mut open_keys = OPEN_KEYS.lock();
    open_keys.insert(handle, OpenKeyHandle { root_hkey, path });

    unsafe {
        *phk_result = handle;
    }
    ERROR_SUCCESS
}

/// Emulates Win32 RegCreateKeyExA creating or opening a subkey (ANSI).
pub unsafe fn RegCreateKeyExA(
    h_key: HKEY,
    lp_sub_key: *const u8,
    _reserved: u32,
    _lp_class: *const u8,
    _dw_options: u32,
    _sam_desired: u32,
    _lp_security_attributes: *const u8,
    phk_result: *mut HKEY,
    lpdw_disposition: *mut u32,
) -> u32 {
    if phk_result.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let subkey_str = match parse_cstr_a(lp_sub_key) {
        Some(s) => s,
        None => String::new(),
    };
    reg_create_key_internal(h_key, &subkey_str, phk_result, lpdw_disposition)
}

/// Emulates Win32 RegCreateKeyExW creating or opening a subkey (UTF-16).
pub unsafe fn RegCreateKeyExW(
    h_key: HKEY,
    lp_sub_key: *const u16,
    _reserved: u32,
    _lp_class: *const u8,
    _dw_options: u32,
    _sam_desired: u32,
    _lp_security_attributes: *const u8,
    phk_result: *mut HKEY,
    lpdw_disposition: *mut u32,
) -> u32 {
    if phk_result.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let subkey_str = match parse_cstr_w(lp_sub_key) {
        Some(s) => s,
        None => String::new(),
    };
    reg_create_key_internal(h_key, &subkey_str, phk_result, lpdw_disposition)
}

fn reg_create_key_internal(
    h_key: HKEY,
    subkey_str: &str,
    phk_result: *mut HKEY,
    lpdw_disposition: *mut u32,
) -> u32 {
    ensure_registry_initialized();
    let (root_hkey, mut path) = match resolve_hkey_context(h_key) {
        Some(ctx) => ctx,
        None => return ERROR_INVALID_HANDLE,
    };

    let sub_parts = split_registry_path(subkey_str);
    path.extend(sub_parts);

    let mut hives = REGISTRY_HIVES.lock();
    let root = match hives.get_mut(&root_hkey) {
        Some(r) => r,
        None => return ERROR_FILE_NOT_FOUND,
    };

    let str_parts: Vec<&str> = path.iter().map(|s| s.as_str()).collect();
    let existed = root.resolve_path(&str_parts).is_some();
    root.create_path(&str_parts);
    drop(hives);

    let disposition = if existed {
        REG_OPENED_EXISTING_KEY
    } else {
        REG_CREATED_NEW_KEY
    };

    if !lpdw_disposition.is_null() {
        unsafe {
            *lpdw_disposition = disposition;
        }
    }

    let mut gen = NEXT_HKEY.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let mut open_keys = OPEN_KEYS.lock();
    open_keys.insert(handle, OpenKeyHandle { root_hkey, path });

    unsafe {
        *phk_result = handle;
    }
    ERROR_SUCCESS
}

/// Emulates Win32 RegCloseKey releasing an open registry handle.
pub unsafe fn RegCloseKey(h_key: HKEY) -> u32 {
    match h_key {
        HKEY_CLASSES_ROOT
        | HKEY_CURRENT_USER
        | HKEY_LOCAL_MACHINE
        | HKEY_USERS
        | HKEY_PERFORMANCE_DATA
        | HKEY_CURRENT_CONFIG => ERROR_SUCCESS,
        _ => {
            let mut open_keys = OPEN_KEYS.lock();
            if open_keys.remove(&h_key).is_some() {
                ERROR_SUCCESS
            } else {
                ERROR_INVALID_HANDLE
            }
        }
    }
}

/// Emulates Win32 RegDeleteKeyA deleting a subkey (ANSI).
pub unsafe fn RegDeleteKeyA(h_key: HKEY, lp_sub_key: *const u8) -> u32 {
    let subkey_str = match parse_cstr_a(lp_sub_key) {
        Some(s) => s,
        None => return ERROR_INVALID_PARAMETER,
    };
    reg_delete_key_internal(h_key, &subkey_str)
}

/// Emulates Win32 RegDeleteKeyW deleting a subkey (UTF-16).
pub unsafe fn RegDeleteKeyW(h_key: HKEY, lp_sub_key: *const u16) -> u32 {
    let subkey_str = match parse_cstr_w(lp_sub_key) {
        Some(s) => s,
        None => return ERROR_INVALID_PARAMETER,
    };
    reg_delete_key_internal(h_key, &subkey_str)
}

fn reg_delete_key_internal(h_key: HKEY, subkey_str: &str) -> u32 {
    ensure_registry_initialized();
    let (root_hkey, mut path) = match resolve_hkey_context(h_key) {
        Some(ctx) => ctx,
        None => return ERROR_INVALID_HANDLE,
    };

    let sub_parts = split_registry_path(subkey_str);
    if sub_parts.is_empty() {
        return ERROR_INVALID_PARAMETER;
    }
    path.extend(sub_parts);

    let leaf_name = path.pop().unwrap();
    let mut hives = REGISTRY_HIVES.lock();
    let root = match hives.get_mut(&root_hkey) {
        Some(r) => r,
        None => return ERROR_FILE_NOT_FOUND,
    };

    let parent_parts: Vec<&str> = path.iter().map(|s| s.as_str()).collect();
    let parent = match root.resolve_path_mut(&parent_parts) {
        Some(p) => p,
        None => return ERROR_FILE_NOT_FOUND,
    };

    if parent.delete_subkey(&leaf_name) {
        ERROR_SUCCESS
    } else {
        ERROR_FILE_NOT_FOUND
    }
}
