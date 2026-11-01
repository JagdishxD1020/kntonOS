//! Zero-copy ring-buffer message queues between userland tasks and service daemons.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use super::port::IpcError;

pub const MAX_PACKET_PAYLOAD: usize = 128;
pub const DEFAULT_RING_CAPACITY: usize = 16;

pub const FLAG_REQUEST: u16 = 0x0001;
pub const FLAG_RESPONSE: u16 = 0x0002;
pub const FLAG_ASYNC: u16 = 0x0004;
pub const FLAG_URGENT: u16 = 0x0008;

/// Fixed-size IPC packet format optimized for zero-copy memory mapping and fast serialization.
#[derive(Debug, Clone, Copy)]
#[repr(C, align(8))]
pub struct IpcPacket {
    pub sender_pid: u32,
    pub receiver_pid: u32,
    pub msg_id: u32,
    pub sequence: u64,
    pub flags: u16,
    pub payload_len: u16,
    pub reserved: u32,
    pub payload: [u8; MAX_PACKET_PAYLOAD],
}

impl IpcPacket {
    pub const fn empty() -> Self {
        Self {
            sender_pid: 0,
            receiver_pid: 0,
            msg_id: 0,
            sequence: 0,
            flags: 0,
            payload_len: 0,
            reserved: 0,
            payload: [0; MAX_PACKET_PAYLOAD],
        }
    }

    pub fn new(sender_pid: u32, receiver_pid: u32, msg_id: u32, seq: u64, flags: u16, data: &[u8]) -> Self {
        let mut pkt = Self::empty();
        pkt.sender_pid = sender_pid;
        pkt.receiver_pid = receiver_pid;
        pkt.msg_id = msg_id;
        pkt.sequence = seq;
        pkt.flags = flags;
        let copy_len = if data.len() > MAX_PACKET_PAYLOAD { MAX_PACKET_PAYLOAD } else { data.len() };
        pkt.payload[..copy_len].copy_from_slice(&data[..copy_len]);
        pkt.payload_len = copy_len as u16;
        pkt
    }

    pub fn payload_slice(&self) -> &[u8] {
        &self.payload[..self.payload_len as usize]
    }
}

/// Zero-copy circular ring buffer queue supporting thread-safe enqueuing and dequeuing.
pub struct ZeroCopyRingQueue<const CAPACITY: usize = DEFAULT_RING_CAPACITY> {
    storage: [UnsafeCell<IpcPacket>; CAPACITY],
    head: AtomicUsize,
    tail: AtomicUsize,
    count: AtomicUsize,
    lock: AtomicBool,
}

unsafe impl<const CAPACITY: usize> Sync for ZeroCopyRingQueue<CAPACITY> {}
unsafe impl<const CAPACITY: usize> Send for ZeroCopyRingQueue<CAPACITY> {}

impl<const CAPACITY: usize> ZeroCopyRingQueue<CAPACITY> {
    pub const fn new() -> Self {
        const EMPTY_PACKET: UnsafeCell<IpcPacket> = UnsafeCell::new(IpcPacket::empty());
        Self {
            storage: [EMPTY_PACKET; CAPACITY],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
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

    pub fn is_empty(&self) -> bool {
        self.count.load(Ordering::Acquire) == 0
    }

    pub fn is_full(&self) -> bool {
        self.count.load(Ordering::Acquire) >= CAPACITY
    }

    pub fn len(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }

    /// Enqueues a packet into the ring buffer with backpressure protection.
    pub fn enqueue(&self, packet: &IpcPacket) -> Result<(), IpcError> {
        self.acquire();
        if self.count.load(Ordering::Relaxed) >= CAPACITY {
            self.release();
            return Err(IpcError::QueueFull);
        }

        let tail = self.tail.load(Ordering::Relaxed);
        unsafe {
            *self.storage[tail].get() = *packet;
        }

        self.tail.store((tail + 1) % CAPACITY, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Release);
        self.release();
        Ok(())
    }

    /// Dequeues the next pending packet from the ring buffer.
    pub fn dequeue(&self) -> Option<IpcPacket> {
        self.acquire();
        if self.count.load(Ordering::Relaxed) == 0 {
            self.release();
            return None;
        }

        let head = self.head.load(Ordering::Relaxed);
        let packet = unsafe { *self.storage[head].get() };

        self.head.store((head + 1) % CAPACITY, Ordering::Relaxed);
        self.count.fetch_sub(1, Ordering::Release);
        self.release();
        Some(packet)
    }

    /// Inspects the head packet without removing it from the queue.
    pub fn peek(&self) -> Option<IpcPacket> {
        self.acquire();
        if self.count.load(Ordering::Relaxed) == 0 {
            self.release();
            return None;
        }

        let head = self.head.load(Ordering::Relaxed);
        let packet = unsafe { *self.storage[head].get() };
        self.release();
        Some(packet)
    }

    /// Allows in-place zero-copy construction of packet payload.
    pub fn write_in_place<F: FnOnce(&mut IpcPacket)>(&self, f: F) -> Result<(), IpcError> {
        self.acquire();
        if self.count.load(Ordering::Relaxed) >= CAPACITY {
            self.release();
            return Err(IpcError::QueueFull);
        }

        let tail = self.tail.load(Ordering::Relaxed);
        let slot = unsafe { &mut *self.storage[tail].get() };
        *slot = IpcPacket::empty();
        f(slot);

        self.tail.store((tail + 1) % CAPACITY, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Release);
        self.release();
        Ok(())
    }
}
