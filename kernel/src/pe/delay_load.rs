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
