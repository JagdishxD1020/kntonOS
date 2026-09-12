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

/// Maximum presentation timestamp (PTS) queue capacity.
pub const PTS_QUEUE_CAPACITY: usize = 16;

/// Presentation timestamp (PTS) entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtsEntry {
    pub frame_id: u64,
    pub submit_time_us: u64,
    pub target_present_us: u64,
}

/// Zero-stutter frame pacing smoother with microsecond jitter filtering.
#[derive(Debug, Clone)]
pub struct FramePacingSmoother {
    pub queue: [PtsEntry; PTS_QUEUE_CAPACITY],
    pub queue_len: usize,
    pub history_intervals_us: [u32; 8],
    pub history_count: usize,
    pub history_idx: usize,
    pub last_submit_us: u64,
    pub last_scheduled_present_us: u64,
    pub target_frame_us: u32,
}

impl FramePacingSmoother {
    /// Creates a new frame pacing smoother initialized for 60Hz presentation.
    pub const fn new() -> Self {
        Self {
            queue: [PtsEntry { frame_id: 0, submit_time_us: 0, target_present_us: 0 }; PTS_QUEUE_CAPACITY],
            queue_len: 0,
            history_intervals_us: [16666; 8],
            history_count: 0,
            history_idx: 0,
            last_submit_us: 0,
            last_scheduled_present_us: 0,
            target_frame_us: 16666,
        }
    }

    /// Enqueues a newly submitted frame and computes smoothed presentation target timestamp.
    pub fn enqueue_frame(&mut self, frame_id: u64, submit_time_us: u64) -> Option<u64> {
        if self.queue_len >= PTS_QUEUE_CAPACITY {
            return None;
        }

        // Calculate delta interval from previous frame submit
        if self.last_submit_us > 0 && submit_time_us > self.last_submit_us {
            let interval = (submit_time_us - self.last_submit_us) as u32;
            self.history_intervals_us[self.history_idx] = interval;
            self.history_idx = (self.history_idx + 1) % 8;
            if self.history_count < 8 {
                self.history_count += 1;
            }
        }
        self.last_submit_us = submit_time_us;

        // Compute moving average smoothed interval to filter out CPU render jitter
        let smoothed_interval_us = if self.history_count > 0 {
            let sum: u32 = self.history_intervals_us[..self.history_count].iter().sum();
            sum / (self.history_count as u32)
        } else {
            self.target_frame_us
        };

        // Schedule next presentation without micro-stutter
        let target_present_us = if self.last_scheduled_present_us == 0 {
            submit_time_us + smoothed_interval_us as u64
        } else {
            let earliest = submit_time_us;
            let cadence = self.last_scheduled_present_us + smoothed_interval_us as u64;
            if cadence < earliest {
                earliest
            } else {
                cadence
            }
        };

        self.last_scheduled_present_us = target_present_us;

        let entry = PtsEntry {
            frame_id,
            submit_time_us,
            target_present_us,
        };
        self.queue[self.queue_len] = entry;
        self.queue_len += 1;

        Some(target_present_us)
    }

    /// Pops the next ready frame for presentation if timestamp is reached.
    pub fn pop_ready_frame(&mut self, current_time_us: u64) -> Option<PtsEntry> {
        if self.queue_len == 0 {
            return None;
        }

        if current_time_us >= self.queue[0].target_present_us {
            let entry = self.queue[0];
            // Shift remaining items
            for i in 1..self.queue_len {
                self.queue[i - 1] = self.queue[i];
            }
            self.queue_len -= 1;
            Some(entry)
        } else {
            None
        }
    }
}

