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


/// Standard daylight D65 white point reference temperature.
pub const CCT_DAYLIGHT_D65: u32 = 6500;
/// Default warm white night mode temperature.
pub const CCT_WARM_NIGHT_DEFAULT: u32 = 3500;
/// Minimum supported warm temperature (deep amber / candlelight).
pub const CCT_MIN_KELVIN: u32 = 2000;
/// Maximum supported correlated color temperature.
pub const CCT_MAX_KELVIN: u32 = 6500;

/// Normalized RGB channel gains in Q8 fixed-point (256 = 1.0x unity gain).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbColorGain {
    pub r_q8: u16,
    pub g_q8: u16,
    pub b_q8: u16,
}

impl RgbColorGain {
    pub const fn unity() -> Self {
        Self {
            r_q8: 256,
            g_q8: 256,
            b_q8: 256,
        }
    }

    /// Calculate blue attenuation percentage relative to daylight (0% to 100% blue reduction).
    pub fn blue_reduction_pct(&self) -> u8 {
        if self.b_q8 >= 256 {
            0
        } else {
            let reduced = 256 - self.b_q8;
            ((reduced as u32 * 100) / 256) as u8
        }
    }
}

/// 3x3 Color Correction Matrix (CCM / CTM) for display hardware DRM KMS pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorTransformMatrix3x3 {
    pub matrix_q8: [[u16; 3]; 3],
}

impl ColorTransformMatrix3x3 {
    pub const fn identity() -> Self {
        Self {
            matrix_q8: [
                [256, 0, 0],
                [0, 256, 0],
                [0, 0, 256],
            ],
        }
    }

    pub fn from_rgb_gain(gain: RgbColorGain) -> Self {
        Self {
            matrix_q8: [
                [gain.r_q8, 0, 0],
                [0, gain.g_q8, 0],
                [0, 0, gain.b_q8],
            ],
        }
    }
}

/// Operating policy for night mode blue-light filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NightModePolicy {
    /// Night mode disabled (always 6500K daylight).
    Disabled,
    /// Manually locked to custom Kelvin target.
    Manual,
    /// Scheduled activation by local time minute of day (0 to 1439).
    Scheduled { start_minute: u16, end_minute: u16 },
    /// Automatically engage warm white when ambient lux drops below threshold.
    AmbientLuxGuided { trigger_lux: u32 },
}

/// Governor coordinating CCT color temperature Kelvin warm white shift and blue light filter.
#[derive(Debug, Clone)]
pub struct NightModeGovernor {
    current_cct_q8: u32, // Fixed-point Q8 Kelvin for ultra-smooth ramp
    target_cct: u32,
    policy: NightModePolicy,
    configured_warm_target: u32,
    transition_speed_kelvin_per_sec: u32,
    is_active: bool,
}

impl NightModeGovernor {
    pub const fn new() -> Self {
        Self {
            current_cct_q8: CCT_DAYLIGHT_D65 << 8,
            target_cct: CCT_DAYLIGHT_D65,
            policy: NightModePolicy::Disabled,
            configured_warm_target: CCT_WARM_NIGHT_DEFAULT,
            transition_speed_kelvin_per_sec: 150, // 150 Kelvin/s transition rate
            is_active: false,
        }
    }

    pub fn set_policy(&mut self, policy: NightModePolicy) {
        self.policy = policy;
    }

    pub fn set_warm_target(&mut self, kelvin: u32) {
        let clamped = if kelvin < CCT_MIN_KELVIN {
            CCT_MIN_KELVIN
        } else if kelvin > CCT_MAX_KELVIN {
            CCT_MAX_KELVIN
        } else {
            kelvin
        };
        self.configured_warm_target = clamped;
    }

    pub fn current_cct(&self) -> u32 {
        self.current_cct_q8 >> 8
    }

    pub fn target_cct(&self) -> u32 {
        self.target_cct
    }

    pub fn is_night_mode_active(&self) -> bool {
        self.is_active
    }

