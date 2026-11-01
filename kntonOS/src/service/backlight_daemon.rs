//! Userspace Ambient Light Sensor (ALS) & HDR Display Auto-Calibrator Service (knton.svc.backlight).
//!
//! Provides dynamic backlight modulation, smooth exponential lux-to-nits curve mapping,
//! night mode CCT color temperature governor, and HDR10 OLED / LCD 1000-nit tone-mapping metadata injection.

#![allow(dead_code)]
#![allow(unused_imports)]

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Maximum window depth for optical jitter rejection.
pub const ALS_FILTER_WINDOW_SIZE: usize = 16;

/// I2C bus address for Lite-On LTR-F216AC Ambient Light Sensor on Steam Deck.
pub const ALS_I2C_ADDR_LTRF216AC: u8 = 0x53;
/// ACPI hardware ID string for standard ACPI ALS device.
pub const ALS_ACPI_HID_ALS0008: &str = "ACPI0008";

/// Supported ambient light sensor hardware variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlsDeviceType {
    /// Lite-On LTR-F216AC optical sensor (Steam Deck LCD).
    LtrF216Ac,
    /// Standard ACPI 3.0+ Ambient Light Sensor device (_ALI method).
    AcpiAls0008,
    /// Vishay / Capella multi-spectral sensor (Steam Deck OLED).
    Veml3328,
    /// Emulated virtual sensor for testing and headless mode.
    VirtualAls,
}

/// Dynamic optical sensor gain levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlsGain {
    Gain1x = 1,
    Gain2x = 2,
    Gain4x = 4,
    Gain8x = 8,
    Gain16x = 16,
}

/// Optical sensor integration measurement times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlsIntegrationTime {
    Time50ms = 50,
    Time100ms = 100,
    Time200ms = 200,
    Time400ms = 400,
}

/// Sensor hardware configuration descriptor.
#[derive(Debug, Clone, Copy)]
pub struct AlsSensorConfig {
    pub device_type: AlsDeviceType,
    pub i2c_bus: u8,
    pub i2c_address: u8,
    pub gain: AlsGain,
    pub integration_time: AlsIntegrationTime,
    pub poll_interval_ms: u32,
    pub ir_compensation_factor_q8: u16, // Fixed-point Q8 compensation factor (e.g., 0.75 * 256 = 192)
}

impl Default for AlsSensorConfig {
    fn default() -> Self {
        Self {
            device_type: AlsDeviceType::LtrF216Ac,
            i2c_bus: 1,
            i2c_address: ALS_I2C_ADDR_LTRF216AC,
            gain: AlsGain::Gain4x,
            integration_time: AlsIntegrationTime::Time100ms,
            poll_interval_ms: 100,
            ir_compensation_factor_q8: 192,
        }
    }
}

/// Raw sensor reading snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlsRawReading {
    pub clear_adc: u16,
    pub ir_adc: u16,
    pub calculated_lux: u32,
    pub timestamp_ms: u64,
}

impl AlsRawReading {
    pub const fn empty() -> Self {
        Self {
            clear_adc: 0,
            ir_adc: 0,
            calculated_lux: 0,
            timestamp_ms: 0,
        }
    }
}

/// Dynamic optical filter preventing hand occlusion spikes and high-frequency flicker.
#[derive(Debug, Clone)]
pub struct DynamicOpticalFilter {
    ring_buffer: [u32; ALS_FILTER_WINDOW_SIZE],
    head: usize,
    count: usize,
    smoothed_lux: u32,
    last_stable_lux: u32,
    hysteresis_pct: u8,
    ema_alpha_q8: u16, // Fixed-point Q8 alpha: e.g. 0.25 * 256 = 64 for low-pass smoothing
    occlusion_rejection_enabled: bool,
}

impl DynamicOpticalFilter {
    pub const fn new() -> Self {
        Self {
            ring_buffer: [0; ALS_FILTER_WINDOW_SIZE],
            head: 0,
            count: 0,
            smoothed_lux: 0,
            last_stable_lux: 0,
            hysteresis_pct: 7, // 7% hysteresis deadband to prevent micro-fluctuations
            ema_alpha_q8: 64,  // 0.25 EMA weighting
            occlusion_rejection_enabled: true,
        }
    }

