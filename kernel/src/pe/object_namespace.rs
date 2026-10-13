//! Win32 Kernel Object Namespace Directory, Symbolic Link Engine & Object Manager Security.
//!
//! Implements Windows NT hierarchical object directory namespace, symbolic link resolution,
//! object type registration, security descriptor DACL validation, and anti-cheat object hook defense.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::ntdll::{
    nt_error, nt_status_to_str, nt_success, NtStatus, ObjectAttributes, UnicodeString,
    STATUS_BUFFER_TOO_SMALL, STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_NO_MEMORY,
    STATUS_OBJECT_NAME_COLLISION, STATUS_OBJECT_NAME_NOT_FOUND, STATUS_SUCCESS,
};
pub use crate::pe::synch::HANDLE;
pub use crate::pe::token_security::{
    SecurityDescriptor, ACCESS_MASK, DELETE, GENERIC_ALL, GENERIC_EXECUTE, GENERIC_READ,
    GENERIC_WRITE, READ_CONTROL, SID, STANDARD_RIGHTS_ALL, STANDARD_RIGHTS_EXECUTE,
    STANDARD_RIGHTS_READ, STANDARD_RIGHTS_REQUIRED, STANDARD_RIGHTS_WRITE, SYNCHRONIZE, WRITE_DAC,
    WRITE_OWNER,
};

// Additional NT Status codes
pub const STATUS_ACCESS_DENIED: NtStatus = 0xC000_0022;
pub const STATUS_OBJECT_TYPE_MISMATCH: NtStatus = 0xC000_0024;
pub const STATUS_OBJECT_PATH_INVALID: NtStatus = 0xC000_0039;
pub const STATUS_OBJECT_PATH_NOT_FOUND: NtStatus = 0xC000_003A;
pub const STATUS_OBJECT_PATH_SYNTAX_BAD: NtStatus = 0xC000_003B;
pub const STATUS_TOO_MANY_LINKS: NtStatus = 0xC000_0266;
pub const STATUS_CIRCULAR_REFERENCE: NtStatus = 0xC000_0267;
pub const STATUS_DIRECTORY_NOT_EMPTY: NtStatus = 0xC000_0101;

// Object Attributes flags
pub const OBJ_INHERIT: u32 = 0x0000_0002;
pub const OBJ_PERMANENT: u32 = 0x0000_0010;
pub const OBJ_EXCLUSIVE: u32 = 0x0000_0020;
pub const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;
pub const OBJ_OPENIF: u32 = 0x0000_0080;
pub const OBJ_OPENLINK: u32 = 0x0000_0100;
pub const OBJ_KERNEL_HANDLE: u32 = 0x0000_0200;
pub const OBJ_FORCE_ACCESS_CHECK: u32 = 0x0000_0400;
pub const OBJ_DONT_REPARSE: u32 = 0x0000_0800;

// Directory specific access rights
pub const DIRECTORY_QUERY: ACCESS_MASK = 0x0001;
pub const DIRECTORY_TRAVERSE: ACCESS_MASK = 0x0002;
pub const DIRECTORY_CREATE_OBJECT: ACCESS_MASK = 0x0004;
pub const DIRECTORY_CREATE_SUBDIRECTORY: ACCESS_MASK = 0x0008;
pub const DIRECTORY_ALL_ACCESS: ACCESS_MASK = STANDARD_RIGHTS_REQUIRED | 0x000F;

// Symbolic Link specific access rights
pub const SYMBOLIC_LINK_QUERY: ACCESS_MASK = 0x0001;
pub const SYMBOLIC_LINK_SET: ACCESS_MASK = 0x0002;
pub const SYMBOLIC_LINK_ALL_ACCESS: ACCESS_MASK = STANDARD_RIGHTS_REQUIRED | 0x0001;

// Maximum symbolic link resolution depth to avoid infinite loops
pub const MAX_SYMBOLIC_LINK_DEPTH: usize = 32;

/// Enumeration of supported NT object types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectTypeKind {
    Directory,
    SymbolicLink,
    Device,
    Section,
    Event,
    Mutant,
    Semaphore,
    Port,
    Key,
    Job,
    File,
    Custom,
}

/// Caller integrity and trust level for anti-cheat verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProcessTrustLevel {
    Untrusted = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    System = 4,
    ProtectedGame = 5,
}


/// NT Generic Mapping structure defining translation from generic to specific access masks.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenericMapping {
    pub GenericRead: ACCESS_MASK,
    pub GenericWrite: ACCESS_MASK,
    pub GenericExecute: ACCESS_MASK,
    pub GenericAll: ACCESS_MASK,
}

impl GenericMapping {
    pub fn new(read: ACCESS_MASK, write: ACCESS_MASK, exec: ACCESS_MASK, all: ACCESS_MASK) -> Self {
        Self {
            GenericRead: read,
            GenericWrite: write,
            GenericExecute: exec,
            GenericAll: all,
        }
    }

