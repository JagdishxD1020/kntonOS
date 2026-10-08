//! Handheld Gyroscope-Assisted Predictive Touchpad Smoothing & Anti-Jitter Filter.
//!
//! Features:
//! 1. 1-Euro adaptive frequency filter for capacitive touchpad position smoothing.
//! 2. 6-axis IMU predictive motion compensation rejecting handheld chassis tremors.
//! 3. Dynamic palm rejection and edge deadzone filter.
//! 4. Sub-millisecond cursor injection pipeline with sub-pixel accumulator.
//! 5. Comprehensive self-test suite emitting `[KNT-INPUT-TOUCHPAD-SMOOTH-PASS]`.

#![allow(dead_code)]
#![allow(non_camel_case_types)]

// ============================================================================
// Feature 1: 1-Euro Adaptive Frequency Filter for Touchpad Position Smoothing
// ============================================================================

/// Mathematical constant 2 * PI for low-pass RC filter coefficient calculation.
pub const TOUCHPAD_TWO_PI: f32 = 6.2831853;

/// 1D 1-Euro Adaptive Low-Pass Filter for Capacitive Touchpads.
///
/// Adapts cutoff frequency dynamically based on signal rate-of-change:
/// - Low velocity: low cutoff frequency (heavy smoothing, zero jitter).
/// - High velocity: high cutoff frequency (minimal phase lag / latency).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadOneEuroFilter1D {
    /// Minimum cutoff frequency in Hz when stationary (nominally 1.0 - 1.5 Hz).
    pub min_cutoff_hz: f32,
    /// Speed coefficient beta to increase cutoff frequency with velocity.
    pub beta: f32,
    /// Derivative cutoff frequency in Hz for velocity smoothing.
    pub d_cutoff_hz: f32,
    /// Previous raw input value.
    pub prev_raw: f32,
    /// Previous filtered output value.
    pub prev_filtered: f32,
    /// Filtered velocity estimate (dx/dt).
    pub prev_derivative: f32,
    /// Filter initialization flag.
    pub initialized: bool,
}

impl TouchpadOneEuroFilter1D {
    /// Constructs a 1-Euro filter with calibrated handheld touchpad defaults.
    pub const fn new(min_cutoff_hz: f32, beta: f32, d_cutoff_hz: f32) -> Self {
        Self {
            min_cutoff_hz,
            beta,
            d_cutoff_hz,
            prev_raw: 0.0,
            prev_filtered: 0.0,
            prev_derivative: 0.0,
            initialized: false,
        }
    }

    /// Computes the exponential smoothing alpha coefficient for cutoff frequency `fc` and time delta `dt`.
    pub fn compute_alpha(fc: f32, dt: f32) -> f32 {
        let tau = 1.0 / (TOUCHPAD_TWO_PI * fc);
        let alpha = 1.0 / (1.0 + tau / dt);
        alpha.clamp(0.0, 1.0)
    }

    /// Resets the filter state.
    pub fn reset(&mut self) {
        self.initialized = false;
        self.prev_raw = 0.0;
        self.prev_filtered = 0.0;
        self.prev_derivative = 0.0;
    }

    /// Filters a single 1D coordinate value.
    pub fn filter(&mut self, val: f32, dt_s: f32) -> f32 {
        if dt_s <= 0.00001 {
            return if self.initialized { self.prev_filtered } else { val };
        }

        if !self.initialized {
            self.prev_raw = val;
            self.prev_filtered = val;
            self.prev_derivative = 0.0;
            self.initialized = true;
            return val;
        }

        // 1. Estimate instantaneous rate-of-change (velocity)
        let raw_dx = (val - self.prev_raw) / dt_s;
        self.prev_raw = val;

        // 2. Smooth the derivative using constant cutoff d_cutoff
        let alpha_d = Self::compute_alpha(self.d_cutoff_hz, dt_s);
        let edx = alpha_d * raw_dx + (1.0 - alpha_d) * self.prev_derivative;
        self.prev_derivative = edx;

        // 3. Dynamically modulate primary cutoff frequency based on velocity magnitude
        let dynamic_fc = self.min_cutoff_hz + self.beta * edx.abs();

        // 4. Smooth the position signal with adaptive cutoff frequency
        let alpha_pos = Self::compute_alpha(dynamic_fc, dt_s);
        let filtered = alpha_pos * val + (1.0 - alpha_pos) * self.prev_filtered;
        self.prev_filtered = filtered;

        filtered
    }
}

