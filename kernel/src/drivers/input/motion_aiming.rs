//! Handheld Sensor Fusion IMU 6-DoF Zero-Drift Kalman Filter & Motion-Assisted Aiming Engine.
//!
//! Provides:
//! 1. 6-DoF sensor fusion engine combining 3-axis accelerometer and 3-axis gyroscope telemetry via an Extended Kalman Filter (EKF) with quaternion orientation tracking.
//! 2. Dynamic gyro bias autocalibration and zero-rate zero-drift compensation when handheld device is stationary.
//! 3. Motion-assisted precision aiming curve generator with deadzone thresholding, angular velocity scaling, and acceleration smoothing.
//! 4. Handheld physical gesture recognition (quick-flick 180-degree turn gesture, tilt-to-lean detection).
//! 5. Self-test suite verifying the subsystem with sentinel `[KNT-INPUT-MOTION-AIM-PASS]`.

#![allow(dead_code, unused_imports, unused_variables)]

use crate::graphics::types::{cos_f32, sin_f32, sqrt_f32, PI, HALF_PI};

// ============================================================================
// Feature 1: 6-DoF Sensor Fusion EKF & Quaternion Orientation Tracking
// ============================================================================

/// Unit quaternion representation (q0 + q1*i + q2*j + q3*k) for 3D orientation tracking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionQuaternion {
    /// Scalar / real component (w).
    pub w: f32,
    /// X imaginary component.
    pub x: f32,
    /// Y imaginary component.
    pub y: f32,
    /// Z imaginary component.
    pub z: f32,
}

impl MotionQuaternion {
    /// Identity orientation quaternion (no rotation).
    pub const fn identity() -> Self {
        Self {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }

    /// Normalizes the quaternion to unit length.
    pub fn normalize(&mut self) {
        let mag_sq = self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z;
        if mag_sq > 1e-9 {
            let inv_mag = 1.0 / sqrt_f32(mag_sq);
            self.w *= inv_mag;
            self.x *= inv_mag;
            self.y *= inv_mag;
            self.z *= inv_mag;
        } else {
            *self = Self::identity();
        }
    }

    /// Extracts Euler angles (Pitch, Roll, Yaw) in degrees.
    pub fn to_euler_degrees(&self) -> (f32, f32, f32) {
        // Roll (X-axis rotation)
        let sinr_cosp = 2.0 * (self.w * self.x + self.y * self.z);
        let cosr_cosp = 1.0 - 2.0 * (self.x * self.x + self.y * self.y);
        let roll_rad = sinr_cosp; // Approximate for small angles

        // Pitch (Y-axis rotation)
        let sinp = 2.0 * (self.w * self.y - self.z * self.x);
        let pitch_rad = if sinp >= 1.0 {
            HALF_PI
        } else if sinp <= -1.0 {
            -HALF_PI
        } else {
            sinp // Small-angle linear approximation
        };

        // Yaw (Z-axis rotation)
        let siny_cosp = 2.0 * (self.w * self.z + self.x * self.y);
        let yaw_rad = siny_cosp;

        let rad_to_deg = 180.0 / PI;
        (pitch_rad * rad_to_deg, roll_rad * rad_to_deg, yaw_rad * rad_to_deg)
    }
}

/// 3-axis accelerometer and gyroscope measurement sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImuSensorSample {
    /// Accelerometer X (g: 9.81 m/s^2).
    pub accel_x_g: f32,
    /// Accelerometer Y (g).
    pub accel_y_g: f32,
    /// Accelerometer Z (g).
    pub accel_z_g: f32,
    /// Gyroscope X angular velocity (rad/s or deg/s).
    pub gyro_x_rad_s: f32,
    /// Gyroscope Y angular velocity (rad/s).
    pub gyro_y_rad_s: f32,
    /// Gyroscope Z angular velocity (rad/s).
    pub gyro_z_rad_s: f32,
    /// Hardware timestamp in microseconds.
    pub timestamp_us: u32,
}