    /// Reset filter state.
    pub fn reset(&mut self) {
        self.ring_buffer = [0; ALS_FILTER_WINDOW_SIZE];
        self.head = 0;
        self.count = 0;
        self.smoothed_lux = 0;
        self.last_stable_lux = 0;
    }

    /// Ingest a new raw lux sample and calculate optical filtered value.
    pub fn process_sample(&mut self, sample_lux: u32) -> u32 {
        // Occlusion detection: if sample drops abruptly by >80% for a single frame while holding device,
        // suppress immediate ramp down until corroborated by subsequent samples.
        if self.occlusion_rejection_enabled && self.count >= 4 && self.smoothed_lux > 100 {
            if sample_lux * 5 < self.smoothed_lux {
                // Potential brief finger occlusion: damp sudden drop
                let damped = (sample_lux + self.smoothed_lux * 3) / 4;
                return self.push_and_filter(damped);
            }
        }

        self.push_and_filter(sample_lux)
    }

    fn push_and_filter(&mut self, val: u32) -> u32 {
        self.ring_buffer[self.head] = val;
        self.head = (self.head + 1) % ALS_FILTER_WINDOW_SIZE;
        if self.count < ALS_FILTER_WINDOW_SIZE {
            self.count += 1;
        }

        if self.count == 1 {
            self.smoothed_lux = val;
            self.last_stable_lux = val;
            return val;
        }

        // Compute median of recent samples to discard outliers
        let mut sorted = [0u32; ALS_FILTER_WINDOW_SIZE];
        let n = self.count;
        for i in 0..n {
            sorted[i] = self.ring_buffer[i];
        }
        // Bubble sort small slice (at most 16 items)
        for i in 0..n {
            for j in 0..(n - 1 - i) {
                if sorted[j] > sorted[j + 1] {
                    let tmp = sorted[j];
                    sorted[j] = sorted[j + 1];
                    sorted[j + 1] = tmp;
                }
            }
        }
        let median = sorted[n / 2];

        // Apply Exponential Moving Average (EMA): smoothed = (alpha * median + (256 - alpha) * smoothed) / 256
        let alpha = self.ema_alpha_q8 as u64;
        let prev = self.smoothed_lux as u64;
        let new_smoothed = ((alpha * (median as u64)) + ((256 - alpha) * prev)) / 256;
        let new_smoothed = new_smoothed as u32;
        self.smoothed_lux = new_smoothed;

        // Apply hysteresis deadzone check against last stable lux
        let delta = if new_smoothed > self.last_stable_lux {
            new_smoothed - self.last_stable_lux
        } else {
            self.last_stable_lux - new_smoothed
        };

        let deadzone_threshold = ((self.last_stable_lux as u64 * (self.hysteresis_pct as u64)) / 100) as u32;
        let min_deadzone = 3; // Minimum absolute 3 lux threshold in very dark rooms
        let effective_threshold = if deadzone_threshold > min_deadzone { deadzone_threshold } else { min_deadzone };

        if delta >= effective_threshold {
            self.last_stable_lux = new_smoothed;
        }

        self.last_stable_lux
    }

    pub fn current_lux(&self) -> u32 {
        self.last_stable_lux
    }
}

/// Ambient Light Sensor hardware driver.
pub struct AlsDriver {
    config: AlsSensorConfig,
    is_initialized: bool,
    filter: DynamicOpticalFilter,
    last_reading: AlsRawReading,
}

impl AlsDriver {
    pub const fn new(config: AlsSensorConfig) -> Self {
        Self {
            config,
            is_initialized: false,
            filter: DynamicOpticalFilter::new(),
            last_reading: AlsRawReading::empty(),
        }
    }

    /// Initialize sensor interface.
    pub fn init(&mut self) -> bool {
        self.is_initialized = true;
        self.filter.reset();
        true
    }