    /// Translates generic access rights (GENERIC_READ, etc.) to object-specific access rights.
    pub fn map_mask(&self, mask: &mut ACCESS_MASK) {
        if (*mask & GENERIC_READ) != 0 {
            *mask &= !GENERIC_READ;
            *mask |= self.GenericRead;
        }
        if (*mask & GENERIC_WRITE) != 0 {
            *mask &= !GENERIC_WRITE;
            *mask |= self.GenericWrite;
        }
        if (*mask & GENERIC_EXECUTE) != 0 {
            *mask &= !GENERIC_EXECUTE;
            *mask |= self.GenericExecute;
        }
        if (*mask & GENERIC_ALL) != 0 {
            *mask &= !GENERIC_ALL;
            *mask |= self.GenericAll;
        }
    }
}

/// Standard object type metadata registered in the object manager.
pub struct ObjectType {
    pub name: String,
    pub type_kind: ObjectTypeKind,
    pub generic_mapping: GenericMapping,
    pub default_security_descriptor: Option<SecurityDescriptor>,
    pub total_objects: AtomicU32,
    pub total_handles: AtomicU32,
}

impl ObjectType {
    pub fn new(name: &str, kind: ObjectTypeKind, mapping: GenericMapping) -> Self {
        Self {
            name: name.to_string(),
            type_kind: kind,
            generic_mapping: mapping,
            default_security_descriptor: None,
            total_objects: AtomicU32::new(0),
            total_handles: AtomicU32::new(0),
        }
    }
}

/// Caller security token credentials for DACL evaluation and audit checks.
#[derive(Debug, Clone)]
pub struct CallerSecurityContext {
    pub process_id: u64,
    pub client_sids: Vec<SID>,
    pub trust_level: ProcessTrustLevel,
    pub is_kernel: bool,
}

impl CallerSecurityContext {
    pub fn new_user(pid: u64, user_sid: SID) -> Self {
        Self {
            process_id: pid,
            client_sids: alloc::vec![user_sid, SID::world_sid()],
            trust_level: ProcessTrustLevel::Medium,
            is_kernel: false,
        }
    }

    pub fn new_system(pid: u64) -> Self {
        Self {
            process_id: pid,
            client_sids: alloc::vec![
                SID::local_system_sid(),
                SID::administrators_sid(),
                SID::world_sid(),
            ],
            trust_level: ProcessTrustLevel::System,
            is_kernel: true,
        }
    }

    pub fn new_untrusted(pid: u64) -> Self {
        Self {
            process_id: pid,
            client_sids: alloc::vec![SID::world_sid()],
            trust_level: ProcessTrustLevel::Untrusted,
            is_kernel: false,
        }
    }
}

/// Common header present on all NT kernel objects.
#[derive(Clone)]
pub struct ObjectHeader {
    pub name: String,
    pub type_kind: ObjectTypeKind,
    pub attributes: u32,
    pub pointer_count: usize,
    pub handle_count: usize,
    pub security_descriptor: Option<SecurityDescriptor>,
    pub is_permanent: bool,
    pub is_protected: bool,
    pub min_trust_level: ProcessTrustLevel,
}

/// Internal payload for directory nodes.
pub struct ObjectDirectory {
    pub entries: BTreeMap<String, Arc<Spinlock<NtObject>>>,
    pub is_protected_system_dir: bool,
}

impl ObjectDirectory {
    pub fn new(is_protected: bool) -> Self {
        Self {
            entries: BTreeMap::new(),
            is_protected_system_dir: is_protected,
        }
    }
}

/// Internal payload for symbolic link nodes.
pub struct SymbolicLinkObject {
    pub target_path: String,
    pub creation_time: u64,
}

/// Payload variants for NT objects.
pub enum ObjectPayload {
    Directory(ObjectDirectory),
    SymbolicLink(SymbolicLinkObject),
    Device { device_type: u32, device_number: u32 },
    Section { size: u64, section_handle: u64 },
    Custom { tag: u32 },
}

/// Complete in-memory NT kernel object.
pub struct NtObject {
    pub header: ObjectHeader,
    pub payload: ObjectPayload,
}

impl NtObject {
    pub fn new_directory(
        name: &str,
        attributes: u32,
        is_protected: bool,
        min_trust: ProcessTrustLevel,
    ) -> Self {
        Self {
            header: ObjectHeader {
                name: name.to_string(),
                type_kind: ObjectTypeKind::Directory,
                attributes,
                pointer_count: 1,
                handle_count: 0,
                security_descriptor: None,
                is_permanent: (attributes & OBJ_PERMANENT) != 0,
                is_protected,
                min_trust_level: min_trust,
            },
            payload: ObjectPayload::Directory(ObjectDirectory {
                entries: BTreeMap::new(),
                is_protected_system_dir: is_protected,
            }),
        }
    }