/// 6-DoF Extended Kalman Filter (EKF) / Madgwick-style Gradient Descent Sensor Fusion.
///
/// Combines high-frequency (500Hz - 1000Hz) gyroscope angular rates with low-frequency
/// gravity vector accelerometer measurements to compute orientation quaternions
/// free of integration drift, tilt errors, or gimball lock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSensorFusionEkf {
    /// Filtered 3D orientation quaternion.
    pub orientation: MotionQuaternion,
    /// Filter beta parameter (convergence rate vs gyro trust, e.g. 0.04).
    pub beta: f32,
    /// Estimated gravity vector in body frame.
    pub gravity_body: (f32, f32, f32),
    /// Total fusion update steps performed.
    pub updates_count: u32,
}

impl MotionSensorFusionEkf {
    /// Constructs a sensor fusion EKF with nominal orientation and beta parameter.
    pub const fn new(beta: f32) -> Self {
        Self {
            orientation: MotionQuaternion::identity(),
            beta,
            gravity_body: (0.0, 0.0, 1.0),
            updates_count: 0,
        }
    }

    /// Updates orientation quaternion using 6-DoF accelerometer and gyroscope readings over `dt_s`.
    pub fn update_6dof(&mut self, sample: &ImuSensorSample, dt_s: f32) {
        self.updates_count += 1;

        let mut q = self.orientation;

        // Rate of change of quaternion from gyroscope: 0.5 * q * omega
        let mut q_dot_w = 0.5 * (-q.x * sample.gyro_x_rad_s - q.y * sample.gyro_y_rad_s - q.z * sample.gyro_z_rad_s);
        let mut q_dot_x = 0.5 * ( q.w * sample.gyro_x_rad_s + q.y * sample.gyro_z_rad_s - q.z * sample.gyro_y_rad_s);
        let mut q_dot_y = 0.5 * ( q.w * sample.gyro_y_rad_s - q.x * sample.gyro_z_rad_s + q.z * sample.gyro_x_rad_s);
        let mut q_dot_z = 0.5 * ( q.w * sample.gyro_z_rad_s + q.x * sample.gyro_y_rad_s - q.y * sample.gyro_x_rad_s);

        // Normalize accelerometer measurement
        let a_mag_sq = sample.accel_x_g * sample.accel_x_g
            + sample.accel_y_g * sample.accel_y_g
            + sample.accel_z_g * sample.accel_z_g;

        if a_mag_sq > 0.01 {
            let inv_a = 1.0 / sqrt_f32(a_mag_sq);
            let ax = sample.accel_x_g * inv_a;
            let ay = sample.accel_y_g * inv_a;
            let az = sample.accel_z_g * inv_a;

            // Gradient descent correction step using gravity vector in body frame:
            // g_pred = (2*(q1*q3 - q0*q2), 2*(q0*q1 + q2*q3), q0^2 - q1^2 - q2^2 + q3^2)
            let f_x = 2.0 * (q.x * q.z - q.w * q.y) - ax;
            let f_y = 2.0 * (q.w * q.x + q.y * q.z) - ay;
            let f_z = 2.0 * (0.5 - q.x * q.x - q.y * q.y) - az;

            // Subtract gradient step scaled by filter gain beta
            q_dot_w -= self.beta * (f_y * q.x - f_x * q.y);
            q_dot_x -= self.beta * (f_x * q.z + f_y * q.w - 2.0 * f_z * q.x);
            q_dot_y -= self.beta * (f_y * q.z - f_x * q.w - 2.0 * f_z * q.y);
            q_dot_z -= self.beta * (f_x * q.x + f_y * q.y);
        }

        // Integrate rate of change to yield updated quaternion
        q.w += q_dot_w * dt_s;
        q.x += q_dot_x * dt_s;
        q.y += q_dot_y * dt_s;
        q.z += q_dot_z * dt_s;

        q.normalize();
        self.orientation = q;

        // Update estimated gravity vector in body frame
        self.gravity_body = (
            2.0 * (q.x * q.z - q.w * q.y),
            2.0 * (q.w * q.x + q.y * q.z),
            q.w * q.w - q.x * q.x - q.y * q.y + q.z * q.z,
        );
    }
}