    /// Evaluate current policy against clock minute and ambient lux.
    pub fn update_policy_state(&mut self, current_minute_of_day: u16, ambient_lux: u32) {
        let should_activate = match self.policy {
            NightModePolicy::Disabled => false,
            NightModePolicy::Manual => true,
            NightModePolicy::Scheduled { start_minute, end_minute } => {
                if start_minute <= end_minute {
                    current_minute_of_day >= start_minute && current_minute_of_day < end_minute
                } else {
                    // Wrap-around midnight (e.g. 21:00 (1260) to 07:00 (420))
                    current_minute_of_day >= start_minute || current_minute_of_day < end_minute
                }
            }
            NightModePolicy::AmbientLuxGuided { trigger_lux } => ambient_lux <= trigger_lux,
        };

        self.is_active = should_activate;
        if should_activate {
            self.target_cct = self.configured_warm_target;
        } else {
            self.target_cct = CCT_DAYLIGHT_D65;
        }
    }

    /// Advance governor simulation time and perform smooth transition.
    pub fn advance_time(&mut self, delta_ms: u32) -> u32 {
        let target_q8 = self.target_cct << 8;
        if self.current_cct_q8 == target_q8 || delta_ms == 0 {
            return self.current_cct();
        }

        let step_q8 = ((self.transition_speed_kelvin_per_sec as u64 * 256 * delta_ms as u64) / 1000) as u32;
        let step_q8 = if step_q8 == 0 { 1 } else { step_q8 };

        if self.current_cct_q8 < target_q8 {
            if self.current_cct_q8 + step_q8 >= target_q8 {
                self.current_cct_q8 = target_q8;
            } else {
                self.current_cct_q8 += step_q8;
            }
        } else {
            if self.current_cct_q8 <= target_q8 + step_q8 {
                self.current_cct_q8 = target_q8;
            } else {
                self.current_cct_q8 -= step_q8;
            }
        }

        self.current_cct()
    }

    /// Convert Correlated Color Temperature (Kelvin) to RGB channel gains.
    pub fn compute_rgb_gain(&self) -> RgbColorGain {
        let kelvin = self.current_cct();
        Self::kelvin_to_rgb(kelvin)
    }

    /// Produce 3x3 color transform matrix for DRM KMS scanout pipeline.
    pub fn to_display_ctm(&self) -> ColorTransformMatrix3x3 {
        let gain = self.compute_rgb_gain();
        ColorTransformMatrix3x3::from_rgb_gain(gain)
    }

    /// Convert Kelvin (2000K to 6500K) to calibrated fixed-point Q8 RGB gains.
    pub fn kelvin_to_rgb(kelvin: u32) -> RgbColorGain {
        let k = if kelvin < CCT_MIN_KELVIN {
            CCT_MIN_KELVIN
        } else if kelvin > CCT_MAX_KELVIN {
            CCT_MAX_KELVIN
        } else {
            kelvin
        };

        // Red is 100% (256) for all temperatures <= 6500K.
        let r_q8 = 256u16;

        // Piecewise lookup table for Green and Blue attenuation from 2000K to 6500K in 500K steps:
        // 2000K: G=140, B=35
        // 2500K: G=168, B=72
        // 3000K: G=188, B=108
        // 3500K: G=204, B=140
        // 4000K: G=216, B=168
        // 4500K: G=226, B=192
        // 5000K: G=236, B=212
        // 5500K: G=244, B=230
        // 6000K: G=250, B=244
        // 6500K: G=256, B=256
        const TABLE_K: [u32; 10] = [2000, 2500, 3000, 3500, 4000, 4500, 5000, 5500, 6000, 6500];
        const TABLE_G: [u16; 10] = [140, 168, 188, 204, 216, 226, 236, 244, 250, 256];
        const TABLE_B: [u16; 10] = [35, 72, 108, 140, 168, 192, 212, 230, 244, 256];

        let mut idx = 0;
        for i in 0..9 {
            if k >= TABLE_K[i] && k <= TABLE_K[i + 1] {
                idx = i;
                break;
            }
        }

        let k0 = TABLE_K[idx];
        let k1 = TABLE_K[idx + 1];
        let span = k1 - k0;
        let offset = k - k0;

        let g0 = TABLE_G[idx] as u32;
        let g1 = TABLE_G[idx + 1] as u32;
        let g_interp = g0 + ((offset * (g1 - g0)) / span);

        let b0 = TABLE_B[idx] as u32;
        let b1 = TABLE_B[idx + 1] as u32;
        let b_interp = b0 + ((offset * (b1 - b0)) / span);

        RgbColorGain {
            r_q8,
            g_q8: g_interp as u16,
            b_q8: b_interp as u16,
        }
    }
}


