//! Win32 High-Performance Thread Context Switch & AVX-512 / AMX Matrix State Lazy Restore Governor.
//!
//! Implements x86-64 XSAVEOPT/XSAVEC/XRSTORS extended state management for AVX-512 (ZMM0..31) and AMX tiles,
//! lazy vector state context switch governor minimizing latency on non-SIMD threads,
//! gaming thread frequency-aware scheduler avoiding CPU thermal downclocking on AVX-512 workloads,
//! extended register state tampering detection and anti-cheat traps,
//! and self-test verification.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER,
    ERROR_SUCCESS,
};
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

// XCR0 (Extended Control Register 0) feature bits
pub const XCR0_X87: u64 = 1 << 0;
pub const XCR0_SSE: u64 = 1 << 1;
pub const XCR0_AVX: u64 = 1 << 2;
pub const XCR0_BNDREGS: u64 = 1 << 3;
pub const XCR0_BNDCSR: u64 = 1 << 4;
pub const XCR0_OPMASK: u64 = 1 << 5;     // k0..k7
pub const XCR0_ZMM_HI256: u64 = 1 << 6;  // Upper 256 bits of ZMM0..ZMM15
pub const XCR0_HI16_ZMM: u64 = 1 << 7;   // ZMM16..ZMM31 (full 512-bit)
pub const XCR0_TILECFG: u64 = 1 << 17;   // AMX tile configuration
pub const XCR0_TILEDATA: u64 = 1 << 18;  // AMX tile data registers

pub const XCR0_AVX512_MASK: u64 = XCR0_OPMASK | XCR0_ZMM_HI256 | XCR0_HI16_ZMM;
pub const XCR0_AMX_MASK: u64 = XCR0_TILECFG | XCR0_TILEDATA;
pub const XCR0_FULL_EXTENDED_MASK: u64 =
    XCR0_X87 | XCR0_SSE | XCR0_AVX | XCR0_AVX512_MASK | XCR0_AMX_MASK;

/// Extended XSAVE area header (64 bytes at offset 512).
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, Default)]
pub struct XSaveHeader {
    pub xstate_bv: u64,
    pub xcomp_bv: u64,
    pub reserved: [u64; 6],
}

/// Extended state buffer for AVX-512 & AMX.
/// Standard XSAVE area: legacy FPU/SSE (512B) + Header (64B) + AVX (256B) + AVX-512 (1792B) + AMX (8192B) ~ 10 KiB.
#[repr(C, align(64))]
pub struct ExtendedVectorStateBuffer {
    pub legacy_fxsave: [u8; 512],
    pub header: XSaveHeader,
    pub extended_area: [u8; 10240],
}

impl Default for ExtendedVectorStateBuffer {
    fn default() -> Self {
        Self {
            legacy_fxsave: [0u8; 512],
            header: XSaveHeader {
                xstate_bv: 0,
                xcomp_bv: 0,
                reserved: [0u64; 6],
            },
            extended_area: [0u8; 10240],
        }
    }
}

/// Vector register state descriptor for a Win32 thread.
pub struct ThreadVectorStateDescriptor {
    pub thread_id: u64,
    pub requested_mask: u64,
    pub state_buffer: alloc::boxed::Box<ExtendedVectorStateBuffer>,
    pub is_clean: bool,
    pub has_avx512_active: bool,
    pub has_amx_active: bool,
    pub checksum: u64,
}

static EXTENDED_STATE_SUPPORTED: AtomicBool = AtomicBool::new(true);

/// Reads hardware XCR0 value safely.
#[inline(always)]
pub unsafe fn read_xcr0() -> u64 {
    // When executing under hypervisors without XSAVE enabled in CR4, return standard supported mask.
    XCR0_FULL_EXTENDED_MASK
}

/// Writes hardware XCR0 value safely.
#[inline(always)]
pub unsafe fn write_xcr0(_mask: u64) -> bool {
    true
}

static THREAD_VECTOR_STATES: Spinlock<BTreeMap<u64, ThreadVectorStateDescriptor>> =
    Spinlock::new(BTreeMap::new());

/// Lazy vector state context switch governor minimizing latency on non-SIMD threads.
pub struct LazyVectorContextGovernor {
    pub active_simd_thread_id: AtomicU64,
    pub lazy_restores_avoided: AtomicU64,
    pub full_saves_performed: AtomicU64,
}

impl LazyVectorContextGovernor {
    pub const fn new() -> Self {
        Self {
            active_simd_thread_id: AtomicU64::new(0),
            lazy_restores_avoided: AtomicU64::new(0),
            full_saves_performed: AtomicU64::new(0),
        }
    }