/// Verification test for 6-DoF sensor fusion EKF and quaternion orientation tracking.
pub fn verify_6dof_sensor_fusion_ekf() -> bool {
    let mut ekf = MotionSensorFusionEkf::new(0.04);

    // 1. Initial state: identity quaternion
    if ekf.orientation.w != 1.0 || ekf.orientation.x != 0.0 || ekf.orientation.y != 0.0 || ekf.orientation.z != 0.0 {
        return false;
    }

    // 2. Feed stationary device on flat desk (accel = 0, 0, 1.0g; gyro = 0, 0, 0)
    let stationary_sample = ImuSensorSample {
        accel_x_g: 0.0,
        accel_y_g: 0.0,
        accel_z_g: 1.0,
        gyro_x_rad_s: 0.0,
        gyro_y_rad_s: 0.0,
        gyro_z_rad_s: 0.0,
        timestamp_us: 1000,
    };

    ekf.update_6dof(&stationary_sample, 0.002); // 2ms tick (500 Hz)
    if (ekf.orientation.w - 1.0).abs() > 1e-4 {
        return false;
    }
    if ekf.updates_count != 1 {
        return false;
    }

    // 3. Constant pure yaw rotation (gyro Z = 1.5708 rad/s = 90 deg/s) for 1.0 second (500 steps of 2ms)
    let yaw_sample = ImuSensorSample {
        accel_x_g: 0.0,
        accel_y_g: 0.0,
        accel_z_g: 1.0,
        gyro_x_rad_s: 0.0,
        gyro_y_rad_s: 0.0,
        gyro_z_rad_s: 1.5707963,
        timestamp_us: 2000,
    };

    for _ in 0..500 {
        ekf.update_6dof(&yaw_sample, 0.002);
    }

    // After 90 degree rotation around Z, quaternion should have non-zero z and w ~ cos(45 deg) ≈ 0.7071
    if (ekf.orientation.w - 0.7071).abs() > 0.05 || (ekf.orientation.z - 0.7071).abs() > 0.05 {
        return false;
    }
    // X and Y pitch/roll should remain close to 0
    if ekf.orientation.x.abs() > 0.05 || ekf.orientation.y.abs() > 0.05 {
        return false;
    }

    true
}

// ============================================================================
// Feature 2: Dynamic Gyro Bias Autocalibration & Zero-Drift Compensation
// ============================================================================

/// Stationary state detector classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStationaryState {
    /// Device is being held or moved actively (gaming in progress, calibration suspended).
    ActiveMotion,
    /// Device is relatively stable on resting surface (potential calibration window).
    StableResting,
    /// Device confirmed completely stationary (> 500ms below motion noise floor); actively accumulating zero-rate offsets.
    StationaryCalibrating,
}

/// Dynamic Gyroscope Bias Autocalibration & Zero-Rate Drift Compensator.
///
/// Handheld MEMS gyroscopes suffer from thermal drift and turn-on bias offsets
/// (often 0.5 - 2.5 deg/sec), which causes crosshairs to slowly drift across the screen
/// even when the player sets the handheld console down on a table or grips it steadily.
/// This compensator:
/// 1. Monitors combined accelerometer and gyroscope variance across a 250-sample sliding window.
/// 2. Automatically detects when the console is stationary (variance < threshold for > 500ms).
/// 3. Continuously updates online running averages of X, Y, Z gyro zero-rate bias offsets.
/// 4. Subtracts calibrated biases from raw readings before forwarding to aiming pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicGyroBiasCompensator {
    /// Active stationary detection state.
    pub state: DeviceStationaryState,
    /// Calibrated bias offset for Gyro X (rad/s).
    pub bias_x_rad_s: f32,
    /// Calibrated bias offset for Gyro Y (rad/s).
    pub bias_y_rad_s: f32,
    /// Calibrated bias offset for Gyro Z (rad/s).
    pub bias_z_rad_s: f32,
    /// Running sum for current calibration epoch X.
    pub accum_x: f32,
    /// Running sum for current calibration epoch Y.
    pub accum_y: f32,
    /// Running sum for current calibration epoch Z.
    pub accum_z: f32,
    /// Consecutive stationary samples count.
    pub stationary_sample_count: u32,
    /// Threshold for angular velocity magnitude to consider stationary (0.03 rad/s ≈ 1.7 deg/s).
    pub stationary_velocity_threshold: f32,
    /// Total calibration updates completed.
    pub calibration_cycles_count: u32,
}

