//! Win32 Named Pipes & Anonymous IPC Engine.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
type HANDLE = u64;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use crate::memory::spinlock::Spinlock;

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

/// Unidirectional anonymous pipe channel state.
#[derive(Debug)]
pub struct AnonymousPipeChannel {
    pub buffer: PipeBuffer,
    pub has_reader: bool,
    pub has_writer: bool,
}

/// State machine lifecycle for a named pipe instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedPipeState {
    Listening,
    Connected,
    Disconnected,
}

/// Instance state for a bidirectional or unidirectional named pipe.
pub struct NamedPipeInstance {
    pub name: String,
    pub open_mode: u32,
    pub pipe_mode: u32,
    pub state: NamedPipeState,
    pub server_to_client: PipeBuffer,
    pub client_to_server: PipeBuffer,
    pub server_handle: Option<HANDLE>,
    pub client_handle: Option<HANDLE>,
}

/// Endpoint encapsulation for anonymous and named pipe handles.
#[derive(Clone)]
pub enum PipeEndpoint {
    Anonymous(Arc<Spinlock<AnonymousPipeChannel>>, PipeHandleRole),
    Named(Arc<Spinlock<NamedPipeInstance>>, PipeHandleRole),
}

static NEXT_PIPE_HANDLE: Spinlock<HANDLE> = Spinlock::new(0x000E_0001);
static PIPE_HANDLE_TABLE: Spinlock<BTreeMap<HANDLE, PipeEndpoint>> = Spinlock::new(BTreeMap::new());

/// Emulates Win32 CreatePipe creating an anonymous unidirectional pipe.
pub unsafe fn CreatePipe(
    ph_read_pipe: *mut HANDLE,
    ph_write_pipe: *mut HANDLE,
    _lp_pipe_attributes: *const u8,
    n_size: u32,
) -> bool {
    if ph_read_pipe.is_null() || ph_write_pipe.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    let mut gen = NEXT_PIPE_HANDLE.lock();
    let h_read = *gen;
    let h_write = h_read.saturating_add(1);
    *gen = h_write.saturating_add(1);
    drop(gen);

    let channel = Arc::new(Spinlock::new(AnonymousPipeChannel {
        buffer: PipeBuffer::new(n_size as usize),
        has_reader: true,
        has_writer: true,
    }));

    let mut table = PIPE_HANDLE_TABLE.lock();
    table.insert(h_read, PipeEndpoint::Anonymous(Arc::clone(&channel), PipeHandleRole::AnonymousReader));
    table.insert(h_write, PipeEndpoint::Anonymous(channel, PipeHandleRole::AnonymousWriter));

    *ph_read_pipe = h_read;
    *ph_write_pipe = h_write;
    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Checks whether the given handle refers to a registered pipe object.
pub fn is_pipe_handle(h_object: HANDLE) -> bool {
    let table = PIPE_HANDLE_TABLE.lock();
    table.contains_key(&h_object)
}

/// Closes a pipe handle, adjusting channel lifecycle state.
pub fn win32_close_pipe(h_object: HANDLE) -> bool {
    let mut table = PIPE_HANDLE_TABLE.lock();
    if let Some(endpoint) = table.remove(&h_object) {
        match endpoint {
            PipeEndpoint::Anonymous(channel, role) => {
                let mut chan = channel.lock();
                match role {
                    PipeHandleRole::AnonymousReader => chan.has_reader = false,
                    PipeHandleRole::AnonymousWriter => chan.has_writer = false,
                    _ => {}
                }
            }
            PipeEndpoint::Named(instance, role) => {
                let mut inst = instance.lock();
                match role {
                    PipeHandleRole::Server => {
                        inst.server_handle = None;
                        inst.state = NamedPipeState::Disconnected;
                    }
                    PipeHandleRole::Client => {
                        inst.client_handle = None;
                        inst.state = NamedPipeState::Disconnected;
                    }
                    _ => {}
                }
            }
        }
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        true
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        false
    }
}

/// Reads bytes from a pipe handle.
pub unsafe fn win32_read_pipe(
    h_pipe: HANDLE,
    lp_buffer: *mut u8,
    n_number_of_bytes_to_read: u32,
    lp_number_of_bytes_read: *mut u32,
) -> bool {
    if lp_buffer.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    let endpoint = {
        let table = PIPE_HANDLE_TABLE.lock();
        match table.get(&h_pipe).cloned() {
            Some(e) => e,
            None => {
                crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
                return false;
            }
        }
    };

    match endpoint {
        PipeEndpoint::Anonymous(channel, role) => {
            if role != PipeHandleRole::AnonymousReader {
                crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_ACCESS_DENIED);
                return false;
            }

            let mut chan = channel.lock();
            let dest = core::slice::from_raw_parts_mut(lp_buffer, n_number_of_bytes_to_read as usize);
            if chan.buffer.is_empty() {
                if !chan.has_writer {
                    if !lp_number_of_bytes_read.is_null() {
                        *lp_number_of_bytes_read = 0;
                    }
                    crate::pe::kernel32::win32_set_last_error(ERROR_BROKEN_PIPE);
                    return false;
                }
                if !lp_number_of_bytes_read.is_null() {
                    *lp_number_of_bytes_read = 0;
                }
                crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
                return true;
            }

            match chan.buffer.read(dest, false) {
                Ok(bytes) => {
                    if !lp_number_of_bytes_read.is_null() {
                        *lp_number_of_bytes_read = bytes as u32;
                    }
                    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
                    true
                }
                Err((bytes, err)) => {
                    if !lp_number_of_bytes_read.is_null() {
                        *lp_number_of_bytes_read = bytes as u32;
                    }
                    crate::pe::kernel32::win32_set_last_error(err);
                    false
                }
            }
        }
        PipeEndpoint::Named(instance, role) => {
            let mut inst = instance.lock();
            if inst.state != NamedPipeState::Connected {
                crate::pe::kernel32::win32_set_last_error(ERROR_PIPE_NOT_CONNECTED);
                return false;
            }

            let is_msg = (inst.pipe_mode & PIPE_READMODE_MESSAGE) != 0;
            let dest = core::slice::from_raw_parts_mut(lp_buffer, n_number_of_bytes_to_read as usize);
            let res = match role {
                PipeHandleRole::Server => inst.client_to_server.read(dest, is_msg),
                PipeHandleRole::Client => inst.server_to_client.read(dest, is_msg),
                _ => {
                    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_ACCESS_DENIED);
                    return false;
                }
            };

            match res {
                Ok(bytes) => {
                    if !lp_number_of_bytes_read.is_null() {
                        *lp_number_of_bytes_read = bytes as u32;
                    }
                    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
                    true
                }
                Err((bytes, err)) => {
                    if !lp_number_of_bytes_read.is_null() {
                        *lp_number_of_bytes_read = bytes as u32;
                    }
                    crate::pe::kernel32::win32_set_last_error(err);
                    false
                }
            }
        }
    }
}

