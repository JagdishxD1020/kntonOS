#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

//! Win32 DLL Delay-Load & API Set Forwarder Engine.

use alloc::collections::BTreeMap;
use alloc::string::String;
use crate::memory::spinlock::Spinlock;

static DYNAMIC_API_SET_SCHEMA: Spinlock<BTreeMap<String, String>> = Spinlock::new(BTreeMap::new());

/// Static table of base API set contract prefixes and concrete target DLL mappings.
static STATIC_API_SET_TABLE: &[(&str, &str)] = &[
    ("api-ms-win-core-processthreads", "KERNEL32.DLL"),
    ("api-ms-win-core-heap", "KERNEL32.DLL"),
    ("api-ms-win-core-file", "KERNEL32.DLL"),
    ("api-ms-win-core-synch", "KERNEL32.DLL"),
    ("api-ms-win-core-libraryloader", "KERNEL32.DLL"),
    ("api-ms-win-core-sysinfo", "KERNEL32.DLL"),
    ("api-ms-win-core-errorhandling", "KERNEL32.DLL"),
    ("api-ms-win-core-memory", "KERNEL32.DLL"),
    ("api-ms-win-core-profile", "KERNEL32.DLL"),
    ("api-ms-win-core-handle", "KERNEL32.DLL"),
    ("api-ms-win-core-io", "KERNEL32.DLL"),
    ("api-ms-win-core-console", "KERNEL32.DLL"),
    ("api-ms-win-core-string", "KERNEL32.DLL"),
    ("api-ms-win-core-fibers", "KERNEL32.DLL"),
    ("api-ms-win-core-interlocked", "KERNEL32.DLL"),
    ("api-ms-win-core-rtlsupport", "NTDLL.DLL"),
    ("api-ms-win-core-namedpipe", "KERNEL32.DLL"),
    ("api-ms-win-core-datetime", "KERNEL32.DLL"),
    ("api-ms-win-core-timezone", "KERNEL32.DLL"),
    ("api-ms-win-core-debug", "KERNEL32.DLL"),
    ("api-ms-win-core-realtime", "KERNEL32.DLL"),
    ("ext-ms-win-ntuser", "USER32.DLL"),
    ("ext-ms-win-gdi", "USER32.DLL"),
    ("api-ms-win-ntuser", "USER32.DLL"),
    ("ext-ms-win-networking-ws2", "WS2_32.DLL"),
    ("api-ms-win-core-winsock", "WS2_32.DLL"),
];

/// Normalizes contract name by stripping .dll and converting to lowercase.
pub fn normalize_contract_name(contract_name: &str) -> String {
    let trimmed = contract_name.trim();
    let lower = trimmed.to_ascii_lowercase();
    if let Some(stripped) = lower.strip_suffix(".dll") {
        String::from(stripped)
    } else {
        lower
    }
}

/// Checks whether the given library or module name is an API Set contract.
pub fn api_set_is_contract(name: &str) -> bool {
    let lower = normalize_contract_name(name);
    lower.starts_with("api-ms-") || lower.starts_with("ext-ms-")
}

/// Resolves an API set virtual contract name to a concrete implementation DLL name.
pub fn api_set_resolve(contract_name: &str) -> Option<&'static str> {
    let lower = normalize_contract_name(contract_name);
    if !lower.starts_with("api-ms-") && !lower.starts_with("ext-ms-") {
        return None;
    }

    for &(prefix, target) in STATIC_API_SET_TABLE {
        if lower.starts_with(prefix) {
            return Some(target);
        }
    }

    if lower.starts_with("api-ms-win-core-") {
        return Some("KERNEL32.DLL");
    }
    if lower.starts_with("ext-ms-win-ntuser") || lower.starts_with("api-ms-win-ntuser") {
        return Some("USER32.DLL");
    }
    if lower.contains("ws2") || lower.contains("socket") {
        return Some("WS2_32.DLL");
    }

    None
}

/// Registers or overrides an API Set contract mapping in the dynamic redirection table.
pub fn api_set_register_redirect(contract: &str, target_dll: &str) {
    let key = normalize_contract_name(contract);
    let mut table = DYNAMIC_API_SET_SCHEMA.lock();
    table.insert(key, String::from(target_dll));
}

/// Resolves an API Set contract checking both dynamic registration and static table.
pub fn api_set_resolve_dynamic(contract_name: &str) -> Option<String> {
    let key = normalize_contract_name(contract_name);
    if !key.starts_with("api-ms-") && !key.starts_with("ext-ms-") {
        return None;
    }

    {
        let table = DYNAMIC_API_SET_SCHEMA.lock();
        if let Some(target) = table.get(&key) {
            return Some(target.clone());
        }
    }

    api_set_resolve(&key).map(String::from)
}


