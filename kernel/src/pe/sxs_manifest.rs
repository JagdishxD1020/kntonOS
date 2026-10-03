//! Win32 Side-by-Side (SxS) Assembly Isolation & Manifest Activation Context Engine.
//!
//! Provides activation context lifecycle management (CreateActCtx, ActivateActCtx, DeactivateActCtx),
//! XML manifest parsing for assembly dependencies and binding redirects, and per-thread activation
//! stack tracking for isolating game mods and graphics overlays from runtime library version conflicts.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
pub use crate::pe::srw_futex::{DWORD, PVOID, SIZE_T};
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

pub const ACTCTX_FLAG_PROCESSOR_ARCHITECTURE_VALID: DWORD = 0x0000_0001;
pub const ACTCTX_FLAG_LANGID_VALID: DWORD = 0x0000_0002;
pub const ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID: DWORD = 0x0000_0004;
pub const ACTCTX_FLAG_RESOURCE_NAME_VALID: DWORD = 0x0000_0008;
pub const ACTCTX_FLAG_SET_PROCESS_DEFAULT: DWORD = 0x0000_0010;
pub const ACTCTX_FLAG_APPLICATION_NAME_VALID: DWORD = 0x0000_0020;
pub const ACTCTX_FLAG_SOURCE_IS_ASSEMBLYREF: DWORD = 0x0000_0040;
pub const ACTCTX_FLAG_HMODULE_VALID: DWORD = 0x0000_0080;

pub const INVALID_ACTCTX: HANDLE = 0xFFFF_FFFF_FFFF_FFFF as HANDLE;

pub type ULONG_PTR = usize;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ACTCTX_A {
    pub cbSize: DWORD,
    pub dwFlags: DWORD,
    pub lpSource: *const u8,
    pub wProcessorArchitecture: u16,
    pub wLangId: u16,
    pub lpAssemblyDirectory: *const u8,
    pub lpResourceName: *const u8,
    pub lpApplicationName: *const u8,
    pub hModule: HANDLE,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ACTCTX_W {
    pub cbSize: DWORD,
    pub dwFlags: DWORD,
    pub lpSource: *const u16,
    pub wProcessorArchitecture: u16,
    pub wLangId: u16,
    pub lpAssemblyDirectory: *const u16,
    pub lpResourceName: *const u16,
    pub lpApplicationName: *const u16,
    pub hModule: HANDLE,
}

/// Description of a dependent assembly required by an application or mod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependentAssembly {
    pub name: String,
    pub version: String,
    pub processor_architecture: String,
    pub public_key_token: String,
    pub language: String,
}

/// Dynamic binding redirect rule mapping an old version range to a new runtime version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingRedirect {
    pub name: String,
    pub old_version_low: String,
    pub old_version_high: String,
    pub new_version: String,
}

/// Activation context descriptor encapsulating manifest rules and DLL redirections.
#[derive(Clone, Debug)]
pub struct ActivationContextInner {
    pub id: HANDLE,
    pub source_path: String,
    pub assembly_name: String,
    pub assembly_version: String,
    pub dependent_assemblies: Vec<DependentAssembly>,
    pub binding_redirects: Vec<BindingRedirect>,
    pub dll_redirections: BTreeMap<String, String>,
    pub ref_count: u32,
    pub is_process_default: bool,
}

static NEXT_ACTCTX_ID: AtomicU64 = AtomicU64::new(0x8000);
static ACTIVATION_CONTEXTS: Spinlock<BTreeMap<HANDLE, ActivationContextInner>> = Spinlock::new(BTreeMap::new());
static PROCESS_DEFAULT_ACTCTX: Spinlock<HANDLE> = Spinlock::new(0);
static THREAD_ACTCTX_STACKS: Spinlock<BTreeMap<u32, Vec<HANDLE>>> = Spinlock::new(BTreeMap::new());

/// Extracts the value of an attribute inside an XML tag string.
fn extract_xml_attr(tag: &str, attr_name: &str) -> Option<String> {
    let pattern = String::from(attr_name) + "=\"";
    if let Some(pos) = tag.find(&pattern) {
        let rest = &tag[pos + pattern.len()..];
        if let Some(end_pos) = rest.find('"') {
            return Some(String::from(&rest[..end_pos]));
        }
    }
    None
}

/// Parses an XML assembly manifest string, extracting assembly identity and dependencies.
pub fn parse_manifest_xml(xml: &str) -> (String, String, Vec<DependentAssembly>, Vec<BindingRedirect>) {
    let mut assembly_name = String::new();
    let mut assembly_version = String::new();
    let mut dependent_assemblies = Vec::new();
    let mut binding_redirects = Vec::new();

    let mut cursor = 0;
    while let Some(start) = xml[cursor..].find('<') {
        let tag_start = cursor + start;
        if let Some(end) = xml[tag_start..].find('>') {
            let tag_end = tag_start + end;
            let tag_content = &xml[tag_start + 1..tag_end];
            cursor = tag_end + 1;

            if tag_content.starts_with("assemblyIdentity") {
                if let Some(name) = extract_xml_attr(tag_content, "name") {
                    let version = extract_xml_attr(tag_content, "version").unwrap_or_default();
                    let arch = extract_xml_attr(tag_content, "processorArchitecture").unwrap_or_default();
                    let pkt = extract_xml_attr(tag_content, "publicKeyToken").unwrap_or_default();
                    let lang = extract_xml_attr(tag_content, "language").unwrap_or_default();

                    if assembly_name.is_empty() {
                        assembly_name = name.clone();
                        assembly_version = version.clone();
                    } else {
                        dependent_assemblies.push(DependentAssembly {
                            name,
                            version,
                            processor_architecture: arch,
                            public_key_token: pkt,
                            language: lang,
                        });
                    }
                }
            } else if tag_content.starts_with("bindingRedirect") {
                if let Some(old_ver) = extract_xml_attr(tag_content, "oldVersion") {
                    let new_ver = extract_xml_attr(tag_content, "newVersion").unwrap_or_default();
                    let mut parts = old_ver.split('-');
                    let low = parts.next().unwrap_or(&old_ver);
                    let high = parts.next().unwrap_or(low);

                    binding_redirects.push(BindingRedirect {
                        name: String::new(),
                        old_version_low: String::from(low),
                        old_version_high: String::from(high),
                        new_version: new_ver,
                    });
                }
            }
        } else {
            break;
        }
    }

    (assembly_name, assembly_version, dependent_assemblies, binding_redirects)
}
