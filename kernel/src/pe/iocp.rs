//! Win32 I/O Completion Ports (IOCP) & Asynchronous Packet Dispatcher.
//!
//! Emulates Windows NT I/O completion ports, asynchronous packet queuing,
//! multi-threaded worker dispatch, and high-performance game networking.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_SUCCESS, INVALID_HANDLE_VALUE,
};
pub use crate::pe::synch::{HANDLE, INFINITE, WAIT_TIMEOUT};
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

pub const ERROR_TIMEOUT: u32 = 1460;
pub const ERROR_ABANDONED_WAIT_0: u32 = 735;

/// Win32 OVERLAPPED structure used in asynchronous I/O operations.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct OVERLAPPED {
    pub Internal: usize,
    pub InternalHigh: usize,
    pub Offset: u32,
    pub OffsetHigh: u32,
    pub hEvent: HANDLE,
}
pub type LPOVERLAPPED = *mut OVERLAPPED;

/// Win32 OVERLAPPED_ENTRY structure for batch IOCP packet dequeue.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct OVERLAPPED_ENTRY {
    pub lpCompletionKey: usize,
    pub lpOverlapped: LPOVERLAPPED,
    pub Internal: usize,
    pub dwNumberOfBytesTransferred: u32,
}
pub type LPOVERLAPPED_ENTRY = *mut OVERLAPPED_ENTRY;

/// Completion packet queued in an I/O completion port.
#[derive(Debug, Clone, Copy)]
pub struct CompletionPacket {
    pub bytes_transferred: u32,
    pub completion_key: usize,
    pub overlapped_ptr: usize,
    pub status: u32,
}

/// Internal descriptor for an I/O completion port.
pub struct IoCompletionPortInner {
    pub id: HANDLE,
    pub max_concurrency: u32,
    pub active_threads: AtomicU32,
    pub queue: VecDeque<CompletionPacket>,
    pub associated_handles: BTreeMap<HANDLE, usize>,
    pub closed: bool,
}

impl IoCompletionPortInner {
    /// Pushes a completed I/O packet onto the completion queue.
    pub fn push_packet(&mut self, packet: CompletionPacket) {
        self.queue.push_back(packet);
    }

    /// Pops the next completed I/O packet in FIFO order.
    pub fn pop_packet(&mut self) -> Option<CompletionPacket> {
        self.queue.pop_front()
    }

    /// Checks if the port has reached its maximum concurrent running threads limit.
    pub fn is_at_concurrency_limit(&self) -> bool {
        self.active_threads.load(Ordering::SeqCst) >= self.max_concurrency
    }

    /// Increments the active running thread count when a thread wakes up with a packet.
    pub fn enter_active_thread(&self) {
        self.active_threads.fetch_add(1, Ordering::SeqCst);
    }

    /// Decrements the active running thread count when a thread blocks or yields.
    pub fn leave_active_thread(&self) {
        self.active_threads.fetch_sub(1, Ordering::SeqCst);
    }
}

static NEXT_IOCP_ID: AtomicU64 = AtomicU64::new(0x9000_1000);
pub static IOCP_PORTS: Spinlock<BTreeMap<HANDLE, IoCompletionPortInner>> =
    Spinlock::new(BTreeMap::new());

/// Creates an I/O completion port or associates a file handle with an existing port.
pub unsafe extern "system" fn CreateIoCompletionPort(
    FileHandle: HANDLE,
    ExistingCompletionPort: HANDLE,
    CompletionKey: usize,
    NumberOfConcurrentThreads: u32,
) -> HANDLE {
    let mut lock = IOCP_PORTS.lock();

    if ExistingCompletionPort == 0 {
        // Create a new completion port.
        let port_id = NEXT_IOCP_ID.fetch_add(1, Ordering::SeqCst) as HANDLE;
        let max_threads = if NumberOfConcurrentThreads == 0 { 4 } else { NumberOfConcurrentThreads };
        let mut port = IoCompletionPortInner {
            id: port_id,
            max_concurrency: max_threads,
            active_threads: AtomicU32::new(0),
            queue: VecDeque::new(),
            associated_handles: BTreeMap::new(),
            closed: false,
        };

        if FileHandle != (INVALID_HANDLE_VALUE as HANDLE) && FileHandle != 0 {
            port.associated_handles.insert(FileHandle, CompletionKey);
        }

        lock.insert(port_id, port);
        win32_set_last_error(ERROR_SUCCESS);
        port_id
    } else {
        // Associate an existing completion port with FileHandle.
        if let Some(port) = lock.get_mut(&ExistingCompletionPort) {
            if FileHandle != (INVALID_HANDLE_VALUE as HANDLE) && FileHandle != 0 {
                port.associated_handles.insert(FileHandle, CompletionKey);
            }
            win32_set_last_error(ERROR_SUCCESS);
            ExistingCompletionPort
        } else {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            0
        }
    }
}

/// Enqueues an I/O completion packet to the specified completion port.
pub unsafe extern "system" fn PostQueuedCompletionStatus(
    CompletionPort: HANDLE,
    dwNumberOfBytesTransferred: u32,
    dwCompletionKey: usize,
    lpOverlapped: LPOVERLAPPED,
) -> BOOL {
    if CompletionPort == 0 {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        return FALSE;
    }

    let mut lock = IOCP_PORTS.lock();
    if let Some(port) = lock.get_mut(&CompletionPort) {
        if port.closed {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }

        let packet = CompletionPacket {
            bytes_transferred: dwNumberOfBytesTransferred,
            completion_key: dwCompletionKey,
            overlapped_ptr: lpOverlapped as usize,
            status: 0,
        };

        port.push_packet(packet);
        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        win32_set_last_error(ERROR_INVALID_HANDLE);
        FALSE
    }
}