/// Panel display technology variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdrPanelType {
    /// Steam Deck LCD (SDR 400-nit, 68% sRGB).
    SteamDeckLcdSdr,
    /// Steam Deck OLED (HDR10 1000-nit peak, 600-nit sustained, 110% DCI-P3).
    SteamDeckOledHdr10,
    /// External HDMI 2.0 / DP 1.4 HDR10 monitor/TV connected via dock.
    ExternalDisplayHdr,
}

/// CIE 1931 xy chromaticity coordinates (normalized in units of 0.00002 per CTA-861-G).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromaticityPoint {
    pub x: u16,
    pub y: u16,
}

impl ChromaticityPoint {
    pub const fn from_f32_approx(x: u32, y: u32) -> Self {
        // x and y in 1/100,000ths
        Self {
            x: ((x * 50000) / 100000) as u16,
            y: ((y * 50000) / 100000) as u16,
        }
    }
}

/// SMPTE ST 2086 mastering display color primaries and white point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MasteringDisplayPrimaries {
    pub red: ChromaticityPoint,
    pub green: ChromaticityPoint,
    pub blue: ChromaticityPoint,
    pub white: ChromaticityPoint,
}

/// DCI-P3 color gamut primaries for Steam Deck OLED panel.
pub const STEAM_DECK_OLED_PRIMARIES: MasteringDisplayPrimaries = MasteringDisplayPrimaries {
    // Red: x = 0.680, y = 0.320 -> x = 34000, y = 16000
    red: ChromaticityPoint { x: 34000, y: 16000 },
    // Green: x = 0.265, y = 0.690 -> x = 13250, y = 34500
    green: ChromaticityPoint { x: 13250, y: 34500 },
    // Blue: x = 0.150, y = 0.060 -> x = 7500, y = 3000
    blue: ChromaticityPoint { x: 7500, y: 3000 },
    // D65 White: x = 0.3127, y = 0.3290 -> x = 15635, y = 16450
    white: ChromaticityPoint { x: 15635, y: 16450 },
};

/// SMPTE ST 2086 / CTA-861-G static HDR mastering display metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdrMasteringMetadata {
    pub primaries: MasteringDisplayPrimaries,
    pub max_luminance_nits: u32,
    pub min_luminance_subnits: u32, // In 0.0001 cd/m² units (5 = 0.0005 nits true black for OLED)
}

/// CTA-861.3 content light level metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdrContentLightLevel {
    pub max_cll_nits: u16,  // Maximum Content Light Level (peak specular highlight)
    pub max_fall_nits: u16, // Maximum Frame-Average Light Level
}

/// Tone-mapping algorithm operator for HDR scene compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneMappingOperator {
    /// Passthrough (1:1 linear PQ without compression).
    Passthrough,
    /// Reinhard highlight compression.
    Reinhard,
    /// Filmic S-curve tone mapper (preserves shadow contrast and highlight rolloff).
    FilmicHable,
    /// AMD FreeSync Premium Pro / Valve dynamic HDR tone mapper.
    AmdDynamicHdr,
}

/// Dynamic tone-mapping state calibrated against ambient room illumination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicToneMappingParameters {
    pub operator: ToneMappingOperator,
    pub paper_white_nits: u32,
    pub peak_specular_nits: u32,
    pub shadow_boost_pct: u8,
    pub contrast_exponent_q8: u16, // 256 = 1.0 gamma exponent
}

