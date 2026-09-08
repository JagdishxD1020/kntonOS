//! Win32 Thread Local Storage (TLS) and Fiber Cooperative Multitasking Subsystem.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;

pub use crate::pe::synch::HANDLE;

pub const TLS_OUT_OF_INDEXES: u32 = 0xFFFF_FFFF;
pub const TLS_MINIMUM_AVAILABLE: usize = 64;
pub const TLS_EXPANSION_SLOTS: usize = 1024;
pub const TLS_MAX_SLOTS: usize = TLS_MINIMUM_AVAILABLE + TLS_EXPANSION_SLOTS;

const BITMAP_WORDS: usize = (TLS_MAX_SLOTS + 63) / 64;

struct TlsManager {
    allocated_bitmap: [u64; BITMAP_WORDS],
    thread_values: BTreeMap<u32, Vec<usize>>,
}

impl TlsManager {
    const fn new() -> Self {
        Self {
            allocated_bitmap: [0; BITMAP_WORDS],
            thread_values: BTreeMap::new(),
        }
    }

    fn is_slot_allocated(&self, slot: usize) -> bool {
        if slot >= TLS_MAX_SLOTS {
            return false;
        }
        let word = slot / 64;
        let bit = slot % 64;
        (self.allocated_bitmap[word] & (1 << bit)) != 0
    }

    fn allocate_slot(&mut self) -> u32 {
        for slot in 0..TLS_MAX_SLOTS {
            let word = slot / 64;
            let bit = slot % 64;
            if (self.allocated_bitmap[word] & (1 << bit)) == 0 {
                self.allocated_bitmap[word] |= 1 << bit;
                return slot as u32;
            }
        }
        TLS_OUT_OF_INDEXES
    }

    fn free_slot(&mut self, slot: usize) -> bool {
        if slot >= TLS_MAX_SLOTS || !self.is_slot_allocated(slot) {
            return false;
        }
        let word = slot / 64;
        let bit = slot % 64;
        self.allocated_bitmap[word] &= !(1 << bit);

        for values in self.thread_values.values_mut() {
            if slot < values.len() {
                values[slot] = 0;
            }
        }
        true
    }

    fn get_value(&self, tid: u32, slot: usize) -> usize {
        if !self.is_slot_allocated(slot) {
            return 0;
        }
        if let Some(values) = self.thread_values.get(&tid) {
            if slot < values.len() {
                return values[slot];
            }
        }
        0
    }

    fn set_value(&mut self, tid: u32, slot: usize, value: usize) -> bool {
        if !self.is_slot_allocated(slot) {
            return false;
        }
        let values = self.thread_values.entry(tid).or_insert_with(Vec::new);
        if slot >= values.len() {
            values.resize(slot + 1, 0);
        }
        values[slot] = value;
        true
    }
}

static TLS_MANAGER: Spinlock<TlsManager> = Spinlock::new(TlsManager::new());

fn get_current_tid() -> u32 {
    let tid = crate::process::scheduler::current_thread_id() as u32;
    if tid == 0 { 1 } else { tid }
}

/// Allocates a dynamic TLS index for the calling process.
pub fn win32_tls_alloc() -> u32 {
    let mut mgr = TLS_MANAGER.lock();
    let slot = mgr.allocate_slot();
    if slot == TLS_OUT_OF_INDEXES {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_NOT_ENOUGH_MEMORY);
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    }
    slot
}

/// Releases a TLS index previously allocated with win32_tls_alloc.
pub fn win32_tls_free(dw_tls_index: u32) -> bool {
    let mut mgr = TLS_MANAGER.lock();
    let ok = mgr.free_slot(dw_tls_index as usize);
    if ok {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
    }
    ok
}

/// Retrieves the value in the calling thread's TLS slot for the specified TLS index.
pub fn win32_tls_get_value(dw_tls_index: u32) -> usize {
    let mgr = TLS_MANAGER.lock();
    let tid = get_current_tid();
    let val = mgr.get_value(tid, dw_tls_index as usize);
    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    val
}

/// Stores a value in the calling thread's TLS slot for the specified TLS index.
pub fn win32_tls_set_value(dw_tls_index: u32, lp_tls_value: usize) -> bool {
    let mut mgr = TLS_MANAGER.lock();
    let tid = get_current_tid();
    let ok = mgr.set_value(tid, dw_tls_index as usize, lp_tls_value);
    if ok {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
    }
    ok
}