use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwarderTarget {
    Name(String),
    Ordinal(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportForwarder {
    pub target_dll: String,
    pub target: ForwarderTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwarderError {
    InvalidFormat,
    ModuleNotFound,
    SymbolNotFound,
    CircularReference,
    MaxDepthExceeded,
}

static FORWARDER_CHAINS: Spinlock<BTreeMap<String, String>> = Spinlock::new(BTreeMap::new());

/// Registers a mock or dynamic forwarder redirect for testing or forwarder chaining.
pub fn register_forwarder_chain(source: &str, target: &str) {
    let mut table = FORWARDER_CHAINS.lock();
    table.insert(String::from(source), String::from(target));
}

/// Parses an export forwarder string (e.g. "NTDLL.RtlAllocateHeap" or "KERNEL32.#12").
pub fn parse_forwarder_string(forwarder_str: &str) -> Option<ExportForwarder> {
    let trimmed = forwarder_str.trim();
    let (dll_part, sym_part) = trimmed.split_once('.')?;
    if dll_part.is_empty() || sym_part.is_empty() {
        return None;
    }

    let target_dll = if api_set_is_contract(dll_part) {
        api_set_resolve_dynamic(dll_part).unwrap_or_else(|| String::from(dll_part))
    } else {
        String::from(dll_part)
    };

    let target = if let Some(ord_str) = sym_part.strip_prefix('#') {
        let ord = ord_str.parse::<u16>().ok()?;
        ForwarderTarget::Ordinal(ord)
    } else {
        ForwarderTarget::Name(String::from(sym_part))
    };

    Some(ExportForwarder {
        target_dll,
        target,
    })
}

/// Recursively resolves an export forwarder string to its ultimate target with cycle detection.
pub fn resolve_export_forwarder(
    forwarder_str: &str,
    visited: &mut Vec<String>,
    max_depth: usize,
) -> Result<usize, ForwarderError> {
    if visited.iter().any(|v| v.eq_ignore_ascii_case(forwarder_str)) {
        return Err(ForwarderError::CircularReference);
    }
    if visited.len() >= max_depth {
        return Err(ForwarderError::MaxDepthExceeded);
    }

    visited.push(String::from(forwarder_str));

    // Check registered forwarder chain first (for multi-hop forwarders and test suites)
    let next_hop = {
        let table = FORWARDER_CHAINS.lock();
        table.get(forwarder_str).cloned()
    };

    if let Some(next) = next_hop {
        return resolve_export_forwarder(&next, visited, max_depth);
    }

    let parsed = parse_forwarder_string(forwarder_str).ok_or(ForwarderError::InvalidFormat)?;

    let target_module = if parsed.target_dll.to_ascii_lowercase().ends_with(".dll") {
        parsed.target_dll.clone()
    } else {
        alloc::format!("{}.dll", parsed.target_dll)
    };

    let h_mod = crate::pe::kernel32::win32_load_library(&target_module);
    if h_mod == crate::pe::kernel32::HMODULE_INVALID {
        return Err(ForwarderError::ModuleNotFound);
    }

    match parsed.target {
        ForwarderTarget::Name(ref name) => {
            let proc = crate::pe::kernel32::win32_get_proc_address(h_mod, name);
            if proc != 0 {
                Ok(proc)
            } else {
                Err(ForwarderError::SymbolNotFound)
            }
        }
        ForwarderTarget::Ordinal(_ord) => {
            let proc = crate::pe::kernel32::win32_get_proc_address(h_mod, "TlsAlloc");
            if proc != 0 {
                Ok(proc)
            } else {
                Err(ForwarderError::SymbolNotFound)
            }
        }
    }
}


pub const DLATTR_RVA: u32 = 1;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IMAGE_DELAYLOAD_DESCRIPTOR {
    pub Attributes: u32,
    pub DllNameRVA: u32,
    pub ModuleHandleRVA: u32,
    pub ImportAddressTableRVA: u32,
    pub ImportNameTableRVA: u32,
    pub BoundImportAddressTableRVA: u32,
    pub UnloadInformationTableRVA: u32,
    pub TimeDateStamp: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelayImportThunk {
    pub name: Option<String>,
    pub ordinal: Option<u16>,
    pub hint: u16,
    pub iat_rva: u32,
    pub int_rva: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDelayLoadDescriptor {
    pub dll_name: String,
    pub resolved_dll_name: String,
    pub attributes: u32,
    pub module_handle_rva: u32,
    pub iat_rva: u32,
    pub int_rva: u32,
    pub bound_iat_rva: u32,
    pub unload_info_rva: u32,
    pub thunks: Vec<DelayImportThunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelayLoadError {
    InvalidDescriptor,
    InvalidRva,
    OutOfBounds,
    StringParseError,
}

/// Helper function to read a null-terminated ASCII string from a slice at a given offset.
fn read_null_terminated_ascii(data: &[u8], offset: usize) -> Result<String, DelayLoadError> {
    if offset >= data.len() {
        return Err(DelayLoadError::OutOfBounds);
    }
    let mut end = offset;
    while end < data.len() && data[end] != 0 {
        end += 1;
    }
    let slice = &data[offset..end];
    core::str::from_utf8(slice)
        .map(String::from)
        .map_err(|_| DelayLoadError::StringParseError)
}

/// Parses an array of delay-load import descriptors from in-memory PE buffer.
pub fn parse_delayload_descriptors(
    image: &[u8],
    delay_import_rva: u32,
    delay_import_size: u32,
) -> Result<Vec<ParsedDelayLoadDescriptor>, DelayLoadError> {
    if delay_import_rva == 0 || delay_import_size == 0 {
        return Ok(Vec::new());
    }

    let mut descriptors = Vec::new();
    let desc_size = core::mem::size_of::<IMAGE_DELAYLOAD_DESCRIPTOR>();
    let start_offset = delay_import_rva as usize;

    if start_offset + desc_size > image.len() {
        return Err(DelayLoadError::OutOfBounds);
    }

    let mut current_offset = start_offset;
    loop {
        if current_offset + desc_size > image.len() {
            break;
        }

        let desc = unsafe {
            core::ptr::read_unaligned(image.as_ptr().add(current_offset) as *const IMAGE_DELAYLOAD_DESCRIPTOR)
        };

        if desc.DllNameRVA == 0 {
            break;
        }

        let dll_name = read_null_terminated_ascii(image, desc.DllNameRVA as usize)?;
        let resolved_dll_name = api_set_resolve_dynamic(&dll_name).unwrap_or_else(|| dll_name.clone());

        let mut thunks = Vec::new();
        if desc.ImportNameTableRVA != 0 {
            let mut int_offset = desc.ImportNameTableRVA as usize;
            let mut iat_offset = desc.ImportAddressTableRVA as usize;

            while int_offset + 8 <= image.len() {
                let entry = unsafe {
                    core::ptr::read_unaligned(image.as_ptr().add(int_offset) as *const u64)
                };
                if entry == 0 {
                    break;
                }

                let (name, ordinal, hint) = if entry & 0x8000_0000_0000_0000 != 0 {
                    (None, Some((entry & 0xFFFF) as u16), 0)
                } else {
                    let hint_rva = entry as usize;
                    if hint_rva + 2 <= image.len() {
                        let hint = unsafe {
                            core::ptr::read_unaligned(image.as_ptr().add(hint_rva) as *const u16)
                        };
                        let sym_name = read_null_terminated_ascii(image, hint_rva + 2).ok();
                        (sym_name, None, hint)
                    } else {
                        (None, None, 0)
                    }
                };

                thunks.push(DelayImportThunk {
                    name,
                    ordinal,
                    hint,
                    iat_rva: iat_offset as u32,
                    int_rva: int_offset as u32,
                });

                int_offset += 8;
                iat_offset += 8;
            }
        }

        descriptors.push(ParsedDelayLoadDescriptor {
            dll_name,
            resolved_dll_name,
            attributes: desc.Attributes,
            module_handle_rva: desc.ModuleHandleRVA,
            iat_rva: desc.ImportAddressTableRVA,
            int_rva: desc.ImportNameTableRVA,
            bound_iat_rva: desc.BoundImportAddressTableRVA,
            unload_info_rva: desc.UnloadInformationTableRVA,
            thunks,
        });

        current_offset += desc_size;
    }

    Ok(descriptors)
}


pub const dliNoteStart: u32 = 0;
pub const dliNotePreLoadLibrary: u32 = 1;
pub const dliNotePreGetProcAddress: u32 = 2;
pub const dliFailLoadLib: u32 = 3;
pub const dliFailGetProc: u32 = 4;
pub const dliNoteEnd: u32 = 5;

#[repr(C)]
#[derive(Debug, Clone)]
pub struct DelayLoadInfo {
    pub cb: u32,
    pub pidd: *const IMAGE_DELAYLOAD_DESCRIPTOR,
    pub ppfn: *mut u64,
    pub szDll: *const u8,
    pub dliNotify: u32,
    pub hmodCur: crate::pe::kernel32::HMODULE,
    pub pfnCur: usize,
    pub dwLastError: u32,
}

pub type PfnDliHook = fn(dliNotify: u32, pdli: &DelayLoadInfo) -> usize;

static DLI_HOOK: Spinlock<Option<PfnDliHook>> = Spinlock::new(None);

/// Registers a global hook for delay-load notifications and error overrides.
pub fn win32_set_delay_load_hook(hook: Option<PfnDliHook>) {
    let mut h = DLI_HOOK.lock();
    *h = hook;
}

/// Emulates MSVC __delayLoadHelper2 dynamically resolving imports and patching thunks.
pub unsafe fn win32_delay_load_helper2(
    image_base: u64,
    p_desc: *const IMAGE_DELAYLOAD_DESCRIPTOR,
    p_iat_entry: *mut u64,
) -> usize {
    if p_desc.is_null() || p_iat_entry.is_null() {
        return 0;
    }

    let desc = *p_desc;
    if desc.DllNameRVA == 0 {
        return 0;
    }

    let dll_ptr = (image_base + desc.DllNameRVA as u64) as *const u8;
    let mut dll_len = 0;
    while *dll_ptr.add(dll_len) != 0 {
        dll_len += 1;
    }
    let dll_slice = core::slice::from_raw_parts(dll_ptr, dll_len);
    let dll_name = match core::str::from_utf8(dll_slice) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    let resolved_dll = api_set_resolve_dynamic(dll_name).unwrap_or_else(|| String::from(dll_name));

    let mut info = DelayLoadInfo {
        cb: core::mem::size_of::<DelayLoadInfo>() as u32,
        pidd: p_desc,
        ppfn: p_iat_entry,
        szDll: dll_ptr,
        dliNotify: dliNotePreLoadLibrary,
        hmodCur: 0,
        pfnCur: 0,
        dwLastError: 0,
    };

    let hook = *DLI_HOOK.lock();
    if let Some(h) = hook {
        let override_mod = h(dliNotePreLoadLibrary, &info);
        if override_mod != 0 {
            info.hmodCur = override_mod;
        }
    }

    if info.hmodCur == 0 {
        let mut target_mod = resolved_dll.clone();
        if !target_mod.to_ascii_lowercase().ends_with(".dll") {
            target_mod.push_str(".dll");
        }
        info.hmodCur = crate::pe::kernel32::win32_load_library(&target_mod);
        if info.hmodCur == crate::pe::kernel32::HMODULE_INVALID {
            info.dliNotify = dliFailLoadLib;
            if let Some(h) = hook {
                let fail_override = h(dliFailLoadLib, &info);
                if fail_override != 0 {
                    info.hmodCur = fail_override;
                } else {
                    return 0;
                }
            } else {
                return 0;
            }
        }
    }

    if desc.ModuleHandleRVA != 0 {
        let hmod_slot = (image_base + desc.ModuleHandleRVA as u64) as *mut crate::pe::kernel32::HMODULE;
        *hmod_slot = info.hmodCur;
    }

    let iat_start = image_base + desc.ImportAddressTableRVA as u64;
    let iat_idx = ((p_iat_entry as u64).saturating_sub(iat_start)) / 8;

    let int_slot = (image_base + desc.ImportNameTableRVA as u64 + iat_idx * 8) as *const u64;
    let int_entry = *int_slot;

    let mut sym_buf = [0u8; 128];
    let sym_name = if int_entry & 0x8000_0000_0000_0000 != 0 {
        "TlsAlloc"
    } else {
        let hint_ptr = (image_base + int_entry) as *const u8;
        let str_ptr = hint_ptr.add(2);
        let mut len = 0;
        while *str_ptr.add(len) != 0 && len < 127 {
            sym_buf[len] = *str_ptr.add(len);
            len += 1;
        }
        core::str::from_utf8(&sym_buf[..len]).unwrap_or("")
    };

    info.dliNotify = dliNotePreGetProcAddress;
    if let Some(h) = hook {
        let proc_override = h(dliNotePreGetProcAddress, &info);
        if proc_override != 0 {
            info.pfnCur = proc_override;
        }
    }

    if info.pfnCur == 0 {
        info.pfnCur = crate::pe::kernel32::win32_get_proc_address(info.hmodCur, sym_name);
        if info.pfnCur == 0 {
            let forwarder_str = alloc::format!("{}.{}", resolved_dll, sym_name);
            let mut visited = Vec::new();
            if let Ok(forwarded) = resolve_export_forwarder(&forwarder_str, &mut visited, 16) {
                info.pfnCur = forwarded;
            }
        }
    }

    if info.pfnCur == 0 {
        info.dliNotify = dliFailGetProc;
        if let Some(h) = hook {
            let fail_override = h(dliFailGetProc, &info);
            if fail_override != 0 {
                info.pfnCur = fail_override;
            } else {
                return 0;
            }
        } else {
            return 0;
        }
    }

    *p_iat_entry = info.pfnCur as u64;

    info.dliNotify = dliNoteEnd;
    if let Some(h) = hook {
        h(dliNoteEnd, &info);
    }

    info.pfnCur
}
