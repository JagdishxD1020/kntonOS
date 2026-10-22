//! Win32 Thread Pool High-Priority Game Worker Queues & Work-Stealing Fiber Scheduler.
//!
//! Implements a Chase-Lev work-stealing double-ended queue (deque) for thread pool workers,
//! high-priority game task scheduling with performance core pinning, and fiber migration.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;

pub const CHASE_LEV_INITIAL_CAPACITY: usize = 64;

/// Work item task wrapper for worker queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadPoolWorkItem {
    pub work_id: u64,
    pub task_fn: usize,
    pub context: usize,
    pub priority: u8, // 0 = normal, 1 = high, 2 = critical game render/audio
    pub enqueue_tsc: u64,
}

/// Chase-Lev work-stealing deque for thread pool worker threads.
/// Single producer (worker thread) pushes/pops at bottom; multiple thieves steal at top.
pub struct ChaseLevDeque {
    buffer: Spinlock<Vec<ThreadPoolWorkItem>>,
    top: AtomicIsize,
    bottom: AtomicIsize,
}

impl ChaseLevDeque {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Spinlock::new(Vec::with_capacity(capacity)),
            top: AtomicIsize::new(0),
            bottom: AtomicIsize::new(0),
        }
    }

    /// Owner thread pushes a work item onto the bottom of the deque.
    pub fn push(&self, item: ThreadPoolWorkItem) {
        let mut b = self.bottom.load(Ordering::Relaxed);
        let t = self.top.load(Ordering::Acquire);
        let mut buf = self.buffer.lock();

        let size = b - t;
        if size < 0 {
            // Underflow reset
            self.top.store(0, Ordering::Relaxed);
            self.bottom.store(0, Ordering::Relaxed);
            b = 0;
        }

        buf.push(item);
        self.bottom.store(b + 1, Ordering::Release);
    }

    /// Owner thread pops a work item from the bottom of the deque (LIFO order).
    pub fn pop(&self) -> Option<ThreadPoolWorkItem> {
        let b = self.bottom.load(Ordering::Relaxed) - 1;
        self.bottom.store(b, Ordering::Relaxed);
        core::sync::atomic::fence(Ordering::SeqCst);

        let t = self.top.load(Ordering::Relaxed);
        if t <= b {
            // Non-empty deque
            let mut buf = self.buffer.lock();
            if let Some(item) = buf.pop() {
                if t == b {
                    // Single item contention with concurrent thief
                    if self.top.compare_exchange(t, t + 1, Ordering::SeqCst, Ordering::Relaxed).is_err() {
                        // Lost race to a thief
                        self.bottom.store(t + 1, Ordering::Relaxed);
                        return None;
                    }
                    self.bottom.store(t + 1, Ordering::Relaxed);
                }
                return Some(item);
            }
            self.bottom.store(t, Ordering::Relaxed);
            None
        } else {
            // Empty deque
            self.bottom.store(t, Ordering::Relaxed);
            None
        }
    }

    /// Thief thread steals a work item from the top of the deque (FIFO order).
    pub fn steal(&self) -> Option<ThreadPoolWorkItem> {
        let t = self.top.load(Ordering::Acquire);
        core::sync::atomic::fence(Ordering::SeqCst);
        let b = self.bottom.load(Ordering::Acquire);

        if t < b {
            let mut buf = self.buffer.lock();
            if !buf.is_empty() {
                if self.top.compare_exchange(t, t + 1, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
                    return Some(buf.remove(0));
                }
            }
            None
        } else {
            None
        }
    }

    /// Check if deque is empty.
    pub fn is_empty(&self) -> bool {
        self.top.load(Ordering::Acquire) >= self.bottom.load(Ordering::Acquire)
    }
}
