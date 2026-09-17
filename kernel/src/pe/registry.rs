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
