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

/// Comprehensive verification self-test for User-Mode Memory Allocator & Dynamic Syscall Stubs.
pub fn verify_usermode_allocator_subsystem() -> bool {
    // 1. TEB Pointer Validation Checks
    let valid_teb = 0x0000_7FFF_FFFF_0000u64;
    let stack_base = 0x0000_7FFF_FFFF_8000u64;
    let stack_limit = 0x0000_7FFF_FFF0_0000u64;
    if !validate_teb_pointer(valid_teb, stack_base, stack_limit) {
        return false;
    }
    if validate_teb_pointer(0, stack_base, stack_limit) {
        return false;
    }
    if validate_teb_pointer(valid_teb | 0x03, stack_base, stack_limit) {
        return false;
    }
    if validate_teb_pointer(valid_teb, stack_limit, stack_base) {
        return false;
    }

    // 2. Dynamic Syscall Wrappers
    if sys_yield() != STATUS_SUCCESS {
        return false;
    }
    if get_yield_count() == 0 {
        return false;
    }
    if sys_spawn(0x0000_0000_0040_1000, 16384) != STATUS_SUCCESS {
        return false;
    }
    if sys_spawn(0, 100) == STATUS_SUCCESS {
        return false;
    }

    // 3. User-Mode Buddy & Slab Allocator Direct Operations
    let mut alloc = UserBuddySlabAllocator::new();
    let arena_ptr = unsafe { sys_alloc(65536, 0) };
    if arena_ptr.is_null() {
        return false;
    }
    unsafe {
        alloc.init(arena_ptr, 65536);
    }

    let p32 = unsafe { alloc.allocate(32, 8) };
    let p64 = unsafe { alloc.allocate(64, 8) };
    let p128 = unsafe { alloc.allocate(128, 16) };
    let p1024 = unsafe { alloc.allocate(1024, 16) };

    if p32.is_none() || p64.is_none() || p128.is_none() || p1024.is_none() {
        return false;
    }

    let (ptr32, ptr64, ptr128, ptr1024) = (p32.unwrap(), p64.unwrap(), p128.unwrap(), p1024.unwrap());
    unsafe {
        ptr32.write_volatile(0xAA);
        ptr64.write_volatile(0xBB);
        ptr128.write_volatile(0xCC);
        ptr1024.write_volatile(0xDD);

        if ptr32.read_volatile() != 0xAA
            || ptr64.read_volatile() != 0xBB
            || ptr128.read_volatile() != 0xCC
            || ptr1024.read_volatile() != 0xDD
        {
            return false;
        }

        alloc.deallocate(ptr32, 32, 8);
        alloc.deallocate(ptr64, 64, 8);
        alloc.deallocate(ptr128, 128, 16);
        alloc.deallocate(ptr1024, 1024, 16);
    }

    // Test multi-page allocation
    let p8k = unsafe { alloc.allocate(8192, 4096) };
    if p8k.is_none() {
        return false;
    }
    let ptr8k = p8k.unwrap();
    unsafe {
        ptr8k.write_volatile(0xEE);
        if ptr8k.read_volatile() != 0xEE {
            return false;
        }
        alloc.deallocate(ptr8k, 8192, 4096);
    }

    // 4. UserGlobalAlloc via core::alloc::GlobalAlloc
    let layout = core::alloc::Layout::from_size_align(256, 16).ok();
    if layout.is_none() {
        return false;
    }
    let l = layout.unwrap();
    let gptr = unsafe { core::alloc::GlobalAlloc::alloc(&USER_GLOBAL_ALLOCATOR, l) };
    if gptr.is_null() {
        return false;
    }
    unsafe {
        gptr.write_volatile(0x42);
        if gptr.read_volatile() != 0x42 {
            return false;
        }
        core::alloc::GlobalAlloc::dealloc(&USER_GLOBAL_ALLOCATOR, gptr, l);
    }

    true
}
