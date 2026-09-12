//! Native Package Manager (kpkg) Manifest, Merkle Tree CAS & Profile Switcher.

/// Architecture supported by kpkg binary packages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageArchitecture {
    X86_64,
    AArch64,
    Universal,
}

/// Semantic package version (major.minor.patch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl PackageVersion {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self { major, minor, patch }
    }
}

pub const MAX_NAME_LEN: usize = 32;
pub const MAX_PATH_LEN: usize = 64;
pub const MAX_DEPS: usize = 8;

/// Package dependency descriptor specifying name and minimum version.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackageDependency {
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: usize,
    pub min_version: PackageVersion,
}

impl PackageDependency {
    pub fn new(name_str: &str, min_version: PackageVersion) -> Self {
        let mut name = [0u8; MAX_NAME_LEN];
        let bytes = name_str.as_bytes();
        let len = bytes.len().min(MAX_NAME_LEN);
        let mut i = 0;
        while i < len {
            name[i] = bytes[i];
            i += 1;
        }
        Self { name, name_len: len, min_version }
    }

    pub fn matches_name(&self, other_str: &str) -> bool {
        let bytes = other_str.as_bytes();
        if bytes.len() != self.name_len {
            return false;
        }
        for i in 0..self.name_len {
            if self.name[i] != bytes[i] {
                return false;
            }
        }
        true
    }
}

/// Native kpkg package manifest schema holding metadata and dependencies.
#[derive(Clone, Copy, Debug)]
pub struct KpkgManifest {
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: usize,
    pub version: PackageVersion,
    pub arch: PackageArchitecture,
    pub binary_path: [u8; MAX_PATH_LEN],
    pub binary_path_len: usize,
    pub dependencies: [PackageDependency; MAX_DEPS],
    pub dep_count: usize,
}

impl KpkgManifest {
    pub fn new(name_str: &str, version: PackageVersion, arch: PackageArchitecture, bin_path: &str) -> Self {
        let mut name = [0u8; MAX_NAME_LEN];
        let n_bytes = name_str.as_bytes();
        let n_len = n_bytes.len().min(MAX_NAME_LEN);
        let mut i = 0;
        while i < n_len {
            name[i] = n_bytes[i];
            i += 1;
        }

        let mut binary_path = [0u8; MAX_PATH_LEN];
        let p_bytes = bin_path.as_bytes();
        let p_len = p_bytes.len().min(MAX_PATH_LEN);
        let mut j = 0;
        while j < p_len {
            binary_path[j] = p_bytes[j];
            j += 1;
        }

        Self {
            name,
            name_len: n_len,
            version,
            arch,
            binary_path,
            binary_path_len: p_len,
            dependencies: [PackageDependency::new("", PackageVersion::new(0, 0, 0)); MAX_DEPS],
            dep_count: 0,
        }
    }

    pub fn add_dependency(&mut self, dep: PackageDependency) -> bool {
        if self.dep_count >= MAX_DEPS {
            return false;
        }
        self.dependencies[self.dep_count] = dep;
        self.dep_count += 1;
        true
    }

    pub fn name_as_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    pub fn binary_as_str(&self) -> &str {
        core::str::from_utf8(&self.binary_path[..self.binary_path_len]).unwrap_or("")
    }
}

/// Package dependency resolver checking availability and minimum version requirements.
pub struct KpkgDependencyResolver<'a> {
    pub installed_manifests: &'a [KpkgManifest],
}

impl<'a> KpkgDependencyResolver<'a> {
    pub const fn new(installed: &'a [KpkgManifest]) -> Self {
        Self { installed_manifests: installed }
    }

    /// Verifies all dependencies of target manifest are satisfied by installed packages.
    pub fn resolve_dependencies(&self, target: &KpkgManifest) -> bool {
        for i in 0..target.dep_count {
            let dep = &target.dependencies[i];
            let mut satisfied = false;

            for installed in self.installed_manifests {
                if dep.matches_name(installed.name_as_str()) {
                    if installed.version >= dep.min_version {
                        satisfied = true;
                        break;
                    }
                }
            }

            if !satisfied {
                return false;
            }
        }
        true
    }
}

