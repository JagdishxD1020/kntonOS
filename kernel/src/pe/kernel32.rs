//! Win32 Kernel32 API & Dynamic Library Symbol Resolution Emulation.
//! Emulates Windows dynamic library export tables, module loader, and core Kernel32 APIs.

pub type HMODULE = usize;
pub type FARPROC = usize;

pub const HMODULE_INVALID: HMODULE = 0;
pub const HMODULE_KERNEL32: HMODULE = 0x7FFF_0001_0000;
pub const HMODULE_USER32: HMODULE = 0x7FFF_0002_0000;
pub const HMODULE_GDI32: HMODULE = 0x7FFF_0003_0000;
pub const HMODULE_NTDLL: HMODULE = 0x7FFF_0004_0000;

/// Synthetic export token descriptor for simulated DLL exports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportEntry {
    pub name: &'static str,
    pub address: FARPROC,
}

/// Simulated dynamic library export directory table.
#[derive(Debug, Clone, Copy)]
pub struct ModuleExportDirectory {
    pub module_name: &'static str,
    pub handle: HMODULE,
    pub exports: &'static [ExportEntry],
}

pub const KERNEL32_EXPORTS: &[ExportEntry] = &[
    ExportEntry { name: "VirtualAlloc", address: 0x7FFF_0001_1010 },
    ExportEntry { name: "VirtualFree", address: 0x7FFF_0001_1020 },
    ExportEntry { name: "VirtualProtect", address: 0x7FFF_0001_1030 },
    ExportEntry { name: "CreateFileA", address: 0x7FFF_0001_1040 },
    ExportEntry { name: "CreateFileW", address: 0x7FFF_0001_1048 },
    ExportEntry { name: "ReadFile", address: 0x7FFF_0001_1050 },
    ExportEntry { name: "WriteFile", address: 0x7FFF_0001_1060 },
    ExportEntry { name: "CloseHandle", address: 0x7FFF_0001_1070 },
    ExportEntry { name: "GetProcAddress", address: 0x7FFF_0001_1080 },
    ExportEntry { name: "LoadLibraryA", address: 0x7FFF_0001_1090 },
    ExportEntry { name: "LoadLibraryW", address: 0x7FFF_0001_1098 },
    ExportEntry { name: "GetModuleHandleA", address: 0x7FFF_0001_10A0 },
    ExportEntry { name: "GetModuleHandleW", address: 0x7FFF_0001_10A8 },
    ExportEntry { name: "GetCommandLineA", address: 0x7FFF_0001_10B0 },
    ExportEntry { name: "GetCommandLineW", address: 0x7FFF_0001_10B8 },
    ExportEntry { name: "ExitProcess", address: 0x7FFF_0001_10C0 },
    ExportEntry { name: "GetLastError", address: 0x7FFF_0001_10D0 },
    ExportEntry { name: "SetLastError", address: 0x7FFF_0001_10E0 },
    ExportEntry { name: "GetCurrentProcess", address: 0x7FFF_0001_10F0 },
    ExportEntry { name: "GetCurrentThread", address: 0x7FFF_0001_1100 },
];

pub const USER32_EXPORTS: &[ExportEntry] = &[
    ExportEntry { name: "MessageBoxA", address: 0x7FFF_0002_1010 },
    ExportEntry { name: "MessageBoxW", address: 0x7FFF_0002_1018 },
    ExportEntry { name: "PeekMessageA", address: 0x7FFF_0002_1020 },
    ExportEntry { name: "DispatchMessageA", address: 0x7FFF_0002_1030 },
    ExportEntry { name: "GetMessageA", address: 0x7FFF_0002_1040 },
    ExportEntry { name: "DefWindowProcA", address: 0x7FFF_0002_1050 },
];

pub const GDI32_EXPORTS: &[ExportEntry] = &[
    ExportEntry { name: "CreateCompatibleDC", address: 0x7FFF_0003_1010 },
    ExportEntry { name: "CreateCompatibleBitmap", address: 0x7FFF_0003_1020 },
    ExportEntry { name: "SelectObject", address: 0x7FFF_0003_1030 },
    ExportEntry { name: "BitBlt", address: 0x7FFF_0003_1040 },
    ExportEntry { name: "DeleteObject", address: 0x7FFF_0003_1050 },
];

pub const NTDLL_EXPORTS: &[ExportEntry] = &[
    ExportEntry { name: "NtAllocateVirtualMemory", address: 0x7FFF_0004_1010 },
    ExportEntry { name: "NtFreeVirtualMemory", address: 0x7FFF_0004_1020 },
    ExportEntry { name: "NtCreateFile", address: 0x7FFF_0004_1030 },
    ExportEntry { name: "NtReadFile", address: 0x7FFF_0004_1040 },
    ExportEntry { name: "NtWriteFile", address: 0x7FFF_0004_1050 },
    ExportEntry { name: "NtClose", address: 0x7FFF_0004_1060 },
    ExportEntry { name: "NtDeviceIoControlFile", address: 0x7FFF_0004_1070 },
    ExportEntry { name: "NtQuerySystemInformation", address: 0x7FFF_0004_1080 },
];

pub const REGISTERED_MODULES: &[ModuleExportDirectory] = &[
    ModuleExportDirectory {
        module_name: "kernel32.dll",
        handle: HMODULE_KERNEL32,
        exports: KERNEL32_EXPORTS,
    },
    ModuleExportDirectory {
        module_name: "user32.dll",
        handle: HMODULE_USER32,
        exports: USER32_EXPORTS,
    },
    ModuleExportDirectory {
        module_name: "gdi32.dll",
        handle: HMODULE_GDI32,
        exports: GDI32_EXPORTS,
    },
    ModuleExportDirectory {
        module_name: "ntdll.dll",
        handle: HMODULE_NTDLL,
        exports: NTDLL_EXPORTS,
    },
];

/// Compares two strings case-insensitively without heap allocation.
pub fn str_eq_ignore_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

/// Simulates Win32 LoadLibraryA loading a dynamic link library module handle.
pub fn win32_load_library(lib_name: &str) -> HMODULE {
    for mod_dir in REGISTERED_MODULES {
        if str_eq_ignore_case(lib_name, mod_dir.module_name) {
            return mod_dir.handle;
        }
        // Match name without .dll extension
        if let Some(stripped) = mod_dir.module_name.strip_suffix(".dll") {
            if str_eq_ignore_case(lib_name, stripped) {
                return mod_dir.handle;
            }
        }
    }
    HMODULE_INVALID
}

/// Simulates Win32 GetProcAddress resolving an exported procedure address by name.
pub fn win32_get_proc_address(h_module: HMODULE, proc_name: &str) -> FARPROC {
    if h_module == HMODULE_INVALID {
        return 0;
    }

    for mod_dir in REGISTERED_MODULES {
        if mod_dir.handle == h_module {
            for exp in mod_dir.exports {
                if exp.name == proc_name {
                    return exp.address;
                }
            }
            break;
        }
    }
    0
}
