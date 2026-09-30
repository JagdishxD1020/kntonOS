//! Handheld Precision Gyroscope Calibration, Dynamic Anti-Drift Zero-Point Filter & Complementary Orientation Fusion.
//!
//! Provides motionless stationary gyro bias tracking, temperature-dependent scale factor
//! compensation across chassis thermal gradients (30°C to 65°C), and low-overhead 600Hz
//! complementary quaternion attitude fusion.

use crate::drivers::input::imu::{sqrt_f32, Vector3};

/// Standard nominal gravity in g (1.0 g).
pub const NOMINAL_GRAVITY_G: f32 = 1.0;

/// Default sample rate for handheld gyro processing pipeline (600 Hz).
pub const GYRO_SAMPLE_RATE_HZ: f32 = 600.0;
/// Default sample interval in seconds (1 / 600 s ≈ 0.001667 s).
pub const GYRO_SAMPLE_DT_S: f32 = 1.0 / GYRO_SAMPLE_RATE_HZ;

// ============================================================================
// Feature 1: Dynamic Zero-Point Drift Filter & Motionless Bias Estimator
// ============================================================================

/// Dynamic Zero-Point Drift Filter tracking and suppressing stationary gyro bias.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicZeroPointFilter {
    /// Low-pass filtered angular velocity vector (rad/s).
    pub gyro_lpf: Vector3,
    /// Low-pass filtered linear acceleration vector (g).
    pub accel_lpf: Vector3,
    /// Learned stationary zero-point bias offset (rad/s).
    pub gyro_bias: Vector3,
    /// Number of consecutive motionless samples observed.
    pub stationary_samples: u32,
    /// Current stationary flag state.
    pub is_stationary: bool,
    /// Low-pass filter smoothing coefficient (0.0 < alpha <= 1.0).
    pub lpf_alpha: f32,
    /// Angular velocity magnitude threshold (rad/s) below which motion is deemed stationary.
    pub gyro_motion_thresh_rads: f32,
    /// Acceleration deviation from 1.0g threshold (g) for stationary detection.
    pub accel_grav_tolerance_g: f32,
    /// Exponential learning rate for stationary zero-point bias adaptation.
    pub bias_alpha: f32,
    /// Minimum consecutive samples required to confirm motionless state.
    pub min_stationary_samples: u32,
}

impl DynamicZeroPointFilter {
    /// Constructs a zero-point drift filter with calibrated handheld defaults.
    pub const fn new() -> Self {
        Self {
            gyro_lpf: Vector3::zero(),
            accel_lpf: Vector3::new(0.0, 0.0, 1.0),
            gyro_bias: Vector3::zero(),
            stationary_samples: 0,
            is_stationary: false,
            lpf_alpha: 0.25,
            gyro_motion_thresh_rads: 0.035, // ~2.0 deg/sec
            accel_grav_tolerance_g: 0.08,    // +/- 80mg around 1.0g
            bias_alpha: 0.015,
            min_stationary_samples: 25,
        }
    }

    /// Resets the filter state and accumulated bias.
    pub fn reset(&mut self) {
        self.gyro_lpf = Vector3::zero();
        self.accel_lpf = Vector3::new(0.0, 0.0, 1.0);
        self.gyro_bias = Vector3::zero();
        self.stationary_samples = 0;
        self.is_stationary = false;
    }