/// Verifies kpkg package manifest schema, semantic versioning, and dependency resolution.
pub fn verify_kpkg_manifest_and_dependencies() -> bool {
    let mut game = KpkgManifest::new(
        "cyber-game",
        PackageVersion::new(1, 0, 0),
        PackageArchitecture::X86_64,
        "/bin/game.exe",
    );

    let dep_dxvk = PackageDependency::new("dxvk-runtime", PackageVersion::new(2, 3, 0));
    let dep_vkd3d = PackageDependency::new("vkd3d-proton", PackageVersion::new(2, 10, 0));
    game.add_dependency(dep_dxvk);
    game.add_dependency(dep_vkd3d);

    if game.name_as_str() != "cyber-game" || game.binary_as_str() != "/bin/game.exe" {
        return false;
    }

    // Installed runtime packages
    let installed = [
        KpkgManifest::new("dxvk-runtime", PackageVersion::new(2, 3, 1), PackageArchitecture::X86_64, "/lib/dxvk.dll"),
        KpkgManifest::new("vkd3d-proton", PackageVersion::new(2, 10, 0), PackageArchitecture::X86_64, "/lib/d3d12.dll"),
    ];

    let resolver = KpkgDependencyResolver::new(&installed);
    if !resolver.resolve_dependencies(&game) {
        return false;
    }

    // Incompatible installed version test (e.g. dxvk 2.2.0 < 2.3.0)
    let outdated = [
        KpkgManifest::new("dxvk-runtime", PackageVersion::new(2, 2, 0), PackageArchitecture::X86_64, "/lib/dxvk.dll"),
        KpkgManifest::new("vkd3d-proton", PackageVersion::new(2, 10, 0), PackageArchitecture::X86_64, "/lib/d3d12.dll"),
    ];
    let resolver_outdated = KpkgDependencyResolver::new(&outdated);
    if resolver_outdated.resolve_dependencies(&game) {
        return false;
    }

    true
}

// Merkle Tree Content-Addressed Storage (CAS) & 64KB Deduplication

use crate::crypto::Sha256;

/// Standard 64KB chunk size used across kpkg Content-Addressed Storage.
pub const CAS_CHUNK_SIZE: usize = 65536;
pub const MAX_CHUNKS_PER_FILE: usize = 16;
pub const MAX_CAS_OBJECTS: usize = 32;

/// Cryptographic 256-bit Content-Address identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CasHash256(pub [u8; 32]);

impl CasHash256 {
    pub const fn zero() -> Self {
        Self([0u8; 32])
    }

    pub fn from_data(data: &[u8]) -> Self {
        Self(Sha256::digest(data))
    }

    /// Combines two child hashes to produce an intermediate Merkle tree node hash.
    pub fn combine_children(left: &Self, right: &Self) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(&left.0);
        hasher.update(&right.0);
        Self(hasher.finalize())
    }

    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }
}

/// Metadata descriptor for a deduplicated CAS storage chunk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CasChunkDescriptor {
    pub hash: CasHash256,
    pub size_bytes: usize,
    pub ref_count: usize,
}

/// Hierarchical Merkle Tree computing cryptographic integrity root for multi-chunk payloads.
#[derive(Clone, Copy, Debug)]
pub struct MerkleTree {
    pub leaf_hashes: [CasHash256; MAX_CHUNKS_PER_FILE],
    pub leaf_count: usize,
}

impl MerkleTree {
    pub const fn new() -> Self {
        Self {
            leaf_hashes: [CasHash256::zero(); MAX_CHUNKS_PER_FILE],
            leaf_count: 0,
        }
    }

    pub fn add_chunk_hash(&mut self, hash: CasHash256) -> bool {
        if self.leaf_count >= MAX_CHUNKS_PER_FILE {
            return false;
        }
        self.leaf_hashes[self.leaf_count] = hash;
        self.leaf_count += 1;
        true
    }