/// Unit verification for Frame Pacing Smoother.
pub fn verify_frame_pacing_smoother() -> bool {
    let mut smoother = FramePacingSmoother::new();

    // Enqueue 4 frames with slight render submission jitter around 16.6ms (16666 us)
    let p1 = smoother.enqueue_frame(1, 100_000);
    let p2 = smoother.enqueue_frame(2, 116_000); // 16000 us
    let p3 = smoother.enqueue_frame(3, 133_000); // 17000 us
    let p4 = smoother.enqueue_frame(4, 149_500); // 16500 us

    if p1.is_none() || p2.is_none() || p3.is_none() || p4.is_none() {
        return false;
    }
    if smoother.queue_len != 4 {
        return false;
    }

    // Attempt pop before target presentation time: should return None
    if smoother.pop_ready_frame(105_000).is_some() {
        return false;
    }

    // Target time reached: should pop frame 1
    let ready1 = smoother.pop_ready_frame(117_000);
    if ready1.is_none() || ready1.unwrap().frame_id != 1 {
        return false;
    }
    if smoother.queue_len != 3 {
        return false;
    }

    true
}

/// OLED gamma flicker mitigation lookup table and hardware brightness stabilizer.
#[derive(Debug, Clone)]
pub struct OledGammaFlickerMitigator {
    pub base_luminance_nits: u16,
    pub current_refresh_hz: u32,
    pub gamma_lut: [u16; 16],
    pub compensation_gain_q8: u16,
}

impl OledGammaFlickerMitigator {
    /// Creates a new OLED gamma flicker mitigator.
    pub const fn new() -> Self {
        // Pre-computed inverse luminance decay compensation curve indexed by (Hz - 40) / 8
        // Expressed in Q8 fixed-point (256 = 1.0x gain)
        let mut lut = [256u16; 16];
        let mut i = 0;
        while i < 16 {
            // At lower refresh rates, OLED pixel emission window elongates causing luminance surge.
            // Gain scales down slightly from 256 to stabilize perceived nits.
            lut[i] = (256 - ((15 - i) as u16 * 4)) as u16; // e.g. 196 at 40Hz to 256 at 160Hz+
            i += 1;
        }

        Self {
            base_luminance_nits: 500,
            current_refresh_hz: 60,
            gamma_lut: lut,
            compensation_gain_q8: 256,
        }
    }

    /// Updates current refresh rate and recalculates stabilized compensation gain.
    pub fn update_refresh_hz(&mut self, refresh_hz: u32) -> u16 {
        self.current_refresh_hz = refresh_hz;
        let clamped_hz = if refresh_hz < VRR_MIN_REFRESH_HZ {
            VRR_MIN_REFRESH_HZ
        } else if refresh_hz > VRR_MAX_REFRESH_HZ {
            VRR_MAX_REFRESH_HZ
        } else {
            refresh_hz
        };

        let bucket = ((clamped_hz - VRR_MIN_REFRESH_HZ) / 8).min(15) as usize;
        self.compensation_gain_q8 = self.gamma_lut[bucket];
        self.compensation_gain_q8
    }

    /// Applies gamma flicker compensation gain to input RGB color channels.
    pub fn compensate_color_rgb(&self, r: u8, g: u8, b: u8) -> (u8, u8, u8) {
        let cr = ((r as u32 * self.compensation_gain_q8 as u32) / 256) as u8;
        let cg = ((g as u32 * self.compensation_gain_q8 as u32) / 256) as u8;
        let cb = ((b as u32 * self.compensation_gain_q8 as u32) / 256) as u8;
        (cr, cg, cb)
    }
}

/// Unit verification for OLED gamma flicker mitigation.
pub fn verify_oled_gamma_flicker_mitigator() -> bool {
    let mut mitigator = OledGammaFlickerMitigator::new();

    let gain_160 = mitigator.update_refresh_hz(160);
    if gain_160 != 256 {
        return false;
    }

    // Drop to 40Hz: gain must scale down to counteract OLED emission surge
    let gain_40 = mitigator.update_refresh_hz(40);
    if gain_40 >= 256 || gain_40 != 196 {
        return false;
    }

    let (r, g, b) = mitigator.compensate_color_rgb(200, 200, 200);
    if r >= 200 || g >= 200 || b >= 200 {
        return false;
    }

    true
}