    pub fn new_symbolic_link(
        name: &str,
        target_path: &str,
        attributes: u32,
        is_protected: bool,
        min_trust: ProcessTrustLevel,
    ) -> Self {
        Self {
            header: ObjectHeader {
                name: name.to_string(),
                type_kind: ObjectTypeKind::SymbolicLink,
                attributes,
                pointer_count: 1,
                handle_count: 0,
                security_descriptor: None,
                is_permanent: (attributes & OBJ_PERMANENT) != 0,
                is_protected,
                min_trust_level: min_trust,
            },
            payload: ObjectPayload::SymbolicLink(SymbolicLinkObject {
                target_path: target_path.to_string(),
                creation_time: 0,
            }),
        }
    }
}

/// Handle table entry tracking an open object.
#[derive(Clone)]
pub struct HandleEntry {
    pub object: Arc<Spinlock<NtObject>>,
    pub granted_access: ACCESS_MASK,
    pub process_id: u64,
}

/// Normalizes path by stripping redundant delimiters and handling case insensitivity.
pub fn normalize_nt_path(path: &str) -> String {
    let mut normalized = String::new();
    let mut prev_slash = false;

    for ch in path.chars() {
        if ch == '\\' || ch == '/' {
            if !prev_slash {
                normalized.push('\\');
                prev_slash = true;
            }
        } else {
            normalized.push(ch);
            prev_slash = false;
        }
    }

    if normalized.len() > 1 && normalized.ends_with('\\') {
        normalized.pop();
    }

    normalized
}

/// Splits a normalized NT path into individual component names.
pub fn split_nt_path(path: &str) -> Vec<String> {
    let norm = normalize_nt_path(path);
    norm.split('\\')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}


/// Types of anti-cheat namespace tampering violations detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntiCheatViolationKind {
    None,
    RogueKnownDllInjection,
    ProtectedDirectoryHook,
    SymbolicLinkHijack,
    UnauthorizedDaclModification,
    RogueObjectCreation,
}

/// Diagnostic record capturing an anti-cheat tampering attempt.
#[derive(Debug, Clone)]
pub struct TamperViolationRecord {
    pub timestamp: u64,
    pub process_id: u64,
    pub caller_trust_level: ProcessTrustLevel,
    pub target_path: String,
    pub violation_kind: AntiCheatViolationKind,
}

/// Anti-cheat namespace hook protection and rogue object injection defense governor.
pub struct AntiCheatNamespaceGuard {
    pub protected_prefixes: Vec<String>,
    pub tamper_records: Vec<TamperViolationRecord>,
    pub tamper_count: AtomicU32,
    pub strict_enforcement: AtomicBool,
}

impl AntiCheatNamespaceGuard {
    pub fn new() -> Self {
        Self {
            protected_prefixes: alloc::vec![
                String::from("\\knowndlls"),
                String::from("\\kernelobjects"),
                String::from("\\device\\physicalmemory"),
            ],
            tamper_records: Vec::new(),
            tamper_count: AtomicU32::new(0),
            strict_enforcement: AtomicBool::new(true),
        }
    }

    /// Evaluates whether an object creation or modification request in a namespace path violates anti-cheat policy.
    pub fn verify_operation(
        &mut self,
        target_path: &str,
        caller: &CallerSecurityContext,
        is_symlink: bool,
    ) -> Result<(), NtStatus> {
        let norm_path = normalize_nt_path(target_path).to_ascii_lowercase();

        // 1. Rogue KnownDlls injection defense: only System trust can add/alter \KnownDlls
        if norm_path.starts_with("\\knowndlls") {
            if caller.trust_level < ProcessTrustLevel::System {
                self.record_violation(
                    caller,
                    target_path,
                    AntiCheatViolationKind::RogueKnownDllInjection,
                );
                return Err(STATUS_ACCESS_DENIED);
            }
        }

        // 2. KernelObjects directory protection
        if norm_path.starts_with("\\kernelobjects") {
            if caller.trust_level < ProcessTrustLevel::System {
                self.record_violation(
                    caller,
                    target_path,
                    AntiCheatViolationKind::ProtectedDirectoryHook,
                );
                return Err(STATUS_ACCESS_DENIED);
            }
        }

        // 3. Symbolic link hijacking check in BaseNamedObjects or critical roots
        if is_symlink {
            if norm_path.starts_with("\\basenamedobjects\\anticheat")
                || norm_path.starts_with("\\basenamedobjects\\system")
            {
                if caller.trust_level < ProcessTrustLevel::High {
                    self.record_violation(
                        caller,
                        target_path,
                        AntiCheatViolationKind::SymbolicLinkHijack,
                    );
                    return Err(STATUS_ACCESS_DENIED);
                }
            }
        }

        // 4. Untrusted process restriction on BaseNamedObjects protected game prefixes
        if caller.trust_level == ProcessTrustLevel::Untrusted {
            if norm_path.starts_with("\\basenamedobjects") {
                // Untrusted caller attempted rogue injection
                self.record_violation(
                    caller,
                    target_path,
                    AntiCheatViolationKind::RogueObjectCreation,
                );
                return Err(STATUS_ACCESS_DENIED);
            }
        }

        Ok(())
    }