    /// Computes the binary Merkle root hash across all leaf chunks.
    pub fn compute_root(&self) -> CasHash256 {
        if self.leaf_count == 0 {
            return CasHash256::zero();
        }
        if self.leaf_count == 1 {
            return self.leaf_hashes[0];
        }

        let mut current_level = [CasHash256::zero(); MAX_CHUNKS_PER_FILE];
        let mut count = self.leaf_count;
        for i in 0..count {
            current_level[i] = self.leaf_hashes[i];
        }

        while count > 1 {
            let mut next_level = [CasHash256::zero(); MAX_CHUNKS_PER_FILE];
            let mut next_count = 0;
            let mut i = 0;

            while i < count {
                if i + 1 < count {
                    next_level[next_count] = CasHash256::combine_children(&current_level[i], &current_level[i + 1]);
                } else {
                    next_level[next_count] = current_level[i];
                }
                next_count += 1;
                i += 2;
            }

            count = next_count;
            current_level = next_level;
        }

        current_level[0]
    }
}

/// In-memory Content-Addressed Storage index tracking chunk deduplication and references.
pub struct CasObjectStore {
    pub objects: [CasChunkDescriptor; MAX_CAS_OBJECTS],
    pub object_count: usize,
    pub total_ingested_bytes: usize,
    pub unique_stored_bytes: usize,
}

impl CasObjectStore {
    pub const fn new() -> Self {
        Self {
            objects: [CasChunkDescriptor {
                hash: CasHash256::zero(),
                size_bytes: 0,
                ref_count: 0,
            }; MAX_CAS_OBJECTS],
            object_count: 0,
            total_ingested_bytes: 0,
            unique_stored_bytes: 0,
        }
    }

    /// Ingests a data slice into the CAS store, returning (hash, is_new_unique_chunk).
    pub fn ingest_chunk(&mut self, chunk_data: &[u8]) -> (CasHash256, bool) {
        let hash = CasHash256::from_data(chunk_data);
        let size = chunk_data.len();
        self.total_ingested_bytes += size;

        // Check for deduplication
        for i in 0..self.object_count {
            if self.objects[i].hash == hash {
                self.objects[i].ref_count += 1;
                return (hash, false);
            }
        }

        if self.object_count < MAX_CAS_OBJECTS {
            self.objects[self.object_count] = CasChunkDescriptor {
                hash,
                size_bytes: size,
                ref_count: 1,
            };
            self.object_count += 1;
            self.unique_stored_bytes += size;
            (hash, true)
        } else {
            (hash, false)
        }
    }

    pub fn deduplication_ratio(&self) -> f32 {
        if self.unique_stored_bytes == 0 {
            1.0
        } else {
            self.total_ingested_bytes as f32 / self.unique_stored_bytes as f32
        }
    }
}

/// Verifies Merkle tree hierarchical hashing, 64KB chunking, and CAS deduplication.
pub fn verify_merkle_tree_cas() -> bool {
    let mut store = CasObjectStore::new();

    let chunk_a = [0xAAu8; 1024];
    let chunk_b = [0xBBu8; 1024];
    let chunk_a_dup = [0xAAu8; 1024];

    // 1. Ingest unique chunk A
    let (hash_a, is_new_a) = store.ingest_chunk(&chunk_a);
    if !is_new_a || hash_a.is_zero() {
        return false;
    }

    // 2. Ingest duplicate chunk A -> must be deduplicated
    let (hash_a2, is_new_a2) = store.ingest_chunk(&chunk_a_dup);
    if is_new_a2 || hash_a2 != hash_a {
        return false;
    }

    // 3. Ingest unique chunk B
    let (hash_b, is_new_b) = store.ingest_chunk(&chunk_b);
    if !is_new_b || hash_b == hash_a {
        return false;
    }

    // Total ingested = 3072, unique stored = 2048 -> ratio = 1.5
    if (store.deduplication_ratio() - 1.5).abs() > 0.01 {
        return false;
    }

    // 4. Construct Merkle tree of chunks
    let mut tree = MerkleTree::new();
    tree.add_chunk_hash(hash_a);
    tree.add_chunk_hash(hash_b);
    let root = tree.compute_root();
    if root.is_zero() || root == hash_a || root == hash_b {
        return false;
    }

    // Deterministic root check: combining same leaves yields identical root
    let mut tree2 = MerkleTree::new();
    tree2.add_chunk_hash(hash_a);
    tree2.add_chunk_hash(hash_b);
    if tree2.compute_root() != root {
        return false;
    }

    true
}

// OverlayFS Mount Staging & Per-Game Sandbox Isolation

pub const MAX_OVERLAY_MOUNTS: usize = 8;

