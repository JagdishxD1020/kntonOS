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

// MicroSD filesystem format type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsType {
    Ext4,
    Btrfs,
    Unknown,
}

// MicroSD mount state in handheld environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountState {
    Unmounted,
    Probing,
    Mounted,
    Ejected,
}

// Fixed-size cache block for eliminating redundant MicroSD read round-trips.
pub const CACHE_BLOCK_SIZE: usize = 512;

// MicroSD card mount point watcher and read-ahead block cache.
pub struct MicroSdMountWatcher {
    pub state: MountState,
    pub fs_type: FsType,
    pub mount_path: [u8; MAX_PATH_LEN],
    pub mount_path_len: usize,
    pub capacity_bytes: u64,
    pub free_bytes: u64,
    cache_buffer: [u8; CACHE_BLOCK_SIZE],
    cache_offset: u32,
    cache_valid_len: usize,
    pub cache_hits: u32,
    pub cache_misses: u32,
}

impl MicroSdMountWatcher {
    // Construct mount watcher for default Steam Deck SD card path.
    pub const fn new(default_path: &str, fs_type: FsType) -> Self {
        let mut path_buf = [0u8; MAX_PATH_LEN];
        let bytes = default_path.as_bytes();
        let copy_len = if bytes.len() > MAX_PATH_LEN { MAX_PATH_LEN } else { bytes.len() };
        let mut i = 0;
        while i < copy_len {
            path_buf[i] = bytes[i];
            i += 1;
        }

        Self {
            state: MountState::Unmounted,
            fs_type,
            mount_path: path_buf,
            mount_path_len: copy_len,
            capacity_bytes: 0,
            free_bytes: 0,
            cache_buffer: [0; CACHE_BLOCK_SIZE],
            cache_offset: u32::MAX,
            cache_valid_len: 0,
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    // Process card insertion and filesystem mount event.
    pub fn on_card_inserted(&mut self, path: &str, fs_type: FsType, capacity: u64, free: u64) {
        let bytes = path.as_bytes();
        let copy_len = bytes.len().min(MAX_PATH_LEN);
        self.mount_path[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.mount_path_len = copy_len;
        self.fs_type = fs_type;
        self.capacity_bytes = capacity;
        self.free_bytes = free;
        self.state = MountState::Mounted;
        self.invalidate_cache();
    }

    // Process card ejection event.
    pub fn on_card_ejected(&mut self) {
        self.state = MountState::Ejected;
        self.capacity_bytes = 0;
        self.free_bytes = 0;
        self.invalidate_cache();
    }

    // Check if card is currently mounted and accessible.
    pub fn is_mounted(&self) -> bool {
        self.state == MountState::Mounted
    }

    // Invalidate read-ahead block cache.
    pub fn invalidate_cache(&mut self) {
        self.cache_offset = u32::MAX;
        self.cache_valid_len = 0;
    }

    // Fetch block data using read-ahead cache to eliminate slow flash transactions.
    pub fn read_cached<'a>(&'a mut self, file_offset: u32, length: usize, raw_source: &[u8]) -> &'a [u8] {
        let len = length.min(CACHE_BLOCK_SIZE);

        // Check cache hit: requested range fits inside current cached window
        if self.cache_offset != u32::MAX
            && file_offset >= self.cache_offset
            && (file_offset + len as u32) <= (self.cache_offset + self.cache_valid_len as u32)
        {
            self.cache_hits += 1;
            let rel_offset = (file_offset - self.cache_offset) as usize;
            return &self.cache_buffer[rel_offset..rel_offset + len];
        }

        // Cache miss: fill read-ahead buffer from source
        self.cache_misses += 1;
        self.cache_offset = file_offset;

        let src_start = (file_offset as usize).min(raw_source.len());
        let src_end = (src_start + CACHE_BLOCK_SIZE).min(raw_source.len());
        let fill_len = src_end.saturating_sub(src_start);

        self.cache_buffer[..fill_len].copy_from_slice(&raw_source[src_start..src_end]);
        self.cache_valid_len = fill_len;

        let res_len = len.min(fill_len);
        &self.cache_buffer[..res_len]
    }

    // Compute read-ahead cache hit ratio percentage [0..100].
    pub fn cache_hit_ratio_pct(&self) -> u8 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0
        } else {
            (((self.cache_hits as u64) * 100) / (total as u64)) as u8
        }
    }
}

// Library directory hierarchy scanning phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScannerStatus {
    Idle,
    Scanning,
    Ready,
    Error,
}

// Statistical performance counters for library indexing scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanMetrics {
    pub manifests_scanned: u32,
    pub proton_games_count: u32,
    pub native_games_count: u32,
    pub total_bytes_discovered: u64,
}