/// Retrieves a thread's TLS value for a given explicit thread ID (used for testing).
pub fn win32_tls_get_value_for_thread(tid: u32, dw_tls_index: u32) -> usize {
    let mgr = TLS_MANAGER.lock();
    mgr.get_value(tid, dw_tls_index as usize)
}

/// Sets a thread's TLS value for a given explicit thread ID (used for testing).
pub fn win32_tls_set_value_for_thread(tid: u32, dw_tls_index: u32, lp_tls_value: usize) -> bool {
    let mut mgr = TLS_MANAGER.lock();
    mgr.set_value(tid, dw_tls_index as usize, lp_tls_value)
}

pub const IMAGE_DIRECTORY_ENTRY_TLS: usize = 9;

pub const DLL_PROCESS_DETACH: u32 = 0;
pub const DLL_PROCESS_ATTACH: u32 = 1;
pub const DLL_THREAD_ATTACH: u32 = 2;
pub const DLL_THREAD_DETACH: u32 = 3;

/// Win32 64-bit PE TLS directory descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImageTlsDirectory64 {
    pub start_address_of_raw_data: u64,
    pub end_address_of_raw_data: u64,
    pub address_of_index: u64,
    pub address_of_callbacks: u64,
    pub size_of_zero_fill: u32,
    pub characteristics: u32,
}

#[allow(non_camel_case_types)]
pub type PIMAGE_TLS_CALLBACK = Option<unsafe extern "system" fn(dll_handle: *mut u8, reason: u32, reserved: *mut u8)>;

#[derive(Debug, Clone)]
pub struct StaticTlsRegistration {
    pub image_base: usize,
    pub tls_dir: ImageTlsDirectory64,
    pub callbacks: Vec<usize>,
}

static STATIC_TLS_TABLE: Spinlock<Vec<StaticTlsRegistration>> = Spinlock::new(Vec::new());

/// Registers a PE module's static TLS directory descriptor and callback array.
pub fn win32_register_static_tls(
    image_base: usize,
    tls_dir: ImageTlsDirectory64,
    callbacks: &[usize],
) {
    let mut table = STATIC_TLS_TABLE.lock();
    if let Some(pos) = table.iter().position(|r| r.image_base == image_base) {
        table[pos] = StaticTlsRegistration {
            image_base,
            tls_dir,
            callbacks: callbacks.to_vec(),
        };
    } else {
        table.push(StaticTlsRegistration {
            image_base,
            tls_dir,
            callbacks: callbacks.to_vec(),
        });
    }
}

/// Unregisters static TLS for a module.
pub fn win32_unregister_static_tls(image_base: usize) -> bool {
    let mut table = STATIC_TLS_TABLE.lock();
    if let Some(pos) = table.iter().position(|r| r.image_base == image_base) {
        table.remove(pos);
        true
    } else {
        false
    }
}

/// Dispatches static TLS callbacks for all registered modules matching the given reason.
pub unsafe fn win32_execute_tls_callbacks(dll_handle: *mut u8, reason: u32) -> usize {
    let callbacks_to_run: Vec<usize> = {
        let table = STATIC_TLS_TABLE.lock();
        let mut list = Vec::new();
        for entry in table.iter() {
            if dll_handle.is_null() || dll_handle as usize == entry.image_base {
                for &cb in &entry.callbacks {
                    if cb != 0 {
                        list.push(cb);
                    }
                }
            }
        }
        list
    };

    let mut executed = 0;
    for cb_addr in callbacks_to_run {
        let callback: unsafe extern "system" fn(*mut u8, u32, *mut u8) =
            core::mem::transmute(cb_addr);
        callback(dll_handle, reason, core::ptr::null_mut());
        executed += 1;
    }

    executed
}

pub const FIBER_FLAG_FLOAT_SWITCH: u32 = 0x0000_0001;
pub const FIBER_DEFAULT_STACK_SIZE: usize = 16 * 1024;

/// Callee-saved x86-64 execution state for cooperative fiber multitasking.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FiberContext {
    pub rip: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub fiber_data: usize,
}

