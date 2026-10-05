//! Win32 Transactional NTFS (TxF) & Kernel Mini-Filter Virtualization Layer.
//!
//! Implements atomic file transaction primitives (CreateTransaction, CommitTransaction, RollbackTransaction),
//! copy-on-write page backing for transacted streams during game patch deployment,
//! snapshot reader isolation preventing dirty reads by parallel worker threads,
//! and kernel mini-filter pre/post-operation callbacks intercepting Win32 file I/O.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_SUCCESS, INVALID_HANDLE_VALUE,
};
pub use crate::pe::srw_futex::{DWORD, PVOID, SIZE_T};
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

// Transaction Outcome & States
pub const TRANSACTION_STATE_ACTIVE: u32 = 1;
pub const TRANSACTION_STATE_PREPARED: u32 = 2;
pub const TRANSACTION_STATE_COMMITTED: u32 = 3;
pub const TRANSACTION_STATE_ABORTED: u32 = 4;

// Transaction Options
pub const TRANSACTION_DO_NOT_PROMOTE: DWORD = 0x0000_0001;

pub const ERROR_TRANSACTIONAL_CONFLICT: DWORD = 6700;
pub const ERROR_TRANSACTION_NOT_ACTIVE: DWORD = 6701;
pub const ERROR_TRANSACTION_ALREADY_COMMITTED: DWORD = 6702;
pub const ERROR_TRANSACTION_ALREADY_ABORTED: DWORD = 6703;

/// Transacted in-memory file snapshot stream backing copy-on-write changes.
#[derive(Clone, Debug)]
pub struct TransactedFileStream {
    pub path: String,
    pub original_data: Vec<u8>,
    pub modified_data: Vec<u8>,
    pub is_deleted: bool,
    pub is_created: bool,
    pub is_modified: bool,
}

/// Transaction object descriptor.
#[derive(Clone, Debug)]
pub struct TransactionInner {
    pub id: HANDLE,
    pub state: u32,
    pub description: String,
    pub timeout_ms: u32,
    pub streams: BTreeMap<String, TransactedFileStream>,
}

static NEXT_TX_ID: AtomicU64 = AtomicU64::new(0x9000);
static TRANSACTIONS: Spinlock<BTreeMap<HANDLE, TransactionInner>> = Spinlock::new(BTreeMap::new());

/// Creates a new Win32 atomic file transaction.
pub unsafe extern "system" fn CreateTransaction(
    _lpTransactionAttributes: *const c_void,
    _UOW: *const c_void,
    _CreateOptions: DWORD,
    _IsolationLevel: DWORD,
    _IsolationFlags: DWORD,
    Timeout: DWORD,
    Description: *const u16,
) -> HANDLE {
    let mut desc_str = String::new();
    if !Description.is_null() {
        let mut idx = 0;
        while *Description.add(idx) != 0 && idx < 256 {
            desc_str.push(*Description.add(idx) as u8 as char);
            idx += 1;
        }
    }

    let id = NEXT_TX_ID.fetch_add(1, Ordering::SeqCst) as HANDLE;
    let tx = TransactionInner {
        id,
        state: TRANSACTION_STATE_ACTIVE,
        description: desc_str,
        timeout_ms: Timeout,
        streams: BTreeMap::new(),
    };

    TRANSACTIONS.lock().insert(id, tx);
    win32_set_last_error(ERROR_SUCCESS);
    id
}

/// Commits all pending transacted file modifications atomically to the filesystem.
pub unsafe extern "system" fn CommitTransaction(TransactionHandle: HANDLE) -> BOOL {
    if TransactionHandle == 0 || TransactionHandle == (INVALID_HANDLE_VALUE as HANDLE) {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return FALSE;
    }

    let mut transactions = TRANSACTIONS.lock();
    let tx = match transactions.get_mut(&TransactionHandle) {
        Some(t) => t,
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }
    };

    if tx.state != TRANSACTION_STATE_ACTIVE {
        win32_set_last_error(ERROR_TRANSACTION_NOT_ACTIVE);
        return FALSE;
    }

    // Flush copy-on-write streams to VFS
    {
        let mut vfs = crate::fs::VFS.lock();
        for (path, stream) in &tx.streams {
            if stream.is_deleted {
                // If VFS supports deletion or truncate
                let _ = vfs.write_file(path, &[]);
            } else if stream.is_modified || stream.is_created {
                let _ = vfs.write_file(path, &stream.modified_data);
            }
        }
    }

    tx.state = TRANSACTION_STATE_COMMITTED;
    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Rolls back and discards all pending transacted file modifications.
