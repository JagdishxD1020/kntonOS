//! Handheld Ambient Light Sensor (ALS) Dynamic Screen Dimming & Color Governor.

#![allow(dead_code)]

/// Maximum supported ambient light level in lux.
pub const ALS_MAX_LUX: u32 = 100_000;
/// Minimum supported ambient light level in lux.
pub const ALS_MIN_LUX: u32 = 0;

/// ALS hardware sensor interface and debounced lux sample filter.
#[derive(Debug, Clone, Copy)]
pub struct AlsSensorModel {
    pub raw_lux: u32,
    pub filtered_lux: u32,
    pub debounce_window: [u32; 4],
    pub debounce_idx: usize,
    pub debounce_count: usize,
}

impl AlsSensorModel {
    /// Creates a new ALS sensor model initialized at default indoor ambient lux (300 lux).
    pub const fn new() -> Self {
        Self {
            raw_lux: 300,
            filtered_lux: 300,
            debounce_window: [300; 4],
            debounce_idx: 0,
            debounce_count: 4,
        }
    }

    /// Ingests a raw I2C/ACPI sensor lux reading (clamped 0 to 100,000 lux) and applies 4-sample debouncing.
    pub fn ingest_raw_sample(&mut self, lux: u32) -> u32 {
        let clamped = if lux > ALS_MAX_LUX { ALS_MAX_LUX } else { lux };
        self.raw_lux = clamped;
        self.debounce_window[self.debounce_idx] = clamped;
        self.debounce_idx = (self.debounce_idx + 1) % 4;
        if self.debounce_count < 4 {
            self.debounce_count += 1;
        }

        // Compute 4-sample debounced average
        let sum: u32 = self.debounce_window[..self.debounce_count].iter().sum();
        self.filtered_lux = sum / (self.debounce_count as u32);
        self.filtered_lux
    }

    /// Sets immediate filtered lux without history lag (e.g. on fresh init or wake).
    pub fn set_immediate_lux(&mut self, lux: u32) {
        let clamped = if lux > ALS_MAX_LUX { ALS_MAX_LUX } else { lux };
        self.raw_lux = clamped;
        self.filtered_lux = clamped;
        self.debounce_window = [clamped; 4];
        self.debounce_idx = 0;
        self.debounce_count = 4;
    }
}

/// Unit verification for ALS hardware sensor model and debouncer.
pub fn verify_als_sensor_model() -> bool {
    let mut als = AlsSensorModel::new();
    if als.filtered_lux != 300 {
        return false;
    }

    // Ingest sample above maximum limit: must clamp to 100,000 lux
    als.set_immediate_lux(120_000);
    if als.filtered_lux != ALS_MAX_LUX {
        return false;
    }

    // Debounce transient noise spike: 400 lux base, spike to 2000 lux
    als.set_immediate_lux(400);
    let smoothed = als.ingest_raw_sample(2000); // (400*3 + 2000) / 4 = 3200 / 4 = 800
    if smoothed != 800 || als.raw_lux != 2000 {
        return false;
    }

    true
}
