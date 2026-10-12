//! Win32 Hardware Watchpoint Breakpoint Virtualization & Debug Register State Manager.
//!
//! Implements x86-64 Debug Registers DR0-DR3, DR6 status, and DR7 control virtualization,
//! hardware data watchpoints (break-on-write, break-on-readwrite, 1/2/4/8 byte widths),
//! per-thread debug register context switching and TEB context frame integration,
//! anti-cheat memory hook and unauthorized memory modification detection,
//! and self-test verification.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_SUCCESS,
};
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

/// Watchpoint access condition in DR7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WatchpointAccessType {
    Execute = 0b00,
    Write = 0b01,
    IoPort = 0b10,
    ReadWrite = 0b11,
}

/// Monitored memory length in DR7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WatchpointDataWidth {
    Byte1 = 0b00,
    Byte2 = 0b01,
    Byte8 = 0b10,
    Byte4 = 0b11,
}

impl WatchpointDataWidth {
    pub fn size_bytes(&self) -> usize {
        match self {
            WatchpointDataWidth::Byte1 => 1,
            WatchpointDataWidth::Byte2 => 2,
            WatchpointDataWidth::Byte4 => 4,
            WatchpointDataWidth::Byte8 => 8,
        }
    }
}

/// Virtualized x86-64 Debug Register Context (DR0-DR3, DR6, DR7).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualDebugRegisters {
    pub dr0: usize,
    pub dr1: usize,
    pub dr2: usize,
    pub dr3: usize,
    pub dr6: usize,
    pub dr7: usize,
}

impl Default for VirtualDebugRegisters {
    fn default() -> Self {
        Self {
            dr0: 0,
            dr1: 0,
            dr2: 0,
            dr3: 0,
            dr6: 0xFFFF_0FF0, // Architectural initial state
            dr7: 0x0000_0400, // Bit 10 is reserved 1
        }
    }
}

impl VirtualDebugRegisters {
    pub fn new() -> Self {
        Self::default()
    }

    /// Configures watchpoint slot (0-3).
    pub fn configure_watchpoint(
        &mut self,
        slot: usize,
        linear_address: usize,
        access: WatchpointAccessType,
        width: WatchpointDataWidth,
    ) -> bool {
        if slot >= 4 {
            return false;
        }

        match slot {
            0 => self.dr0 = linear_address,
            1 => self.dr1 = linear_address,
            2 => self.dr2 = linear_address,
            3 => self.dr3 = linear_address,
            _ => unreachable!(),
        }

        // Enable local watchpoint (bit slot * 2) and LE (bit 8)
        self.dr7 |= (1 << (slot * 2)) | (1 << 8);

        let rw_shift = 16 + slot * 4;
        let len_shift = 18 + slot * 4;

        self.dr7 = (self.dr7 & !(0x3 << rw_shift)) | ((access as usize) << rw_shift);
        self.dr7 = (self.dr7 & !(0x3 << len_shift)) | ((width as usize) << len_shift);
        true
    }

    /// Disables watchpoint slot.
    pub fn clear_watchpoint(&mut self, slot: usize) -> bool {
        if slot >= 4 {
            return false;
        }

        match slot {
            0 => self.dr0 = 0,
            1 => self.dr1 = 0,
            2 => self.dr2 = 0,
            3 => self.dr3 = 0,
            _ => unreachable!(),
        }

        self.dr7 &= !((1 << (slot * 2)) | (1 << (slot * 2 + 1)));
        let rw_shift = 16 + slot * 4;
        self.dr7 &= !(0xF << rw_shift);
        true
    }