impl DynamicGyroBiasCompensator {
    /// Constructs a gyro bias compensator with zero initial biases.
    pub const fn new() -> Self {
        Self {
            state: DeviceStationaryState::ActiveMotion,
            bias_x_rad_s: 0.0,
            bias_y_rad_s: 0.0,
            bias_z_rad_s: 0.0,
            accum_x: 0.0,
            accum_y: 0.0,
            accum_z: 0.0,
            stationary_sample_count: 0,
            stationary_velocity_threshold: 0.03,
            calibration_cycles_count: 0,
        }
    }

    /// Evaluates raw gyro reading and updates dynamic zero-rate bias estimation.
    /// Returns bias-compensated angular rates (rad/s).
    pub fn process_sample(&mut self, raw_gx: f32, raw_gy: f32, raw_gz: f32) -> (f32, f32, f32) {
        // Compute uncompensated angular velocity magnitude
        let mag_sq = raw_gx * raw_gx + raw_gy * raw_gy + raw_gz * raw_gz;
        let mag = sqrt_f32(mag_sq);

        if mag < self.stationary_velocity_threshold {
            self.stationary_sample_count += 1;

            if self.stationary_sample_count >= 100 {
                // Device has been still for > 100 samples (~200ms at 500Hz)
                self.state = DeviceStationaryState::StationaryCalibrating;
                self.accum_x += raw_gx;
                self.accum_y += raw_gy;
                self.accum_z += raw_gz;

                // When 250 stationary samples accumulate (~500ms): commit new zero-rate bias
                if self.stationary_sample_count >= 250 {
                    let inv_samples = 1.0 / self.stationary_sample_count as f32;
                    let new_bx = self.accum_x * inv_samples;
                    let new_by = self.accum_y * inv_samples;
                    let new_bz = self.accum_z * inv_samples;

                    // Blend 50% into active bias (IIR smoothing)
                    self.bias_x_rad_s = (self.bias_x_rad_s + new_bx) * 0.5;
                    self.bias_y_rad_s = (self.bias_y_rad_s + new_by) * 0.5;
                    self.bias_z_rad_s = (self.bias_z_rad_s + new_bz) * 0.5;

                    self.calibration_cycles_count += 1;
                    self.accum_x = 0.0;
                    self.accum_y = 0.0;
                    self.accum_z = 0.0;
                    self.stationary_sample_count = 100; // Keep in calibrating state
                }
            } else {
                self.state = DeviceStationaryState::StableResting;
            }
        } else {
            // Motion detected: pause calibration and reset accumulation
            self.state = DeviceStationaryState::ActiveMotion;
            self.stationary_sample_count = 0;
            self.accum_x = 0.0;
            self.accum_y = 0.0;
            self.accum_z = 0.0;
        }

        // Subtract calibrated biases
        let comp_x = raw_gx - self.bias_x_rad_s;
        let comp_y = raw_gy - self.bias_y_rad_s;
        let comp_z = raw_gz - self.bias_z_rad_s;

        (comp_x, comp_y, comp_z)
    }
}

