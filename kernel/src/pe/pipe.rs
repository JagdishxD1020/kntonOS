//! Win32 Named Pipes & Anonymous IPC Engine.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use alloc::collections::VecDeque;
use crate::pe::synch::HANDLE;

// Pipe open modes
pub const PIPE_ACCESS_INBOUND: u32 = 0x00000001;
pub const PIPE_ACCESS_OUTBOUND: u32 = 0x00000002;
pub const PIPE_ACCESS_DUPLEX: u32 = 0x00000003;

// Pipe type, read, and wait modes
pub const PIPE_TYPE_BYTE: u32 = 0x00000000;
pub const PIPE_TYPE_MESSAGE: u32 = 0x00000004;
pub const PIPE_READMODE_BYTE: u32 = 0x00000000;
pub const PIPE_READMODE_MESSAGE: u32 = 0x00000002;
pub const PIPE_WAIT: u32 = 0x00000000;
pub const PIPE_NOWAIT: u32 = 0x00000001;
pub const PIPE_ACCEPT_REMOTE_CLIENTS: u32 = 0x00000000;
pub const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x00000008;

// Named pipe limits and timeouts
pub const PIPE_UNLIMITED_INSTANCES: u32 = 255;
pub const NMPWAIT_WAIT_FOREVER: u32 = 0xffffffff;
pub const NMPWAIT_NOWAIT: u32 = 0x00000001;
pub const NMPWAIT_USE_DEFAULT_WAIT: u32 = 0x00000000;

// Pipe specific Win32 error codes
pub const ERROR_BROKEN_PIPE: u32 = 109;
pub const ERROR_BAD_PIPE: u32 = 230;
pub const ERROR_PIPE_BUSY: u32 = 231;
pub const ERROR_NO_DATA: u32 = 232;
pub const ERROR_PIPE_NOT_CONNECTED: u32 = 233;
pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_PIPE_CONNECTED: u32 = 535;

pub use crate::pe::kernel32::INVALID_HANDLE_VALUE;

/// Identifies the role and directionality of an allocated pipe handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeHandleRole {
    Server,
    Client,
    AnonymousReader,
    AnonymousWriter,
}

/// Metadata entry associated with an active pipe handle.
#[derive(Debug, Clone)]
pub struct PipeHandleEntry {
    pub handle: HANDLE,
    pub role: PipeHandleRole,
}

/// Thread-safe circular buffer supporting streaming bytes and framed message packets.
#[derive(Debug, Clone)]
pub struct PipeBuffer {
    data: VecDeque<u8>,
    messages: VecDeque<usize>,
    capacity: usize,
}

impl PipeBuffer {
    pub const DEFAULT_CAPACITY: usize = 65536;

    /// Creates a new circular buffer with the given capacity limit (0 for default).
    pub fn new(capacity: usize) -> Self {
        let cap = if capacity == 0 { Self::DEFAULT_CAPACITY } else { capacity };
        Self {
            data: VecDeque::with_capacity(cap.min(4096)),
            messages: VecDeque::new(),
            capacity: cap,
        }
    }

    /// Writes data into buffer, recording message boundary if is_message is true.
    pub fn write(&mut self, bytes: &[u8], is_message: bool) -> usize {
        if bytes.is_empty() {
            return 0;
        }
        let available_space = if self.capacity > 0 {
            self.capacity.saturating_sub(self.data.len())
        } else {
            bytes.len()
        };
        let to_write = core::cmp::min(bytes.len(), available_space);
        if to_write == 0 {
            return 0;
        }

        for &b in &bytes[..to_write] {
            self.data.push_back(b);
        }

        if is_message {
            self.messages.push_back(to_write);
        } else if let Some(last) = self.messages.back_mut() {
            *last += to_write;
        } else {
            self.messages.push_back(to_write);
        }
        to_write
    }

    /// Reads data from buffer adhering to message mode or byte mode.
    pub fn read(&mut self, dest: &mut [u8], message_mode: bool) -> Result<usize, (usize, u32)> {
        if dest.is_empty() || self.data.is_empty() {
            return Ok(0);
        }

        if message_mode {
            let cur_msg_len = self.messages.front().copied().unwrap_or(self.data.len());
            if dest.len() >= cur_msg_len {
                for b in dest[..cur_msg_len].iter_mut() {
                    *b = self.data.pop_front().unwrap_or(0);
                }
                self.messages.pop_front();
                Ok(cur_msg_len)
            } else {
                let to_read = dest.len();
                for b in dest.iter_mut() {
                    *b = self.data.pop_front().unwrap_or(0);
                }
                if let Some(front) = self.messages.front_mut() {
                    *front = front.saturating_sub(to_read);
                }
                Err((to_read, ERROR_MORE_DATA))
            }
        } else {
            let to_read = core::cmp::min(dest.len(), self.data.len());
            for b in dest[..to_read].iter_mut() {
                *b = self.data.pop_front().unwrap_or(0);
            }
            let mut remaining_to_deduct = to_read;
            while remaining_to_deduct > 0 && !self.messages.is_empty() {
                if let Some(front) = self.messages.front_mut() {
                    if *front <= remaining_to_deduct {
                        remaining_to_deduct -= *front;
                        self.messages.pop_front();
                    } else {
                        *front -= remaining_to_deduct;
                        remaining_to_deduct = 0;
                    }
                }
            }
            Ok(to_read)
        }
    }

    /// Previews buffer contents without advancing read pointers.
    pub fn peek(&self, dest: &mut [u8]) -> (usize, usize, usize) {
        let total_avail = self.data.len();
        let to_copy = core::cmp::min(dest.len(), total_avail);
        for (i, b) in dest[..to_copy].iter_mut().enumerate() {
            *b = self.data[i];
        }
        let bytes_left_this_message = self.messages.front().copied().unwrap_or(0);
        (to_copy, total_avail, bytes_left_this_message)
    }

    /// Returns the number of unread bytes in buffer.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Checks whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Clears all buffered data and message headers.
    pub fn clear(&mut self) {
        self.data.clear();
        self.messages.clear();
    }
}