    /// Ingests raw gyro (rad/s) and accel (g) readings, updates stationary state, and returns debiased gyro.
    pub fn update(&mut self, raw_gyro: Vector3, raw_accel: Vector3) -> Vector3 {
        // 1. Update high-frequency single-pole IIR low-pass filter
        self.gyro_lpf.x += self.lpf_alpha * (raw_gyro.x - self.gyro_lpf.x);
        self.gyro_lpf.y += self.lpf_alpha * (raw_gyro.y - self.gyro_lpf.y);
        self.gyro_lpf.z += self.lpf_alpha * (raw_gyro.z - self.gyro_lpf.z);

        self.accel_lpf.x += self.lpf_alpha * (raw_accel.x - self.accel_lpf.x);
        self.accel_lpf.y += self.lpf_alpha * (raw_accel.y - self.accel_lpf.y);
        self.accel_lpf.z += self.lpf_alpha * (raw_accel.z - self.accel_lpf.z);

        // 2. Evaluate motionless conditions: accel magnitude near 1.0g and gyro rate near bias
        let accel_mag = self.accel_lpf.norm();
        let grav_dev = if accel_mag >= NOMINAL_GRAVITY_G {
            accel_mag - NOMINAL_GRAVITY_G
        } else {
            NOMINAL_GRAVITY_G - accel_mag
        };

        let diff_x = self.gyro_lpf.x - self.gyro_bias.x;
        let diff_y = self.gyro_lpf.y - self.gyro_bias.y;
        let diff_z = self.gyro_lpf.z - self.gyro_bias.z;
        let gyro_diff_mag = sqrt_f32(diff_x * diff_x + diff_y * diff_y + diff_z * diff_z);

        let is_motionless = grav_dev <= self.accel_grav_tolerance_g
            && gyro_diff_mag <= self.gyro_motion_thresh_rads;

        if is_motionless {
            if self.stationary_samples < u32::MAX {
                self.stationary_samples += 1;
            }
            if self.stationary_samples >= self.min_stationary_samples {
                self.is_stationary = true;
                // Adaptively track gyro stationary zero offset
                self.gyro_bias.x += self.bias_alpha * (self.gyro_lpf.x - self.gyro_bias.x);
                self.gyro_bias.y += self.bias_alpha * (self.gyro_lpf.y - self.gyro_bias.y);
                self.gyro_bias.z += self.bias_alpha * (self.gyro_lpf.z - self.gyro_bias.z);
            }
        } else {
            self.stationary_samples = 0;
            self.is_stationary = false;
        }

        // 3. Return debiased angular rate
        Vector3::new(
            raw_gyro.x - self.gyro_bias.x,
            raw_gyro.y - self.gyro_bias.y,
            raw_gyro.z - self.gyro_bias.z,
        )
    }
}

