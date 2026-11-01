//! SteamDeck MicroSD Card Gaming Library Fast-Indexing Service.

// Physical storage media mount source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameInstallSource {
    InternalNvme,
    MicroSdExt4,
    MicroSdBtrfs,
}

// Windows/Linux runtime compatibility translation tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityTool {
    NativeLinux,
    ProtonExperimental,
    ProtonHotfix,
    ProtonGE,
    CustomWrapper,
}

// Maximum limits for in-memory string fields in `#![no_std]`.
pub const MAX_TITLE_LEN: usize = 64;
pub const MAX_PATH_LEN: usize = 64;
pub const MAX_TARGET_LEN: usize = 48;
pub const MAX_INDEXED_GAMES: usize = 64;

// High-speed fixed-size indexed game metadata entry.
#[derive(Debug, Clone, Copy)]
pub struct IndexedGameEntry {
    pub appid: u32,
    pub title: [u8; MAX_TITLE_LEN],
    pub title_len: usize,
    pub install_dir: [u8; MAX_PATH_LEN],
    pub install_dir_len: usize,
    pub size_bytes: u64,
    pub build_id: u32,
    pub compat_tool: CompatibilityTool,
    pub has_proton_prefix: bool,
    pub prefix_path: [u8; MAX_PATH_LEN],
    pub prefix_path_len: usize,
    pub executable_target: [u8; MAX_TARGET_LEN],
    pub executable_target_len: usize,
    pub mount_source: GameInstallSource,
}

impl IndexedGameEntry {
    // Construct zeroed empty game entry.
    pub const fn empty() -> Self {
        Self {
            appid: 0,
            title: [0; MAX_TITLE_LEN],
            title_len: 0,
            install_dir: [0; MAX_PATH_LEN],
            install_dir_len: 0,
            size_bytes: 0,
            build_id: 0,
            compat_tool: CompatibilityTool::NativeLinux,
            has_proton_prefix: false,
            prefix_path: [0; MAX_PATH_LEN],
            prefix_path_len: 0,
            executable_target: [0; MAX_TARGET_LEN],
            executable_target_len: 0,
            mount_source: GameInstallSource::MicroSdExt4,
        }
    }