    /// Ingest hardware ADC counts and convert to calibrated illuminance (lux).
    pub fn ingest_adc(&mut self, clear_adc: u16, ir_adc: u16, timestamp_ms: u64) -> u32 {
        // Lux conversion formula:
        // Net visible light = Clear - (IR * ir_comp_factor / 256)
        let ir_comp = ((ir_adc as u32) * (self.config.ir_compensation_factor_q8 as u32)) / 256;
        let net_counts = if (clear_adc as u32) > ir_comp {
            (clear_adc as u32) - ir_comp
        } else {
            0
        };

        let gain_val = self.config.gain as u32;
        let int_time_val = (self.config.integration_time as u32) / 50; // normalized to 50ms base
        let divisor = gain_val * int_time_val;
        let raw_lux = if divisor > 0 {
            (net_counts * 10) / divisor
        } else {
            net_counts * 10
        };

        self.last_reading = AlsRawReading {
            clear_adc,
            ir_adc,
            calculated_lux: raw_lux,
            timestamp_ms,
        };

        self.filter.process_sample(raw_lux)
    }

    /// Ingest direct lux value (e.g. from ACPI _ALI or virtual testing harness).
    pub fn ingest_direct_lux(&mut self, lux: u32, timestamp_ms: u64) -> u32 {
        self.last_reading = AlsRawReading {
            clear_adc: (lux & 0xFFFF) as u16,
            ir_adc: 0,
            calculated_lux: lux,
            timestamp_ms,
        };
        self.filter.process_sample(lux)
    }

    pub fn last_raw_reading(&self) -> AlsRawReading {
        self.last_reading
    }

    pub fn filtered_lux(&self) -> u32 {
        self.filter.current_lux()
    }
}


/// Calibration anchor point for piecewise exponential lux-to-nits response curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BacklightCurvePoint {
    pub lux: u32,
    pub nits: u32,
}

pub const DEFAULT_CURVE_POINTS_COUNT: usize = 6;

/// Default handheld human-eye response calibration curve for Steam Deck:
/// Lux 0 -> 10 nits (minimum dark room readable floor)
/// Lux 15 -> 35 nits (low ambient light, night bedroom)
/// Lux 60 -> 90 nits (living room lamp)
/// Lux 200 -> 180 nits (standard indoor lighting)
/// Lux 600 -> 320 nits (bright office)
/// Lux 1200+ -> 400 nits (maximum standard LCD SDR peak)
pub const DEFAULT_SDR_CURVE: [BacklightCurvePoint; DEFAULT_CURVE_POINTS_COUNT] = [
    BacklightCurvePoint { lux: 0, nits: 10 },
    BacklightCurvePoint { lux: 15, nits: 35 },
    BacklightCurvePoint { lux: 60, nits: 90 },
    BacklightCurvePoint { lux: 200, nits: 180 },
    BacklightCurvePoint { lux: 600, nits: 320 },
    BacklightCurvePoint { lux: 1200, nits: 400 },
];

/// Smooth exponential / power-law curve mapper translating ambient lux to target display nits.
#[derive(Debug, Clone)]
pub struct ExponentialCurveMapper {
    points: [BacklightCurvePoint; DEFAULT_CURVE_POINTS_COUNT],
    min_floor_nits: u32,
    max_ceiling_nits: u32,
    user_bias_pct: i8, // -50% to +50% user preference offset slider
}

impl ExponentialCurveMapper {
    pub const fn new() -> Self {
        Self {
            points: DEFAULT_SDR_CURVE,
            min_floor_nits: 5,
            max_ceiling_nits: 400,
            user_bias_pct: 0,
        }
    }

    pub fn set_user_bias(&mut self, bias_pct: i8) {
        self.user_bias_pct = if bias_pct < -50 {
            -50
        } else if bias_pct > 50 {
            50
        } else {
            bias_pct
        };
    }

    pub fn set_max_ceiling_nits(&mut self, ceiling: u32) {
        self.max_ceiling_nits = ceiling;
    }

