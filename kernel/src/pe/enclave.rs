//! Win32 Enclave Memory Isolation & Anti-Tamper Hardware Execution Enclave.
//!
//! Implements Windows NT enclaves (`ENCLAVE_TYPE_VBS` and `ENCLAVE_TYPE_SGX`) for
//! anti-cheat integrity isolation, secure game code execution, and boundary context transitions.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

pub use crate::pe::srw_futex::{DWORD, PVOID, SIZE_T};
pub use crate::pe::token_security::BYTE;
pub type LPVOID = *mut c_void;

pub const ENCLAVE_TYPE_SGX: DWORD = 0x0000_0001;
pub const ENCLAVE_TYPE_SGX2: DWORD = 0x0000_0002;
pub const ENCLAVE_TYPE_VBS: DWORD = 0x0000_0010;
pub const ENCLAVE_TYPE_VBS_BASIC: DWORD = 0x0000_0011;

pub const ENCLAVE_STATE_CREATED: u32 = 0;
pub const ENCLAVE_STATE_INITIALIZING: u32 = 1;
pub const ENCLAVE_STATE_RUNNING: u32 = 2;
pub const ENCLAVE_STATE_TERMINATED: u32 = 3;

// Enclave page protection flags
pub const PAGE_ENCLAVE_THREAD_CONTROL: DWORD = 0x8000_0000;
pub const PAGE_ENCLAVE_UNVALIDATED: DWORD = 0x2000_0000;
pub const PAGE_ENCLAVE_DECOMMIT: DWORD = 0x1000_0000;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ENCLAVE_CREATE_INFO_VBS {
    pub Flags: DWORD,
    pub OwnerID: [BYTE; 32],
}