    /// Records an anti-cheat tampering violation event.
    pub fn record_violation(
        &mut self,
        caller: &CallerSecurityContext,
        target_path: &str,
        kind: AntiCheatViolationKind,
    ) {
        self.tamper_count.fetch_add(1, Ordering::SeqCst);
        let record = TamperViolationRecord {
            timestamp: 0,
            process_id: caller.process_id,
            caller_trust_level: caller.trust_level,
            target_path: target_path.to_string(),
            violation_kind: kind,
        };
        self.tamper_records.push(record);
    }
}

/// Central Object Manager and Namespace Directory Tree Governor.
pub struct ObjectNamespaceManager {
    pub root_directory: Arc<Spinlock<NtObject>>,
    pub object_types: BTreeMap<String, Arc<ObjectType>>,
    pub handle_table: BTreeMap<HANDLE, HandleEntry>,
    pub next_handle: usize,
    pub anti_cheat_guard: AntiCheatNamespaceGuard,
}

impl ObjectNamespaceManager {
    /// Constructs a fully populated standard NT object directory namespace.
    pub fn new() -> Self {
        let root = Arc::new(Spinlock::new(NtObject::new_directory(
            "",
            OBJ_PERMANENT | OBJ_CASE_INSENSITIVE,
            false,
            ProcessTrustLevel::Untrusted,
        )));

let mut mgr = Self {
            root_directory: root,
            object_types: BTreeMap::new(),
            handle_table: BTreeMap::new(),
            next_handle: 0x100,
            anti_cheat_guard: AntiCheatNamespaceGuard::new(),
        };

        mgr.register_standard_types();
        // Initialize standard NT hierarchical directories
        mgr.init_standard_directories();
        mgr
    }

    /// Initializes standard NT namespace hierarchy.
    fn init_standard_directories(&mut self) {
        let std_dirs = [
            ("Device", false, ProcessTrustLevel::Untrusted),
            ("DosDevices", false, ProcessTrustLevel::Untrusted),
            ("BaseNamedObjects", false, ProcessTrustLevel::Untrusted),
            ("KernelObjects", true, ProcessTrustLevel::System),
            ("Session", false, ProcessTrustLevel::Untrusted),
            ("KnownDlls", true, ProcessTrustLevel::System),
            ("GLOBAL??", false, ProcessTrustLevel::Untrusted),
        ];

        for (name, is_protected, min_trust) in std_dirs {
            let _ = self.create_subdirectory_internal(&self.root_directory.clone(), name, is_protected, min_trust);
        }

        // Initialize \Session\0 and \Session\0\BaseNamedObjects
        if let Ok(session_dir) = self.lookup_entry_direct(&self.root_directory.clone(), "Session") {
            let session_0 = self.create_subdirectory_internal(&session_dir, "0", false, ProcessTrustLevel::Untrusted);
            if let Ok(s0) = session_0 {
                let _ = self.create_subdirectory_internal(&s0, "BaseNamedObjects", false, ProcessTrustLevel::Untrusted);
            }
        }
    }

    /// Creates an internal subdirectory node.
    pub fn create_subdirectory_internal(
        &self,
        parent: &Arc<Spinlock<NtObject>>,
        name: &str,
        is_protected: bool,
        min_trust: ProcessTrustLevel,
    ) -> Result<Arc<Spinlock<NtObject>>, NtStatus> {
        let mut p = parent.lock();
        if let ObjectPayload::Directory(ref mut dir) = p.payload {
            let lookup_key = name.to_ascii_lowercase();
            if dir.entries.contains_key(&lookup_key) {
                return Err(STATUS_OBJECT_NAME_COLLISION);
            }

            let new_dir = Arc::new(Spinlock::new(NtObject::new_directory(
                name,
                OBJ_PERMANENT | OBJ_CASE_INSENSITIVE,
                is_protected,
                min_trust,
            )));
            dir.entries.insert(lookup_key, new_dir.clone());
            Ok(new_dir)
        } else {
            Err(STATUS_OBJECT_TYPE_MISMATCH)
        }
    }

    /// Lookups direct child entry in a directory by name (case-insensitive).
    pub fn lookup_entry_direct(
        &self,
        dir: &Arc<Spinlock<NtObject>>,
        name: &str,
    ) -> Result<Arc<Spinlock<NtObject>>, NtStatus> {
        let d = dir.lock();
        if let ObjectPayload::Directory(ref directory) = d.payload {
            let lookup_key = name.to_ascii_lowercase();
            directory
                .entries
                .get(&lookup_key)
                .cloned()
                .ok_or(STATUS_OBJECT_NAME_NOT_FOUND)
        } else {
            Err(STATUS_OBJECT_TYPE_MISMATCH)
        }
    }