/// HDR display tone-mapping auto-calibrator coordinating metadata injection.
#[derive(Debug, Clone)]
pub struct HdrAutoCalibrator {
    panel_type: HdrPanelType,
    hdr_enabled: bool,
    mastering_meta: HdrMasteringMetadata,
    content_light_level: HdrContentLightLevel,
    tone_mapping: DynamicToneMappingParameters,
}

impl HdrAutoCalibrator {
    pub const fn new(panel_type: HdrPanelType) -> Self {
        let (max_nits, min_subnits) = match panel_type {
            HdrPanelType::SteamDeckLcdSdr => (400, 5000),      // 0.5 nits IPS black level
            HdrPanelType::SteamDeckOledHdr10 => (1000, 5),     // 0.0005 nits OLED true black
            HdrPanelType::ExternalDisplayHdr => (1000, 100),   // 0.01 nits typical HDR monitor
        };

        Self {
            panel_type,
            hdr_enabled: match panel_type {
                HdrPanelType::SteamDeckOledHdr10 | HdrPanelType::ExternalDisplayHdr => true,
                HdrPanelType::SteamDeckLcdSdr => false,
            },
            mastering_meta: HdrMasteringMetadata {
                primaries: STEAM_DECK_OLED_PRIMARIES,
                max_luminance_nits: max_nits,
                min_luminance_subnits: min_subnits,
            },
            content_light_level: HdrContentLightLevel {
                max_cll_nits: max_nits as u16,
                max_fall_nits: (max_nits / 2) as u16,
            },
            tone_mapping: DynamicToneMappingParameters {
                operator: ToneMappingOperator::FilmicHable,
                paper_white_nits: 200,
                peak_specular_nits: max_nits,
                shadow_boost_pct: 0,
                contrast_exponent_q8: 256,
            },
        }
    }

    pub fn is_hdr_supported(&self) -> bool {
        self.panel_type != HdrPanelType::SteamDeckLcdSdr
    }

    pub fn is_hdr_active(&self) -> bool {
        self.hdr_enabled && self.is_hdr_supported()
    }

    pub fn set_hdr_enabled(&mut self, enabled: bool) -> bool {
        if enabled && !self.is_hdr_supported() {
            return false;
        }
        self.hdr_enabled = enabled;
        true
    }

    pub fn panel_type(&self) -> HdrPanelType {
        self.panel_type
    }

    pub fn set_panel_type(&mut self, panel: HdrPanelType) {
        self.panel_type = panel;
        let (max_nits, min_subnits) = match panel {
            HdrPanelType::SteamDeckLcdSdr => (400, 5000),
            HdrPanelType::SteamDeckOledHdr10 => (1000, 5),
            HdrPanelType::ExternalDisplayHdr => (1000, 100),
        };
        self.mastering_meta.max_luminance_nits = max_nits;
        self.mastering_meta.min_luminance_subnits = min_subnits;
        self.content_light_level.max_cll_nits = max_nits as u16;
        self.content_light_level.max_fall_nits = (max_nits / 2) as u16;
        self.tone_mapping.peak_specular_nits = max_nits;
    }

    pub fn mastering_metadata(&self) -> HdrMasteringMetadata {
        self.mastering_meta
    }

    pub fn content_light_level(&self) -> HdrContentLightLevel {
        self.content_light_level
    }

    pub fn tone_mapping_params(&self) -> DynamicToneMappingParameters {
        self.tone_mapping
    }

