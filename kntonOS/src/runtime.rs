//! User-mode execution runtime and syscall callgate interface.

pub mod allocator;
pub mod syscall;

pub use allocator::*;
pub use syscall::*;

/// Process Environment Block (PEB) representation for userland processes.
#[derive(Debug, Clone, Copy)]
pub struct ProcessEnvironmentBlock {
    pub process_id: u32,
    pub image_base_address: u64,
    pub heap_base_address: u64,
    pub total_threads: u32,
    pub is_gui_subsystem: bool,
}

impl ProcessEnvironmentBlock {
    pub const fn new(pid: u32, image_base: u64) -> Self {
        Self {
            process_id: pid,
            image_base_address: image_base,
            heap_base_address: 0x0000_0001_0000_0000,
            total_threads: 1,
            is_gui_subsystem: false,
        }
    }
}

/// Thread Environment Block (TEB) representation for userland execution fibers.
#[derive(Debug, Clone, Copy)]
pub struct ThreadEnvironmentBlock {
    pub thread_id: u32,
    pub stack_base: u64,
    pub stack_limit: u64,
    pub tls_slots: [u64; 64],
}

impl ThreadEnvironmentBlock {
    pub const fn new(tid: u32, stack_base: u64, stack_size: u64) -> Self {
        Self {
            thread_id: tid,
            stack_base,
            stack_limit: stack_base.saturating_sub(stack_size),
            tls_slots: [0; 64],
        }
    }
}

/// Fast syscall dispatch simulation for userspace operations.
pub fn fast_syscall(syscall_num: u32, arg1: u64, arg2: u64, arg3: u64) -> i64 {
    dynamic_syscall(syscall_num, arg1, arg2, arg3)
}

/// Verification self-test for userspace runtime.
pub fn verify_user_runtime() -> bool {
    let peb = ProcessEnvironmentBlock::new(101, 0x0000_0000_0040_0000);
    if peb.process_id != 101 || peb.image_base_address != 0x0040_0000 {
        return false;
    }

    let teb = ThreadEnvironmentBlock::new(202, 0x0000_7FFF_FFFF_0000, 0x10_000);
    if teb.thread_id != 202 || teb.stack_limit >= teb.stack_base {
        return false;
    }

    if !validate_teb_pointer(0x0000_7FFF_FFFF_1000, teb.stack_base, teb.stack_limit) {
        return false;
    }

    let ret = fast_syscall(0x01, 10, 20, 30);
    if ret != 60 {
        return false;
    }

    true
}