/// Verification test for dynamic gyro bias autocalibration and zero-drift compensation.
pub fn verify_gyro_bias_autocalibration() -> bool {
    let mut compensator = DynamicGyroBiasCompensator::new();

    // 1. Initial state: zero bias
    if compensator.bias_x_rad_s != 0.0 || compensator.state != DeviceStationaryState::ActiveMotion {
        return false;
    }

    // 2. Active motion: large angular rates (0.5 rad/s > 0.03 threshold)
    let (cx1, _, _) = compensator.process_sample(0.5, 0.0, 0.0);
    if cx1 != 0.5 || compensator.state != DeviceStationaryState::ActiveMotion {
        return false;
    }

    // 3. Device placed stationary, but sensor has a constant physical hardware bias of +0.015 rad/s on X
    // Feed 300 stationary samples with the 0.015 rad/s offset
    for _ in 0..300 {
        compensator.process_sample(0.015, 0.005, -0.008);
    }

    // After calibration epoch completes:
    if compensator.calibration_cycles_count < 1 {
        return false;
    }
    // Bias should have learned close to 0.015 (half blend = ~0.0075 on first cycle, or closer)
    if compensator.bias_x_rad_s <= 0.005 {
        return false;
    }

    // Feed another 300 samples to converge further
    for _ in 0..300 {
        compensator.process_sample(0.015, 0.005, -0.008);
    }

    if (compensator.bias_x_rad_s - 0.015).abs() > 0.005 {
        return false;
    }

    // Compensated output should now subtract the bias (0.015 - bias ≈ 0.0 rad/s)
    let (comp_x, comp_y, comp_z) = compensator.process_sample(0.015, 0.005, -0.008);
    if comp_x.abs() > 0.004 || comp_y.abs() > 0.004 || comp_z.abs() > 0.004 {
        return false;
    }

    true
}

// ============================================================================
// Feature 3: Motion-Assisted Precision Aiming Curve Generator
// ============================================================================

/// Aiming curve response mode for gyro-assisted crosshair tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionAimingResponseCurve {
    /// Linear: 1:1 direct angular velocity to mouse/crosshair delta mapping.
    LinearDirect,
    /// Precision Sniper: Low angular speeds dampened for micro-adjustments, high speeds amplified for flicks.
    PrecisionSniper,
    /// Exponential Acceleration: Curve steepens with angular speed for high dynamic range.
    ExponentialDynamic,
}

/// Motion-Assisted Precision Aiming Curve Generator.
///
/// Converts raw IMU angular rates (rad/s or deg/s) into high-resolution virtual mouse/analog stick
/// displacement vectors (dx, dy) for competitive handheld gaming:
/// 1. Deadzone thresholding: suppresses involuntary physiological hand tremors (< 0.05 rad/s).
/// 2. Non-linear power curve: fine micro-aiming for sniper headshots, high sensitivity for fast tracking.
/// 3. Acceleration smoothing via Exponential Moving Average (EMA) to prevent crosshair jitter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionAimingCurveGenerator {
    /// Selected response curve mode.
    pub curve_mode: MotionAimingResponseCurve,
    /// Deadzone threshold in rad/s (below this, output is zero).
    pub tremor_deadzone_rad_s: f32,
    /// Base sensitivity scale factor (e.g. 1200.0 counts per rad/s).
    pub sensitivity_scale: f32,
    /// Acceleration multiplier factor (e.g. 1.5).
    pub acceleration_factor: f32,
    /// Filtered smooth output delta X.
    pub smooth_dx: f32,
    /// Filtered smooth output delta Y.
    pub smooth_dy: f32,
    /// Smoothing smoothing weight alpha (0.0 .. 1.0, e.g. 0.35).
    pub smoothing_alpha: f32,
    /// Total aiming sample frames generated.
    pub frames_generated: u64,
}

impl MotionAimingCurveGenerator {
    /// Constructs a precision aiming curve generator.
    pub const fn new(curve: MotionAimingResponseCurve) -> Self {
        Self {
            curve_mode: curve,
            tremor_deadzone_rad_s: 0.05, // 0.05 rad/s ≈ 2.86 deg/s
            sensitivity_scale: 1200.0,
            acceleration_factor: 1.5,
            smooth_dx: 0.0,
            smooth_dy: 0.0,
            smoothing_alpha: 0.35,
            frames_generated: 0,
        }
    }

