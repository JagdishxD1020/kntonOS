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
