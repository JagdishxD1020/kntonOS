//! Bidirectional synchronous RPC request-reply and asynchronous notification channels.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use super::port::IpcError;
use super::ring::{IpcPacket, ZeroCopyRingQueue, FLAG_ASYNC, FLAG_REQUEST, FLAG_RESPONSE};

pub const MAX_SUBSCRIBERS: usize = 16;
pub const RPC_QUEUE_DEPTH: usize = 16;
pub const NOTIF_QUEUE_DEPTH: usize = 32;

/// Bidirectional synchronous RPC channel pairing request and reply queues.
pub struct RpcChannel {
    request_queue: ZeroCopyRingQueue<RPC_QUEUE_DEPTH>,
    reply_queue: ZeroCopyRingQueue<RPC_QUEUE_DEPTH>,
    sequence_gen: AtomicU64,
}

impl RpcChannel {
    pub const fn new() -> Self {
        Self {
            request_queue: ZeroCopyRingQueue::new(),
            reply_queue: ZeroCopyRingQueue::new(),
            sequence_gen: AtomicU64::new(1),
        }
    }

    /// Dispatches a synchronous RPC request into the server queue.
    pub fn send_request(&self, sender_pid: u32, receiver_pid: u32, msg_id: u32, payload: &[u8]) -> Result<u64, IpcError> {
        let seq = self.sequence_gen.fetch_add(1, Ordering::SeqCst);
        let packet = IpcPacket::new(sender_pid, receiver_pid, msg_id, seq, FLAG_REQUEST, payload);
        self.request_queue.enqueue(&packet)?;
        Ok(seq)
    }

    /// Server side: dequeues the next incoming RPC request.
    pub fn receive_request(&self) -> Option<IpcPacket> {
        self.request_queue.dequeue()
    }

    /// Server side: sends response matching the request sequence number.
    pub fn send_reply(&self, sender_pid: u32, receiver_pid: u32, seq: u64, msg_id: u32, payload: &[u8]) -> Result<(), IpcError> {
        let packet = IpcPacket::new(sender_pid, receiver_pid, msg_id, seq, FLAG_RESPONSE, payload);
        self.reply_queue.enqueue(&packet)
    }

    /// Client side: polls for the response packet matching the expected sequence number.
    pub fn receive_reply(&self, expected_seq: u64) -> Option<IpcPacket> {
        // Linear scan of pending replies
        let count = self.reply_queue.len();
        for _ in 0..count {
            if let Some(pkt) = self.reply_queue.dequeue() {
                if pkt.sequence == expected_seq {
                    return Some(pkt);
                }
                // Put back if not matching
                let _ = self.reply_queue.enqueue(&pkt);
            }
        }
        None
    }

    /// Full synchronous RPC roundtrip helper (send request and immediately poll for reply).
    pub fn call_rpc(&self, sender_pid: u32, receiver_pid: u32, msg_id: u32, payload: &[u8]) -> Result<IpcPacket, IpcError> {
        let seq = self.send_request(sender_pid, receiver_pid, msg_id, payload)?;
        match self.receive_reply(seq) {
            Some(pkt) => Ok(pkt),
            None => Err(IpcError::Timeout),
        }
    }
}

struct NotificationInner {
    subscribers: [u32; MAX_SUBSCRIBERS],
    count: usize,
}

/// Asynchronous notification broadcast channel.
pub struct NotificationChannel {
    event_queue: ZeroCopyRingQueue<NOTIF_QUEUE_DEPTH>,
    inner: UnsafeCell<NotificationInner>,
    lock: AtomicBool,
}

unsafe impl Sync for NotificationChannel {}
unsafe impl Send for NotificationChannel {}

impl NotificationChannel {
    pub const fn new() -> Self {
        Self {
            event_queue: ZeroCopyRingQueue::new(),
            inner: UnsafeCell::new(NotificationInner {
                subscribers: [0; MAX_SUBSCRIBERS],
                count: 0,
            }),
            lock: AtomicBool::new(false),
        }
    }

    fn acquire(&self) {
        while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
    }

    fn release(&self) {
        self.lock.store(false, Ordering::Release);
    }

    /// Subscribes a userland task PID to async notifications.
    pub fn subscribe(&self, pid: u32) -> Result<(), IpcError> {
        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };

        for i in 0..inner.count {
            if inner.subscribers[i] == pid {
                self.release();
                return Ok(());
            }
        }

        if inner.count >= MAX_SUBSCRIBERS {
            self.release();
            return Err(IpcError::PortTableFull);
        }

        inner.subscribers[inner.count] = pid;
        inner.count += 1;
        self.release();
        Ok(())
    }

    /// Unsubscribes a task PID.
    pub fn unsubscribe(&self, pid: u32) -> Result<(), IpcError> {
        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };

        for i in 0..inner.count {
            if inner.subscribers[i] == pid {
                inner.subscribers[i] = inner.subscribers[inner.count - 1];
                inner.subscribers[inner.count - 1] = 0;
                inner.count -= 1;
                self.release();
                return Ok(());
            }
        }

        self.release();
        Err(IpcError::PortNotFound)
    }

    /// Publishes an asynchronous event to all active subscribers.
    pub fn publish(&self, sender_pid: u32, msg_id: u32, payload: &[u8]) -> Result<usize, IpcError> {
        self.acquire();
        let inner = unsafe { &*self.inner.get() };
        let subs = inner.subscribers;
        let count = inner.count;
        self.release();

        let mut delivered = 0;
        for i in 0..count {
            let target_pid = subs[i];
            let pkt = IpcPacket::new(sender_pid, target_pid, msg_id, 0, FLAG_ASYNC, payload);
            if self.event_queue.enqueue(&pkt).is_ok() {
                delivered += 1;
            }
        }
        Ok(delivered)
    }

    /// Polls for the next asynchronous notification destined for the given task PID.
    pub fn poll_notification(&self, pid: u32) -> Option<IpcPacket> {
        let count = self.event_queue.len();
        for _ in 0..count {
            if let Some(pkt) = self.event_queue.dequeue() {
                if pkt.receiver_pid == pid {
                    return Some(pkt);
                }
                let _ = self.event_queue.enqueue(&pkt);
            }
        }
        None
    }

    /// Returns current subscriber count.
    pub fn subscriber_count(&self) -> usize {
        self.acquire();
        let inner = unsafe { &*self.inner.get() };
        let count = inner.count;
        self.release();
        count
    }
}