/// Writes bytes to a pipe handle.
pub unsafe fn win32_write_pipe(
    h_pipe: HANDLE,
    lp_buffer: *const u8,
    n_number_of_bytes_to_write: u32,
    lp_number_of_bytes_written: *mut u32,
) -> bool {
    if lp_buffer.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    let endpoint = {
        let table = PIPE_HANDLE_TABLE.lock();
        match table.get(&h_pipe).cloned() {
            Some(e) => e,
            None => {
                crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
                return false;
            }
        }
    };

    match endpoint {
        PipeEndpoint::Anonymous(channel, role) => {
            if role != PipeHandleRole::AnonymousWriter {
                crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_ACCESS_DENIED);
                return false;
            }

            let mut chan = channel.lock();
            if !chan.has_reader {
                crate::pe::kernel32::win32_set_last_error(ERROR_BROKEN_PIPE);
                return false;
            }

            let src = core::slice::from_raw_parts(lp_buffer, n_number_of_bytes_to_write as usize);
            let written = chan.buffer.write(src, false);
            if !lp_number_of_bytes_written.is_null() {
                *lp_number_of_bytes_written = written as u32;
            }
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
            true
        }
        PipeEndpoint::Named(instance, role) => {
            let mut inst = instance.lock();
            if inst.state != NamedPipeState::Connected {
                crate::pe::kernel32::win32_set_last_error(ERROR_PIPE_NOT_CONNECTED);
                return false;
            }

            let is_msg = (inst.pipe_mode & PIPE_TYPE_MESSAGE) != 0;
            let src = core::slice::from_raw_parts(lp_buffer, n_number_of_bytes_to_write as usize);
            let written = match role {
                PipeHandleRole::Server => inst.server_to_client.write(src, is_msg),
                PipeHandleRole::Client => inst.client_to_server.write(src, is_msg),
                _ => {
                    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_ACCESS_DENIED);
                    return false;
                }
            };

            if !lp_number_of_bytes_written.is_null() {
                *lp_number_of_bytes_written = written as u32;
            }
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
            true
        }
    }
}

