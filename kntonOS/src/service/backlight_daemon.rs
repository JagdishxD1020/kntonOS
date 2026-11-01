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