/// Verification test for dynamic zero-point drift filter and stationary bias estimator.
pub fn verify_zero_point_drift_filter() -> bool {
    let mut filter = DynamicZeroPointFilter::new();

    // Injected stationary bias: [0.030, -0.020, 0.015] rad/s
    let true_bias = Vector3::new(0.030, -0.020, 0.015);
    let static_accel = Vector3::new(0.0, 0.0, 1.0);

    // Initial output should have full bias uncorrected
    let initial_out = filter.update(true_bias, static_accel);
    if (initial_out.x - 0.030).abs() > 0.005 {
        return false;
    }
    if filter.is_stationary {
        return false;
    }

    // Feed 150 motionless stationary samples (~0.25 seconds at 600Hz)
    for _ in 0..150 {
        filter.update(true_bias, static_accel);
    }

    // Must be marked stationary and bias estimator must have adapted close to true bias
    if !filter.is_stationary {
        return false;
    }
    if (filter.gyro_bias.x - true_bias.x).abs() > 0.005 {
        return false;
    }
    if (filter.gyro_bias.y - true_bias.y).abs() > 0.005 {
        return false;
    }
    if (filter.gyro_bias.z - true_bias.z).abs() > 0.005 {
        return false;
    }

    // Debiased output should now be close to 0.0 rad/s
    let debiased = filter.update(true_bias, static_accel);
    if debiased.norm() > 0.005 {
        return false;
    }

    // Now introduce heavy motion: angular rate 2.5 rad/s (> motion threshold)
    let dynamic_gyro = Vector3::new(2.5, 0.0, 0.0);
    let motion_out = filter.update(dynamic_gyro, static_accel);

    if filter.is_stationary {
        return false;
    }
    if filter.stationary_samples != 0 {
        return false;
    }
    // High-rate motion should be preserved minus calibrated stationary bias
    if (motion_out.x - (2.5 - true_bias.x)).abs() > 0.01 {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Thermal Scale Factor Compensation Across Chassis Gradients
// ============================================================================

/// Reference calibration temperature in degrees Celsius (nominal ambient handheld PCB).
pub const THERMAL_REF_TEMP_CELSIUS: f32 = 30.0;
/// Minimum validated operating chassis temperature in degrees Celsius.
pub const CHASSIS_TEMP_MIN_CELSIUS: f32 = 20.0;
/// Maximum validated operating chassis temperature in degrees Celsius.
pub const CHASSIS_TEMP_MAX_CELSIUS: f32 = 85.0;

/// Thermal Scale Factor & Zero-Rate Drift Compensator across 30°C to 65°C gradients.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThermalScaleCompensator {
    /// Baseline calibration temperature (nominally 30.0°C).
    pub ref_temp_celsius: f32,
    /// Sensitivity scale factor temperature coefficient (1/°C, ~350 ppm/°C).
    pub scale_tempco_per_c: f32,
    /// Axis-specific thermal bias drift coefficients (rad/s per °C).
    pub bias_tempco: Vector3,
}

impl ThermalScaleCompensator {
    /// Constructs a thermal compensator tuned for handheld APU thermal gradients.
    pub const fn new() -> Self {
        Self {
            ref_temp_celsius: THERMAL_REF_TEMP_CELSIUS,
            scale_tempco_per_c: 0.00035, // +0.035% per °C
            bias_tempco: Vector3::new(0.00018, -0.00015, 0.00022),
        }
    }

    /// Clamps temperature within safe thermal limits.
    #[inline(always)]
    pub fn clamp_temperature(temp_celsius: f32) -> f32 {
        if temp_celsius < CHASSIS_TEMP_MIN_CELSIUS {
            CHASSIS_TEMP_MIN_CELSIUS
        } else if temp_celsius > CHASSIS_TEMP_MAX_CELSIUS {
            CHASSIS_TEMP_MAX_CELSIUS
        } else {
            temp_celsius
        }
    }

    /// Computes sensitivity scale factor at the specified chassis temperature.
    #[inline(always)]
    pub fn scale_factor(&self, temp_celsius: f32) -> f32 {
        let t = Self::clamp_temperature(temp_celsius);
        let delta_t = t - self.ref_temp_celsius;
        1.0 + self.scale_tempco_per_c * delta_t
    }

    /// Computes thermal zero-rate bias drift vector at the specified temperature.
    #[inline(always)]
    pub fn thermal_bias(&self, temp_celsius: f32) -> Vector3 {
        let t = Self::clamp_temperature(temp_celsius);
        let delta_t = t - self.ref_temp_celsius;
        Vector3::new(
            self.bias_tempco.x * delta_t,
            self.bias_tempco.y * delta_t,
            self.bias_tempco.z * delta_t,
        )
    }

    /// Compensates gyro angular velocity against thermal sensitivity drift and zero-rate bias.
    pub fn compensate(&self, uncompensated_gyro: Vector3, temp_celsius: f32) -> Vector3 {
        let s = self.scale_factor(temp_celsius);
        let b = self.thermal_bias(temp_celsius);

        let debiased_x = uncompensated_gyro.x - b.x;
        let debiased_y = uncompensated_gyro.y - b.y;
        let debiased_z = uncompensated_gyro.z - b.z;

        let inv_s = 1.0 / s;
        Vector3::new(debiased_x * inv_s, debiased_y * inv_s, debiased_z * inv_s)
    }
}

/// Verification test for thermal scale factor compensation across 30°C to 65°C gradients.
pub fn verify_thermal_scale_factor_compensation() -> bool {
    let comp = ThermalScaleCompensator::new();

    // 1. Nominal ambient reference test at 30°C: scale must be 1.0, thermal bias zero
    let s_ref = comp.scale_factor(30.0);
    if (s_ref - 1.0).abs() > 1e-6 {
        return false;
    }
    let b_ref = comp.thermal_bias(30.0);
    if b_ref.norm() > 1e-6 {
        return false;
    }
    let raw_rate = Vector3::new(1.0, -1.0, 0.5);
    let comp_ref = comp.compensate(raw_rate, 30.0);
    if (comp_ref.x - raw_rate.x).abs() > 1e-6 || (comp_ref.y - raw_rate.y).abs() > 1e-6 {
        return false;
    }

    // 2. High-temp gaming APU test at 65°C (delta_t = +35°C)
    let s_65 = comp.scale_factor(65.0);
    let expected_s_65 = 1.0 + 0.00035 * 35.0; // 1.01225
    if (s_65 - expected_s_65).abs() > 1e-5 {
        return false;
    }
    let b_65 = comp.thermal_bias(65.0);
    let expected_bx_65 = 0.00018 * 35.0;
    if (b_65.x - expected_bx_65).abs() > 1e-5 {
        return false;
    }

    // 3. Test correction accuracy on an uncompensated sensor reading experiencing +35°C shift
    let true_rate = Vector3::new(1.20, -0.80, 0.40);
    // Simulating sensor raw output = true_rate * S(T) + B(T)
    let expanded_raw = Vector3::new(
        true_rate.x * expected_s_65 + b_65.x,
        true_rate.y * expected_s_65 + b_65.y,
        true_rate.z * expected_s_65 + b_65.z,
    );
    let restored = comp.compensate(expanded_raw, 65.0);
    if (restored.x - true_rate.x).abs() > 1e-5 {
        return false;
    }
    if (restored.y - true_rate.y).abs() > 1e-5 {
        return false;
    }
    if (restored.z - true_rate.z).abs() > 1e-5 {
        return false;
    }

    // 4. Test intermediate chassis gradient at 47.5°C
    let s_mid = comp.scale_factor(47.5);
    if s_mid <= 1.0 || s_mid >= s_65 {
        return false;
    }

    true
}