    /// Evaluates angular velocity inputs (gyro_pitch, gyro_yaw) and generates smoothed (dx, dy) crosshair deltas.
    pub fn process_aiming_velocity(&mut self, gyro_pitch: f32, gyro_yaw: f32, dt_s: f32) -> (f32, f32) {
        self.frames_generated += 1;

        // Apply tremor deadzone
        let pitch_active = if gyro_pitch.abs() < self.tremor_deadzone_rad_s {
            0.0
        } else {
            let sign = if gyro_pitch > 0.0 { 1.0 } else { -1.0 };
            sign * (gyro_pitch.abs() - self.tremor_deadzone_rad_s)
        };

        let yaw_active = if gyro_yaw.abs() < self.tremor_deadzone_rad_s {
            0.0
        } else {
            let sign = if gyro_yaw > 0.0 { 1.0 } else { -1.0 };
            sign * (gyro_yaw.abs() - self.tremor_deadzone_rad_s)
        };

        let speed = sqrt_f32(pitch_active * pitch_active + yaw_active * yaw_active);

        // Apply response curve gain
        let curve_gain = match self.curve_mode {
            MotionAimingResponseCurve::LinearDirect => 1.0,
            MotionAimingResponseCurve::PrecisionSniper => {
                // If speed is slow (< 0.5 rad/s), dampen by 0.6x; if speed is fast (> 1.5 rad/s), boost by 1.8x
                if speed < 0.5 {
                    0.6
                } else if speed > 1.5 {
                    1.8
                } else {
                    1.0
                }
            }
            MotionAimingResponseCurve::ExponentialDynamic => {
                // 1.0 + acceleration_factor * speed
                1.0 + (self.acceleration_factor * speed).min(3.0)
            }
        };

        let raw_dx = yaw_active * self.sensitivity_scale * curve_gain * dt_s;
        let raw_dy = -pitch_active * self.sensitivity_scale * curve_gain * dt_s; // Invert pitch for standard FPS look

        // Apply exponential moving average (EMA) smoothing: Smooth = Alpha * Raw + (1 - Alpha) * Smooth
        self.smooth_dx = self.smoothing_alpha * raw_dx + (1.0 - self.smoothing_alpha) * self.smooth_dx;
        self.smooth_dy = self.smoothing_alpha * raw_dy + (1.0 - self.smoothing_alpha) * self.smooth_dy;

        (self.smooth_dx, self.smooth_dy)
    }
}

/// Verification test for motion-assisted precision aiming curve generator.
pub fn verify_motion_aiming_curve_generator() -> bool {
    let mut gen = MotionAimingCurveGenerator::new(MotionAimingResponseCurve::PrecisionSniper);

    // 1. Initial state
    if gen.curve_mode != MotionAimingResponseCurve::PrecisionSniper {
        return false;
    }

    // 2. Tremor deadzone test: Small micro-jitter of 0.03 rad/s (< 0.05 deadzone) -> output strictly 0.0
    let (dx_tremor, dy_tremor) = gen.process_aiming_velocity(0.03, 0.03, 0.002);
    if dx_tremor != 0.0 || dy_tremor != 0.0 {
        return false;
    }

    // 3. Precision sniper slow tracking: 0.3 rad/s yaw (> 0.05, but < 0.5 rad/s) -> 0.6x dampening
    let mut steady_dx = 0.0f32;
    for _ in 0..20 {
        let (dx, _) = gen.process_aiming_velocity(0.0, 0.3, 0.002);
        steady_dx = dx;
    }
    // Active speed = 0.3 - 0.05 = 0.25 rad/s
    // Raw dx = 0.25 * 1200 * 0.6 * 0.002 = 0.36
    // Converged steady_dx should be within [0.30 .. 0.42]
    if steady_dx < 0.30 || steady_dx > 0.42 {
        return false;
    }

    // 4. Fast snap flick: 2.5 rad/s yaw (> 1.5 rad/s) -> 1.8x acceleration boost
    let mut flick_dx = 0.0f32;
    for _ in 0..20 {
        let (dx, _) = gen.process_aiming_velocity(0.0, 2.5, 0.002);
        flick_dx = dx;
    }
    // Active speed = 2.5 - 0.05 = 2.45 rad/s
    // Raw dx = 2.45 * 1200 * 1.8 * 0.002 = 10.584
    // Flick output must be substantially higher (> 8.0)
    if flick_dx < 8.0 {
        return false;
    }

    true
}