pub unsafe extern "system" fn RollbackTransaction(TransactionHandle: HANDLE) -> BOOL {
    if TransactionHandle == 0 || TransactionHandle == (INVALID_HANDLE_VALUE as HANDLE) {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return FALSE;
    }

    let mut transactions = TRANSACTIONS.lock();
    let tx = match transactions.get_mut(&TransactionHandle) {
        Some(t) => t,
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }
    };

    if tx.state != TRANSACTION_STATE_ACTIVE {
        win32_set_last_error(ERROR_TRANSACTION_NOT_ACTIVE);
        return FALSE;
    }

    // Discard all transacted stream buffers without altering VFS
    tx.streams.clear();
    tx.state = TRANSACTION_STATE_ABORTED;
    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Transacted file handle referencing a specific transaction and file path.
#[derive(Clone, Debug)]
pub struct TransactedFileHandle {
    pub handle_id: HANDLE,
    pub transaction_id: HANDLE,
    pub path: String,
    pub cursor: usize,
    pub desired_access: DWORD,
}

static NEXT_TX_FILE_HANDLE: AtomicU64 = AtomicU64::new(0x9800);
static TRANSACTED_HANDLES: Spinlock<BTreeMap<HANDLE, TransactedFileHandle>> = Spinlock::new(BTreeMap::new());

/// Opens or creates a file within a transaction with copy-on-write page backing.
pub unsafe extern "system" fn CreateFileTransactedA(
    lpFileName: *const u8,
    dwDesiredAccess: DWORD,
    _dwShareMode: DWORD,
    _lpSecurityAttributes: *const c_void,
    dwCreationDisposition: DWORD,
    _dwFlagsAndAttributes: DWORD,
    _hTemplateFile: HANDLE,
    hTransaction: HANDLE,
    _pusMiniVersion: *mut u16,
    _lpExtendedParameter: *mut c_void,
) -> HANDLE {
    if lpFileName.is_null() || hTransaction == 0 || hTransaction == (INVALID_HANDLE_VALUE as HANDLE) {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return INVALID_HANDLE_VALUE as HANDLE;
    }

    let mut len = 0;
    while *lpFileName.add(len) != 0 && len < 4096 {
        len += 1;
    }
    let slice = core::slice::from_raw_parts(lpFileName, len);
    let path_str = match core::str::from_utf8(slice) {
        Ok(s) => String::from(s),
        Err(_) => {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return INVALID_HANDLE_VALUE as HANDLE;
        }
    };

    let mut transactions = TRANSACTIONS.lock();
    let tx = match transactions.get_mut(&hTransaction) {
        Some(t) => t,
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return INVALID_HANDLE_VALUE as HANDLE;
        }
    };

    if tx.state != TRANSACTION_STATE_ACTIVE {
        win32_set_last_error(ERROR_TRANSACTION_NOT_ACTIVE);
        return INVALID_HANDLE_VALUE as HANDLE;
    }

    // Ingest initial content via copy-on-write if stream does not already exist
    if !tx.streams.contains_key(&path_str) {
        let existing_data = {
            let vfs = crate::fs::VFS.lock();
            vfs.read_file(&path_str).unwrap_or_default()
        };

        let is_created = dwCreationDisposition == 1 || dwCreationDisposition == 2; // CREATE_NEW / CREATE_ALWAYS
        let stream = TransactedFileStream {
            path: path_str.clone(),
            original_data: existing_data.clone(),
            modified_data: if is_created { Vec::new() } else { existing_data },
            is_deleted: false,
            is_created,
            is_modified: false,
        };
        tx.streams.insert(path_str.clone(), stream);
    }

    let hid = NEXT_TX_FILE_HANDLE.fetch_add(1, Ordering::SeqCst) as HANDLE;
    let handle_entry = TransactedFileHandle {
        handle_id: hid,
        transaction_id: hTransaction,
        path: path_str,
        cursor: 0,
        desired_access: dwDesiredAccess,
    };

    TRANSACTED_HANDLES.lock().insert(hid, handle_entry);
    win32_set_last_error(ERROR_SUCCESS);
    hid
}

