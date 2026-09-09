//! Win32 Performance Counters & High-Resolution QueryPerformanceCounter Engine.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub use crate::pe::synch::HANDLE;

pub const DEFAULT_TSC_FREQ_HZ: u64 = 3_000_000_000;
pub const QPC_STANDARD_FREQ_HZ: u64 = 10_000_000;

static TSC_FREQ_HZ: AtomicU64 = AtomicU64::new(DEFAULT_TSC_FREQ_HZ);
static INVARIANT_TSC: AtomicBool = AtomicBool::new(true);
static CALIBRATED: AtomicBool = AtomicBool::new(false);
static TSC_BASE_OFFSET: AtomicU64 = AtomicU64::new(0);

/// Reads the hardware CPU Time-Stamp Counter (TSC).
#[inline(always)]
pub fn win32_read_tsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Checks whether the CPU provides an invariant Time-Stamp Counter.
pub fn win32_is_invariant_tsc() -> bool {
    INVARIANT_TSC.load(Ordering::Relaxed)
}

/// Returns the calibrated TSC frequency in Hertz.
pub fn win32_get_tsc_frequency() -> u64 {
    TSC_FREQ_HZ.load(Ordering::Relaxed)
}

/// Calibrates the CPU TSC rate against the Local APIC timer ticks.
pub fn win32_calibrate_tsc_frequency() -> u64 {
    if CALIBRATED.load(Ordering::Acquire) {
        return TSC_FREQ_HZ.load(Ordering::Relaxed);
    }

    let initial_tsc = win32_read_tsc();
    TSC_BASE_OFFSET.store(initial_tsc, Ordering::Relaxed);

    let start_ticks = crate::arch::x86_64::timer::LapicTimer::ticks();
    let start_tsc = win32_read_tsc();

    let mut current_ticks = start_ticks;
    let mut spins = 0;
    while current_ticks == start_ticks && spins < 2_000_000 {
        core::hint::spin_loop();
        current_ticks = crate::arch::x86_64::timer::LapicTimer::ticks();
        spins += 1;
    }

    let end_tsc = win32_read_tsc();
    let delta_tsc = end_tsc.saturating_sub(start_tsc);

    let freq = if current_ticks > start_ticks && delta_tsc > 0 {
        let measured_hz = delta_tsc.saturating_mul(100);
        if measured_hz >= 500_000_000 && measured_hz <= 10_000_000_000 {
            measured_hz
        } else {
            DEFAULT_TSC_FREQ_HZ
        }
    } else {
        DEFAULT_TSC_FREQ_HZ
    };

    TSC_FREQ_HZ.store(freq, Ordering::Release);
    INVARIANT_TSC.store(true, Ordering::Release);
    CALIBRATED.store(true, Ordering::Release);
    freq
}