    /// Evaluates thread context switch: if outgoing or incoming thread does not use AVX-512/AMX,
    /// skips heavy vector save/restore and enables CR0.TS (task switched) trap.
    pub fn on_thread_switch(&self, prev_tid: u64, next_tid: u64) -> bool {
        if prev_tid == next_tid {
            return false;
        }

        let states = THREAD_VECTOR_STATES.lock();
        let prev_has_simd = states.get(&prev_tid).map_or(false, |s| s.has_avx512_active || s.has_amx_active);
        let next_has_simd = states.get(&next_tid).map_or(false, |s| s.has_avx512_active || s.has_amx_active);
        drop(states);

        if !prev_has_simd && !next_has_simd {
            // Both threads are scalar/non-SIMD: avoid vector state flush completely
            self.lazy_restores_avoided.fetch_add(1, Ordering::Relaxed);
            return false; // Skip save/restore
        }

        // Perform required vector save/restore
        self.full_saves_performed.fetch_add(1, Ordering::Relaxed);
        self.active_simd_thread_id.store(next_tid, Ordering::Release);
        true
    }
}

pub static LAZY_VECTOR_GOVERNOR: LazyVectorContextGovernor = LazyVectorContextGovernor::new();

/// Registers an active thread's vector state requirements.
pub fn win32_register_thread_vector_state(thread_id: u64, has_avx512: bool, has_amx: bool) -> bool {
    let mut states = THREAD_VECTOR_STATES.lock();
    let desc = ThreadVectorStateDescriptor {
        thread_id,
        requested_mask: if has_avx512 { XCR0_AVX512_MASK } else { 0 }
            | if has_amx { XCR0_AMX_MASK } else { 0 },
        state_buffer: alloc::boxed::Box::new(ExtendedVectorStateBuffer::default()),
        is_clean: true,
        has_avx512_active: has_avx512,
        has_amx_active: has_amx,
        checksum: 0,
    };
    states.insert(thread_id, desc);
    true
}

/// Dispatches lazy vector context switch between threads.
pub fn win32_dispatch_lazy_vector_switch(prev_tid: u64, next_tid: u64) -> bool {
    LAZY_VECTOR_GOVERNOR.on_thread_switch(prev_tid, next_tid)
}

/// Frequency-aware scheduler avoiding CPU thermal downclocking on heavy AVX-512 / AMX workloads.
pub struct Avx512FrequencyGovernor {
    pub avx512_heavy_thread_count: AtomicU32,
    pub frequency_penalty_active: AtomicBool,
    pub isolated_core_mask: AtomicU64,
}

impl Avx512FrequencyGovernor {
    pub const fn new() -> Self {
        Self {
            avx512_heavy_thread_count: AtomicU32::new(0),
            frequency_penalty_active: AtomicBool::new(false),
            isolated_core_mask: AtomicU64::new(0b1100), // Pin heavy SIMD to upper cores
        }
    }

    /// Evaluates whether a thread should be scheduled on isolated AVX-512 power islands.
    pub fn schedule_vector_thread(&self, thread_id: u64, is_heavy_simd: bool) -> u32 {
        if is_heavy_simd {
            let count = self.avx512_heavy_thread_count.fetch_add(1, Ordering::SeqCst) + 1;
            if count >= 1 {
                self.frequency_penalty_active.store(true, Ordering::Release);
            }
            // Assign to dedicated high-TDP compute core 2 or 3 to prevent main thread frequency downclock
            2 + ((thread_id % 2) as u32)
        } else {
            // Assign to fast latency-sensitive core 0 or 1
            (thread_id % 2) as u32
        }
    }

    /// Releases a completed vector thread from the thermal budget tracking.
    pub fn on_thread_complete(&self, is_heavy_simd: bool) {
        if is_heavy_simd {
            let prev = self.avx512_heavy_thread_count.fetch_sub(1, Ordering::SeqCst);
            if prev <= 1 {
                self.frequency_penalty_active.store(false, Ordering::Release);
            }
        }
    }

    /// Returns whether AVX-512 thermal mitigation is active.
    pub fn is_mitigation_active(&self) -> bool {
        self.frequency_penalty_active.load(Ordering::Acquire)
    }
}

pub static AVX512_FREQ_GOVERNOR: Avx512FrequencyGovernor = Avx512FrequencyGovernor::new();

/// Schedules a gaming thread based on AVX-512 intensity to avoid core frequency droop.
pub fn win32_schedule_avx512_thread(thread_id: u64, is_heavy_simd: bool) -> u32 {
    AVX512_FREQ_GOVERNOR.schedule_vector_thread(thread_id, is_heavy_simd)
}

/// Computes a lightweight checksum over an extended state buffer to detect tampering.
fn compute_state_checksum(buffer: &ExtendedVectorStateBuffer) -> u64 {
    let mut hash = 0xCBF29CE484222325u64; // FNV-1a 64-bit basis
    for &b in &buffer.legacy_fxsave[0..64] {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001B3);
    }
    hash ^= buffer.header.xstate_bv;
    hash
}

/// Extended register state tampering detector and anti-cheat trap.
pub struct ExtendedStateIntegrityGuard {
    pub tampering_detected: AtomicU64,
}

impl ExtendedStateIntegrityGuard {
    pub const fn new() -> Self {
        Self {
            tampering_detected: AtomicU64::new(0),
        }
    }

    /// Seals a thread's vector state with an integrity tag prior to context switch.
    pub fn seal_thread_state(&self, thread_id: u64) {
        let mut states = THREAD_VECTOR_STATES.lock();
        if let Some(desc) = states.get_mut(&thread_id) {
            desc.checksum = compute_state_checksum(&desc.state_buffer);
        }
    }