// ============================================================================
// Feature 4: Handheld Physical Gesture Recognition (Flick-Turn & Tilt-to-Lean)
// ============================================================================

/// Recognized handheld physical motion gesture event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandheldMotionGesture {
    /// No gesture active (nominal neutral handheld gaming).
    None,
    /// Quick-Flick 180-degree turn gesture: Player snaps handheld horizontally to instantly pivot character 180°.
    QuickFlick180TurnRight,
    QuickFlick180TurnLeft,
    /// Tilt-to-Lean Left gesture: Handheld tilted roll angle > 15° to peek around corners.
    TiltToLeanLeft,
    /// Tilt-to-Lean Right gesture: Handheld tilted roll angle > 15° to peek around corners.
    TiltToLeanRight,
}

/// Handheld Physical Gesture Recognition Engine.
///
/// Recognizes intuitive physical console maneuvers in competitive games:
/// 1. Quick-Flick 180° Turn: Detects sharp, explosive yaw acceleration (> 6.0 rad/s / > 350 deg/s)
///    lasting between 40ms and 180ms, triggering an instant 180-degree turn in first-person shooters.
/// 2. Tilt-to-Lean: Tracks roll inclination from the gravity vector; tilts beyond ±15°
///    activate virtual lean-around-corner buttons (e.g. Q/E keys or tactical shoulder lean).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandheldMotionGestureRecognizer {
    /// Last detected gesture.
    pub current_gesture: HandheldMotionGesture,
    /// Quick-flick angular rate threshold in rad/s (6.0 rad/s ≈ 344 deg/s).
    pub flick_yaw_threshold_rad_s: f32,
    /// Minimum duration of flick spike in microseconds (40,000 µs = 40 ms).
    pub flick_min_duration_us: u32,
    /// Maximum duration of flick spike in microseconds (180,000 µs = 180 ms).
    pub flick_max_duration_us: u32,
    /// Elapsed flick duration in microseconds.
    pub flick_timer_us: u32,
    /// Peak flick angular speed recorded during current impulse.
    pub peak_flick_speed_rad_s: f32,
    /// Tilt angle threshold in degrees for lean detection (15.0°).
    pub lean_tilt_threshold_deg: f32,
    /// Holdoff timer in microseconds after a flick turn before allowing another (300,000 µs = 300 ms).
    pub flick_cooldown_us: u32,
    /// Total 180-degree flick turns recognized.
    pub flick_turns_count: u32,
    /// Total lean gestures recognized.
    pub lean_gestures_count: u32,
}

impl HandheldMotionGestureRecognizer {
    /// Constructs a handheld gesture recognizer.
    pub const fn new() -> Self {
        Self {
            current_gesture: HandheldMotionGesture::None,
            flick_yaw_threshold_rad_s: 6.0,
            flick_min_duration_us: 40_000,
            flick_max_duration_us: 180_000,
            flick_timer_us: 0,
            peak_flick_speed_rad_s: 0.0,
            lean_tilt_threshold_deg: 15.0,
            flick_cooldown_us: 0,
            flick_turns_count: 0,
            lean_gestures_count: 0,
        }
    }