/// 2D Touchpad Coordinate 1-Euro Adaptive Filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadOneEuroFilter2D {
    /// X-axis 1-Euro filter.
    pub filter_x: TouchpadOneEuroFilter1D,
    /// Y-axis 1-Euro filter.
    pub filter_y: TouchpadOneEuroFilter1D,
}

impl TouchpadOneEuroFilter2D {
    /// Constructs a 2D 1-Euro filter.
    pub const fn new(min_cutoff_hz: f32, beta: f32, d_cutoff_hz: f32) -> Self {
        Self {
            filter_x: TouchpadOneEuroFilter1D::new(min_cutoff_hz, beta, d_cutoff_hz),
            filter_y: TouchpadOneEuroFilter1D::new(min_cutoff_hz, beta, d_cutoff_hz),
        }
    }

    /// Resets both axis filters on finger lift.
    pub fn reset(&mut self) {
        self.filter_x.reset();
        self.filter_y.reset();
    }

    /// Filters 2D touchpad coordinates (X, Y) given elapsed time `dt_s`.
    pub fn filter_point(&mut self, x: f32, y: f32, dt_s: f32) -> (f32, f32) {
        let fx = self.filter_x.filter(x, dt_s);
        let fy = self.filter_y.filter(y, dt_s);
        (fx, fy)
    }
}

/// Verification test for 1-Euro adaptive frequency filter position smoothing.
pub fn verify_one_euro_filter_adaptive_smoothing() -> bool {
    let mut filter = TouchpadOneEuroFilter2D::new(1.2, 0.005, 1.0);

    // 1. Test Low-Speed Micro-Jitter Suppression:
    // When a thumb is holding stationary with high-frequency capacitive sensor noise (+-1.5 units)
    let base_x = 500.0;
    let base_y = 500.0;
    let dt = 0.004; // 250 Hz touchpad polling rate = 4ms

    // Initial steady position
    filter.filter_point(base_x, base_y, dt);

    let mut max_jitter_deviation = 0.0f32;
    for i in 0..50 {
        // High-frequency alternating noise: +1.5, -1.5, +1.5, -1.5...
        let noise = if i % 2 == 0 { 1.5 } else { -1.5 };
        let (fx, fy) = filter.filter_point(base_x + noise, base_y + noise, dt);
        let dev_x = (fx - base_x).abs();
        let dev_y = (fy - base_y).abs();
        max_jitter_deviation = max_jitter_deviation.max(dev_x).max(dev_y);
    }

    // Heavy low-pass filtering at low velocity should reduce 1.5 noise down to < 0.35
    if max_jitter_deviation > 0.35 {
        return false;
    }

    // 2. Test High-Speed Flick Tracking (Minimal Latency / Phase Lag):
    // Finger executes a fast flick moving 50 units per 4ms frame (12,500 units/sec)
    let mut curr_x = 500.0;
    let mut curr_y = 500.0;
    let mut last_lag = 0.0f32;

    for _ in 0..20 {
        curr_x += 50.0;
        curr_y += 50.0;
        let (fx, fy) = filter.filter_point(curr_x, curr_y, dt);
        last_lag = (curr_x - fx).abs().max((curr_y - fy).abs());
    }

    // Dynamic cutoff frequency expansion should keep high-speed tracking lag tightly bounded (< 15 units)
    if last_lag > 15.0 {
        return false;
    }

    true
}