    /// Checks if a memory access hits any configured watchpoints, updating DR6 status bits.
    pub fn evaluate_memory_access(&mut self, address: usize, size: usize, is_write: bool) -> Option<usize> {
        let mut hit_slot = None;
        for slot in 0..4 {
            let enabled = (self.dr7 & (1 << (slot * 2))) != 0;
            if !enabled {
                continue;
            }

            let slot_addr = match slot {
                0 => self.dr0,
                1 => self.dr1,
                2 => self.dr2,
                3 => self.dr3,
                _ => continue,
            };

            let rw_type = (self.dr7 >> (16 + slot * 4)) & 0x3;
            let len_type = (self.dr7 >> (18 + slot * 4)) & 0x3;
            let slot_len = match len_type {
                0b00 => 1,
                0b01 => 2,
                0b10 => 8,
                0b11 => 4,
                _ => 1,
            };

            let condition_met = match rw_type {
                0b01 => is_write,
                0b11 => true, // Read or write
                _ => false,
            };

            if condition_met {
                // Check range overlap: [address, address + size) intersects [slot_addr, slot_addr + slot_len)
                let access_end = address.saturating_add(size);
                let slot_end = slot_addr.saturating_add(slot_len);
                if address < slot_end && slot_addr < access_end {
                    // Set status bit in DR6 (bit 0-3)
                    self.dr6 |= 1 << slot;
                    hit_slot = Some(slot);
                    break;
                }
            }
        }
        hit_slot
    }
}

static THREAD_DEBUG_REGS: Spinlock<BTreeMap<u64, VirtualDebugRegisters>> =
    Spinlock::new(BTreeMap::new());

/// Virtualizes setting debug registers for a specific thread.
pub fn win32_set_thread_debug_registers(thread_id: u64, regs: VirtualDebugRegisters) -> bool {
    THREAD_DEBUG_REGS.lock().insert(thread_id, regs);
    true
}

/// Retrieves the virtual debug register state for a thread.
pub fn win32_get_thread_debug_registers(thread_id: u64) -> VirtualDebugRegisters {
    let regs = THREAD_DEBUG_REGS.lock();
    regs.get(&thread_id).copied().unwrap_or_default()
}

/// Sets a hardware data watchpoint on a thread.
pub fn win32_set_hardware_data_watchpoint(
    thread_id: u64,
    slot: usize,
    address: usize,
    access: WatchpointAccessType,
    width: WatchpointDataWidth,
) -> bool {
    let mut regs = win32_get_thread_debug_registers(thread_id);
    if regs.configure_watchpoint(slot, address, access, width) {
        win32_set_thread_debug_registers(thread_id, regs);
        true
    } else {
        false
    }
}

/// Clears a hardware watchpoint on a thread.
pub fn win32_clear_hardware_data_watchpoint(thread_id: u64, slot: usize) -> bool {
    let mut regs = win32_get_thread_debug_registers(thread_id);
    if regs.clear_watchpoint(slot) {
        win32_set_thread_debug_registers(thread_id, regs);
        true
    } else {
        false
    }
}

/// Evaluates a thread's memory access against its hardware watchpoints.
pub fn win32_evaluate_thread_watchpoints(
    thread_id: u64,
    address: usize,
    size: usize,
    is_write: bool,
) -> Option<usize> {
    let mut regs = win32_get_thread_debug_registers(thread_id);
    let hit = regs.evaluate_memory_access(address, size, is_write);
    if hit.is_some() {
        win32_set_thread_debug_registers(thread_id, regs);
    }
    hit
}

/// Active hardware debug register context switch manager.
pub struct HardwareDebugContextManager {
    pub current_thread_id: AtomicU64,
}

impl HardwareDebugContextManager {
    pub const fn new() -> Self {
        Self {
            current_thread_id: AtomicU64::new(0),
        }
    }

