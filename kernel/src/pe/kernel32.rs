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

use core::sync::atomic::{AtomicU32, Ordering};
use super::ntdll::*;
use crate::syscall::NT_CURRENT_PROCESS;

pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_MOD_NOT_FOUND: u32 = 126;
pub const ERROR_PROC_NOT_FOUND: u32 = 127;
pub const ERROR_ALREADY_EXISTS: u32 = 183;

static LAST_ERROR: AtomicU32 = AtomicU32::new(0);

/// Retrieves calling thread's last-error code value.
pub fn win32_get_last_error() -> u32 {
    LAST_ERROR.load(Ordering::Relaxed)
}

/// Sets calling thread's last-error code value.
pub fn win32_set_last_error(err_code: u32) {
    LAST_ERROR.store(err_code, Ordering::Relaxed);
}

use crate::pe::virtual_alloc::*;

pub const INVALID_HANDLE_VALUE: u64 = !0u64;

pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const GENERIC_EXECUTE: u32 = 0x2000_0000;
pub const GENERIC_ALL: u32 = 0x1000_0000;

pub const FILE_SHARE_READ: u32 = 0x0000_0001;
pub const FILE_SHARE_WRITE: u32 = 0x0000_0002;
pub const FILE_SHARE_DELETE: u32 = 0x0000_0004;

pub const CREATE_NEW: u32 = 1;
pub const CREATE_ALWAYS: u32 = 2;
pub const OPEN_EXISTING: u32 = 3;
pub const OPEN_ALWAYS: u32 = 4;
pub const TRUNCATE_EXISTING: u32 = 5;

pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;

/// Emulates Win32 CreateFile creating or opening a file or device.
pub unsafe fn win32_create_file(
    lp_file_name: &str,
    dw_desired_access: u32,
    dw_share_mode: u32,
    dw_creation_disposition: u32,
    dw_flags_and_attributes: u32,
) -> u64 {
    if lp_file_name.is_empty() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return INVALID_HANDLE_VALUE;
    }

    let mut handle: u64 = 0;
    let mut iosb = IoStatusBlock::default();

    let status = nt_create_file(
        &mut handle as *mut u64,
        dw_desired_access,
        core::ptr::null(),
        &mut iosb as *mut IoStatusBlock,
        core::ptr::null(),
        dw_flags_and_attributes,
        dw_share_mode,
        dw_creation_disposition,
        0,
        core::ptr::null(),
        0,
    );

    if nt_success(status) {
        win32_set_last_error(ERROR_SUCCESS);
        handle
    } else {
        win32_set_last_error(ERROR_FILE_NOT_FOUND);
        INVALID_HANDLE_VALUE
    }
}

/// Emulates Win32 ReadFile reading bytes from an open file or device.
pub unsafe fn win32_read_file(
    h_file: u64,
    lp_buffer: *mut u8,
    n_number_of_bytes_to_read: u32,
    lp_number_of_bytes_read: *mut u32,
) -> bool {
    if h_file == INVALID_HANDLE_VALUE || h_file == 0 || lp_buffer.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return false;
    }

    let mut iosb = IoStatusBlock::default();
    let status = nt_read_file(
        h_file,
        0,
        0,
        0,
        &mut iosb as *mut IoStatusBlock,
        lp_buffer,
        n_number_of_bytes_to_read,
        core::ptr::null(),
        core::ptr::null(),
    );

    if !lp_number_of_bytes_read.is_null() {
        *lp_number_of_bytes_read = iosb.information as u32;
    }

    if nt_success(status) {
        win32_set_last_error(ERROR_SUCCESS);
        true
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        false
    }
}

/// Emulates Win32 WriteFile writing bytes to an open file or device.
pub unsafe fn win32_write_file(
    h_file: u64,
    lp_buffer: *const u8,
    n_number_of_bytes_to_write: u32,
    lp_number_of_bytes_written: *mut u32,
) -> bool {
    if h_file == INVALID_HANDLE_VALUE || h_file == 0 || lp_buffer.is_null() {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return false;
    }

    let mut iosb = IoStatusBlock::default();
    let status = nt_write_file(
        h_file,
        0,
        0,
        0,
        &mut iosb as *mut IoStatusBlock,
        lp_buffer,
        n_number_of_bytes_to_write,
        core::ptr::null(),
        core::ptr::null(),
    );

    if !lp_number_of_bytes_written.is_null() {
        *lp_number_of_bytes_written = iosb.information as u32;
    }

    if nt_success(status) {
        win32_set_last_error(ERROR_SUCCESS);
        true
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        false
    }
}

/// Emulates Win32 CloseHandle closing an open object handle.
pub unsafe fn win32_close_handle(h_object: u64) -> bool {
    if h_object == INVALID_HANDLE_VALUE || h_object == 0 {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return false;
    }

    let status = nt_close(h_object);
    if nt_success(status) {
        win32_set_last_error(ERROR_SUCCESS);
        true
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        false
    }
}

/// Emulates Win32 GetModuleHandle retrieving a module handle for the specified module.
pub fn win32_get_module_handle(module_name: Option<&str>) -> HMODULE {
    match module_name {
        None => HMODULE_KERNEL32,
        Some(name) => {
            let h = win32_load_library(name);
            if h == HMODULE_INVALID {
                win32_set_last_error(ERROR_MOD_NOT_FOUND);
            } else {
                win32_set_last_error(ERROR_SUCCESS);
            }
            h
        }
    }
}