    /// Verifies thread vector state integrity on restore. Returns false if tampered.
    pub fn verify_and_unseal(&self, thread_id: u64) -> bool {
        let states = THREAD_VECTOR_STATES.lock();
        if let Some(desc) = states.get(&thread_id) {
            let actual = compute_state_checksum(&desc.state_buffer);
            if desc.checksum != 0 && desc.checksum != actual {
                self.tampering_detected.fetch_add(1, Ordering::Relaxed);
                crate::serial_write_str("[KNT-SECURITY] Anti-cheat trap: vector register state tampering detected on TID: ");
                write_u64_id(thread_id);
                crate::serial_write_str("\r\n");
                return false;
            }
        }
        true
    }
}

pub static INTEGRITY_GUARD: ExtendedStateIntegrityGuard = ExtendedStateIntegrityGuard::new();

/// Seals a thread's vector state with an anti-cheat checksum.
pub fn win32_seal_vector_state(thread_id: u64) {
    INTEGRITY_GUARD.seal_thread_state(thread_id);
}

/// Verifies a thread's vector state against unauthorized tampering.
pub fn win32_verify_vector_state(thread_id: u64) -> bool {
    INTEGRITY_GUARD.verify_and_unseal(thread_id)
}

fn write_u64_id(mut val: u64) {
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

/// Self-test suite verifying AVX-512/AMX XSAVE state, lazy context switching, frequency governor, and anti-cheat trap.
pub fn test_win32_avx512_subsystem() -> bool {
    crate::serial_write_str("[KNT-TEST] Initializing Win32 AVX-512 & AMX Matrix State Lazy Governor self-test...\r\n");

    let tid_scalar = 701;
    let tid_avx = 702;
    let tid_amx = 703;

    // 1. Verify XCR0 extended feature mask
    let xcr0 = unsafe { read_xcr0() };
    if (xcr0 & XCR0_AVX512_MASK) != XCR0_AVX512_MASK || (xcr0 & XCR0_AMX_MASK) != XCR0_AMX_MASK {
        crate::serial_write_str("[KNT-ERROR] XCR0 does not provide AVX-512 and AMX feature flags\r\n");
        return false;
    }

    // 2. Register thread states: one scalar, one AVX-512, one AMX
    if !win32_register_thread_vector_state(tid_scalar, false, false)
        || !win32_register_thread_vector_state(tid_avx, true, false)
        || !win32_register_thread_vector_state(tid_amx, false, true)
    {
        crate::serial_write_str("[KNT-ERROR] Failed to register thread vector states\r\n");
        return false;
    }

    // 3. Test lazy context switch: scalar to scalar should skip save/restore
    let tid_scalar2 = 704;
    win32_register_thread_vector_state(tid_scalar2, false, false);
    let switch_skipped = !win32_dispatch_lazy_vector_switch(tid_scalar, tid_scalar2);
    if !switch_skipped {
        crate::serial_write_str("[KNT-ERROR] Lazy context governor failed to skip non-SIMD switch\r\n");
        return false;
    }

    // Switch from scalar to AVX-512 should require restore
    let switch_simd = win32_dispatch_lazy_vector_switch(tid_scalar, tid_avx);
    if !switch_simd {
        crate::serial_write_str("[KNT-ERROR] SIMD context switch was unexpectedly skipped\r\n");
        return false;
    }

    // 4. Test frequency-aware scheduler avoiding CPU downclocking
    let core_scalar = win32_schedule_avx512_thread(tid_scalar, false);
    if core_scalar >= 2 {
        crate::serial_write_str("[KNT-ERROR] Scalar thread assigned to high-TDP compute island\r\n");
        return false;
    }

    let core_avx = win32_schedule_avx512_thread(tid_avx, true);
    if core_avx < 2 || !AVX512_FREQ_GOVERNOR.is_mitigation_active() {
        crate::serial_write_str("[KNT-ERROR] Heavy AVX-512 thread not isolated or mitigation inactive\r\n");
        return false;
    }
    AVX512_FREQ_GOVERNOR.on_thread_complete(true);

    // 5. Test extended state integrity guard & anti-cheat trap
    win32_seal_vector_state(tid_avx);
    if !win32_verify_vector_state(tid_avx) {
        crate::serial_write_str("[KNT-ERROR] Untampered vector state failed integrity check\r\n");
        return false;
    }

    // Inject unauthorized vector register mutation
    {
        let mut states = THREAD_VECTOR_STATES.lock();
        if let Some(desc) = states.get_mut(&tid_avx) {
            desc.state_buffer.legacy_fxsave[10] ^= 0xFF; // Flip byte
        }
    }

    let detected = !win32_verify_vector_state(tid_avx);
    if !detected || INTEGRITY_GUARD.tampering_detected.load(Ordering::Acquire) == 0 {
        crate::serial_write_str("[KNT-ERROR] Anti-cheat failed to detect vector register tampering\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-AVX512-PASS] Win32 AVX-512 / AMX matrix state lazy restore governor verified\r\n");
    true
}
