//! Inter-process communication port architecture.

pub mod port;
pub mod ring;
pub mod channel;

pub use port::*;
pub use ring::*;
pub use channel::*;

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

/// Comprehensive verification self-test for Userland IPC Port Service & Message Dispatcher.
pub fn verify_ipc_dispatcher_subsystem() -> bool {
    // 1. Port Registration, Duplicate Prevention & Lookup Table
    let port_name = "knton.ipc.sysmgr";
    let h1 = match PORT_REGISTRY.register_port(port_name, 10, PORT_FLAG_SERVER) {
        Ok(h) => h,
        Err(_) => return false,
    };

    // Duplicate registration under same name must fail
    if PORT_REGISTRY.register_port(port_name, 20, 0).is_ok() {
        return false;
    }

    // Lookup by name must match handle
    if PORT_REGISTRY.lookup_port(port_name) != Some(h1) {
        return false;
    }

    // Descriptor inspection
    let ep = match PORT_REGISTRY.get_endpoint(h1) {
        Some(e) => e,
        None => return false,
    };
    if ep.owner_pid != 10 || (ep.flags & PORT_FLAG_SERVER) == 0 {
        return false;
    }

    // 2. Zero-Copy Circular Ring-Buffer Queue
    let ring: ZeroCopyRingQueue<4> = ZeroCopyRingQueue::new();
    if !ring.is_empty() || ring.len() != 0 {
        return false;
    }

    let p1 = IpcPacket::new(10, 20, 0x101, 1, FLAG_REQUEST, b"PING");
    let p2 = IpcPacket::new(10, 20, 0x102, 2, FLAG_REQUEST, b"DATA");
    if ring.enqueue(&p1).is_err() || ring.enqueue(&p2).is_err() {
        return false;
    }
    if ring.len() != 2 {
        return false;
    }

    let p_peek = ring.peek();
    if p_peek.is_none() || p_peek.unwrap().sequence != 1 {
        return false;
    }

    let d1 = ring.dequeue();
    if d1.is_none() || d1.unwrap().payload_slice() != b"PING" {
        return false;
    }

    // Test in-place zero-copy write
    let res = ring.write_in_place(|pkt| {
        pkt.sender_pid = 99;
        pkt.sequence = 999;
        pkt.payload[0] = 0x77;
        pkt.payload_len = 1;
    });
    if res.is_err() {
        return false;
    }

    // 3. Bidirectional Synchronous RPC Channel
    let rpc = RpcChannel::new();
    let req_seq = match rpc.send_request(101, 202, 0x4001, b"GET_SYSTEM_LOAD") {
        Ok(seq) => seq,
        Err(_) => return false,
    };

    // Server receives request
    let incoming_req = match rpc.receive_request() {
        Some(r) => r,
        None => return false,
    };
    if incoming_req.sequence != req_seq || incoming_req.payload_slice() != b"GET_SYSTEM_LOAD" {
        return false;
    }

    // Server sends response
    if rpc.send_reply(202, 101, req_seq, 0x4002, b"LOAD_OK_15PCT").is_err() {
        return false;
    }

    // Client receives response
    let reply = match rpc.receive_reply(req_seq) {
        Some(rep) => rep,
        None => return false,
    };
    if reply.sequence != req_seq || reply.payload_slice() != b"LOAD_OK_15PCT" {
        return false;
    }

    // 4. Asynchronous Notification Channel
    let notif = NotificationChannel::new();
    if notif.subscribe(301).is_err() || notif.subscribe(302).is_err() {
        return false;
    }
    if notif.subscriber_count() != 2 {
        return false;
    }

    // Publish event
    let delivered = match notif.publish(10, 0x8001, b"TDP_CHANGED_25W") {
        Ok(d) => d,
        Err(_) => return false,
    };
    if delivered != 2 {
        return false;
    }

    // Both subscribers poll their notification
    let n1 = notif.poll_notification(301);
    let n2 = notif.poll_notification(302);
    if n1.is_none() || n2.is_none() {
        return false;
    }
    if n1.unwrap().payload_slice() != b"TDP_CHANGED_25W" || n2.unwrap().payload_slice() != b"TDP_CHANGED_25W" {
        return false;
    }

    // Cleanup port
    if PORT_REGISTRY.unregister_port(h1).is_err() {
        return false;
    }

    true
}