    /// Traverses a hierarchical path from root without resolving terminal symlinks.
    pub fn traverse_directory_path(
        &self,
        path: &str,
    ) -> Result<(Arc<Spinlock<NtObject>>, Option<String>), NtStatus> {
        let components = split_nt_path(path);
        if components.is_empty() {
            return Ok((self.root_directory.clone(), None));
        }

        let mut current = self.root_directory.clone();

        for (i, comp) in components.iter().enumerate() {
            let is_last = i == components.len() - 1;
            if is_last {
                return Ok((current, Some(comp.clone())));
            }

            let next = self.lookup_entry_direct(&current, comp)?;
            current = next;
        }

        Ok((current, None))
    }

    /// Resolves an object by full absolute NT path directly.
    pub fn resolve_object_direct(&self, path: &str) -> Result<Arc<Spinlock<NtObject>>, NtStatus> {
        let components = split_nt_path(path);
        if components.is_empty() {
            return Ok(self.root_directory.clone());
        }

        let mut current = self.root_directory.clone();
        for comp in &components {
            let next = self.lookup_entry_direct(&current, comp)?;
            current = next;
        }

        Ok(current)
    }

    /// Opens an existing directory by path.
    pub fn open_directory(
        &mut self,
        path: &str,
        desired_access: ACCESS_MASK,
        process_id: u64,
    ) -> Result<HANDLE, NtStatus> {
        let obj = self.resolve_object_direct(path)?;
        {
            let o = obj.lock();
            if o.header.type_kind != ObjectTypeKind::Directory {
                return Err(STATUS_OBJECT_TYPE_MISMATCH);
            }
        }
        Ok(self.allocate_handle(obj, desired_access, process_id))
    }

    /// Creates a directory object in the namespace.
    pub fn create_directory_secure(
        &mut self,
        path: &str,
        desired_access: ACCESS_MASK,
        caller: &CallerSecurityContext,
    ) -> Result<HANDLE, NtStatus> {
        self.anti_cheat_guard.verify_operation(path, caller, false)?;
        let (parent_dir, leaf_name) = self.traverse_directory_path(path)?;
        let name = leaf_name.ok_or(STATUS_OBJECT_PATH_INVALID)?;
        let new_dir = self.create_subdirectory_internal(
            &parent_dir,
            &name,
            false,
            caller.trust_level,
        )?;
        Ok(self.allocate_handle(new_dir, desired_access, caller.process_id))
    }

    pub fn create_directory(
        &mut self,
        path: &str,
        desired_access: ACCESS_MASK,
        process_id: u64,
    ) -> Result<HANDLE, NtStatus> {
        let caller = CallerSecurityContext::new_system(process_id);
        self.create_directory_secure(path, desired_access, &caller)
    }

    pub fn create_directory_old(
        &mut self,
        path: &str,
        desired_access: ACCESS_MASK,
        process_id: u64,
    ) -> Result<HANDLE, NtStatus> {
        let (parent_dir, leaf_name) = self.traverse_directory_path(path)?;
        let name = leaf_name.ok_or(STATUS_OBJECT_PATH_INVALID)?;
        let new_dir = self.create_subdirectory_internal(
            &parent_dir,
            &name,
            false,
            ProcessTrustLevel::Untrusted,
        )?;
        Ok(self.allocate_handle(new_dir, desired_access, process_id))
    }


    /// Creates a symbolic link in the object namespace.
    pub fn create_symbolic_link_secure(
        &mut self,
        link_path: &str,
        target_path: &str,
        desired_access: ACCESS_MASK,
        caller: &CallerSecurityContext,
        is_protected: bool,
    ) -> Result<HANDLE, NtStatus> {
        self.anti_cheat_guard.verify_operation(link_path, caller, true)?;
        let (parent_dir, leaf_name) = self.traverse_directory_path(link_path)?;
        let name = leaf_name.ok_or(STATUS_OBJECT_PATH_INVALID)?;

        let mut p = parent_dir.lock();
        if let ObjectPayload::Directory(ref mut dir) = p.payload {
            let lookup_key = name.to_ascii_lowercase();
            if dir.entries.contains_key(&lookup_key) {
                return Err(STATUS_OBJECT_NAME_COLLISION);
            }

            let link_obj = Arc::new(Spinlock::new(NtObject::new_symbolic_link(
                &name,
                target_path,
                OBJ_CASE_INSENSITIVE,
                is_protected,
                caller.trust_level,
            )));
            dir.entries.insert(lookup_key, link_obj.clone());
            drop(p);

            Ok(self.allocate_handle(link_obj, desired_access, caller.process_id))
        } else {
            Err(STATUS_OBJECT_TYPE_MISMATCH)
        }
    }

