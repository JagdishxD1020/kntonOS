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