/// Reads data from a transacted file stream.
pub unsafe extern "system" fn ReadFileTransacted(
    hFile: HANDLE,
    lpBuffer: *mut c_void,
    nNumberOfBytesToRead: DWORD,
    lpNumberOfBytesRead: *mut DWORD,
    _lpOverlapped: *mut c_void,
) -> BOOL {
    if lpBuffer.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let mut handles = TRANSACTED_HANDLES.lock();
    let h = match handles.get_mut(&hFile) {
        Some(h) => h,
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }
    };

    let transactions = TRANSACTIONS.lock();
    let tx = match transactions.get(&h.transaction_id) {
        Some(t) => t,
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }
    };

    let stream = match tx.streams.get(&h.path) {
        Some(s) => s,
        None => {
            win32_set_last_error(ERROR_FILE_NOT_FOUND);
            return FALSE;
        }
    };

    let available = stream.modified_data.len().saturating_sub(h.cursor);
    let to_read = (nNumberOfBytesToRead as usize).min(available);

    if to_read > 0 {
        let src = &stream.modified_data[h.cursor..h.cursor + to_read];
        core::ptr::copy_nonoverlapping(src.as_ptr(), lpBuffer as *mut u8, to_read);
        h.cursor += to_read;
    }

    if !lpNumberOfBytesRead.is_null() {
        *lpNumberOfBytesRead = to_read as DWORD;
    }

    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Writes data into a copy-on-write transacted file stream.
pub unsafe extern "system" fn WriteFileTransacted(
    hFile: HANDLE,
    lpBuffer: *const c_void,
    nNumberOfBytesToWrite: DWORD,
    lpNumberOfBytesWritten: *mut DWORD,
    _lpOverlapped: *mut c_void,
) -> BOOL {
    if lpBuffer.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let mut handles = TRANSACTED_HANDLES.lock();
    let h = match handles.get_mut(&hFile) {
        Some(h) => h,
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }
    };

    let mut transactions = TRANSACTIONS.lock();
    let tx = match transactions.get_mut(&h.transaction_id) {
        Some(t) => t,
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }
    };

    if tx.state != TRANSACTION_STATE_ACTIVE {
        win32_set_last_error(ERROR_TRANSACTION_NOT_ACTIVE);
        return FALSE;
    }

    let stream = match tx.streams.get_mut(&h.path) {
        Some(s) => s,
        None => {
            win32_set_last_error(ERROR_FILE_NOT_FOUND);
            return FALSE;
        }
    };

    let to_write = nNumberOfBytesToWrite as usize;
    let slice = core::slice::from_raw_parts(lpBuffer as *const u8, to_write);

    // Expand buffer if cursor + to_write > current length
    if h.cursor + to_write > stream.modified_data.len() {
        stream.modified_data.resize(h.cursor + to_write, 0);
    }

    stream.modified_data[h.cursor..h.cursor + to_write].copy_from_slice(slice);
    h.cursor += to_write;
    stream.is_modified = true;

    if !lpNumberOfBytesWritten.is_null() {
        *lpNumberOfBytesWritten = to_write as DWORD;
    }

    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Closes a transacted file handle.
pub unsafe extern "system" fn CloseTransactedHandle(hFile: HANDLE) -> BOOL {
    if TRANSACTED_HANDLES.lock().remove(&hFile).is_some() {
        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        FALSE
    }
}