    pub fn create_symbolic_link(
        &mut self,
        link_path: &str,
        target_path: &str,
        desired_access: ACCESS_MASK,
        process_id: u64,
        is_protected: bool,
        min_trust: ProcessTrustLevel,
    ) -> Result<HANDLE, NtStatus> {
        let caller = CallerSecurityContext {
            process_id,
            client_sids: alloc::vec![SID::world_sid()],
            trust_level: min_trust,
            is_kernel: min_trust >= ProcessTrustLevel::System,
        };
        self.create_symbolic_link_secure(link_path, target_path, desired_access, &caller, is_protected)
    }

    pub fn create_symbolic_link_legacy(
        &mut self,
        link_path: &str,
        target_path: &str,
        desired_access: ACCESS_MASK,
        process_id: u64,
        is_protected: bool,
        min_trust: ProcessTrustLevel,
    ) -> Result<HANDLE, NtStatus> {
        let (parent_dir, leaf_name) = self.traverse_directory_path(link_path)?;
        let name = leaf_name.ok_or(STATUS_OBJECT_PATH_INVALID)?;

        let mut p = parent_dir.lock();
        if let ObjectPayload::Directory(ref mut dir) = p.payload {
            let lookup_key = name.to_ascii_lowercase();
            if dir.entries.contains_key(&lookup_key) {
                return Err(STATUS_OBJECT_NAME_COLLISION);
            }

            let link_obj = Arc::new(Spinlock::new(NtObject::new_symbolic_link(
                &name,
                target_path,
                OBJ_CASE_INSENSITIVE,
                is_protected,
                min_trust,
            )));
            dir.entries.insert(lookup_key, link_obj.clone());
            drop(p);

            Ok(self.allocate_handle(link_obj, desired_access, process_id))
        } else {
            Err(STATUS_OBJECT_TYPE_MISMATCH)
        }
    }

    /// Opens an existing symbolic link object without following the link.
    pub fn open_symbolic_link(
        &mut self,
        link_path: &str,
        desired_access: ACCESS_MASK,
        process_id: u64,
    ) -> Result<HANDLE, NtStatus> {
        let obj = self.resolve_path(link_path, false)?;
        {
            let o = obj.lock();
            if o.header.type_kind != ObjectTypeKind::SymbolicLink {
                return Err(STATUS_OBJECT_TYPE_MISMATCH);
            }
        }
        Ok(self.allocate_handle(obj, desired_access, process_id))
    }

    /// Queries the target string of a symbolic link by handle.
    pub fn query_symbolic_link(&self, handle: HANDLE) -> Result<String, NtStatus> {
        let entry = self.handle_table.get(&handle).ok_or(STATUS_INVALID_HANDLE)?;
        let obj = entry.object.lock();
        if let ObjectPayload::SymbolicLink(ref symlink) = obj.payload {
            Ok(symlink.target_path.clone())
        } else {
            Err(STATUS_OBJECT_TYPE_MISMATCH)
        }
    }

    /// Resolves an NT path completely, resolving intermediate and optional terminal symbolic links.
    /// Incorporates cycle detection and aborts if depth exceeds MAX_SYMBOLIC_LINK_DEPTH.
    pub fn resolve_path(
        &self,
        initial_path: &str,
        follow_terminal_link: bool,
    ) -> Result<Arc<Spinlock<NtObject>>, NtStatus> {
        let mut current_path = normalize_nt_path(initial_path);
        let mut depth: usize = 0;

        loop {
            let components = split_nt_path(&current_path);
            if components.is_empty() {
                return Ok(self.root_directory.clone());
            }

            let mut current = self.root_directory.clone();
            let num_components = components.len();
            let mut reparsed = false;

            for (i, comp) in components.iter().enumerate() {
                let is_last = i == num_components - 1;
                let next = self.lookup_entry_direct(&current, comp)?;

                let (is_symlink, target_path) = {
                    let locked = next.lock();
                    match locked.payload {
                        ObjectPayload::SymbolicLink(ref sl) => (true, sl.target_path.clone()),
                        _ => (false, String::new()),
                    }
                };

                if is_symlink {
                    if is_last && !follow_terminal_link {
                        return Ok(next);
                    }

                    depth += 1;
                    if depth > MAX_SYMBOLIC_LINK_DEPTH {
                        return Err(STATUS_TOO_MANY_LINKS);
                    }

                    // Suffix components after this symbolic link
                    let suffix = if !is_last {
                        let mut s = String::new();
                        for rem in &components[i + 1..] {
                            s.push('\\');
                            s.push_str(rem);
                        }
                        s
                    } else {
                        String::new()
                    };

                    let mut expanded = String::new();
                    if target_path.starts_with('\\') {
                        expanded.push_str(&target_path);
                    } else {
                        // Relative link: prepend directory path traversed so far
                        expanded.push_str("\\");
                        for prev in &components[0..i] {
                            expanded.push_str(prev);
                            expanded.push('\\');
                        }
                        expanded.push_str(&target_path);
                    }

                    expanded.push_str(&suffix);
                    current_path = normalize_nt_path(&expanded);
                    reparsed = true;
                    break;
                }

                current = next;
            }

            if !reparsed {
                return Ok(current);
            }
        }
    }