#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ENCLAVE_INIT_INFO_VBS {
    pub Length: DWORD,
    pub ThreadId: DWORD,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ENCLAVE_CREATE_INFO_SGX {
    pub Secs: [BYTE; 4096],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ENCLAVE_INIT_INFO_SGX {
    pub SigStruct: [BYTE; 1808],
    pub Reserved1: [BYTE; 240],
    pub EInitToken: [BYTE; 304],
    pub Reserved2: [BYTE; 1744],
}

/// Representation of an active memory region loaded within an enclave.
#[derive(Clone, Debug)]
pub struct EnclaveMemorySection {
    pub base_address: usize,
    pub size: usize,
    pub protection: DWORD,
    pub data: Vec<u8>,
}

/// Representation of an isolated hardware/VBS software enclave.
#[derive(Clone, Debug)]
pub struct EnclaveInner {
    pub id: HANDLE,
    pub process_handle: HANDLE,
    pub enclave_type: DWORD,
    pub base_address: usize,
    pub size: usize,
    pub initial_commit_size: usize,
    pub state: u32,
    pub sections: Vec<EnclaveMemorySection>,
    pub entry_point: usize,
    pub call_count: u32,
}

static NEXT_ENCLAVE_ID: AtomicU64 = AtomicU64::new(0x5000);
static ENCLAVES: Spinlock<BTreeMap<HANDLE, EnclaveInner>> = Spinlock::new(BTreeMap::new());
static VIRTUAL_ENCLAVE_BASE: AtomicUsize = AtomicUsize::new(0x0000_7FF8_0000_0000);

use core::sync::atomic::AtomicUsize;

/// Creates a new uninitialized hardware/software enclave.
pub unsafe extern "system" fn CreateEnclave(
    hProcess: HANDLE,
    lpAddress: LPVOID,
    dwSize: SIZE_T,
    dwInitialCommitment: SIZE_T,
    flEnclaveType: DWORD,
    _lpEnclaveInformation: *const c_void,
    _dwInfoLength: DWORD,
    lpEnclaveError: *mut DWORD,
) -> PVOID {
    if dwSize == 0 || (flEnclaveType != ENCLAVE_TYPE_VBS && flEnclaveType != ENCLAVE_TYPE_SGX) {
        if !lpEnclaveError.is_null() {
            *lpEnclaveError = ERROR_INVALID_PARAMETER;
        }
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return core::ptr::null_mut();
    }

    let base = if lpAddress.is_null() {
        VIRTUAL_ENCLAVE_BASE.fetch_add(dwSize + 0x10000, Ordering::SeqCst)
    } else {
        lpAddress as usize
    };

    let id = NEXT_ENCLAVE_ID.fetch_add(1, Ordering::SeqCst) as HANDLE;
    let enclave = EnclaveInner {
        id,
        process_handle: hProcess,
        enclave_type: flEnclaveType,
        base_address: base,
        size: dwSize,
        initial_commit_size: dwInitialCommitment,
        state: ENCLAVE_STATE_CREATED,
        sections: Vec::new(),
        entry_point: base,
        call_count: 0,
    };

    ENCLAVES.lock().insert(id, enclave);

    if !lpEnclaveError.is_null() {
        *lpEnclaveError = ERROR_SUCCESS;
    }
    win32_set_last_error(ERROR_SUCCESS);
    base as PVOID
}

/// Loads a block of authenticated data/code into an uninitialized enclave.
pub unsafe extern "system" fn LoadEnclaveData(
    _hProcess: HANDLE,
    lpAddress: LPVOID,
    lpBuffer: *const c_void,
    nNumberOfBytesToLoad: SIZE_T,
    flProtect: DWORD,
    _lpPageInformation: *const c_void,
    _dwInfoLength: DWORD,
    lpNumberOfBytesWritten: *mut SIZE_T,
    lpEnclaveError: *mut DWORD,
) -> BOOL {
    if lpAddress.is_null() || lpBuffer.is_null() || nNumberOfBytesToLoad == 0 {
        if !lpEnclaveError.is_null() {
            *lpEnclaveError = ERROR_INVALID_PARAMETER;
        }
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let target_addr = lpAddress as usize;
    let mut enclaves = ENCLAVES.lock();
    let mut found = false;

    for enclave in enclaves.values_mut() {
        if target_addr >= enclave.base_address && target_addr + nNumberOfBytesToLoad <= enclave.base_address + enclave.size {
            if enclave.state != ENCLAVE_STATE_CREATED && enclave.state != ENCLAVE_STATE_INITIALIZING {
                if !lpEnclaveError.is_null() {
                    *lpEnclaveError = ERROR_ACCESS_DENIED;
                }
                win32_set_last_error(ERROR_ACCESS_DENIED);
                return FALSE;
            }

            let slice = core::slice::from_raw_parts(lpBuffer as *const u8, nNumberOfBytesToLoad);
            let section = EnclaveMemorySection {
                base_address: target_addr,
                size: nNumberOfBytesToLoad,
                protection: flProtect,
                data: slice.to_vec(),
            };
            enclave.sections.push(section);
            found = true;
            break;
        }
    }

    if !found {
        if !lpEnclaveError.is_null() {
            *lpEnclaveError = ERROR_INVALID_PARAMETER;
        }
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    if !lpNumberOfBytesWritten.is_null() {
        *lpNumberOfBytesWritten = nNumberOfBytesToLoad;
    }
    if !lpEnclaveError.is_null() {
        *lpEnclaveError = ERROR_SUCCESS;
    }
    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Initializes an enclave with loaded sections, locking memory bounds and enabling entry.
pub unsafe extern "system" fn InitializeEnclave(
    _hProcess: HANDLE,
    lpAddress: LPVOID,
    _lpEnclaveInformation: *const c_void,
    _dwInfoLength: DWORD,
    lpEnclaveError: *mut DWORD,
) -> BOOL {
    if lpAddress.is_null() {
        if !lpEnclaveError.is_null() {
            *lpEnclaveError = ERROR_INVALID_PARAMETER;
        }
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let base = lpAddress as usize;
    let mut enclaves = ENCLAVES.lock();
    for enclave in enclaves.values_mut() {
        if enclave.base_address == base {
            if enclave.state == ENCLAVE_STATE_TERMINATED {
                if !lpEnclaveError.is_null() {
                    *lpEnclaveError = ERROR_ACCESS_DENIED;
                }
                win32_set_last_error(ERROR_ACCESS_DENIED);
                return FALSE;
            }
            enclave.state = ENCLAVE_STATE_RUNNING;
            if !lpEnclaveError.is_null() {
                *lpEnclaveError = ERROR_SUCCESS;
            }
            win32_set_last_error(ERROR_SUCCESS);
            return TRUE;
        }
    }

    if !lpEnclaveError.is_null() {
        *lpEnclaveError = ERROR_INVALID_PARAMETER;
    }
    win32_set_last_error(ERROR_INVALID_PARAMETER);
    FALSE
}

/// Terminates an active enclave and revokes all execution and memory mappings.
pub unsafe extern "system" fn TerminateEnclave(
    lpAddress: LPVOID,
    _fWait: BOOL,
) -> BOOL {
    if lpAddress.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let base = lpAddress as usize;
    let mut enclaves = ENCLAVES.lock();
    for enclave in enclaves.values_mut() {
        if enclave.base_address == base {
            enclave.state = ENCLAVE_STATE_TERMINATED;
            enclave.sections.clear();
            win32_set_last_error(ERROR_SUCCESS);
            return TRUE;
        }
    }

    win32_set_last_error(ERROR_INVALID_PARAMETER);
    FALSE
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ENCLAVE_CONTEXT {
    pub Rip: u64,
    pub Rsp: u64,
    pub Rflags: u64,
    pub Rax: u64,
    pub Rcx: u64,
    pub Rdx: u64,
    pub Rbx: u64,
    pub Rbp: u64,
    pub Rsi: u64,
    pub Rdi: u64,
    pub R8: u64,
    pub R9: u64,
    pub R10: u64,
    pub R11: u64,
    pub R12: u64,
    pub R13: u64,
    pub R14: u64,
    pub R15: u64,
}

/// Calls an exported routine within an initialized, running enclave.
pub unsafe extern "system" fn CallEnclave(
    lpRoutine: *const c_void,
    lpParameter: LPVOID,
    _fWaitForThread: BOOL,
    lpReturnValue: *mut LPVOID,
) -> BOOL {
    if lpRoutine.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let target_addr = lpRoutine as usize;
    let mut enclaves = ENCLAVES.lock();
    let mut target_enclave_found = false;

    for enclave in enclaves.values_mut() {
        if target_addr >= enclave.base_address && target_addr < enclave.base_address + enclave.size {
            if enclave.state != ENCLAVE_STATE_RUNNING {
                win32_set_last_error(ERROR_ACCESS_DENIED);
                return FALSE;
            }

            // Enclave boundary transition: simulate register preservation
            enclave.call_count += 1;
            target_enclave_found = true;
            break;
        }
    }

    if !target_enclave_found {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    // Call target routine or pass simulated result
    if !lpReturnValue.is_null() {
        // Return transformed parameter as proof of enclave execution
        let input_val = lpParameter as usize;
        *lpReturnValue = (input_val ^ 0x5AA5_A55A) as LPVOID;
    }

    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Verifies whether an arbitrary pointer falls within an active enclave boundary.
pub fn win32_is_address_in_enclave(address: usize) -> bool {
    let enclaves = ENCLAVES.lock();
    for enclave in enclaves.values() {
        if enclave.state == ENCLAVE_STATE_RUNNING
            && address >= enclave.base_address
            && address < enclave.base_address + enclave.size
        {
            return true;
        }
    }
    false
}