static NAMED_PIPES: Spinlock<BTreeMap<String, Vec<Arc<Spinlock<NamedPipeInstance>>>>> =
    Spinlock::new(BTreeMap::new());

/// Normalizes pipe name to canonical format (e.g. \\.\pipe\name in lowercase).
pub fn canonicalize_pipe_name(raw: &str) -> Option<String> {
    let s = raw.trim();
    let lower = s.to_lowercase();
    let name_part = if lower.starts_with(r"\\.\pipe\\") {
        &lower[9..]
    } else if lower.starts_with("//./pipe/") {
        &lower[9..]
    } else {
        return None;
    };
    if name_part.is_empty() {
        return None;
    }
    let mut out = String::from(r"\\.\pipe\\");
    out.push_str(name_part);
    Some(out)
}

/// Emulates Win32 CreateNamedPipeA creating an instance of a named pipe (ANSI).
pub unsafe fn CreateNamedPipeA(
    lp_name: *const u8,
    dw_open_mode: u32,
    dw_pipe_mode: u32,
    n_max_instances: u32,
    n_out_buffer_size: u32,
    n_in_buffer_size: u32,
    _n_default_time_out: u32,
    _lp_security_attributes: *const u8,
) -> HANDLE {
    if lp_name.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return INVALID_HANDLE_VALUE;
    }

    let mut len = 0;
    while *lp_name.add(len) != 0 && len < 260 {
        len += 1;
    }
    let slice = core::slice::from_raw_parts(lp_name, len);
    let s = match core::str::from_utf8(slice) {
        Ok(v) => v,
        Err(_) => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
            return INVALID_HANDLE_VALUE;
        }
    };

    create_named_pipe_internal(
        s,
        dw_open_mode,
        dw_pipe_mode,
        n_max_instances,
        n_out_buffer_size,
        n_in_buffer_size,
    )
}

/// Emulates Win32 CreateNamedPipeW creating an instance of a named pipe (UTF-16).
pub unsafe fn CreateNamedPipeW(
    lp_name: *const u16,
    dw_open_mode: u32,
    dw_pipe_mode: u32,
    n_max_instances: u32,
    n_out_buffer_size: u32,
    n_in_buffer_size: u32,
    _n_default_time_out: u32,
    _lp_security_attributes: *const u8,
) -> HANDLE {
    if lp_name.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return INVALID_HANDLE_VALUE;
    }

    let mut len = 0;
    while *lp_name.add(len) != 0 && len < 260 {
        len += 1;
    }
    let slice = core::slice::from_raw_parts(lp_name, len);
    let chars: Vec<char> = slice.iter().map(|&c| c as u8 as char).collect();
    let s: String = chars.into_iter().collect();

    create_named_pipe_internal(
        &s,
        dw_open_mode,
        dw_pipe_mode,
        n_max_instances,
        n_out_buffer_size,
        n_in_buffer_size,
    )
}

fn create_named_pipe_internal(
    raw_name: &str,
    dw_open_mode: u32,
    dw_pipe_mode: u32,
    n_max_instances: u32,
    n_out_buffer_size: u32,
    n_in_buffer_size: u32,
) -> HANDLE {
    let canonical = match canonicalize_pipe_name(raw_name) {
        Some(c) => c,
        None => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
            return INVALID_HANDLE_VALUE;
        }
    };

    let max_inst = if n_max_instances == 0 || n_max_instances >= PIPE_UNLIMITED_INSTANCES {
        PIPE_UNLIMITED_INSTANCES
    } else {
        n_max_instances
    };

    let mut registry = NAMED_PIPES.lock();
    let instances = registry.entry(canonical.clone()).or_insert_with(Vec::new);

    if instances.len() >= max_inst as usize {
        crate::pe::kernel32::win32_set_last_error(ERROR_PIPE_BUSY);
        return INVALID_HANDLE_VALUE;
    }

    let mut gen = NEXT_PIPE_HANDLE.lock();
    let h_server = *gen;
    *gen = h_server.saturating_add(1);
    drop(gen);

    let instance = Arc::new(Spinlock::new(NamedPipeInstance {
        name: canonical,
        open_mode: dw_open_mode,
        pipe_mode: dw_pipe_mode,
        state: NamedPipeState::Listening,
        server_to_client: PipeBuffer::new(n_out_buffer_size as usize),
        client_to_server: PipeBuffer::new(n_in_buffer_size as usize),
        server_handle: Some(h_server),
        client_handle: None,
    }));

    instances.push(Arc::clone(&instance));

    let mut table = PIPE_HANDLE_TABLE.lock();
    table.insert(h_server, PipeEndpoint::Named(instance, PipeHandleRole::Server));

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    h_server
}