    /// Resolves an NT path and returns the final canonical path string.
    pub fn resolve_path_to_target_string(&self, initial_path: &str) -> Result<String, NtStatus> {
        let mut current_path = normalize_nt_path(initial_path);
        let mut depth: usize = 0;

        loop {
            let components = split_nt_path(&current_path);
            if components.is_empty() {
                return Ok(String::from("\\"));
            }

            let mut current = self.root_directory.clone();
            let num_components = components.len();
            let mut reparsed = false;

            for (i, comp) in components.iter().enumerate() {
                let is_last = i == num_components - 1;
                let next = match self.lookup_entry_direct(&current, comp) {
                    Ok(n) => n,
                    Err(e) => {
                        // If path component not found, but we already resolved links so far, return current_path
                        if depth > 0 {
                            return Ok(current_path);
                        }
                        return Err(e);
                    }
                };

                let (is_symlink, target_path) = {
                    let locked = next.lock();
                    match locked.payload {
                        ObjectPayload::SymbolicLink(ref sl) => (true, sl.target_path.clone()),
                        _ => (false, String::new()),
                    }
                };

                if is_symlink {
                    depth += 1;
                    if depth > MAX_SYMBOLIC_LINK_DEPTH {
                        return Err(STATUS_TOO_MANY_LINKS);
                    }

                    let suffix = if !is_last {
                        let mut s = String::new();
                        for rem in &components[i + 1..] {
                            s.push('\\');
                            s.push_str(rem);
                        }
                        s
                    } else {
                        String::new()
                    };

                    let mut expanded = String::new();
                    if target_path.starts_with('\\') {
                        expanded.push_str(&target_path);
                    } else {
                        expanded.push_str("\\");
                        for prev in &components[0..i] {
                            expanded.push_str(prev);
                            expanded.push('\\');
                        }
                        expanded.push_str(&target_path);
                    }

                    expanded.push_str(&suffix);
                    current_path = normalize_nt_path(&expanded);
                    reparsed = true;
                    break;
                }

                current = next;
            }

            if !reparsed {
                return Ok(current_path);
            }
        }
    }

    /// Allocates an internal handle for an object.
    pub fn allocate_handle(
        &mut self,
        object: Arc<Spinlock<NtObject>>,
        granted_access: ACCESS_MASK,
        process_id: u64,
    ) -> HANDLE {
        let handle = self.next_handle;
        self.next_handle += 4;

        {
            let mut obj = object.lock();
            obj.header.handle_count += 1;
            obj.header.pointer_count += 1;
        }

        self.handle_table.insert(
            handle,
            HandleEntry {
                object,
                granted_access,
                process_id,
            },
        );

        handle
    }


    /// Registers a standard object type with its generic mapping.
    pub fn register_object_type(&mut self, obj_type: ObjectType) {
        let key = obj_type.name.to_ascii_lowercase();
        self.object_types.insert(key, Arc::new(obj_type));
    }

    /// Registers the default system object types.
    fn register_standard_types(&mut self) {
        // Directory generic mapping
        self.register_object_type(ObjectType::new(
            "Directory",
            ObjectTypeKind::Directory,
            GenericMapping::new(
                STANDARD_RIGHTS_READ | DIRECTORY_QUERY | DIRECTORY_TRAVERSE,
                STANDARD_RIGHTS_WRITE | DIRECTORY_CREATE_OBJECT | DIRECTORY_CREATE_SUBDIRECTORY,
                STANDARD_RIGHTS_EXECUTE | DIRECTORY_TRAVERSE,
                DIRECTORY_ALL_ACCESS,
            ),
        ));

        // SymbolicLink generic mapping
        self.register_object_type(ObjectType::new(
            "SymbolicLink",
            ObjectTypeKind::SymbolicLink,
            GenericMapping::new(
                STANDARD_RIGHTS_READ | SYMBOLIC_LINK_QUERY,
                STANDARD_RIGHTS_WRITE | SYMBOLIC_LINK_SET,
                STANDARD_RIGHTS_EXECUTE | SYMBOLIC_LINK_QUERY,
                SYMBOLIC_LINK_ALL_ACCESS,
            ),
        ));

        // Device, Section, Event
        self.register_object_type(ObjectType::new(
            "Device",
            ObjectTypeKind::Device,
            GenericMapping::new(STANDARD_RIGHTS_READ, STANDARD_RIGHTS_WRITE, STANDARD_RIGHTS_EXECUTE, STANDARD_RIGHTS_ALL),
        ));

        self.register_object_type(ObjectType::new(
            "Section",
            ObjectTypeKind::Section,
            GenericMapping::new(0x0004, 0x0002, 0x0008, 0x000F | STANDARD_RIGHTS_REQUIRED),
        ));
    }