#[derive(Debug)]
pub struct Win32Fiber {
    pub handle: HANDLE,
    pub fiber_data: usize,
    pub stack_base: usize,
    pub stack_size: usize,
    pub stack_buffer: Option<Vec<u8>>,
    pub is_primary: bool,
    pub context: FiberContext,
    pub routine: Option<usize>,
    pub parameter: usize,
    pub completed: bool,
}

static NEXT_FIBER_HANDLE: Spinlock<HANDLE> = Spinlock::new(0x000F_0001);
static FIBER_TABLE: Spinlock<BTreeMap<HANDLE, Win32Fiber>> = Spinlock::new(BTreeMap::new());
static CURRENT_FIBER: Spinlock<BTreeMap<u32, HANDLE>> = Spinlock::new(BTreeMap::new());

/// Converts the current thread into a cooperative Win32 fiber.
pub fn win32_convert_thread_to_fiber(lp_parameter: usize) -> HANDLE {
    let tid = get_current_tid();
    let mut current_map = CURRENT_FIBER.lock();
    if let Some(&existing) = current_map.get(&tid) {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_ALREADY_EXISTS);
        return existing;
    }

    let mut gen = NEXT_FIBER_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let fiber = Win32Fiber {
        handle,
        fiber_data: lp_parameter,
        stack_base: 0,
        stack_size: 0,
        stack_buffer: None,
        is_primary: true,
        context: FiberContext {
            fiber_data: lp_parameter,
            ..Default::default()
        },
        routine: None,
        parameter: lp_parameter,
        completed: false,
    };

    let mut table = FIBER_TABLE.lock();
    table.insert(handle, fiber);
    current_map.insert(tid, handle);

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    handle
}

/// Converts the calling fiber back to a regular Win32 thread.
pub fn win32_convert_fiber_to_thread() -> bool {
    let tid = get_current_tid();
    let mut current_map = CURRENT_FIBER.lock();
    if let Some(handle) = current_map.remove(&tid) {
        let mut table = FIBER_TABLE.lock();
        table.remove(&handle);
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        true
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        false
    }
}

/// Retrieves a handle to the currently executing fiber.
pub fn win32_get_current_fiber() -> HANDLE {
    let tid = get_current_tid();
    let current_map = CURRENT_FIBER.lock();
    current_map.get(&tid).copied().unwrap_or(0)
}

/// Retrieves the caller-defined fiber data for the current fiber.
pub fn win32_get_fiber_data() -> usize {
    let handle = win32_get_current_fiber();
    if handle == 0 {
        return 0;
    }
    let table = FIBER_TABLE.lock();
    table.get(&handle).map(|f| f.fiber_data).unwrap_or(0)
}

/// Creates a new cooperative fiber with isolated stack and execution routine.
pub fn win32_create_fiber(
    dw_stack_size: usize,
    lp_start_routine: usize,
    lp_parameter: usize,
) -> HANDLE {
    if lp_start_routine == 0 {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return 0;
    }

    let stack_sz = if dw_stack_size > 0 {
        dw_stack_size.min(FIBER_DEFAULT_STACK_SIZE)
    } else {
        FIBER_DEFAULT_STACK_SIZE
    };

    let mut stack_buf = alloc::vec![0u8; stack_sz];
    let stack_top = (stack_buf.as_mut_ptr() as usize + stack_sz) & !0x0F;

    let mut gen = NEXT_FIBER_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let fiber = Win32Fiber {
        handle,
        fiber_data: lp_parameter,
        stack_base: stack_buf.as_ptr() as usize,
        stack_size: stack_sz,
        stack_buffer: Some(stack_buf),
        is_primary: false,
        context: FiberContext {
            rip: lp_start_routine as u64,
            rsp: stack_top as u64,
            fiber_data: lp_parameter,
            ..Default::default()
        },
        routine: Some(lp_start_routine),
        parameter: lp_parameter,
        completed: false,
    };

    let mut table = FIBER_TABLE.lock();
    table.insert(handle, fiber);

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    handle
}

/// Deletes an existing fiber and releases its allocated stack.
pub fn win32_delete_fiber(lp_fiber: HANDLE) -> bool {
    let tid = get_current_tid();
    let current_map = CURRENT_FIBER.lock();
    if let Some(&cur) = current_map.get(&tid) {
        if cur == lp_fiber {
            crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
            return false;
        }
    }
    drop(current_map);

    let mut table = FIBER_TABLE.lock();
    if table.remove(&lp_fiber).is_some() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        true
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        false
    }
}