    // Set human-readable title from string slice.
    pub fn set_title(&mut self, title_str: &str) {
        let bytes = title_str.as_bytes();
        let copy_len = bytes.len().min(MAX_TITLE_LEN);
        self.title[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.title_len = copy_len;
    }

    // Retrieve title as string slice.
    pub fn title_str(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len]).unwrap_or("")
    }

    // Set install directory name.
    pub fn set_install_dir(&mut self, dir_str: &str) {
        let bytes = dir_str.as_bytes();
        let copy_len = bytes.len().min(MAX_PATH_LEN);
        self.install_dir[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.install_dir_len = copy_len;
    }

    // Retrieve install directory as string slice.
    pub fn install_dir_str(&self) -> &str {
        core::str::from_utf8(&self.install_dir[..self.install_dir_len]).unwrap_or("")
    }

    // Set relative executable target.
    pub fn set_executable_target(&mut self, target_str: &str) {
        let bytes = target_str.as_bytes();
        let copy_len = bytes.len().min(MAX_TARGET_LEN);
        self.executable_target[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.executable_target_len = copy_len;
    }

    // Retrieve relative executable target.
    pub fn executable_target_str(&self) -> &str {
        core::str::from_utf8(&self.executable_target[..self.executable_target_len]).unwrap_or("")
    }
}

// In-memory zero-allocation index table for rapid gaming library lookups.
pub struct GameIndexTable {
    entries: [IndexedGameEntry; MAX_INDEXED_GAMES],
    count: usize,
}

impl GameIndexTable {
    // Construct empty index table.
    pub const fn new() -> Self {
        Self {
            entries: [IndexedGameEntry::empty(); MAX_INDEXED_GAMES],
            count: 0,
        }
    }

    // Insert or update game entry by appid.
    pub fn insert_or_update(&mut self, entry: IndexedGameEntry) -> bool {
        // Check for existing appid to update in-place
        for i in 0..self.count {
            if self.entries[i].appid == entry.appid {
                self.entries[i] = entry;
                return true;
            }
        }

        // Insert new entry if capacity allows
        if self.count < MAX_INDEXED_GAMES {
            self.entries[self.count] = entry;
            self.count += 1;
            true
        } else {
            false
        }
    }

    // Find indexed game entry by numeric Steam AppID.
    pub fn get_by_appid(&self, appid: u32) -> Option<&IndexedGameEntry> {
        for i in 0..self.count {
            if self.entries[i].appid == appid {
                return Some(&self.entries[i]);
            }
        }
        None
    }

    // Total number of actively indexed games.
    pub fn count(&self) -> usize {
        self.count
    }

    // Clear all indexed entries.
    pub fn clear(&mut self) {
        self.count = 0;
    }

    // Calculate aggregated disk footprint across all indexed games.
    pub fn total_library_size_bytes(&self) -> u64 {
        let mut total = 0u64;
        for i in 0..self.count {
            total = total.saturating_add(self.entries[i].size_bytes);
        }
        total
    }
}

// Zero-allocation parser for Steam ACF/VDF AppManifest descriptors.
pub struct SteamAppManifestParser;

impl SteamAppManifestParser {
    // Extract quoted value for a given quoted key within ACF/VDF text.
    pub fn extract_field<'a>(acf_text: &'a str, key: &str) -> Option<&'a str> {
        let mut key_pattern = [0u8; 40];
        let k_bytes = key.as_bytes();
        if k_bytes.len() + 2 > key_pattern.len() {
            return None;
        }

        key_pattern[0] = b'"';
        key_pattern[1..1 + k_bytes.len()].copy_from_slice(k_bytes);
        key_pattern[1 + k_bytes.len()] = b'"';
        let key_str = core::str::from_utf8(&key_pattern[..2 + k_bytes.len()]).ok()?;

        let key_pos = acf_text.find(key_str)?;
        let rest = &acf_text[key_pos + key_str.len()..];

        // Find opening quote of value
        let val_start = rest.find('"')? + 1;
        let val_rest = &rest[val_start..];
        let val_end = val_rest.find('"')?;

        Some(&val_rest[..val_end])
    }

    // Parse unsigned integer from string slice in `#![no_std]`.
    pub fn parse_u64(s: &str) -> Option<u64> {
        let mut val = 0u64;
        let bytes = s.trim().as_bytes();
        if bytes.is_empty() {
            return None;
        }
        for &b in bytes {
            if !b.is_ascii_digit() {
                return None;
            }
            val = val.checked_mul(10)?.checked_add((b - b'0') as u64)?;
        }
        Some(val)
    }

    // Parse ACF appmanifest payload into structured game metadata entry.
    pub fn parse_manifest(acf_text: &str, mount_source: GameInstallSource) -> Option<IndexedGameEntry> {
        let appid_str = Self::extract_field(acf_text, "appid")?;
        let appid = Self::parse_u64(appid_str)? as u32;

        let name_str = Self::extract_field(acf_text, "name").unwrap_or("Unknown Title");
        let installdir_str = Self::extract_field(acf_text, "installdir").unwrap_or("");
        let size_str = Self::extract_field(acf_text, "SizeOnDisk").unwrap_or("0");
        let size_bytes = Self::parse_u64(size_str).unwrap_or(0);
        let buildid_str = Self::extract_field(acf_text, "buildid").unwrap_or("0");
        let buildid = Self::parse_u64(buildid_str).unwrap_or(0) as u32;

        let mut entry = IndexedGameEntry::empty();
        entry.appid = appid;
        entry.set_title(name_str);
        entry.set_install_dir(installdir_str);
        entry.size_bytes = size_bytes;
        entry.build_id = buildid;
        entry.mount_source = mount_source;

        Some(entry)
    }
}

// Proton compatibility data and wineprefix directory analyzer.
pub struct ProtonPrefixDetector;

impl ProtonPrefixDetector {
    // Inspect whether game has an active Proton compatdata prefix sandbox.
    pub fn inspect_prefix(entry: &mut IndexedGameEntry, has_drive_c: bool) {
        if has_drive_c {
            entry.has_proton_prefix = true;
            entry.compat_tool = CompatibilityTool::ProtonExperimental;

            // Build relative path compatdata/<appid>/pfx
            let mut path_buf = [0u8; MAX_PATH_LEN];
            let prefix_header = b"compatdata/";
            let pfx_suffix = b"/pfx";
            let mut pos = 0;

            path_buf[..prefix_header.len()].copy_from_slice(prefix_header);
            pos += prefix_header.len();

            let mut appid_str = [0u8; 10];
            let mut v = entry.appid;
            let mut i = 10;
            if v == 0 {
                path_buf[pos] = b'0';
                pos += 1;
            } else {
                while v > 0 {
                    i -= 1;
                    appid_str[i] = b'0' + (v % 10) as u8;
                    v /= 10;
                }
                let len = 10 - i;
                path_buf[pos..pos + len].copy_from_slice(&appid_str[i..10]);
                pos += len;
            }

            path_buf[pos..pos + pfx_suffix.len()].copy_from_slice(pfx_suffix);
            pos += pfx_suffix.len();

            entry.prefix_path[..pos].copy_from_slice(&path_buf[..pos]);
            entry.prefix_path_len = pos;
        } else {
            entry.has_proton_prefix = false;
            entry.compat_tool = CompatibilityTool::NativeLinux;
            entry.prefix_path_len = 0;
        }
    }
}