/// Connects a client to an existing listening named pipe instance.
pub fn win32_connect_client_pipe(raw_name: &str, _desired_access: u32) -> HANDLE {
    let canonical = match canonicalize_pipe_name(raw_name) {
        Some(c) => c,
        None => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
            return INVALID_HANDLE_VALUE;
        }
    };

    let registry = NAMED_PIPES.lock();
    let instances = match registry.get(&canonical) {
        Some(list) => list,
        None => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_FILE_NOT_FOUND);
            return INVALID_HANDLE_VALUE;
        }
    };

    let target_instance = instances.iter().find(|inst| {
        let guard = inst.lock();
        guard.state == NamedPipeState::Listening
    });

    let target = match target_instance {
        Some(inst) => Arc::clone(inst),
        None => {
            crate::pe::kernel32::win32_set_last_error(ERROR_PIPE_BUSY);
            return INVALID_HANDLE_VALUE;
        }
    };
    drop(registry);

    let mut gen = NEXT_PIPE_HANDLE.lock();
    let h_client = *gen;
    *gen = h_client.saturating_add(1);
    drop(gen);

    {
        let mut inst = target.lock();
        inst.state = NamedPipeState::Connected;
        inst.client_handle = Some(h_client);
    }

    let mut table = PIPE_HANDLE_TABLE.lock();
    table.insert(h_client, PipeEndpoint::Named(target, PipeHandleRole::Client));

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    h_client
}

/// Emulates Win32 ConnectNamedPipe waiting for or acknowledging a client connection.
pub unsafe fn ConnectNamedPipe(h_named_pipe: HANDLE, _lp_overlapped: *const u8) -> bool {
    let endpoint = {
        let table = PIPE_HANDLE_TABLE.lock();
        match table.get(&h_named_pipe).cloned() {
            Some(e) => e,
            None => {
                crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
                return false;
            }
        }
    };

    match endpoint {
        PipeEndpoint::Named(instance, role) => {
            if role != PipeHandleRole::Server {
                crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_ACCESS_DENIED);
                return false;
            }
            let inst = instance.lock();
            match inst.state {
                NamedPipeState::Connected => {
                    crate::pe::kernel32::win32_set_last_error(ERROR_PIPE_CONNECTED);
                    false
                }
                NamedPipeState::Listening => {
                    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
                    true
                }
                NamedPipeState::Disconnected => {
                    crate::pe::kernel32::win32_set_last_error(ERROR_PIPE_NOT_CONNECTED);
                    false
                }
            }
        }
        _ => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
            false
        }
    }
}

/// Emulates Win32 DisconnectNamedPipe disconnecting a server pipe instance from its client.
pub unsafe fn DisconnectNamedPipe(h_named_pipe: HANDLE) -> bool {
    let endpoint = {
        let table = PIPE_HANDLE_TABLE.lock();
        match table.get(&h_named_pipe).cloned() {
            Some(e) => e,
            None => {
                crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
                return false;
            }
        }
    };

    match endpoint {
        PipeEndpoint::Named(instance, role) => {
            if role != PipeHandleRole::Server {
                crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_ACCESS_DENIED);
                return false;
            }
            let mut inst = instance.lock();
            inst.state = NamedPipeState::Disconnected;
            if let Some(client_h) = inst.client_handle.take() {
                drop(inst);
                let mut table = PIPE_HANDLE_TABLE.lock();
                table.remove(&client_h);
            }
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
            true
        }
        _ => {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
            false
        }
    }
}