    /// Adapt HDR tone-mapping parameters dynamically based on ambient lux.
    /// In dark gaming environments (0-50 lux), lowers paper white to 120-160 nits to preserve dark adaptation,
    /// retaining massive 1000-nit specular headroom.
    /// In bright daylight (300-1000+ lux), lifts paper white to 280-350 nits and boosts shadow tones.
    pub fn recalibrate_for_ambient_lux(&mut self, ambient_lux: u32) {
        if !self.is_hdr_active() {
            return;
        }

        if ambient_lux <= 20 {
            // Dark room / night gaming: prevent eye fatigue, keep paper white modest
            self.tone_mapping.paper_white_nits = 140;
            self.tone_mapping.shadow_boost_pct = 0;
            self.tone_mapping.contrast_exponent_q8 = 256;
        } else if ambient_lux <= 150 {
            // Normal indoor lighting
            self.tone_mapping.paper_white_nits = 200;
            self.tone_mapping.shadow_boost_pct = 5;
            self.tone_mapping.contrast_exponent_q8 = 256;
        } else if ambient_lux <= 600 {
            // Bright office / daylight room: lift paper white and compensate shadow detail
            self.tone_mapping.paper_white_nits = 280;
            self.tone_mapping.shadow_boost_pct = 20;
            self.tone_mapping.contrast_exponent_q8 = 240; // slightly brighten near-blacks
        } else {
            // Outdoor sun: push paper white to 350-400 nits and maximize shadow boost
            self.tone_mapping.paper_white_nits = 380;
            self.tone_mapping.shadow_boost_pct = 40;
            self.tone_mapping.contrast_exponent_q8 = 220;
        }
    }

    /// Produce 26-byte CTA-861-G HDR Static Metadata Data Block (HDMI/DP AVI InfoFrame payload).
    pub fn serialize_cta861_metadata_block(&self, out: &mut [u8; 26]) {
        // Byte 0: Extended Tag Code (0x07 = Capability Data Block / HDR Static Metadata)
        out[0] = 0x07;
        // Byte 1: Electro-Optical Transfer Function (0x02 = SMPTE ST 2084 PQ)
        out[1] = if self.is_hdr_active() { 0x02 } else { 0x00 };
        // Byte 2: Static Metadata Descriptor Type (0x00 = Type 1)
        out[2] = 0x00;

        let prim = &self.mastering_meta.primaries;
        // Bytes 3-4: Display Primary 0 (Green) X
        out[3] = (prim.green.x & 0xFF) as u8;
        out[4] = ((prim.green.x >> 8) & 0xFF) as u8;
        // Bytes 5-6: Display Primary 0 (Green) Y
        out[5] = (prim.green.y & 0xFF) as u8;
        out[6] = ((prim.green.y >> 8) & 0xFF) as u8;
        // Bytes 7-8: Display Primary 1 (Blue) X
        out[7] = (prim.blue.x & 0xFF) as u8;
        out[8] = ((prim.blue.x >> 8) & 0xFF) as u8;
        // Bytes 9-10: Display Primary 1 (Blue) Y
        out[9] = (prim.blue.y & 0xFF) as u8;
        out[10] = ((prim.blue.y >> 8) & 0xFF) as u8;
        // Bytes 11-12: Display Primary 2 (Red) X
        out[11] = (prim.red.x & 0xFF) as u8;
        out[12] = ((prim.red.x >> 8) & 0xFF) as u8;
        // Bytes 13-14: Display Primary 2 (Red) Y
        out[13] = (prim.red.y & 0xFF) as u8;
        out[14] = ((prim.red.y >> 8) & 0xFF) as u8;
        // Bytes 15-16: White Point X
        out[15] = (prim.white.x & 0xFF) as u8;
        out[16] = ((prim.white.x >> 8) & 0xFF) as u8;
        // Bytes 17-18: White Point Y
        out[17] = (prim.white.y & 0xFF) as u8;
        out[18] = ((prim.white.y >> 8) & 0xFF) as u8;

        // Bytes 19-20: Max Mastering Luminance (nits)
        let max_lum = self.mastering_meta.max_luminance_nits as u16;
        out[19] = (max_lum & 0xFF) as u8;
        out[20] = ((max_lum >> 8) & 0xFF) as u8;

        // Bytes 21-22: Min Mastering Luminance (0.0001 nits)
        let min_lum = self.mastering_meta.min_luminance_subnits as u16;
        out[21] = (min_lum & 0xFF) as u8;
        out[22] = ((min_lum >> 8) & 0xFF) as u8;

        // Bytes 23-24: MaxCLL
        let max_cll = self.content_light_level.max_cll_nits;
        out[23] = (max_cll & 0xFF) as u8;
        out[24] = ((max_cll >> 8) & 0xFF) as u8;

        // Byte 25: MaxFALL LSB
        out[25] = (self.content_light_level.max_fall_nits & 0xFF) as u8;
    }
}