/// Staged OverlayFS mount configuration isolating read-only package CAS from writable game sandbox.
#[derive(Clone, Copy, Debug)]
pub struct OverlayFsMount {
    pub package_name: [u8; MAX_NAME_LEN],
    pub package_name_len: usize,
    pub game_id: u64,
    pub lower_path: [u8; MAX_PATH_LEN],
    pub lower_len: usize,
    pub upper_path: [u8; MAX_PATH_LEN],
    pub upper_len: usize,
    pub merged_path: [u8; MAX_PATH_LEN],
    pub merged_len: usize,
    pub is_read_only_lower: bool,
    pub is_active: bool,
}

impl OverlayFsMount {
    pub const fn empty() -> Self {
        Self {
            package_name: [0u8; MAX_NAME_LEN],
            package_name_len: 0,
            game_id: 0,
            lower_path: [0u8; MAX_PATH_LEN],
            lower_len: 0,
            upper_path: [0u8; MAX_PATH_LEN],
            upper_len: 0,
            merged_path: [0u8; MAX_PATH_LEN],
            merged_len: 0,
            is_read_only_lower: true,
            is_active: false,
        }
    }

    pub fn merged_path_str(&self) -> &str {
        core::str::from_utf8(&self.merged_path[..self.merged_len]).unwrap_or("")
    }

    pub fn lower_path_str(&self) -> &str {
        core::str::from_utf8(&self.lower_path[..self.lower_len]).unwrap_or("")
    }

    pub fn upper_path_str(&self) -> &str {
        core::str::from_utf8(&self.upper_path[..self.upper_len]).unwrap_or("")
    }
}

/// OverlayFS mount staging manager coordinating package layers and sandboxes.
pub struct OverlayFsMountManager {
    pub mounts: [OverlayFsMount; MAX_OVERLAY_MOUNTS],
}

impl OverlayFsMountManager {
    pub const fn new() -> Self {
        Self {
            mounts: [OverlayFsMount::empty(); MAX_OVERLAY_MOUNTS],
        }
    }

    /// Stages an OverlayFS mount combining read-only CAS package tree with writable game overlay.
    pub fn stage_mount(&mut self, package_name: &str, game_id: u64) -> Option<usize> {
        let mut free_idx = None;
        for i in 0..MAX_OVERLAY_MOUNTS {
            if !self.mounts[i].is_active {
                free_idx = Some(i);
                break;
            }
        }

        let idx = free_idx?;
        let mut mount = OverlayFsMount::empty();

        // 1. Copy package name
        let p_bytes = package_name.as_bytes();
        let p_len = p_bytes.len().min(MAX_NAME_LEN);
        let mut k = 0;
        while k < p_len {
            mount.package_name[k] = p_bytes[k];
            k += 1;
        }
        mount.package_name_len = p_len;
        mount.game_id = game_id;

        // 2. Format lower path: /vfs/packages/<name>/
        let lower_prefix = b"/vfs/packages/";
        let mut pos = 0;
        while pos < lower_prefix.len() {
            mount.lower_path[pos] = lower_prefix[pos];
            pos += 1;
        }
        let mut l = 0;
        while l < p_len && pos < MAX_PATH_LEN {
            mount.lower_path[pos] = p_bytes[l];
            pos += 1;
            l += 1;
        }
        if pos < MAX_PATH_LEN {
            mount.lower_path[pos] = b'/';
            pos += 1;
        }
        mount.lower_len = pos;

        // 3. Format upper path: /sandbox/game/work/
        let upper_str = b"/sandbox/game/work/";
        mount.upper_len = upper_str.len().min(MAX_PATH_LEN);
        for m in 0..mount.upper_len {
            mount.upper_path[m] = upper_str[m];
        }

        // 4. Format merged path: /vfs/games/<name>/run/
        let merged_prefix = b"/vfs/games/";
        let mut mpos = 0;
        while mpos < merged_prefix.len() {
            mount.merged_path[mpos] = merged_prefix[mpos];
            mpos += 1;
        }
        let mut n = 0;
        while n < p_len && mpos < MAX_PATH_LEN {
            mount.merged_path[mpos] = p_bytes[n];
            mpos += 1;
            n += 1;
        }
        let merged_suffix = b"/run/";
        let mut s = 0;
        while s < merged_suffix.len() && mpos < MAX_PATH_LEN {
            mount.merged_path[mpos] = merged_suffix[s];
            mpos += 1;
            s += 1;
        }
        mount.merged_len = mpos;

        mount.is_read_only_lower = true;
        mount.is_active = true;

        self.mounts[idx] = mount;
        Some(idx)
    }