// Fast directory hierarchy scanner parsing steamapps manifests and prefixes.
pub struct SteamLibraryHierarchyScanner {
    pub status: ScannerStatus,
    pub metrics: ScanMetrics,
}

impl SteamLibraryHierarchyScanner {
    // Construct new library scanner instance.
    pub const fn new() -> Self {
        Self {
            status: ScannerStatus::Idle,
            metrics: ScanMetrics {
                manifests_scanned: 0,
                proton_games_count: 0,
                native_games_count: 0,
                total_bytes_discovered: 0,
            },
        }
    }

    // Ingest and index a single manifest payload with associated compatdata.
    pub fn ingest_manifest(
        &mut self,
        table: &mut GameIndexTable,
        acf_text: &str,
        source: GameInstallSource,
        has_proton_pfx: bool,
        relative_exe: &str,
    ) -> bool {
        if let Some(mut entry) = SteamAppManifestParser::parse_manifest(acf_text, source) {
            ProtonPrefixDetector::inspect_prefix(&mut entry, has_proton_pfx);
            entry.set_executable_target(relative_exe);

            self.metrics.manifests_scanned += 1;
            if entry.has_proton_prefix {
                self.metrics.proton_games_count += 1;
            } else {
                self.metrics.native_games_count += 1;
            }
            self.metrics.total_bytes_discovered = self
                .metrics
                .total_bytes_discovered
                .saturating_add(entry.size_bytes);

            table.insert_or_update(entry)
        } else {
            false
        }
    }

    // Reset scan metrics and return to idle state.
    pub fn reset(&mut self) {
        self.status = ScannerStatus::Idle;
        self.metrics = ScanMetrics {
            manifests_scanned: 0,
            proton_games_count: 0,
            native_games_count: 0,
            total_bytes_discovered: 0,
        };
    }
}

// Central microSD gaming library fast-indexing service daemon.
pub struct SdGamingIndexerService {
    pub table: GameIndexTable,
    pub watcher: MicroSdMountWatcher,
    pub scanner: SteamLibraryHierarchyScanner,
}

impl SdGamingIndexerService {
    // Construct initialized gaming library indexer service.
    pub const fn new(default_mount_path: &str, fs_type: FsType) -> Self {
        Self {
            table: GameIndexTable::new(),
            watcher: MicroSdMountWatcher::new(default_mount_path, fs_type),
            scanner: SteamLibraryHierarchyScanner::new(),
        }
    }

    // Lookup game entry by Steam AppID.
    pub fn find_by_appid(&self, appid: u32) -> Option<&IndexedGameEntry> {
        self.table.get_by_appid(appid)
    }

    // Zero-allocation case-insensitive title prefix lookup.
    pub fn find_by_title_prefix(&self, prefix: &str) -> Option<&IndexedGameEntry> {
        let p_bytes = prefix.as_bytes();
        if p_bytes.is_empty() {
            return None;
        }

        for i in 0..self.table.count() {
            let entry = &self.table.entries[i];
            if entry.title_len < p_bytes.len() {
                continue;
            }

            let mut matched = true;
            for j in 0..p_bytes.len() {
                let c1 = entry.title[j].to_ascii_lowercase();
                let c2 = p_bytes[j].to_ascii_lowercase();
                if c1 != c2 {
                    matched = false;
                    break;
                }
            }

            if matched {
                return Some(entry);
            }
        }
        None
    }

    // Calculate aggregated storage footprint, optionally filtered by mount source.
    pub fn calculate_storage_usage(&self, source_filter: Option<GameInstallSource>) -> u64 {
        let mut sum = 0u64;
        for i in 0..self.table.count() {
            let entry = &self.table.entries[i];
            match source_filter {
                Some(src) => {
                    if entry.mount_source == src {
                        sum = sum.saturating_add(entry.size_bytes);
                    }
                }
                None => {
                    sum = sum.saturating_add(entry.size_bytes);
                }
            }
        }
        sum
    }

    // Resolve full execution launch target command into caller-provided buffer without allocation.
    pub fn resolve_launch_target(&self, appid: u32, out_buffer: &mut [u8]) -> Result<usize, &'static str> {
        let entry = self.table.get_by_appid(appid).ok_or("AppID not found")?;
        let mount_str = core::str::from_utf8(&self.watcher.mount_path[..self.watcher.mount_path_len])
            .unwrap_or("/run/media/mmcblk0p1");

        let mut pos = 0;