/// Unified userspace coordinator for ambient light monitoring, backlight modulation,
/// night mode CCT color warmth, and HDR tone mapping.
pub struct BacklightDaemon {
    pub als_driver: AlsDriver,
    pub curve_mapper: ExponentialCurveMapper,
    pub interpolator: SmoothBacklightInterpolator,
    pub night_mode: NightModeGovernor,
    pub hdr_calibrator: HdrAutoCalibrator,
    pub auto_brightness_enabled: bool,
}

impl BacklightDaemon {
    pub fn new(panel_type: HdrPanelType) -> Self {
        let mut daemon = Self {
            als_driver: AlsDriver::new(AlsSensorConfig::default()),
            curve_mapper: ExponentialCurveMapper::new(),
            interpolator: SmoothBacklightInterpolator::new(180),
            night_mode: NightModeGovernor::new(),
            hdr_calibrator: HdrAutoCalibrator::new(panel_type),
            auto_brightness_enabled: true,
        };
        daemon.als_driver.init();
        daemon
    }

    /// Run cyclic service tick with elapsed milliseconds and current time.
    pub fn tick(&mut self, delta_ms: u32, minute_of_day: u16) {
        let current_lux = self.als_driver.filtered_lux();

        if self.auto_brightness_enabled {
            let target_nits = self.curve_mapper.lux_to_target_nits(current_lux);
            self.interpolator.set_target_nits(target_nits);
        }

        self.interpolator.advance_time(delta_ms);

        self.night_mode.update_policy_state(minute_of_day, current_lux);
        self.night_mode.advance_time(delta_ms);

        if self.hdr_calibrator.is_hdr_active() {
            self.hdr_calibrator.recalibrate_for_ambient_lux(current_lux);
        }
    }
}