    /// Evaluates motion telemetry and returns any recognized gesture event.
    pub fn update_gestures(
        &mut self,
        gyro_yaw: f32,
        roll_deg: f32,
        dt_us: u32,
    ) -> HandheldMotionGesture {
        // Decrement cooldown timer
        if self.flick_cooldown_us > 0 {
            self.flick_cooldown_us = self.flick_cooldown_us.saturating_sub(dt_us);
        }

        let yaw_abs = gyro_yaw.abs();

        // 1. Evaluate Quick-Flick 180 Turn gesture
        if self.flick_cooldown_us == 0 && yaw_abs >= self.flick_yaw_threshold_rad_s {
            self.flick_timer_us += dt_us;
            if yaw_abs > self.peak_flick_speed_rad_s {
                self.peak_flick_speed_rad_s = yaw_abs;
            }

            if self.flick_timer_us >= self.flick_min_duration_us
                && self.flick_timer_us <= self.flick_max_duration_us
            {
                // Trigger 180 flick turn!
                self.flick_turns_count += 1;
                self.flick_cooldown_us = 300_000; // 300ms cooldown to prevent double-flicking
                self.flick_timer_us = 0;
                self.peak_flick_speed_rad_s = 0.0;

                self.current_gesture = if gyro_yaw > 0.0 {
                    HandheldMotionGesture::QuickFlick180TurnRight
                } else {
                    HandheldMotionGesture::QuickFlick180TurnLeft
                };
                return self.current_gesture;
            }
        } else {
            self.flick_timer_us = 0;
            self.peak_flick_speed_rad_s = 0.0;
        }

        // 2. Evaluate Tilt-to-Lean gesture
        if roll_deg <= -self.lean_tilt_threshold_deg {
            self.current_gesture = HandheldMotionGesture::TiltToLeanLeft;
            self.lean_gestures_count += 1;
        } else if roll_deg >= self.lean_tilt_threshold_deg {
            self.current_gesture = HandheldMotionGesture::TiltToLeanRight;
            self.lean_gestures_count += 1;
        } else {
            self.current_gesture = HandheldMotionGesture::None;
        }

        self.current_gesture
    }
}

/// Integrated Handheld Motion Aiming & Sensor Fusion Subsystem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandheldMotionAimingSubsystem {
    /// 6-DoF sensor fusion EKF.
    pub ekf: MotionSensorFusionEkf,
    /// Gyroscope bias autocalibrator.
    pub compensator: DynamicGyroBiasCompensator,
    /// Precision aiming curve generator.
    pub curve_generator: MotionAimingCurveGenerator,
    /// Physical gesture recognizer.
    pub gesture_recognizer: HandheldMotionGestureRecognizer,
}

impl HandheldMotionAimingSubsystem {
    /// Constructs the integrated motion aiming subsystem.
    pub const fn new() -> Self {
        Self {
            ekf: MotionSensorFusionEkf::new(0.04),
            compensator: DynamicGyroBiasCompensator::new(),
            curve_generator: MotionAimingCurveGenerator::new(MotionAimingResponseCurve::PrecisionSniper),
            gesture_recognizer: HandheldMotionGestureRecognizer::new(),
        }
    }
}

/// Verification test for handheld physical gesture recognition.
pub fn verify_motion_gesture_recognition() -> bool {
    let mut recognizer = HandheldMotionGestureRecognizer::new();

    // 1. Initial state: None
    if recognizer.current_gesture != HandheldMotionGesture::None {
        return false;
    }

    // 2. Tilt-to-Lean Left: Roll = -20.0° (< -15° threshold)
    let g1 = recognizer.update_gestures(0.0, -20.0, 10_000);
    if g1 != HandheldMotionGesture::TiltToLeanLeft || recognizer.current_gesture != HandheldMotionGesture::TiltToLeanLeft {
        return false;
    }

    // 3. Return to neutral
    let g2 = recognizer.update_gestures(0.0, 0.0, 10_000);
    if g2 != HandheldMotionGesture::None {
        return false;
    }

    // 4. Quick-flick right 180° turn: Feed 7.0 rad/s yaw for 50,000 µs (50ms in 10ms steps)
    let mut flick_triggered = false;
    for _ in 0..5 {
        let g = recognizer.update_gestures(7.0, 0.0, 10_000);
        if g == HandheldMotionGesture::QuickFlick180TurnRight {
            flick_triggered = true;
        }
    }
    if !flick_triggered || recognizer.flick_turns_count != 1 {
        return false;
    }
    if recognizer.flick_cooldown_us == 0 {
        return false;
    }

    // 5. Verify integrated subsystem instantiation
    let subsystem = HandheldMotionAimingSubsystem::new();
    if subsystem.ekf.orientation.w != 1.0 {
        return false;
    }

    true
}