    /// Retrieves the generic mapping for a given object type.
    pub fn get_generic_mapping(&self, kind: ObjectTypeKind) -> GenericMapping {
        match kind {
            ObjectTypeKind::Directory => GenericMapping::new(
                STANDARD_RIGHTS_READ | DIRECTORY_QUERY | DIRECTORY_TRAVERSE,
                STANDARD_RIGHTS_WRITE | DIRECTORY_CREATE_OBJECT | DIRECTORY_CREATE_SUBDIRECTORY,
                STANDARD_RIGHTS_EXECUTE | DIRECTORY_TRAVERSE,
                DIRECTORY_ALL_ACCESS,
            ),
            ObjectTypeKind::SymbolicLink => GenericMapping::new(
                STANDARD_RIGHTS_READ | SYMBOLIC_LINK_QUERY,
                STANDARD_RIGHTS_WRITE | SYMBOLIC_LINK_SET,
                STANDARD_RIGHTS_EXECUTE | SYMBOLIC_LINK_QUERY,
                SYMBOLIC_LINK_ALL_ACCESS,
            ),
            _ => GenericMapping::new(
                STANDARD_RIGHTS_READ,
                STANDARD_RIGHTS_WRITE,
                STANDARD_RIGHTS_EXECUTE,
                STANDARD_RIGHTS_ALL,
            ),
        }
    }

    /// Evaluates requested access mask against object's DACL and caller's credentials.
    pub fn validate_access(
        &self,
        object: &NtObject,
        caller: &CallerSecurityContext,
        requested_access: ACCESS_MASK,
    ) -> Result<ACCESS_MASK, NtStatus> {
        let mapping = self.get_generic_mapping(object.header.type_kind);
        let mut mapped_access = requested_access;
        mapping.map_mask(&mut mapped_access);

        // Kernel or System callers automatically bypass DACL
        if caller.is_kernel || caller.trust_level >= ProcessTrustLevel::System {
            return Ok(mapped_access);
        }

        // If security descriptor is set, evaluate DACL
        if let Some(ref sd) = object.header.security_descriptor {
            if !sd.evaluate_access(&caller.client_sids, mapped_access) {
                return Err(STATUS_ACCESS_DENIED);
            }
        }

        Ok(mapped_access)
    }

    /// Sets the security descriptor on an object identified by path.
    pub fn set_object_security_descriptor(
        &self,
        path: &str,
        sd: SecurityDescriptor,
    ) -> Result<(), NtStatus> {
        let obj = self.resolve_path(path, true)?;
        let mut o = obj.lock();
        o.header.security_descriptor = Some(sd);
        Ok(())
    }


    /// Queries the total count of anti-cheat violations and the most recent record.
    pub fn get_anti_cheat_stats(&self) -> (u32, Option<TamperViolationRecord>) {
        let count = self.anti_cheat_guard.tamper_count.load(Ordering::SeqCst);
        let last = self.anti_cheat_guard.tamper_records.last().cloned();
        (count, last)
    }

    /// Returns true if any anti-cheat tampering violations were recorded.
    pub fn is_namespace_tamper_detected(&self) -> bool {
        self.anti_cheat_guard.tamper_count.load(Ordering::SeqCst) > 0
    }

    /// Closes an open handle.
    pub fn close_handle(&mut self, handle: HANDLE) -> bool {
        if let Some(entry) = self.handle_table.remove(&handle) {
            let mut obj = entry.object.lock();
            if obj.header.handle_count > 0 {
                obj.header.handle_count -= 1;
            }
            if obj.header.pointer_count > 0 {
                obj.header.pointer_count -= 1;
            }
            true
        } else {
            false
        }
    }
}

/// Global system-wide NT Object Namespace Manager instance.
static GLOBAL_OBJECT_NAMESPACE: Spinlock<Option<ObjectNamespaceManager>> = Spinlock::new(None);

/// Initializes the global NT object namespace subsystem.
pub fn init_object_namespace() {
    let mut lock = GLOBAL_OBJECT_NAMESPACE.lock();
    if lock.is_none() {
        *lock = Some(ObjectNamespaceManager::new());
    }
}

/// Accessor for global object namespace manager.
pub fn with_object_namespace<F, R>(f: F) -> R
where
    F: FnOnce(&mut ObjectNamespaceManager) -> R,
{
    init_object_namespace();
    let mut lock = GLOBAL_OBJECT_NAMESPACE.lock();
    f(lock.as_mut().unwrap())
}
