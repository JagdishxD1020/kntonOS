//! Inter-process communication port architecture.

pub mod port;

pub use port::*;

pub const MAX_PORT_PAYLOAD: usize = 64;

/// IPC message packet transmitted between userland tasks.
#[derive(Debug, Clone, Copy)]
pub struct IpcMessage {
    pub sender_pid: u32,
    pub message_id: u32,
    pub payload_len: usize,
    pub payload: [u8; MAX_PORT_PAYLOAD],
}

impl IpcMessage {
    pub const fn new(sender_pid: u32, message_id: u32) -> Self {
        Self {
            sender_pid,
            message_id,
            payload_len: 0,
            payload: [0; MAX_PORT_PAYLOAD],
        }
    }
}

/// Verification test for IPC port primitives.
pub fn verify_ipc_ports() -> bool {
    let msg = IpcMessage::new(42, 0x100);
    if msg.sender_pid != 42 || msg.message_id != 0x100 {
        return false;
    }

    // Verify basic port handle registration and lookup
    let handle = match PORT_REGISTRY.register_port("knton.test.port", 100, PORT_FLAG_SERVER) {
        Ok(h) => h,
        Err(_) => return false,
    };

    if PORT_REGISTRY.lookup_port("knton.test.port") != Some(handle) {
        return false;
    }

    if PORT_REGISTRY.unregister_port(handle).is_err() {
        return false;
    }

    true
}
