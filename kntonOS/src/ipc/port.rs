//! Fast userland IPC port handles, endpoint registration, and lookup table.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

pub const MAX_PORT_NAME_LEN: usize = 32;
pub const MAX_PORTS: usize = 32;

pub const PORT_FLAG_ACTIVE: u16 = 0x0001;
pub const PORT_FLAG_SERVER: u16 = 0x0002;
pub const PORT_FLAG_ASYNC: u16 = 0x0004;

/// IPC subsystem error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    PortNotFound,
    PortTableFull,
    AlreadyExists,
    InvalidHandle,
    QueueFull,
    QueueEmpty,
    Timeout,
    InvalidPayload,
}

/// Opaque userland IPC port handle combining generation tag and slot index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PortHandle(pub u32);

impl PortHandle {
    pub const INVALID: Self = Self(0);

    pub const fn new(index: u16, generation: u16) -> Self {
        Self(((generation as u32) << 16) | (index as u32))
    }

    pub const fn index(&self) -> usize {
        (self.0 & 0xFFFF) as usize
    }

    pub const fn generation(&self) -> u16 {
        (self.0 >> 16) as u16
    }

    pub const fn is_valid(&self) -> bool {
        self.0 != 0
    }
}

/// IPC Port Endpoint descriptor registered by userland tasks and system daemons.
#[derive(Debug, Clone, Copy)]
pub struct PortEndpoint {
    pub handle: PortHandle,
    pub name: [u8; MAX_PORT_NAME_LEN],
    pub name_len: usize,
    pub owner_pid: u32,
    pub flags: u16,
    pub max_connections: u16,
    pub active_connections: u16,
}

impl PortEndpoint {
    pub const fn empty() -> Self {
        Self {
            handle: PortHandle::INVALID,
            name: [0; MAX_PORT_NAME_LEN],
            name_len: 0,
            owner_pid: 0,
            flags: 0,
            max_connections: 16,
            active_connections: 0,
        }
    }

    pub fn matches_name(&self, name_bytes: &[u8]) -> bool {
        if (self.flags & PORT_FLAG_ACTIVE) == 0 || self.name_len != name_bytes.len() {
            return false;
        }
        &self.name[..self.name_len] == name_bytes
    }
}

struct PortRegistryInner {
    endpoints: [PortEndpoint; MAX_PORTS],
    generations: [u16; MAX_PORTS],
    active_count: usize,
}

/// Thread-safe registry and lookup table for userland IPC endpoints.
pub struct PortRegistry {
    inner: UnsafeCell<PortRegistryInner>,
    lock: AtomicBool,
}

unsafe impl Sync for PortRegistry {}
unsafe impl Send for PortRegistry {}

impl PortRegistry {
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(PortRegistryInner {
                endpoints: [const { PortEndpoint::empty() }; MAX_PORTS],
                generations: [1; MAX_PORTS],
                active_count: 0,
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

    /// Registers a new IPC port endpoint under a unique string name.
    pub fn register_port(&self, name: &str, owner_pid: u32, flags: u16) -> Result<PortHandle, IpcError> {
        let name_bytes = name.as_bytes();
        if name_bytes.is_empty() || name_bytes.len() > MAX_PORT_NAME_LEN {
            return Err(IpcError::InvalidPayload);
        }

        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };

        // Ensure name is unique
        for i in 0..MAX_PORTS {
            if inner.endpoints[i].matches_name(name_bytes) {
                self.release();
                return Err(IpcError::AlreadyExists);
            }
        }

        // Find available slot
        let mut target_slot = None;
        for i in 0..MAX_PORTS {
            if (inner.endpoints[i].flags & PORT_FLAG_ACTIVE) == 0 {
                target_slot = Some(i);
                break;
            }
        }

        let slot = match target_slot {
            Some(s) => s,
            None => {
                self.release();
                return Err(IpcError::PortTableFull);
            }
        };

        let gen = inner.generations[slot];
        let handle = PortHandle::new((slot + 1) as u16, gen);

        let mut ep = PortEndpoint::empty();
        ep.handle = handle;
        ep.name[..name_bytes.len()].copy_from_slice(name_bytes);
        ep.name_len = name_bytes.len();
        ep.owner_pid = owner_pid;
        ep.flags = flags | PORT_FLAG_ACTIVE;

        inner.endpoints[slot] = ep;
        inner.active_count += 1;

        self.release();
        Ok(handle)
    }

    /// Looks up a registered port handle by its string identifier.
    pub fn lookup_port(&self, name: &str) -> Option<PortHandle> {
        let name_bytes = name.as_bytes();
        self.acquire();
        let inner = unsafe { &*self.inner.get() };

        for i in 0..MAX_PORTS {
            if inner.endpoints[i].matches_name(name_bytes) {
                let h = inner.endpoints[i].handle;
                self.release();
                return Some(h);
            }
        }

        self.release();
        None
    }

    /// Unregisters an existing port handle and increments the slot generation counter.
    pub fn unregister_port(&self, handle: PortHandle) -> Result<(), IpcError> {
        if !handle.is_valid() {
            return Err(IpcError::InvalidHandle);
        }
        let slot = handle.index().saturating_sub(1);
        if slot >= MAX_PORTS {
            return Err(IpcError::InvalidHandle);
        }

        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };

        if inner.endpoints[slot].handle != handle || (inner.endpoints[slot].flags & PORT_FLAG_ACTIVE) == 0 {
            self.release();
            return Err(IpcError::PortNotFound);
        }

        inner.endpoints[slot] = PortEndpoint::empty();
        inner.generations[slot] = inner.generations[slot].wrapping_add(1).max(1);
        inner.active_count = inner.active_count.saturating_sub(1);

        self.release();
        Ok(())
    }

    /// Retrieves a snapshot of the port endpoint descriptor.
    pub fn get_endpoint(&self, handle: PortHandle) -> Option<PortEndpoint> {
        if !handle.is_valid() {
            return None;
        }
        let slot = handle.index().saturating_sub(1);
        if slot >= MAX_PORTS {
            return None;
        }

        self.acquire();
        let inner = unsafe { &*self.inner.get() };
        let result = if inner.endpoints[slot].handle == handle && (inner.endpoints[slot].flags & PORT_FLAG_ACTIVE) != 0 {
            Some(inner.endpoints[slot])
        } else {
            None
        };
        self.release();
        result
    }

    /// Returns the number of currently active registered ports.
    pub fn active_port_count(&self) -> usize {
        self.acquire();
        let inner = unsafe { &*self.inner.get() };
        let count = inner.active_count;
        self.release();
        count
    }
}

pub static PORT_REGISTRY: PortRegistry = PortRegistry::new();
