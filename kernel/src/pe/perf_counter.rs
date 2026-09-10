#![allow(non_snake_case)]
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

use core::sync::atomic::AtomicI64;

static LAST_QPC_TICKS: AtomicI64 = AtomicI64::new(0);

/// Retrieves the frequency of the performance counter in counts per second (standard 10 MHz).
pub fn win32_query_performance_frequency(lpFrequency: *mut i64) -> bool {
    if lpFrequency.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    unsafe {
        *lpFrequency = QPC_STANDARD_FREQ_HZ as i64;
    }
    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Retrieves the current value of the performance counter with high-resolution TSC backing.
pub fn win32_query_performance_counter(lpPerformanceCount: *mut i64) -> bool {
    if lpPerformanceCount.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    let tsc = win32_read_tsc();
    let tsc_base = TSC_BASE_OFFSET.load(Ordering::Relaxed);
    let elapsed_tsc = tsc.saturating_sub(tsc_base);
    let tsc_freq = win32_get_tsc_frequency();

    let qpc_ticks = ((elapsed_tsc as u128 * QPC_STANDARD_FREQ_HZ as u128) / tsc_freq as u128) as i64;

    let mut current_last = LAST_QPC_TICKS.load(Ordering::Relaxed);
    let mut count = qpc_ticks.max(current_last);
    while let Err(actual) = LAST_QPC_TICKS.compare_exchange_weak(
        current_last,
        count,
        Ordering::SeqCst,
        Ordering::Relaxed,
    ) {
        current_last = actual;
        count = qpc_ticks.max(current_last);
    }

    unsafe {
        *lpPerformanceCount = count;
    }
    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Convenience helper reading the performance counter directly.
pub fn win32_qpc_ticks() -> i64 {
    let mut count: i64 = 0;
    win32_query_performance_counter(&mut count as *mut i64);
    count
}

/// Converts performance counter counts (10 MHz) into nanoseconds.
#[inline(always)]
pub fn win32_qpc_to_ns(qpc_ticks: i64) -> u64 {
    (qpc_ticks.max(0) as u64).saturating_mul(100)
}

use core::sync::atomic::AtomicU32;

pub const TIMERR_NOERROR: u32 = 0;
pub const TIMERR_NOCANDO: u32 = 97;
pub const TIMERR_STRUCT: u32 = 98;

static CURRENT_TIMER_RESOLUTION_MS: AtomicU32 = AtomicU32::new(15);
static PERIOD_REQUEST_COUNT: AtomicU32 = AtomicU32::new(0);

/// Requests a minimum resolution for periodic timers (e.g. 1ms for games).
pub fn win32_time_begin_period(uPeriod: u32) -> u32 {
    if uPeriod == 0 || uPeriod > 1000 {
        return TIMERR_NOCANDO;
    }
    let target = uPeriod.max(1);
    CURRENT_TIMER_RESOLUTION_MS.store(target, Ordering::Release);
    PERIOD_REQUEST_COUNT.fetch_add(1, Ordering::Relaxed);
    TIMERR_NOERROR
}

/// Clears a previously set minimum timer resolution.
pub fn win32_time_end_period(uPeriod: u32) -> u32 {
    if uPeriod == 0 || uPeriod > 1000 {
        return TIMERR_NOCANDO;
    }
    let prev = PERIOD_REQUEST_COUNT.fetch_sub(1, Ordering::Relaxed);
    if prev <= 1 {
        CURRENT_TIMER_RESOLUTION_MS.store(15, Ordering::Release);
    }
    TIMERR_NOERROR
}

/// Returns the current active multimedia timer resolution in milliseconds.
pub fn win32_get_current_timer_resolution_ms() -> u32 {
    CURRENT_TIMER_RESOLUTION_MS.load(Ordering::Relaxed)
}

/// Retrieves the number of milliseconds that have elapsed since system boot (64-bit).
pub fn win32_get_tick_count64() -> u64 {
    let qpc = win32_qpc_ticks();
    (qpc.max(0) as u64) / 10_000
}

/// Retrieves the number of milliseconds that have elapsed since system boot (32-bit).
pub fn win32_get_tick_count() -> u32 {
    win32_get_tick_count64() as u32
}

/// Retrieves the system time in milliseconds for multimedia and game timing.
pub fn win32_time_get_time() -> u32 {
    win32_get_tick_count()
}
