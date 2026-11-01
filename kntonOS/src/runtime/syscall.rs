//! Dynamic system call stubs and Thread Environment Block (TEB) validation.

use core::sync::atomic::{AtomicUsize, Ordering};

pub const SYS_TEST: u32 = 0x01;
pub const SYS_ALLOC: u32 = 0x10;
pub const SYS_FREE: u32 = 0x11;
pub const SYS_SPAWN: u32 = 0x15;
pub const SYS_YIELD: u32 = 0x20;

pub const STATUS_SUCCESS: i32 = 0;
pub const STATUS_INVALID_PARAMETER: i32 = -1;
pub const STATUS_NO_MEMORY: i32 = -2;

/// Simulated user virtual memory arena backing dynamic syscall allocations (256 KiB).
pub const ARENA_SIZE: usize = 512 * 1024;

#[repr(align(4096))]
struct UserVirtualArena {
    storage: [u8; ARENA_SIZE],
}

static mut USER_ARENA: UserVirtualArena = UserVirtualArena {
    storage: [0; ARENA_SIZE],
};

static ARENA_OFFSET: AtomicUsize = AtomicUsize::new(0);
static YIELD_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Validates TEB pointer alignment, canonical user address boundary, and stack limits.
pub fn validate_teb_pointer(teb_ptr: u64, stack_base: u64, stack_limit: u64) -> bool {
    if teb_ptr == 0 || (teb_ptr & 0x07) != 0 {
        return false;
    }
    // Canonical userspace boundary check
    if teb_ptr < 0x0000_0000_0001_0000 || teb_ptr >= 0x0000_8000_0000_0000 {
        return false;
    }
    // Stack sanity: stack_base must exceed stack_limit and maintain 16-byte alignment
    if stack_base <= stack_limit || (stack_base - stack_limit) < 4096 {
        return false;
    }
    if (stack_base & 0x0F) != 0 {
        return false;
    }
    true
}

/// Dynamic syscall dispatcher invoking the configured callgate.
pub fn dynamic_syscall(syscall_num: u32, arg1: u64, arg2: u64, arg3: u64) -> i64 {
    match syscall_num {
        SYS_TEST => (arg1 + arg2 + arg3) as i64,
        SYS_ALLOC => {
            let size = arg1 as usize;
            if size == 0 || size > ARENA_SIZE {
                return 0;
            }
            // Align allocation size to 16 bytes
            let aligned_size = (size + 15) & !15;
            let current = ARENA_OFFSET.fetch_add(aligned_size, Ordering::SeqCst);
            if current + aligned_size > ARENA_SIZE {
                return 0;
            }
            unsafe {
                let base = core::ptr::addr_of_mut!(USER_ARENA.storage) as *mut u8;
                base.add(current) as u64 as i64
            }
        }
        SYS_FREE => {
            let addr = arg1;
            let size = arg2 as usize;
            if addr == 0 || size == 0 {
                return STATUS_INVALID_PARAMETER as i64;
            }
            STATUS_SUCCESS as i64
        }
        SYS_SPAWN => {
            let entry = arg1;
            let stack_size = arg2 as usize;
            if entry < 0x0000_0000_0001_0000 || entry >= 0x0000_8000_0000_0000 || stack_size < 4096 {
                return STATUS_INVALID_PARAMETER as i64;
            }
            STATUS_SUCCESS as i64
        }
        SYS_YIELD => {
            YIELD_COUNTER.fetch_add(1, Ordering::Relaxed);
            STATUS_SUCCESS as i64
        }
        _ => -1,
    }
}

/// User-mode fast syscall wrapper: allocate virtual memory.
pub unsafe fn sys_alloc(size: usize, flags: u32) -> *mut u8 {
    let ret = dynamic_syscall(SYS_ALLOC, size as u64, flags as u64, 0);
    ret as *mut u8
}

/// User-mode fast syscall wrapper: free virtual memory block.
pub unsafe fn sys_free(addr: *mut u8, size: usize) -> i32 {
    dynamic_syscall(SYS_FREE, addr as u64, size as u64, 0) as i32
}

/// User-mode fast syscall wrapper: spawn execution thread.
pub fn sys_spawn(entry: u64, stack_size: usize) -> i32 {
    dynamic_syscall(SYS_SPAWN, entry, stack_size as u64, 0) as i32
}

/// User-mode fast syscall wrapper: yield CPU timeslice.
pub fn sys_yield() -> i32 {
    dynamic_syscall(SYS_YIELD, 0, 0, 0) as i32
}

/// Returns current yield count recorded by the syscall dispatcher.
pub fn get_yield_count() -> usize {
    YIELD_COUNTER.load(Ordering::Relaxed)
}