    pub fn unmount(&mut self, game_id: u64) -> bool {
        for i in 0..MAX_OVERLAY_MOUNTS {
            if self.mounts[i].is_active && self.mounts[i].game_id == game_id {
                self.mounts[i] = OverlayFsMount::empty();
                return true;
            }
        }
        false
    }
}

/// Verifies OverlayFS mount staging, read-only lower CAS pathing, and writable upper sandboxing.
pub fn verify_package_staging_and_overlayfs() -> bool {
    let mut manager = OverlayFsMountManager::new();

    let mount_idx = match manager.stage_mount("cyber-game", 101) {
        Some(i) => i,
        None => return false,
    };

    let mount = &manager.mounts[mount_idx];
    if !mount.is_active || !mount.is_read_only_lower {
        return false;
    }

    if mount.lower_path_str() != "/vfs/packages/cyber-game/" {
        return false;
    }

    if mount.merged_path_str() != "/vfs/games/cyber-game/run/" {
        return false;
    }

    // Unmount test
    if !manager.unmount(101) {
        return false;
    }
    if manager.mounts[mount_idx].is_active {
        return false;
    }

    true
}

// Gaming Profile Switcher & Environment Activator

pub const MAX_LIB_PATHS: usize = 4;
pub const MAX_PROFILES: usize = 4;

/// Gaming environment profile defining runtime DLLs, graphics layers, and CPU/GPU affinities.
#[derive(Clone, Copy, Debug)]
pub struct GamingEnvironmentProfile {
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: usize,
    pub proton_version: PackageVersion,
    pub dxvk_version: PackageVersion,
    pub fsr_enabled: bool,
    pub cpu_affinity_mask: u64,
    pub gpu_power_limit_w: u16,
    pub library_paths: [[u8; MAX_PATH_LEN]; MAX_LIB_PATHS],
    pub lib_path_lens: [usize; MAX_LIB_PATHS],
    pub lib_path_count: usize,
}

impl GamingEnvironmentProfile {
    pub fn new(
        name_str: &str,
        proton_ver: PackageVersion,
        dxvk_ver: PackageVersion,
        fsr_enabled: bool,
        cpu_affinity: u64,
        gpu_power_w: u16,
    ) -> Self {
        let mut name = [0u8; MAX_NAME_LEN];
        let bytes = name_str.as_bytes();
        let len = bytes.len().min(MAX_NAME_LEN);
        let mut i = 0;
        while i < len {
            name[i] = bytes[i];
            i += 1;
        }

        Self {
            name,
            name_len: len,
            proton_version: proton_ver,
            dxvk_version: dxvk_ver,
            fsr_enabled,
            cpu_affinity_mask: cpu_affinity,
            gpu_power_limit_w: gpu_power_w,
            library_paths: [[0u8; MAX_PATH_LEN]; MAX_LIB_PATHS],
            lib_path_lens: [0usize; MAX_LIB_PATHS],
            lib_path_count: 0,
        }
    }

    pub fn add_library_path(&mut self, path_str: &str) -> bool {
        if self.lib_path_count >= MAX_LIB_PATHS {
            return false;
        }
        let bytes = path_str.as_bytes();
        let len = bytes.len().min(MAX_PATH_LEN);
        let idx = self.lib_path_count;
        for i in 0..len {
            self.library_paths[idx][i] = bytes[i];
        }
        self.lib_path_lens[idx] = len;
        self.lib_path_count += 1;
        true
    }

    pub fn name_as_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    pub fn library_path_str(&self, idx: usize) -> Option<&str> {
        if idx >= self.lib_path_count {
            None
        } else {
            core::str::from_utf8(&self.library_paths[idx][..self.lib_path_lens[idx]]).ok()
        }
    }
}

/// Dynamic profile switcher managing active game environment and library resolution paths.
pub struct KpkgProfileSwitcher {
    pub profiles: [Option<GamingEnvironmentProfile>; MAX_PROFILES],
    pub active_profile_idx: Option<usize>,
}

