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

/// Creates an activation context from an ASCII manifest source path or XML buffer.
pub unsafe extern "system" fn CreateActCtxA(pActCtx: *const ACTCTX_A) -> HANDLE {
    if pActCtx.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return INVALID_ACTCTX;
    }

    let src = (*pActCtx).lpSource;
    let mut source_str = String::new();
    if !src.is_null() {
        let mut len = 0;
        while *src.add(len) != 0 && len < 4096 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(src, len);
        if let Ok(s) = core::str::from_utf8(slice) {
            source_str = String::from(s);
        }
    }

    let is_proc_def = ((*pActCtx).dwFlags & ACTCTX_FLAG_SET_PROCESS_DEFAULT) != 0;
    create_actctx_internal(source_str, is_proc_def)
}

/// Creates an activation context from a UTF-16 manifest source path or XML buffer.
pub unsafe extern "system" fn CreateActCtxW(pActCtx: *const ACTCTX_W) -> HANDLE {
    if pActCtx.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return INVALID_ACTCTX;
    }

    let src = (*pActCtx).lpSource;
    let mut source_str = String::new();
    if !src.is_null() {
        let mut len = 0;
        while *src.add(len) != 0 && len < 4096 {
            source_str.push(*src.add(len) as u8 as char);
            len += 1;
        }
    }

    let is_proc_def = ((*pActCtx).dwFlags & ACTCTX_FLAG_SET_PROCESS_DEFAULT) != 0;
    create_actctx_internal(source_str, is_proc_def)
}

fn create_actctx_internal(source_str: String, is_proc_def: bool) -> HANDLE {
    let (name, ver, deps, redirects) = if source_str.contains('<') {
        parse_manifest_xml(&source_str)
    } else {
        (String::from("app.exe"), String::from("1.0.0.0"), Vec::new(), Vec::new())
    };

    let mut redirections = BTreeMap::new();
    for dep in &deps {
        if dep.name.contains("ComCtl32") || dep.name.contains("Microsoft.Windows.Common-Controls") {
            redirections.insert(String::from("comctl32.dll"), String::from("comctl32.v6.dll"));
        } else if dep.name.contains("VC90.CRT") {
            redirections.insert(String::from("msvcr90.dll"), String::from("winsxs/msvcr90.dll"));
            redirections.insert(String::from("msvcp90.dll"), String::from("winsxs/msvcp90.dll"));
        } else if dep.name.contains("VC80.CRT") {
            redirections.insert(String::from("msvcr80.dll"), String::from("winsxs/msvcr80.dll"));
            redirections.insert(String::from("msvcp80.dll"), String::from("winsxs/msvcp80.dll"));
        }
    }

    let id = NEXT_ACTCTX_ID.fetch_add(1, Ordering::SeqCst) as HANDLE;
    let inner = ActivationContextInner {
        id,
        source_path: source_str,
        assembly_name: name,
        assembly_version: ver,
        dependent_assemblies: deps,
        binding_redirects: redirects,
        dll_redirections: redirections,
        ref_count: 1,
        is_process_default: is_proc_def,
    };

    ACTIVATION_CONTEXTS.lock().insert(id, inner);
    if is_proc_def {
        *PROCESS_DEFAULT_ACTCTX.lock() = id;
    }

    win32_set_last_error(ERROR_SUCCESS);
    id
}

/// Activates an activation context for the calling thread, returning a cookie.
pub unsafe extern "system" fn ActivateActCtx(hActCtx: HANDLE, lpCookie: *mut ULONG_PTR) -> BOOL {
    if hActCtx == 0 || hActCtx == INVALID_ACTCTX || lpCookie.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let contexts = ACTIVATION_CONTEXTS.lock();
    if !contexts.contains_key(&hActCtx) {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }
    drop(contexts);

    let tid = get_current_thread_id();
    let mut stacks = THREAD_ACTCTX_STACKS.lock();
    let stack = stacks.entry(tid).or_insert_with(Vec::new);
    stack.push(hActCtx);
    let cookie = stack.len();

    *lpCookie = cookie;
    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Deactivates an activation context corresponding to a previously issued cookie.
pub unsafe extern "system" fn DeactivateActCtx(_dwFlags: DWORD, ulCookie: ULONG_PTR) -> BOOL {
    if ulCookie == 0 {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let tid = get_current_thread_id();
    let mut stacks = THREAD_ACTCTX_STACKS.lock();
    if let Some(stack) = stacks.get_mut(&tid) {
        if !stack.is_empty() {
            stack.pop();
            win32_set_last_error(ERROR_SUCCESS);
            return TRUE;
        }
    }

    win32_set_last_error(ERROR_INVALID_PARAMETER);
    FALSE
}

/// Increments the reference count of an activation context.
pub unsafe extern "system" fn AddRefActCtx(hActCtx: HANDLE) {
    if hActCtx != 0 && hActCtx != INVALID_ACTCTX {
        let mut contexts = ACTIVATION_CONTEXTS.lock();
        if let Some(ctx) = contexts.get_mut(&hActCtx) {
            ctx.ref_count += 1;
        }
    }
}

/// Decrements the reference count of an activation context and frees it when zero.
pub unsafe extern "system" fn ReleaseActCtx(hActCtx: HANDLE) {
    if hActCtx != 0 && hActCtx != INVALID_ACTCTX {
        let mut contexts = ACTIVATION_CONTEXTS.lock();
        if let Some(ctx) = contexts.get_mut(&hActCtx) {
            ctx.ref_count = ctx.ref_count.saturating_sub(1);
            if ctx.ref_count == 0 {
                contexts.remove(&hActCtx);
            }
        }
    }
}

/// Retrieves the active activation context for the calling thread.
pub unsafe extern "system" fn GetCurrentActCtx(lphActCtx: *mut HANDLE) -> BOOL {
    if lphActCtx.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let tid = get_current_thread_id();
    let stacks = THREAD_ACTCTX_STACKS.lock();
    let active = if let Some(stack) = stacks.get(&tid) {
        stack.last().copied().unwrap_or(0)
    } else {
        0
    };
    drop(stacks);

    let result = if active != 0 {
        active
    } else {
        *PROCESS_DEFAULT_ACTCTX.lock()
    };

    *lphActCtx = result;
    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

fn get_current_thread_id() -> u32 {
    let teb = crate::pe::peb_teb::win32_get_current_teb();
    if !teb.is_null() {
        let tid = unsafe { (*teb).client_id.unique_thread as u32 };
        if tid != 0 {
            return tid;
        }
    }
    1
}