/// Emulates Win32 GetCommandLine retrieving the command-line string for current process.
pub fn win32_get_command_line() -> &'static str {
    "init.exe\0"
}

/// Emulates Win32 ExitProcess terminating the calling process and all its threads.
pub fn win32_exit_process(exit_code: u32) {
    crate::syscall::table::dispatch(
        crate::syscall::table::NT_TERMINATE_PROCESS,
        NT_CURRENT_PROCESS,
        exit_code as u64,
        0,
        0,
    );
}

/// Comprehensive self-test suite verifying the Win32 Kernel32 API & Dynamic Symbol Resolution Emulation layer.
pub fn test_kernel32_emulation_subsystem() -> bool {
    crate::serial_write_str("[KNT-KERNEL32-TEST] Executing Win32 Kernel32 API & symbol resolution verification...\r\n");

    // 1. Verify module loading and export symbol resolution
    {
        let h_kernel32 = win32_load_library("kernel32.dll");
        if h_kernel32 != HMODULE_KERNEL32 {
            return false;
        }

        if win32_load_library("KERNEL32") != HMODULE_KERNEL32 {
            return false;
        }
        if win32_load_library("user32") != HMODULE_USER32 {
            return false;
        }
        if win32_load_library("gdi32.dll") != HMODULE_GDI32 {
            return false;
        }
        if win32_load_library("nonexistent.dll") != HMODULE_INVALID {
            return false;
        }

        let p_virtual_alloc = win32_get_proc_address(h_kernel32, "VirtualAlloc");
        if p_virtual_alloc != 0x7FFF_0001_1010 {
            return false;
        }

        let p_create_file = win32_get_proc_address(h_kernel32, "CreateFileA");
        if p_create_file != 0x7FFF_0001_1040 {
            return false;
        }

        let p_get_proc_addr = win32_get_proc_address(h_kernel32, "GetProcAddress");
        if p_get_proc_addr != 0x7FFF_0001_1080 {
            return false;
        }

        let h_user32 = win32_load_library("user32.dll");
        if win32_get_proc_address(h_user32, "MessageBoxA") != 0x7FFF_0002_1010 {
            return false;
        }

        let h_gdi32 = win32_load_library("gdi32.dll");
        if win32_get_proc_address(h_gdi32, "BitBlt") != 0x7FFF_0003_1040 {
            return false;
        }

        if win32_get_proc_address(h_kernel32, "NonExistentFunction") != 0 {
            return false;
        }
    }

    // 2. Verify virtual memory allocation via Win32 page protection flags
    unsafe {
        let mem = win32_virtual_alloc(
            core::ptr::null_mut(),
            4096,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        );
        if mem.is_null() {
            return false;
        }

        let mut old_protect = 0u32;
        let protect_res = win32_virtual_protect(
            mem,
            4096,
            PAGE_EXECUTE_READWRITE,
            &mut old_protect as *mut u32,
        );
        if !protect_res || old_protect != PAGE_READWRITE {
            return false;
        }

        let free_res = win32_virtual_free(mem, 0, MEM_RELEASE);
        if !free_res {
            return false;
        }

        let invalid_alloc = win32_virtual_alloc(
            core::ptr::null_mut(),
            0,
            MEM_COMMIT,
            PAGE_READWRITE,
        );
        if !invalid_alloc.is_null() || win32_get_last_error() != ERROR_INVALID_PARAMETER {
            return false;
        }
    }

    // 3. Verify file handle mapping and table disposal
    unsafe {
        let h_file = win32_create_file(
            "CONOUT$",
            GENERIC_WRITE | GENERIC_READ,
            FILE_SHARE_READ,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
        );
        if h_file == INVALID_HANDLE_VALUE || h_file == 0 {
            return false;
        }

        let banner = b"[KNT-KERNEL32-TEST] Win32 console write verified\r\n";
        let mut bytes_written = 0u32;
        let write_ok = win32_write_file(
            h_file,
            banner.as_ptr(),
            banner.len() as u32,
            &mut bytes_written as *mut u32,
        );
        if !write_ok || bytes_written != banner.len() as u32 {
            return false;
        }

        let mut read_buf = [0u8; 16];
        let mut bytes_read = 0u32;
        let read_ok = win32_read_file(
            h_file,
            read_buf.as_mut_ptr(),
            read_buf.len() as u32,
            &mut bytes_read as *mut u32,
        );
        if !read_ok {
            return false;
        }

        let close_ok = win32_close_handle(h_file);
        if !close_ok {
            return false;
        }

        let bad_close = win32_close_handle(INVALID_HANDLE_VALUE);
        if bad_close || win32_get_last_error() != ERROR_INVALID_HANDLE {
            return false;
        }
    }

    // 4. Verify process environment and module handle helpers
    {
        let h_mod = win32_get_module_handle(None);
        if h_mod != HMODULE_KERNEL32 {
            return false;
        }

        let cmdline = win32_get_command_line();
        if !cmdline.starts_with("init.exe") {
            return false;
        }
    }

    crate::serial_write_str("[KNT-KERNEL32-PASS] Win32 Kernel32 API and dynamic symbol resolution verified\r\n");
    true
}
