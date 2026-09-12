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

/// Low Framerate Compensation (LFC) multiplier mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfcMode {
    Passthrough,
    Double2x,
    Triple3x,
}

/// Low Framerate Compensation state and repeat frame scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LowFramerateCompensator {
    pub min_vrr_hz: u32,
    pub max_vrr_hz: u32,
    pub active_mode: LfcMode,
    pub effective_refresh_hz: u32,
    pub repeats_remaining: u8,
}

impl LowFramerateCompensator {
    /// Creates a new LFC controller for 40Hz - 165Hz VRR displays.
    pub const fn new() -> Self {
        Self {
            min_vrr_hz: VRR_MIN_REFRESH_HZ,
            max_vrr_hz: VRR_MAX_REFRESH_HZ,
            active_mode: LfcMode::Passthrough,
            effective_refresh_hz: 60,
            repeats_remaining: 0,
        }
    }

    /// Evaluates input game framerate and determines LFC repeat multiplier.
    pub fn evaluate_framerate(&mut self, input_hz: u32) -> LfcMode {
        if input_hz >= self.min_vrr_hz {
            self.active_mode = LfcMode::Passthrough;
            self.effective_refresh_hz = input_hz.min(self.max_vrr_hz);
            self.repeats_remaining = 0;
        } else if input_hz * 2 >= self.min_vrr_hz && input_hz * 2 <= self.max_vrr_hz {
            // 2x repeat insertion (e.g. 20Hz - 39Hz -> 40Hz - 78Hz)
            self.active_mode = LfcMode::Double2x;
            self.effective_refresh_hz = input_hz * 2;
            self.repeats_remaining = 1;
        } else {
            // 3x repeat insertion (e.g. below 20Hz -> 3x into VRR range)
            self.active_mode = LfcMode::Triple3x;
            self.effective_refresh_hz = (input_hz * 3).min(self.max_vrr_hz).max(self.min_vrr_hz);
            self.repeats_remaining = 2;
        }
        self.active_mode
    }

    /// Determines if a repeat presentation frame should be displayed.
    pub fn step_repeat(&mut self) -> bool {
        if self.repeats_remaining > 0 {
            self.repeats_remaining -= 1;
            true
        } else {
            false
        }
    }
}

/// Unit verification for Low Framerate Compensation (LFC).
pub fn verify_low_framerate_compensation() -> bool {
    let mut lfc = LowFramerateCompensator::new();
    if lfc.evaluate_framerate(60) != LfcMode::Passthrough || lfc.effective_refresh_hz != 60 {
        return false;
    }
    if lfc.step_repeat() {
        return false;
    }

    // 30 FPS drops below 40Hz: should trigger 2x LFC (60Hz effective)
    if lfc.evaluate_framerate(30) != LfcMode::Double2x || lfc.effective_refresh_hz != 60 {
        return false;
    }
    if !lfc.step_repeat() || lfc.step_repeat() {
        return false;
    }

    // 15 FPS: should trigger 3x LFC (45Hz effective)
    if lfc.evaluate_framerate(15) != LfcMode::Triple3x || lfc.effective_refresh_hz != 45 {
        return false;
    }
    if !lfc.step_repeat() || !lfc.step_repeat() || lfc.step_repeat() {
        return false;
    }

    true
}
