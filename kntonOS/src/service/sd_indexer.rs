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