        let write_slice = |buf: &mut [u8], offset: &mut usize, slice: &[u8]| -> Result<(), &'static str> {
            if *offset + slice.len() > buf.len() {
                return Err("Buffer too small");
            }
            buf[*offset..*offset + slice.len()].copy_from_slice(slice);
            *offset += slice.len();
            Ok(())
        };

        if entry.has_proton_prefix {
            write_slice(out_buffer, &mut pos, b"proton run ")?;
        }

        write_slice(out_buffer, &mut pos, mount_str.as_bytes())?;
        write_slice(out_buffer, &mut pos, b"/steamapps/common/")?;
        write_slice(out_buffer, &mut pos, &entry.install_dir[..entry.install_dir_len])?;
        write_slice(out_buffer, &mut pos, b"/")?;
        write_slice(out_buffer, &mut pos, &entry.executable_target[..entry.executable_target_len])?;

        Ok(pos)
    }
}

// Comprehensive self-test suite verifying MicroSD mount watcher, ACF parser, and query engine.
pub fn verify_sd_indexer_subsystem() -> bool {
    let mut service = SdGamingIndexerService::new("/run/media/mmcblk0p1", FsType::Ext4);

    // 1. Verify Mount Watcher and Read-Ahead Cache
    if service.watcher.is_mounted() {
        return false;
    }

    service.watcher.on_card_inserted(
        "/run/media/mmcblk0p1",
        FsType::Ext4,
        512_000_000_000,
        128_000_000_000,
    );
    if !service.watcher.is_mounted() {
        return false;
    }

    // Populate fake sector data and verify read-ahead caching
    let mut sector_data = [0u8; 1024];
    sector_data[10..15].copy_from_slice(b"STEAM");
    let chunk1 = service.watcher.read_cached(0, 32, &sector_data);
    if &chunk1[10..15] != b"STEAM" {
        return false;
    }
    let chunk2 = service.watcher.read_cached(10, 5, &sector_data);
    if chunk2 != b"STEAM" {
        return false;
    }
    if service.watcher.cache_hits != 1 || service.watcher.cache_misses != 1 {
        return false;
    }

    // 2. Ingest Windows/Proton Title via AppManifest
    let acf_cyberpunk = "\"AppState\"\n{\n    \"appid\" \"1091500\"\n    \"name\" \"Cyberpunk 2077\"\n    \"installdir\" \"Cyberpunk 2077\"\n    \"SizeOnDisk\" \"73829182390\"\n    \"buildid\" \"1234567\"\n}\n";
    let ok1 = service.scanner.ingest_manifest(
        &mut service.table,
        acf_cyberpunk,
        GameInstallSource::MicroSdExt4,
        true,
        "bin/x64/Cyberpunk2077.exe",
    );
    if !ok1 {
        return false;
    }

    // 3. Ingest Native Linux Title via AppManifest
    let acf_stardew = "\"AppState\"\n{\n    \"appid\" \"413150\"\n    \"name\" \"Stardew Valley\"\n    \"installdir\" \"Stardew Valley\"\n    \"SizeOnDisk\" \"500000000\"\n    \"buildid\" \"889900\"\n}\n";
    let ok2 = service.scanner.ingest_manifest(
        &mut service.table,
        acf_stardew,
        GameInstallSource::MicroSdExt4,
        false,
        "StardewValley.bin.x86_64",
    );
    if !ok2 {
        return false;
    }

    if service.table.count() != 2 {
        return false;
    }

    // 4. Verify Queries by AppID and Case-Insensitive Title Prefix
    let g1 = service.find_by_appid(1091500);
    if g1.is_none() {
        return false;
    }
    let e1 = g1.unwrap();
    if e1.title_str() != "Cyberpunk 2077" || !e1.has_proton_prefix {
        return false;
    }

    let g2 = service.find_by_title_prefix("stardew");
    if g2.is_none() {
        return false;
    }
    let e2 = g2.unwrap();
    if e2.appid != 413150 || e2.has_proton_prefix {
        return false;
    }

    let total_size = service.calculate_storage_usage(None);
    if total_size != (73829182390 + 500000000) {
        return false;
    }

    // 5. Verify Zero-Allocation Launch Command Resolution
    let mut cmd_buf = [0u8; 128];
    let len1 = service.resolve_launch_target(1091500, &mut cmd_buf).unwrap();
    let expected_target1 = "proton run /run/media/mmcblk0p1/steamapps/common/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe";
    if core::str::from_utf8(&cmd_buf[..len1]).unwrap_or("") != expected_target1 {
        return false;
    }

    let len2 = service.resolve_launch_target(413150, &mut cmd_buf).unwrap();
    let expected_target2 = "/run/media/mmcblk0p1/steamapps/common/Stardew Valley/StardewValley.bin.x86_64";
    if core::str::from_utf8(&cmd_buf[..len2]).unwrap_or("") != expected_target2 {
        return false;
    }

    // 6. Eject card
    service.watcher.on_card_ejected();
    if service.watcher.is_mounted() {
        return false;
    }

    true
}