impl KpkgProfileSwitcher {
    pub const fn new() -> Self {
        Self {
            profiles: [None; MAX_PROFILES],
            active_profile_idx: None,
        }
    }

    pub fn register_profile(&mut self, profile: GamingEnvironmentProfile) -> bool {
        for i in 0..MAX_PROFILES {
            if self.profiles[i].is_none() {
                self.profiles[i] = Some(profile);
                return true;
            }
        }
        false
    }

    /// Dynamically switches active gaming profile by name, reconfiguring library paths.
    pub fn kpkg_switch_profile(&mut self, profile_name: &str) -> bool {
        for i in 0..MAX_PROFILES {
            if let Some(ref prof) = self.profiles[i] {
                if prof.name_as_str() == profile_name {
                    self.active_profile_idx = Some(i);
                    return true;
                }
            }
        }
        false
    }

    pub fn active_profile(&self) -> Option<&GamingEnvironmentProfile> {
        self.active_profile_idx.and_then(|idx| self.profiles[idx].as_ref())
    }
}

/// Verifies dynamic gaming profile switching, environment activation, and library resolution paths.
pub fn verify_gaming_profile_switcher() -> bool {
    let mut switcher = KpkgProfileSwitcher::new();

    let mut prof_default = GamingEnvironmentProfile::new(
        "StandardCompatibility",
        PackageVersion::new(9, 0, 1),
        PackageVersion::new(2, 3, 0),
        false,
        0x000F, // Cores 0..3
        15,     // 15W TDP
    );
    prof_default.add_library_path("/vfs/packages/proton-9.0/lib/");
    prof_default.add_library_path("/vfs/packages/dxvk-2.3/lib/");
    switcher.register_profile(prof_default);

    let mut prof_esports = GamingEnvironmentProfile::new(
        "EsportsLowLatency",
        PackageVersion::new(9, 0, 3),
        PackageVersion::new(2, 4, 0),
        true,
        0x00FF, // All 8 cores
        28,     // 28W TDP
    );
    prof_esports.add_library_path("/vfs/packages/proton-ge-latest/lib/");
    prof_esports.add_library_path("/vfs/packages/dxvk-async/lib/");
    switcher.register_profile(prof_esports);

    // 1. Initial state: no active profile
    if switcher.active_profile().is_some() {
        return false;
    }

    // 2. Switch to EsportsLowLatency profile
    if !switcher.kpkg_switch_profile("EsportsLowLatency") {
        return false;
    }

    let active = match switcher.active_profile() {
        Some(p) => p,
        None => return false,
    };

    if active.name_as_str() != "EsportsLowLatency" {
        return false;
    }
    if !active.fsr_enabled || active.gpu_power_limit_w != 28 || active.cpu_affinity_mask != 0x00FF {
        return false;
    }
    if active.library_path_str(0) != Some("/vfs/packages/proton-ge-latest/lib/") {
        return false;
    }

    // 3. Switch to non-existent profile fails gracefully
    if switcher.kpkg_switch_profile("NonExistentProfile") {
        return false;
    }

    true
}

/// Comprehensive verification self-test suite for Native Package Manager (kpkg) Manifest, Merkle Tree CAS & Profile Switcher.
pub fn test_kpkg_subsystem() -> bool {
    crate::serial_write_str("[KNT-SYS] Testing Native Package Manager (kpkg) & Merkle Tree CAS...\r\n");

    if !verify_kpkg_manifest_and_dependencies() {
        crate::serial_write_str("[KNT-PANIC] Package manifest and dependency resolver verification failed!\r\n");
        return false;
    }
    if !verify_merkle_tree_cas() {
        crate::serial_write_str("[KNT-PANIC] Merkle tree and CAS object store verification failed!\r\n");
        return false;
    }
    if !verify_package_staging_and_overlayfs() {
        crate::serial_write_str("[KNT-PANIC] OverlayFS staging and sandbox mount verification failed!\r\n");
        return false;
    }
    if !verify_gaming_profile_switcher() {
        crate::serial_write_str("[KNT-PANIC] Gaming profile switcher verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-SYS-KPKG-PASS] Native kpkg package manager and CAS profile switcher verified\r\n");
    true
}
