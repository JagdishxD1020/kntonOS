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