/// Verification self-test suite for Ambient Light & HDR Display Auto-Calibrator subsystem (TASK-KNT-333).
pub fn verify_backlight_daemon_subsystem() -> bool {
    // 1. Verify ALS optical filter, jitter rejection, and occlusion dampening
    let mut als = AlsDriver::new(AlsSensorConfig::default());
    if !als.init() {
        return false;
    }

    // Ingest stable indoor light samples (200 lux)
    for _ in 0..10 {
        als.ingest_direct_lux(200, 100);
    }
    let stable_lux = als.filtered_lux();
    if stable_lux < 180 || stable_lux > 220 {
        return false;
    }

    // Ingest sudden transient finger occlusion (e.g. 20 lux)
    let dampened = als.ingest_direct_lux(20, 200);
    // Optical filter must reject/damp sudden 90% single-frame occlusion
    if dampened < 100 {
        return false;
    }

    // 2. Verify exponential lux-to-nits curve mapping and user bias
    let mut mapper = ExponentialCurveMapper::new();
    let nits_dark = mapper.lux_to_target_nits(0);
    if nits_dark < 5 || nits_dark > 15 {
        return false; // Minimum readable floor must be preserved
    }

    let nits_indoor = mapper.lux_to_target_nits(200);
    if nits_indoor < 160 || nits_indoor > 200 {
        return false;
    }

    let nits_bright = mapper.lux_to_target_nits(1200);
    if nits_bright != 400 {
        return false;
    }

    // User bias test: +25% bias
    mapper.set_user_bias(25);
    let nits_biased = mapper.lux_to_target_nits(200);
    if nits_biased < 210 || nits_biased > 240 {
        return false;
    }

    // 3. Verify temporal smooth backlight rate-limiting
    let mut interp = SmoothBacklightInterpolator::new(50);
    interp.set_target_nits(200);
    // Advance 500ms -> at 120 nits/s ramp up, step should be ~60 nits
    let nits_mid = interp.advance_time(500);
    if nits_mid < 90 || nits_mid > 130 {
        return false;
    }
    // Advance another 1500ms -> should reach 200 nits target
    let nits_final = interp.advance_time(1500);
    if nits_final != 200 {
        return false;
    }

    // Verify ramp down (slower adaptation)
    interp.set_target_nits(50);
    // Advance 1000ms -> at 40 nits/s ramp down, step should be ~40 nits
    let nits_down = interp.advance_time(1000);
    if nits_down < 140 || nits_down > 175 {
        return false;
    }

    // 4. Verify Night Mode CCT Kelvin governor & RGB gain calculation
    let mut governor = NightModeGovernor::new();
    // Default daylight: 6500K
    let gain_d65 = governor.compute_rgb_gain();
    if gain_d65.r_q8 != 256 || gain_d65.g_q8 != 256 || gain_d65.b_q8 != 256 {
        return false;
    }
    if gain_d65.blue_reduction_pct() != 0 {
        return false;
    }

    // Warm white night mode: 3500K
    governor.set_warm_target(3500);
    governor.set_policy(NightModePolicy::Manual);
    governor.update_policy_state(0, 50);
    // Advance transition by 20 seconds (more than enough for 150K/s ramp of 3000K)
    governor.advance_time(20_000);
    if governor.current_cct() != 3500 {
        return false;
    }

    let gain_warm = governor.compute_rgb_gain();
    if gain_warm.r_q8 != 256 || gain_warm.b_q8 >= 200 || gain_warm.blue_reduction_pct() < 30 {
        return false;
    }

    // Test CTM matrix generation
    let ctm = governor.to_display_ctm();
    if ctm.matrix_q8[0][0] != 256 || ctm.matrix_q8[2][2] != gain_warm.b_q8 {
        return false;
    }

    // Test midnight schedule wrap-around: 22:00 (1320) to 06:00 (360)
    governor.set_policy(NightModePolicy::Scheduled {
        start_minute: 1320,
        end_minute: 360,
    });
    governor.update_policy_state(100, 100); // 01:40 AM -> should be active
    if !governor.is_night_mode_active() {
        return false;
    }
    governor.update_policy_state(720, 100); // 12:00 PM -> should be inactive
    if governor.is_night_mode_active() {
        return false;
    }

    // 5. Verify HDR10 OLED metadata injection and ambient tone-mapping
    let mut hdr = HdrAutoCalibrator::new(HdrPanelType::SteamDeckOledHdr10);
    if !hdr.is_hdr_supported() || !hdr.is_hdr_active() {
        return false;
    }
    let meta = hdr.mastering_metadata();
    if meta.max_luminance_nits != 1000 || meta.min_luminance_subnits != 5 {
        return false;
    }

    // Ambient recalibration in dark room
    hdr.recalibrate_for_ambient_lux(10);
    if hdr.tone_mapping_params().paper_white_nits > 160 {
        return false;
    }

    // Ambient recalibration in bright room
    hdr.recalibrate_for_ambient_lux(500);
    if hdr.tone_mapping_params().paper_white_nits < 250 {
        return false;
    }

    // CTA-861-G metadata serialization
    let mut cta_block = [0u8; 26];
    hdr.serialize_cta861_metadata_block(&mut cta_block);
    if cta_block[0] != 0x07 || cta_block[1] != 0x02 {
        return false;
    }
    let max_lum = (cta_block[19] as u16) | ((cta_block[20] as u16) << 8);
    if max_lum != 1000 {
        return false;
    }

    // 6. Verify end-to-end BacklightDaemon integration loop
    let mut daemon = BacklightDaemon::new(HdrPanelType::SteamDeckOledHdr10);
    daemon.als_driver.ingest_direct_lux(150, 1000);
    daemon.tick(100, 600);
    if daemon.interpolator.current_nits() == 0 {
        return false;
    }

    true
}
