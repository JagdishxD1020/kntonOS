//! OLED Variable Refresh Rate (VRR) & Low Framerate Compensation (LFC) Frame Pacer.

#![allow(dead_code)]

/// Minimum supported VRR refresh rate in Hz.
pub const VRR_MIN_REFRESH_HZ: u32 = 40;
/// Maximum supported VRR refresh rate in Hz.
pub const VRR_MAX_REFRESH_HZ: u32 = 165;
/// Base vertical active resolution in lines.
pub const BASE_VACTIVE: u32 = 1200;
/// Base vertical blanking front porch lines at maximum refresh rate (165Hz).
pub const BASE_VFRONT_PORCH_LINES: u32 = 40;

/// VESA Adaptive-Sync VRR timing parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrrTimingModel {
    pub min_hz: u32,
    pub max_hz: u32,
    pub min_interval_us: u32,
    pub max_interval_us: u32,
    pub current_refresh_hz: u32,
    pub modulated_front_porch_lines: u32,
}

impl VrrTimingModel {
    /// Creates a new VRR timing model for 40Hz - 165Hz range.
    pub const fn new() -> Self {
        Self {
            min_hz: VRR_MIN_REFRESH_HZ,
            max_hz: VRR_MAX_REFRESH_HZ,
            min_interval_us: 1_000_000 / VRR_MAX_REFRESH_HZ, // ~6060 us (165 Hz)
            max_interval_us: 1_000_000 / VRR_MIN_REFRESH_HZ, // 25000 us (40 Hz)
            current_refresh_hz: 60,
            modulated_front_porch_lines: BASE_VFRONT_PORCH_LINES,
        }
    }

    /// Converts instantaneous framerate in Hz to microsecond frame interval.
    pub const fn hz_to_interval_us(hz: u32) -> u32 {
        if hz == 0 {
            return 25_000;
        }
        1_000_000 / hz
    }

    /// Converts microsecond frame interval to integer Hz.
    pub const fn interval_us_to_hz(interval_us: u32) -> u32 {
        if interval_us == 0 {
            return VRR_MAX_REFRESH_HZ;
        }
        1_000_000 / interval_us
    }

    /// Modulates the vertical front porch lines based on target interval in microseconds.
    pub fn modulate_front_porch(&mut self, interval_us: u32) -> u32 {
        let clamped_us = if interval_us < self.min_interval_us {
            self.min_interval_us
        } else if interval_us > self.max_interval_us {
            self.max_interval_us
        } else {
            interval_us
        };

        self.current_refresh_hz = Self::interval_us_to_hz(clamped_us);

        // Extended vblank lines proportional to difference from min_interval_us (165Hz)
        let extra_us = clamped_us.saturating_sub(self.min_interval_us);
        let extra_lines = (extra_us * BASE_VACTIVE) / (self.min_interval_us.max(1));
        self.modulated_front_porch_lines = BASE_VFRONT_PORCH_LINES + extra_lines;
        self.modulated_front_porch_lines
    }

    /// Checks if a given framerate is inside the native hardware VRR window.
    pub const fn in_vrr_window(&self, hz: u32) -> bool {
        hz >= self.min_hz && hz <= self.max_hz
    }
}

/// Unit verification for Display VRR timing model.
pub fn verify_vrr_timing_model() -> bool {
    let mut model = VrrTimingModel::new();
    if model.min_hz != 40 || model.max_hz != 165 {
        return false;
    }
    if !model.in_vrr_window(60) || !model.in_vrr_window(40) || !model.in_vrr_window(165) {
        return false;
    }
    if model.in_vrr_window(30) || model.in_vrr_window(180) {
        return false;
    }

    let interval_60 = VrrTimingModel::hz_to_interval_us(60);
    if interval_60 != 16666 {
        return false;
    }

    let fp_60 = model.modulate_front_porch(interval_60);
    if fp_60 <= BASE_VFRONT_PORCH_LINES || model.current_refresh_hz != 60 {
        return false;
    }

    true
}