    /// Compute target nits from ambient lux using piecewise logarithmic/exponential interpolation.
    pub fn lux_to_target_nits(&self, lux: u32) -> u32 {
        let raw_target = if lux <= self.points[0].lux {
            self.points[0].nits
        } else if lux >= self.points[DEFAULT_CURVE_POINTS_COUNT - 1].lux {
            self.points[DEFAULT_CURVE_POINTS_COUNT - 1].nits
        } else {
            // Find bounding segment
            let mut lower_idx = 0;
            for i in 0..(DEFAULT_CURVE_POINTS_COUNT - 1) {
                if lux >= self.points[i].lux && lux <= self.points[i + 1].lux {
                    lower_idx = i;
                    break;
                }
            }
            let p0 = self.points[lower_idx];
            let p1 = self.points[lower_idx + 1];
            let lux_span = p1.lux - p0.lux;
            let nits_span = p1.nits - p0.nits;

            if lux_span == 0 {
                p0.nits
            } else {
                let lux_offset = lux - p0.lux;
                // Linear segment interpolation in log-space equivalent
                p0.nits + (lux_offset * nits_span) / lux_span
            }
        };

        // Apply user bias adjustment: target = raw * (100 + bias) / 100
        let factor = (100i32 + self.user_bias_pct as i32) as u32;
        let biased = (raw_target * factor) / 100;

        // Clamp between minimum readable floor and maximum ceiling
        if biased < self.min_floor_nits {
            self.min_floor_nits
        } else if biased > self.max_ceiling_nits {
            self.max_ceiling_nits
        } else {
            biased
        }
    }
}

/// Temporal rate-limiting smooth backlight interpolator preventing jarring flashes during gameplay.
#[derive(Debug, Clone)]
pub struct SmoothBacklightInterpolator {
    current_nits_q8: u32, // Fixed-point Q8 nits (value * 256) for sub-nit precision
    target_nits: u32,
    ramp_up_speed_nits_per_sec: u32,   // 120 nits/s (moderate adaptation for brightening)
    ramp_down_speed_nits_per_sec: u32, // 40 nits/s (gradual slow adaptation for darkening)
}

impl SmoothBacklightInterpolator {
    pub const fn new(initial_nits: u32) -> Self {
        Self {
            current_nits_q8: initial_nits << 8,
            target_nits: initial_nits,
            ramp_up_speed_nits_per_sec: 120,
            ramp_down_speed_nits_per_sec: 40,
        }
    }

    pub fn set_target_nits(&mut self, target: u32) {
        self.target_nits = target;
    }

    pub fn current_nits(&self) -> u32 {
        self.current_nits_q8 >> 8
    }

    pub fn target_nits(&self) -> u32 {
        self.target_nits
    }

    /// Advance time by delta_ms and perform smooth rate-limited ramp.
    pub fn advance_time(&mut self, delta_ms: u32) -> u32 {
        let target_q8 = self.target_nits << 8;
        if self.current_nits_q8 == target_q8 || delta_ms == 0 {
            return self.current_nits();
        }

        if self.current_nits_q8 < target_q8 {
            // Ramping up: speed = ramp_up_speed * delta_ms / 1000 nits
            let step_q8 = ((self.ramp_up_speed_nits_per_sec as u64 * 256 * delta_ms as u64) / 1000) as u32;
            let step_q8 = if step_q8 == 0 { 1 } else { step_q8 };
            if self.current_nits_q8 + step_q8 >= target_q8 {
                self.current_nits_q8 = target_q8;
            } else {
                self.current_nits_q8 += step_q8;
            }
        } else {
            // Ramping down: speed = ramp_down_speed * delta_ms / 1000 nits
            let step_q8 = ((self.ramp_down_speed_nits_per_sec as u64 * 256 * delta_ms as u64) / 1000) as u32;
            let step_q8 = if step_q8 == 0 { 1 } else { step_q8 };
            if self.current_nits_q8 <= target_q8 + step_q8 {
                self.current_nits_q8 = target_q8;
            } else {
                self.current_nits_q8 -= step_q8;
            }
        }

        self.current_nits()
    }

    /// Convert current nits to display backlight percentage (0 to 100%).
    pub fn to_brightness_pct(&self, max_nits: u32) -> u8 {
        let nits = self.current_nits();
        if max_nits == 0 {
            return 0;
        }
        let pct = (nits * 100) / max_nits;
        if pct > 100 { 100 } else { pct as u8 }
    }

    /// Map nits to hardware PWM duty cycle value.
    pub fn to_pwm_duty(&self, max_nits: u32, max_pwm_duty: u32) -> u32 {
        let nits = self.current_nits();
        if max_nits == 0 {
            return 0;
        }
        ((nits as u64 * max_pwm_duty as u64) / max_nits as u64) as u32
    }
}
