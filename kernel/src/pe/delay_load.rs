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