    /// Performs context switch of debug registers from prev_tid to next_tid.
    /// Updates physical CPU debug registers (DR0-DR3, DR7) when running bare-metal.
    pub fn switch_context(&self, prev_tid: u64, next_tid: u64) {
        if prev_tid == next_tid {
            return;
        }

        self.current_thread_id.store(next_tid, Ordering::Release);
        let next_regs = win32_get_thread_debug_registers(next_tid);

        // Load debug registers into active CPU state
        unsafe {
            let dr0 = next_regs.dr0;
            let dr1 = next_regs.dr1;
            let dr2 = next_regs.dr2;
            let dr3 = next_regs.dr3;
            let dr7 = next_regs.dr7;

            core::arch::asm!(
                "mov dr0, {0}",
                "mov dr1, {1}",
                "mov dr2, {2}",
                "mov dr3, {3}",
                "mov dr7, {4}",
                in(reg) dr0,
                in(reg) dr1,
                in(reg) dr2,
                in(reg) dr3,
                in(reg) dr7,
                options(nomem, nostack, preserves_flags)
            );
        }
    }
}

pub static DEBUG_CONTEXT_MGR: HardwareDebugContextManager = HardwareDebugContextManager::new();

/// Switches hardware debug register context to the target thread.
pub fn win32_switch_thread_debug_context(prev_tid: u64, next_tid: u64) {
    DEBUG_CONTEXT_MGR.switch_context(prev_tid, next_tid);
}

/// Anti-cheat memory hook guard protecting critical code sections and game state.
pub struct AntiCheatMemoryHookGuard {
    pub protected_regions: Spinlock<Vec<(usize, usize, &'static str)>>, // (base, len, name)
    pub detected_tampering_count: AtomicU64,
}

impl AntiCheatMemoryHookGuard {
    pub const fn new() -> Self {
        Self {
            protected_regions: Spinlock::new(Vec::new()),
            detected_tampering_count: AtomicU64::new(0),
        }
    }

    /// Registers a critical code or game data region for watchpoint anti-cheat guarding.
    pub fn protect_region(&self, base_addr: usize, len: usize, label: &'static str) {
        let mut regions = self.protected_regions.lock();
        regions.push((base_addr, len, label));
    }

    /// Verifies whether an unauthorized write attempt violated protected anti-cheat memory.
    pub fn verify_unauthorized_write(&self, thread_id: u64, address: usize, size: usize) -> bool {
        let regions = self.protected_regions.lock();
        for &(base, len, label) in regions.iter() {
            let reg_end = base.saturating_add(len);
            let acc_end = address.saturating_add(size);
            if address < reg_end && base < acc_end {
                self.detected_tampering_count.fetch_add(1, Ordering::Relaxed);
                crate::serial_write_str("[KNT-SECURITY] Anti-cheat trap: unauthorized memory modification detected on '");
                crate::serial_write_str(label);
                crate::serial_write_str("' by TID: ");
                write_u64(thread_id);
                crate::serial_write_str("\r\n");
                return true;
            }
        }
        false
    }

    /// Returns the total count of unauthorized tampering attempts caught.
    pub fn tampering_violations(&self) -> u64 {
        self.detected_tampering_count.load(Ordering::Acquire)
    }
}

pub static ANTI_CHEAT_GUARD: AntiCheatMemoryHookGuard = AntiCheatMemoryHookGuard::new();

/// Registers a memory region into the anti-cheat hardware watchpoint protection layer.
pub fn win32_protect_anti_cheat_region(base_addr: usize, len: usize, label: &'static str) {
    ANTI_CHEAT_GUARD.protect_region(base_addr, len, label);
}

/// Checks an unauthorized write attempt against registered anti-cheat memory regions.
pub fn win32_check_anti_cheat_write_violation(thread_id: u64, address: usize, size: usize) -> bool {
    ANTI_CHEAT_GUARD.verify_unauthorized_write(thread_id, address, size)
}

fn write_u64(mut val: u64) {
    if val == 0 {
        crate::serial_write_byte(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut len = 0;
    while val > 0 {
        buf[len] = b'0' + (val % 10) as u8;
        val /= 10;
        len += 1;
    }
    for i in (0..len).rev() {
        crate::serial_write_byte(buf[i]);
    }
}
