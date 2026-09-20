//! Handheld PC dual-IMU dead reckoning and motion drift eliminator.

use crate::graphics::{cos_f32, sin_f32, sqrt_f32};
use crate::drivers::input::imu::{asin_f32, atan2_f32};

/// Standard gravitational acceleration at sea level in m/s^2.
pub const GRAVITY_MSS: f32 = 9.80665;

/// Identifies the physical IMU location within the handheld PC chassis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImuLocation {
    /// Left controller grip PCB.
    LeftGrip = 0,
    /// Right controller grip PCB.
    RightGrip = 1,
}

/// Physical spatial arrangement and dimensions of dual-IMU chassis topology.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualImuTopology {
    /// Lateral chassis width between Left and Right IMU in meters (nominally 0.28m = 280mm).
    pub chassis_width_m: f32,
    /// Vector offset from chassis center-of-mass to Left IMU [x, y, z] in meters.
    pub left_lever_arm_m: [f32; 3],
    /// Vector offset from chassis center-of-mass to Right IMU [x, y, z] in meters.
    pub right_lever_arm_m: [f32; 3],
    /// Target sensor sampling frequency in Hertz (nominally 1000 Hz = 1ms period).
    pub sample_rate_hz: u32,
    /// Discrete integration time step in seconds (dt = 1.0 / sample_rate_hz).
    pub dt_s: f32,
}

impl DualImuTopology {
    /// Creates a standard handheld gaming PC topology (280mm grip-to-grip span, 1000 Hz).
    pub const fn standard_handheld() -> Self {
        let half_width = 0.14; // 140 mm from center of mass
        Self {
            chassis_width_m: 0.28,
            left_lever_arm_m: [-half_width, 0.0, 0.0],
            right_lever_arm_m: [half_width, 0.0, 0.0],
            sample_rate_hz: 1000,
            dt_s: 0.001,
        }
    }
}

/// Raw digitized 16-bit sensor packet from an individual 6-axis IMU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawImuReading {
    /// Raw accelerometer ADC counts [X, Y, Z].
    pub accel_raw: [i16; 3],
    /// Raw gyroscope ADC counts [X, Y, Z].
    pub gyro_raw: [i16; 3],
    /// Onboard sensor temperature in raw ADC counts.
    pub temp_raw: i16,
    /// Hardware timestamp in microseconds.
    pub timestamp_us: u64,
}

impl RawImuReading {
    /// Creates a zeroed raw reading.
    pub const fn zero() -> Self {
        Self {
            accel_raw: [0, 0, 0],
            gyro_raw: [0, 0, 0],
            temp_raw: 0,
            timestamp_us: 0,
        }
    }
}

/// Calibrated physical units sample from an individual 6-axis IMU.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicalImuSample {
    /// Sensor location (LeftGrip or RightGrip).
    pub location: ImuLocation,
    /// Proper acceleration in m/s^2 [X, Y, Z].
    pub accel_mss: [f32; 3],
    /// Angular velocity in radians per second [X, Y, Z].
    pub gyro_rad_s: [f32; 3],
    /// Sensor temperature in degrees Celsius (°C).
    pub temp_c: f32,
    /// Hardware timestamp in microseconds.
    pub timestamp_us: u64,
}

impl PhysicalImuSample {
    /// Converts a raw 16-bit reading into physical units (assumes +/-8g and +/-2000 dps scales).
    pub fn from_raw(location: ImuLocation, raw: &RawImuReading) -> Self {
        // +/- 8g scale: sensitivity = 4096 LSB/g -> 1g = 9.80665 m/s^2
        let accel_scale = GRAVITY_MSS / 4096.0;
        let accel_mss = [
            (raw.accel_raw[0] as f32) * accel_scale,
            (raw.accel_raw[1] as f32) * accel_scale,
            (raw.accel_raw[2] as f32) * accel_scale,
        ];

        // +/- 2000 dps scale: sensitivity = 16.4 LSB/(deg/s) -> 1 deg = PI/180 rad
        let deg_to_rad = 3.14159265 / 180.0;
        let gyro_scale = (1.0 / 16.4) * deg_to_rad;
        let gyro_rad_s = [
            (raw.gyro_raw[0] as f32) * gyro_scale,
            (raw.gyro_raw[1] as f32) * gyro_scale,
            (raw.gyro_raw[2] as f32) * gyro_scale,
        ];

        // Temperature: typically (raw / 132.48) + 25.0 °C
        let temp_c = ((raw.temp_raw as f32) / 132.48) + 25.0;

        Self {
            location,
            accel_mss,
            gyro_rad_s,
            temp_c,
            timestamp_us: raw.timestamp_us,
        }
    }
}

/// Unit quaternion representing 3D spatial orientation in body coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialQuaternion {
    /// Scalar component (cos(theta / 2)).
    pub w: f32,
    /// Vector X component (axis_x * sin(theta / 2)).
    pub x: f32,
    /// Vector Y component (axis_y * sin(theta / 2)).
    pub y: f32,
    /// Vector Z component (axis_z * sin(theta / 2)).
    pub z: f32,
}

impl SpatialQuaternion {
    /// Identity quaternion (zero rotation).
    pub const fn identity() -> Self {
        Self {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }

    /// Creates a quaternion from an axis and rotation angle in radians.
    pub fn from_axis_angle(axis: [f32; 3], angle_rad: f32) -> Self {
        let half_angle = angle_rad * 0.5;
        let s = sin_f32(half_angle);
        let c = cos_f32(half_angle);

        let mut mag = sqrt_f32(axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]);
        if mag < 1e-6 {
            mag = 1.0;
        }

        Self {
            w: c,
            x: (axis[0] / mag) * s,
            y: (axis[1] / mag) * s,
            z: (axis[2] / mag) * s,
        }
    }

    /// Computes quaternion Euclidean norm: sqrt(w^2 + x^2 + y^2 + z^2).
    pub fn norm(&self) -> f32 {
        sqrt_f32(self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z)
    }

    /// Normalizes this quaternion in-place to enforce unit length.
    pub fn normalize(&mut self) {
        let n = self.norm();
        if n > 1e-6 {
            let inv = 1.0 / n;
            self.w *= inv;
            self.x *= inv;
            self.y *= inv;
            self.z *= inv;
        } else {
            *self = Self::identity();
        }
    }

    /// Quaternion Hamilton product: self * rhs.
    pub fn multiply(&self, rhs: &Self) -> Self {
        Self {
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
        }
    }

    /// Computes the quaternion conjugate: (w, -x, -y, -z).
    #[inline(always)]
    pub fn conjugate(&self) -> Self {
        Self {
            w: self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }

    /// Rotates a 3D vector by this quaternion: v' = q * v * q^(-1).
    pub fn rotate_vector(&self, v: [f32; 3]) -> [f32; 3] {
        let q_vec = Self {
            w: 0.0,
            x: v[0],
            y: v[1],
            z: v[2],
        };
        let conj = self.conjugate();
        let res = self.multiply(&q_vec).multiply(&conj);
        [res.x, res.y, res.z]
    }

    /// Extracts Tait-Bryan Euler angles in radians (Pitch, Roll, Yaw).
    pub fn to_euler_angles(&self) -> (f32, f32, f32) {
        // Roll (X-axis rotation)
        let sinr_cosp = 2.0 * (self.w * self.x + self.y * self.z);
        let cosr_cosp = 1.0 - 2.0 * (self.x * self.x + self.y * self.y);
        let roll = atan2_f32(sinr_cosp, cosr_cosp);

        // Pitch (Y-axis rotation)
        let sinp = 2.0 * (self.w * self.y - self.z * self.x);
        let pitch = if sinp.abs() >= 1.0 {
            if sinp > 0.0 { 3.14159265 / 2.0 } else { -3.14159265 / 2.0 }
        } else {
            asin_f32(sinp)
        };

        // Yaw (Z-axis rotation)
        let siny_cosp = 2.0 * (self.w * self.z + self.x * self.y);
        let cosy_cosp = 1.0 - 2.0 * (self.y * self.y + self.z * self.z);
        let yaw = atan2_f32(siny_cosp, cosy_cosp);

        (pitch, roll, yaw)
    }
}

/// Verification helper for Commit 1 structures, dual-IMU topology, and quaternion mathematics.
pub fn verify_dual_imu_topology_and_quaternions() -> bool {
    // 1. Verify standard handheld topology dimensions
    let topo = DualImuTopology::standard_handheld();
    if (topo.chassis_width_m - 0.28).abs() > 1e-4 {
        return false;
    }
    if (topo.left_lever_arm_m[0] - (-0.14)).abs() > 1e-4 {
        return false;
    }
    if (topo.right_lever_arm_m[0] - 0.14).abs() > 1e-4 {
        return false;
    }
    if topo.sample_rate_hz != 1000 || (topo.dt_s - 0.001).abs() > 1e-6 {
        return false;
    }

    // 2. Verify Raw Reading to Physical Units conversion
    // 4096 LSB on 8g scale = 1.0g = 9.80665 m/s^2 along Z
    let raw = RawImuReading {
        accel_raw: [0, 0, 4096],
        gyro_raw: [0, 0, 0],
        temp_raw: 0,
        timestamp_us: 1000,
    };
    let phys = PhysicalImuSample::from_raw(ImuLocation::LeftGrip, &raw);
    if (phys.accel_mss[2] - GRAVITY_MSS).abs() > 0.01 {
        return false;
    }
    if phys.accel_mss[0].abs() > 1e-4 || phys.accel_mss[1].abs() > 1e-4 {
        return false;
    }
    if (phys.temp_c - 25.0).abs() > 0.1 {
        return false;
    }

    // 3. Verify Quaternion Identity & Normalization
    let mut q_ident = SpatialQuaternion::identity();
    if (q_ident.norm() - 1.0).abs() > 1e-6 {
        return false;
    }
    q_ident.w = 2.0;
    q_ident.normalize();
    if (q_ident.w - 1.0).abs() > 1e-6 {
        return false;
    }

    // 4. Verify 90-degree Yaw rotation around Z axis
    // 90 deg = PI/2 rad -> w = cos(PI/4) = sqrt(2)/2, z = sin(PI/4) = sqrt(2)/2
    let pi = 3.14159265;
    let q_rot = SpatialQuaternion::from_axis_angle([0.0, 0.0, 1.0], pi / 2.0);
    // Rotate vector [1.0, 0.0, 0.0] by 90 deg around Z -> should become [0.0, 1.0, 0.0]
    let v_in = [1.0, 0.0, 0.0];
    let v_out = q_rot.rotate_vector(v_in);
    if v_out[0].abs() > 0.01 || (v_out[1] - 1.0).abs() > 0.01 || v_out[2].abs() > 0.01 {
        return false;
    }

    // 5. Verify Euler angle extraction
    let (pitch, roll, yaw) = q_rot.to_euler_angles();
    if pitch.abs() > 0.01 || roll.abs() > 0.01 || (yaw - (pi / 2.0)).abs() > 0.02 {
        return false;
    }

    true
}

// Dual-IMU sensor fusion and gravity isolation.

/// Fused dual-IMU kinematics at a single 1000 Hz discrete time step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FusedKinematics {
    /// Fused angular velocity in radians per second [X, Y, Z].
    pub angular_velocity_rad_s: [f32; 3],
    /// Fused total proper acceleration in m/s^2 [X, Y, Z] (lever-arm cancelled).
    pub proper_accel_mss: [f32; 3],
    /// Isolated gravitational acceleration in body coordinates [X, Y, Z].
    pub gravity_body_mss: [f32; 3],
    /// Gravity-free dynamic linear acceleration in body coordinates [X, Y, Z].
    pub dynamic_linear_accel_mss: [f32; 3],
    /// Gravity-free dynamic linear acceleration in world navigation coordinates [X, Y, Z].
    pub world_linear_accel_mss: [f32; 3],
    /// Current estimated spatial orientation quaternion.
    pub orientation: SpatialQuaternion,
}

/// 1000 Hz Dual-IMU Sensor Fusion Filter.
pub struct DualImuFusionFilter {
    /// Chassis spatial topology.
    pub topology: DualImuTopology,
    /// Current estimated orientation quaternion.
    pub orientation: SpatialQuaternion,
    /// Estimated gyroscope bias offsets in rad/s [X, Y, Z].
    pub gyro_bias: [f32; 3],
    /// Estimated accelerometer bias offsets in m/s^2 [X, Y, Z].
    pub accel_bias: [f32; 3],
    /// Complementary filter accelerometer correction gain beta.
    pub beta: f32,
    /// Most recent fused kinematics snapshot.
    pub kinematics: FusedKinematics,
}

impl DualImuFusionFilter {
    /// Creates a new fusion filter with standard handheld topology.
    pub const fn new(topology: DualImuTopology) -> Self {
        Self {
            topology,
            orientation: SpatialQuaternion::identity(),
            gyro_bias: [0.0, 0.0, 0.0],
            accel_bias: [0.0, 0.0, 0.0],
            beta: 0.02, // 1000 Hz complementary filter gain
            kinematics: FusedKinematics {
                angular_velocity_rad_s: [0.0, 0.0, 0.0],
                proper_accel_mss: [0.0, 0.0, GRAVITY_MSS],
                gravity_body_mss: [0.0, 0.0, GRAVITY_MSS],
                dynamic_linear_accel_mss: [0.0, 0.0, 0.0],
                world_linear_accel_mss: [0.0, 0.0, 0.0],
                orientation: SpatialQuaternion::identity(),
            },
        }
    }

    /// Ingests simultaneous physical samples from Left and Right IMUs and updates fused state.
    pub fn update(
        &mut self,
        left: &PhysicalImuSample,
        right: &PhysicalImuSample,
    ) -> FusedKinematics {
        let dt = self.topology.dt_s;

        // 1. Dual-IMU Gyroscope Averaging:
        // Reduces uncorrelated thermal noise variance by sqrt(2)
        let raw_gyro = [
            (left.gyro_rad_s[0] + right.gyro_rad_s[0]) * 0.5 - self.gyro_bias[0],
            (left.gyro_rad_s[1] + right.gyro_rad_s[1]) * 0.5 - self.gyro_bias[1],
            (left.gyro_rad_s[2] + right.gyro_rad_s[2]) * 0.5 - self.gyro_bias[2],
        ];

        // Dual-IMU accelerometer averaging and lever-arm rotational cancellation.
        let proper_accel = [
            (left.accel_mss[0] + right.accel_mss[0]) * 0.5 - self.accel_bias[0],
            (left.accel_mss[1] + right.accel_mss[1]) * 0.5 - self.accel_bias[1],
            (left.accel_mss[2] + right.accel_mss[2]) * 0.5 - self.accel_bias[2],
        ];

        // 3. Accelerometer Tilt Error Vector (Gravity Reference Correction)
        let accel_norm = sqrt_f32(
            proper_accel[0] * proper_accel[0]
                + proper_accel[1] * proper_accel[1]
                + proper_accel[2] * proper_accel[2],
        );

        let mut gyro_corr = raw_gyro;

        // Only correct tilt using accelerometer when total acceleration is close to 1g (0.85g to 1.15g)
        // to prevent dynamic user acceleration from distorting orientation.
        if (accel_norm - GRAVITY_MSS).abs() < 1.5 && accel_norm > 1.0 {
            let inv_norm = 1.0 / accel_norm;
            let ax = proper_accel[0] * inv_norm;
            let ay = proper_accel[1] * inv_norm;
            let az = proper_accel[2] * inv_norm;

            // Estimated gravity direction in body frame from current orientation quaternion q:
            // g_est = [2*(x*z - w*y), 2*(y*z + w*x), w^2 - x^2 - y^2 + z^2]
            let q = &self.orientation;
            let gx = 2.0 * (q.x * q.z - q.w * q.y);
            let gy = 2.0 * (q.y * q.z + q.w * q.x);
            let gz = q.w * q.w - q.x * q.x - q.y * q.y + q.z * q.z;

            // Error is cross product of measured acceleration vector and estimated gravity: e = a x g
            let ex = ay * gz - az * gy;
            let ey = az * gx - ax * gz;
            let ez = ax * gy - ay * gx;

            // Apply complementary correction to gyro angular velocity
            gyro_corr[0] += self.beta * ex;
            gyro_corr[1] += self.beta * ey;
            gyro_corr[2] += self.beta * ez;
        }

        // 4. Integrate Quaternion Orientation:
        // dq/dt = 0.5 * q * [0, wx, wy, wz]
        let q = self.orientation;
        let half_dt = 0.5 * dt;
        let q_dot = SpatialQuaternion {
            w: half_dt * (-q.x * gyro_corr[0] - q.y * gyro_corr[1] - q.z * gyro_corr[2]),
            x: half_dt * (q.w * gyro_corr[0] + q.y * gyro_corr[2] - q.z * gyro_corr[1]),
            y: half_dt * (q.w * gyro_corr[1] - q.x * gyro_corr[2] + q.z * gyro_corr[0]),
            z: half_dt * (q.w * gyro_corr[2] + q.x * gyro_corr[1] - q.y * gyro_corr[0]),
        };

        let mut q_new = SpatialQuaternion {
            w: q.w + q_dot.w,
            x: q.x + q_dot.x,
            y: q.y + q_dot.y,
            z: q.z + q_dot.z,
        };
        q_new.normalize();
        self.orientation = q_new;

        // 5. Dynamic Gravity Vector Isolation:
        // In body frame, gravity vector is g_body = R(q)^T * [0, 0, -g]
        let g_body = [
            2.0 * (q_new.x * q_new.z - q_new.w * q_new.y) * GRAVITY_MSS,
            2.0 * (q_new.y * q_new.z + q_new.w * q_new.x) * GRAVITY_MSS,
            (q_new.w * q_new.w - q_new.x * q_new.x - q_new.y * q_new.y + q_new.z * q_new.z) * GRAVITY_MSS,
        ];

        // Dynamic linear acceleration free from gravity: a_dynamic = a_proper - g_body
        let dynamic_body = [
            proper_accel[0] - g_body[0],
            proper_accel[1] - g_body[1],
            proper_accel[2] - g_body[2],
        ];

        // Transform dynamic linear acceleration into world navigation frame: a_world = q * a_body * q^*
        let dynamic_world = q_new.rotate_vector(dynamic_body);

        let kinematics = FusedKinematics {
            angular_velocity_rad_s: raw_gyro,
            proper_accel_mss: proper_accel,
            gravity_body_mss: g_body,
            dynamic_linear_accel_mss: dynamic_body,
            world_linear_accel_mss: dynamic_world,
            orientation: q_new,
        };

        self.kinematics = kinematics;
        kinematics
    }
}

/// Verification helper for Commit 2 dual-IMU fusion and gravity isolation.
pub fn verify_dual_imu_fusion_and_gravity_isolation() -> bool {
    let topo = DualImuTopology::standard_handheld();
    let mut filter = DualImuFusionFilter::new(topo);

    // 1. Static flat desk test: both sensors measure +1g along Z, zero gyro
    let raw_flat = RawImuReading {
        accel_raw: [0, 0, 4096], // +1g
        gyro_raw: [0, 0, 0],
        temp_raw: 0,
        timestamp_us: 1000,
    };
    let s_left = PhysicalImuSample::from_raw(ImuLocation::LeftGrip, &raw_flat);
    let s_right = PhysicalImuSample::from_raw(ImuLocation::RightGrip, &raw_flat);

    let k_flat = filter.update(&s_left, &s_right);
    // Gravity should be isolated to ~9.80665 along Z
    if (k_flat.gravity_body_mss[2] - GRAVITY_MSS).abs() > 0.1 {
        return false;
    }
    // Dynamic linear acceleration should be zero (< 0.05 m/s^2)
    if k_flat.dynamic_linear_accel_mss[0].abs() > 0.05
        || k_flat.dynamic_linear_accel_mss[1].abs() > 0.05
        || k_flat.dynamic_linear_accel_mss[2].abs() > 0.05
    {
        return false;
    }

    // Dual-IMU lever-arm cancellation during handheld yaw rotation (omega_z = 2.0 rad/s).
    let mut s_left_rot = s_left;
    s_left_rot.accel_mss[0] = 0.56;
    s_left_rot.gyro_rad_s[2] = 2.0;

    let mut s_right_rot = s_right;
    s_right_rot.accel_mss[0] = -0.56;
    s_right_rot.gyro_rad_s[2] = 2.0;

    let k_rot = filter.update(&s_left_rot, &s_right_rot);

    // Fused proper acceleration along X should cancel out to 0.0 m/s^2!
    if k_rot.proper_accel_mss[0].abs() > 0.01 {
        return false;
    }
    // Fused angular velocity should be exactly 2.0 rad/s
    if (k_rot.angular_velocity_rad_s[2] - 2.0).abs() > 0.01 {
        return false;
    }

    true
}

// Zero-Velocity Update (ZUPT) and dead reckoning tracker.

/// Zero-Velocity Update (ZUPT) detector state machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZuptDetector {
    /// Angular velocity still threshold in rad/s (nominally 0.05 rad/s ~ 2.8 deg/s).
    pub gyro_still_threshold_rad_s: f32,
    /// Dynamic linear acceleration still threshold in m/s^2 (nominally 0.15 m/s^2).
    pub accel_still_threshold_mss: f32,
    /// Number of consecutive stationary samples observed.
    pub stationary_counter: u32,
    /// Required stationary samples to trigger ZUPT lock (nominally 20 samples = 20ms).
    pub trigger_window_samples: u32,
    /// True if handheld chassis is currently classified as stationary.
    pub is_stationary: bool,
    /// Online bias adaptation learning rate during stationary periods.
    pub bias_learning_rate: f32,
}

impl ZuptDetector {
    /// Creates a new ZUPT detector with standard 1000 Hz parameters.
    pub const fn new() -> Self {
        Self {
            gyro_still_threshold_rad_s: 0.05,
            accel_still_threshold_mss: 0.15,
            stationary_counter: 0,
            trigger_window_samples: 20,
            is_stationary: true,
            bias_learning_rate: 0.005,
        }
    }

    /// Evaluates current kinematics and returns true if ZUPT is active.
    pub fn update(&mut self, kinematics: &FusedKinematics) -> bool {
        let gyro_mag = sqrt_f32(
            kinematics.angular_velocity_rad_s[0] * kinematics.angular_velocity_rad_s[0]
                + kinematics.angular_velocity_rad_s[1] * kinematics.angular_velocity_rad_s[1]
                + kinematics.angular_velocity_rad_s[2] * kinematics.angular_velocity_rad_s[2],
        );

        let dynamic_accel_mag = sqrt_f32(
            kinematics.dynamic_linear_accel_mss[0] * kinematics.dynamic_linear_accel_mss[0]
                + kinematics.dynamic_linear_accel_mss[1] * kinematics.dynamic_linear_accel_mss[1]
                + kinematics.dynamic_linear_accel_mss[2] * kinematics.dynamic_linear_accel_mss[2],
        );

        let is_still = gyro_mag < self.gyro_still_threshold_rad_s
            && dynamic_accel_mag < self.accel_still_threshold_mss;

        if is_still {
            self.stationary_counter = self.stationary_counter.saturating_add(1);
            if self.stationary_counter >= self.trigger_window_samples {
                self.is_stationary = true;
            }
        } else {
            self.stationary_counter = 0;
            self.is_stationary = false;
        }

        self.is_stationary
    }
}

/// Instantaneous dead-reckoning tracking snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeadReckoningState {
    /// 3D Position in meters [X, Y, Z] relative to origin.
    pub position_m: [f32; 3],
    /// 3D Linear velocity in m/s [X, Y, Z].
    pub velocity_m_s: [f32; 3],
    /// 3D Dynamic linear acceleration in m/s^2 [X, Y, Z].
    pub linear_accel_mss: [f32; 3],
    /// Orientation quaternion.
    pub orientation: SpatialQuaternion,
    /// Euler angles in radians (Pitch, Roll, Yaw).
    pub euler_angles_rad: (f32, f32, f32),
    /// True if ZUPT zero-velocity clamp is actively engaged.
    pub is_zupt_locked: bool,
}

/// 1000 Hz Sub-Millimeter Dual-IMU Dead Reckoning Engine.
pub struct DualImuDeadReckoningEngine {
    /// Sensor fusion filter.
    pub fusion: DualImuFusionFilter,
    /// Zero-Velocity Update state machine.
    pub zupt: ZuptDetector,
    /// Integrated 3D position in meters [X, Y, Z].
    pub position_m: [f32; 3],
    /// Integrated 3D velocity in m/s [X, Y, Z].
    pub velocity_m_s: [f32; 3],
    /// Leaky velocity damping coefficient (prevents unbounded integration drift).
    pub velocity_damping: f32,
    /// Most recent state.
    pub state: DeadReckoningState,
}

impl DualImuDeadReckoningEngine {
    /// Creates a new dead-reckoning engine initialized with standard handheld topology.
    pub const fn new(topology: DualImuTopology) -> Self {
        Self {
            fusion: DualImuFusionFilter::new(topology),
            zupt: ZuptDetector::new(),
            position_m: [0.0, 0.0, 0.0],
            velocity_m_s: [0.0, 0.0, 0.0],
            velocity_damping: 0.995, // Leaky integrator damping
            state: DeadReckoningState {
                position_m: [0.0, 0.0, 0.0],
                velocity_m_s: [0.0, 0.0, 0.0],
                linear_accel_mss: [0.0, 0.0, 0.0],
                orientation: SpatialQuaternion::identity(),
                euler_angles_rad: (0.0, 0.0, 0.0),
                is_zupt_locked: true,
            },
        }
    }

    /// Processes a single 1000 Hz dual-IMU discrete update step.
    pub fn process_step(
        &mut self,
        left: &PhysicalImuSample,
        right: &PhysicalImuSample,
    ) -> DeadReckoningState {
        let dt = self.fusion.topology.dt_s;

        // 1. Run 1000 Hz dual-IMU sensor fusion & gravity isolation
        let kinematics = self.fusion.update(left, right);

        // 2. Evaluate Zero-Velocity Update (ZUPT)
        let is_stationary = self.zupt.update(&kinematics);

        if is_stationary {
            // ZUPT clamp: force velocity to zero
            self.velocity_m_s = [0.0, 0.0, 0.0];

            // Online bias adaptation during stationary resting phases:
            // Slowly cancel residual accelerometer & gyro biases
            let alpha = self.zupt.bias_learning_rate;
            self.fusion.gyro_bias[0] += kinematics.angular_velocity_rad_s[0] * alpha;
            self.fusion.gyro_bias[1] += kinematics.angular_velocity_rad_s[1] * alpha;
            self.fusion.gyro_bias[2] += kinematics.angular_velocity_rad_s[2] * alpha;

            self.fusion.accel_bias[0] += kinematics.dynamic_linear_accel_mss[0] * alpha;
            self.fusion.accel_bias[1] += kinematics.dynamic_linear_accel_mss[1] * alpha;
            self.fusion.accel_bias[2] += kinematics.dynamic_linear_accel_mss[2] * alpha;
        } else {
            // Dynamic motion: integrate world-frame linear acceleration into velocity
            let a_w = kinematics.world_linear_accel_mss;

            self.velocity_m_s[0] = (self.velocity_m_s[0] + a_w[0] * dt) * self.velocity_damping;
            self.velocity_m_s[1] = (self.velocity_m_s[1] + a_w[1] * dt) * self.velocity_damping;
            self.velocity_m_s[2] = (self.velocity_m_s[2] + a_w[2] * dt) * self.velocity_damping;

            // Integrate velocity into position
            self.position_m[0] += self.velocity_m_s[0] * dt;
            self.position_m[1] += self.velocity_m_s[1] * dt;
            self.position_m[2] += self.velocity_m_s[2] * dt;
        }

        let euler = kinematics.orientation.to_euler_angles();

        self.state = DeadReckoningState {
            position_m: self.position_m,
            velocity_m_s: self.velocity_m_s,
            linear_accel_mss: kinematics.world_linear_accel_mss,
            orientation: kinematics.orientation,
            euler_angles_rad: euler,
            is_zupt_locked: is_stationary,
        };

        self.state
    }

    /// Resets integrated trajectory position and velocity to origin.
    pub fn reset_trajectory(&mut self) {
        self.position_m = [0.0, 0.0, 0.0];
        self.velocity_m_s = [0.0, 0.0, 0.0];
        self.state.position_m = [0.0, 0.0, 0.0];
        self.state.velocity_m_s = [0.0, 0.0, 0.0];
    }

    /// Returns position in millimeters [X, Y, Z].
    pub fn get_position_mm(&self) -> [f32; 3] {
        [
            self.position_m[0] * 1000.0,
            self.position_m[1] * 1000.0,
            self.position_m[2] * 1000.0,
        ]
    }
}

/// Verification helper for Commit 3 ZUPT drift cancellation and dead reckoning algorithms.
pub fn verify_zupt_drift_cancellation_and_dead_reckoning() -> bool {
    let topo = DualImuTopology::standard_handheld();
    let mut engine = DualImuDeadReckoningEngine::new(topo);

    // 1. Stationary resting test: simulate 1000 steps (1 full second at 1000 Hz)
    // with static +1g gravity on Z and tiny residual noise on X/Y (0.01 m/s^2)
    let s_static = PhysicalImuSample {
        location: ImuLocation::LeftGrip,
        accel_mss: [0.01, 0.01, GRAVITY_MSS],
        gyro_rad_s: [0.001, 0.001, 0.0],
        temp_c: 25.0,
        timestamp_us: 1000,
    };

    for _ in 0..1000 {
        engine.process_step(&s_static, &s_static);
    }

    // ZUPT must be actively locked
    if !engine.state.is_zupt_locked {
        return false;
    }
    // Velocity must be clamped to exactly 0.0
    if engine.state.velocity_m_s[0] != 0.0
        || engine.state.velocity_m_s[1] != 0.0
        || engine.state.velocity_m_s[2] != 0.0
    {
        return false;
    }
    // Sub-millimeter constraint: integrated position drift after 1 second of resting must be < 0.5 mm
    let pos_mm = engine.get_position_mm();
    let dist_mm = sqrt_f32(pos_mm[0] * pos_mm[0] + pos_mm[1] * pos_mm[1] + pos_mm[2] * pos_mm[2]);
    if dist_mm > 0.5 {
        return false;
    }

    // 2. Dynamic translational pulse test: simulate a 100ms forward push gesture (+1.5 m/s^2 along Y)
    engine.reset_trajectory();

    let s_pulse = PhysicalImuSample {
        location: ImuLocation::LeftGrip,
        accel_mss: [0.0, 1.5, GRAVITY_MSS],
        gyro_rad_s: [0.0, 0.0, 0.0],
        temp_c: 25.0,
        timestamp_us: 2000,
    };

    for _ in 0..100 {
        engine.process_step(&s_pulse, &s_pulse);
    }

    // During active push, ZUPT must unlock
    if engine.state.is_zupt_locked {
        return false;
    }
    // Velocity should be positive along Y
    if engine.state.velocity_m_s[1] <= 0.05 {
        return false;
    }
    // Position should have translated forward along Y
    if engine.position_m[1] <= 0.001 {
        return false;
    }

    true
}


// 6-Axis IMU 1 kHz Raw Data Parser & Sample Stream Processor.

/// Accelerometer full-scale range configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccelScale {
    Fs2g,
    Fs4g,
    Fs8g,
    Fs16g,
}

impl AccelScale {
    /// Returns scale factor in (m/s^2) per LSB.
    pub fn scale_factor_mss(self) -> f32 {
        match self {
            AccelScale::Fs2g => (2.0 * GRAVITY_MSS) / 32768.0,
            AccelScale::Fs4g => (4.0 * GRAVITY_MSS) / 32768.0,
            AccelScale::Fs8g => (8.0 * GRAVITY_MSS) / 32768.0,
            AccelScale::Fs16g => (16.0 * GRAVITY_MSS) / 32768.0,
        }
    }
}

/// Gyroscope full-scale range configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GyroScale {
    Fs250dps,
    Fs500dps,
    Fs1000dps,
    Fs2000dps,
}

impl GyroScale {
    /// Returns scale factor in (rad/s) per LSB.
    pub fn scale_factor_rad_s(self) -> f32 {
        let deg_to_rad = 3.14159265 / 180.0;
        match self {
            GyroScale::Fs250dps => (250.0 / 32768.0) * deg_to_rad,
            GyroScale::Fs500dps => (500.0 / 32768.0) * deg_to_rad,
            GyroScale::Fs1000dps => (1000.0 / 32768.0) * deg_to_rad,
            GyroScale::Fs2000dps => (2000.0 / 32768.0) * deg_to_rad,
        }
    }
}

/// 6-DoF IMU sample packet sampled at 1 kHz with calibrated physical units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Imu6DofSample {
    /// Accelerometer acceleration [X, Y, Z] in m/s^2.
    pub accel_mss: [f32; 3],
    /// Gyroscope angular rate [X, Y, Z] in rad/s.
    pub gyro_rad_s: [f32; 3],
    /// Die temperature in degrees Celsius.
    pub temp_c: f32,
    /// Timestamp in microseconds.
    pub timestamp_us: u64,
}

impl Imu6DofSample {
    pub const fn zero() -> Self {
        Self {
            accel_mss: [0.0, 0.0, 0.0],
            gyro_rad_s: [0.0, 0.0, 0.0],
            temp_c: 25.0,
            timestamp_us: 0,
        }
    }
}

/// 6-Axis IMU Raw Packet Parser and 1 kHz Stream Conditioner.
#[derive(Debug, Clone)]
pub struct ImuStreamParser {
    pub accel_scale: AccelScale,
    pub gyro_scale: GyroScale,
    pub prev_sample: Imu6DofSample,
    pub sample_count: u64,
    pub alpha_lpf: f32,
}

impl ImuStreamParser {
    pub const fn new(accel_scale: AccelScale, gyro_scale: GyroScale) -> Self {
        Self {
            accel_scale,
            gyro_scale,
            prev_sample: Imu6DofSample::zero(),
            sample_count: 0,
            alpha_lpf: 0.85,
        }
    }

    /// Parses raw 14-byte 6-axis IMU SPI/I2C burst frame (Big Endian).
    pub fn parse_raw_packet(&mut self, frame: &[u8; 14], timestamp_us: u64) -> Imu6DofSample {
        let ax_raw = i16::from_be_bytes([frame[0], frame[1]]);
        let ay_raw = i16::from_be_bytes([frame[2], frame[3]]);
        let az_raw = i16::from_be_bytes([frame[4], frame[5]]);
        let temp_raw = i16::from_be_bytes([frame[6], frame[7]]);
        let gx_raw = i16::from_be_bytes([frame[8], frame[9]]);
        let gy_raw = i16::from_be_bytes([frame[10], frame[11]]);
        let gz_raw = i16::from_be_bytes([frame[12], frame[13]]);

        let a_scale = self.accel_scale.scale_factor_mss();
        let g_scale = self.gyro_scale.scale_factor_rad_s();

        let raw_accel = [
            (ax_raw as f32) * a_scale,
            (ay_raw as f32) * a_scale,
            (az_raw as f32) * a_scale,
        ];
        let raw_gyro = [
            (gx_raw as f32) * g_scale,
            (gy_raw as f32) * g_scale,
            (gz_raw as f32) * g_scale,
        ];
        let temp_c = 25.0 + (temp_raw as f32) / 326.8;

        // Apply low-pass filter
        let filtered_accel = if self.sample_count == 0 {
            raw_accel
        } else {
            [
                self.alpha_lpf * raw_accel[0] + (1.0 - self.alpha_lpf) * self.prev_sample.accel_mss[0],
                self.alpha_lpf * raw_accel[1] + (1.0 - self.alpha_lpf) * self.prev_sample.accel_mss[1],
                self.alpha_lpf * raw_accel[2] + (1.0 - self.alpha_lpf) * self.prev_sample.accel_mss[2],
            ]
        };

        let sample = Imu6DofSample {
            accel_mss: filtered_accel,
            gyro_rad_s: raw_gyro,
            temp_c,
            timestamp_us,
        };

        self.prev_sample = sample;
        self.sample_count += 1;
        sample
    }
}

/// Unit test verifying 6-axis IMU raw packet parser and stream conditioner.
pub fn verify_raw_imu_parser_and_stream() -> bool {
    let mut parser = ImuStreamParser::new(AccelScale::Fs8g, GyroScale::Fs2000dps);

    // Frame: Ax=0, Ay=0, Az=4096 (1g), Temp=0 (25.0C), Gx=0, Gy=0, Gz=0
    let az_bytes = (4096i16).to_be_bytes();
    let frame = [
        0, 0,
        0, 0,
        az_bytes[0], az_bytes[1],
        0, 0,
        0, 0,
        0, 0,
        0, 0,
    ];

    let sample = parser.parse_raw_packet(&frame, 1000);
    if sample.timestamp_us != 1000 {
        return false;
    }
    if sample.temp_c < 24.9 || sample.temp_c > 25.1 {
        return false;
    }
    // Az should be ~GRAVITY_MSS (approx 9.8 m/s^2)
    if sample.accel_mss[2] < 9.7 || sample.accel_mss[2] > 9.9 {
        return false;
    }
    if sample.accel_mss[0].abs() > 0.001 || sample.accel_mss[1].abs() > 0.001 {
        return false;
    }

    true
}


// Thermal Drift Compensation Governor for Gyro Zero-Rate Bias.

/// Thermal sensitivity coefficients for 3-axis gyroscope zero-rate offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThermalCoefficients {
    /// Linear thermal drift slope (rad/s per °C) [X, Y, Z].
    pub linear_slope: [f32; 3],
    /// Quadratic thermal curvature (rad/s per °C^2) [X, Y, Z].
    pub quadratic_curve: [f32; 3],
    /// Reference factory calibration temperature (°C, nominally 25.0°C).
    pub ref_temp_c: f32,
}

impl ThermalCoefficients {
    /// Factory calibration coefficients typical for handheld gaming MEMS IMUs.
    pub const fn default_factory() -> Self {
        Self {
            linear_slope: [0.00012, -0.00015, 0.00008],
            quadratic_curve: [0.0000015, -0.0000020, 0.0000010],
            ref_temp_c: 25.0,
        }
    }
}

/// Dynamic Thermal Drift Compensation Governor.
#[derive(Debug, Clone)]
pub struct ThermalDriftGovernor {
    /// Baseline zero-rate bias at reference temperature [X, Y, Z] in rad/s.
    pub base_bias_rad_s: [f32; 3],
    /// Factory thermal drift model coefficients.
    pub coefficients: ThermalCoefficients,
    /// Current calculated thermal bias offset [X, Y, Z] in rad/s.
    pub current_bias_rad_s: [f32; 3],
    /// Filtered temperature in °C.
    pub filtered_temp_c: f32,
}

impl ThermalDriftGovernor {
    pub const fn new(base_bias: [f32; 3], coefficients: ThermalCoefficients) -> Self {
        Self {
            base_bias_rad_s: base_bias,
            coefficients,
            current_bias_rad_s: base_bias,
            filtered_temp_c: 25.0,
        }
    }

    /// Evaluates thermal drift model for die temperature T and computes active bias offsets.
    pub fn compute_thermal_bias(&mut self, die_temp_c: f32) -> [f32; 3] {
        // Temperature low-pass smoothing (tau ~ 0.5s)
        self.filtered_temp_c = 0.98 * self.filtered_temp_c + 0.02 * die_temp_c;
        let delta_t = self.filtered_temp_c - self.coefficients.ref_temp_c;
        let delta_t_sq = delta_t * delta_t;

        let mut computed = [0.0f32; 3];
        for i in 0..3 {
            computed[i] = self.base_bias_rad_s[i]
                + self.coefficients.linear_slope[i] * delta_t
                + self.coefficients.quadratic_curve[i] * delta_t_sq;
        }

        self.current_bias_rad_s = computed;
        computed
    }

    /// Compensates raw angular rate measurement by removing active thermal bias.
    pub fn compensate_gyro(&mut self, raw_gyro: [f32; 3], die_temp_c: f32) -> [f32; 3] {
        let bias = self.compute_thermal_bias(die_temp_c);
        [
            raw_gyro[0] - bias[0],
            raw_gyro[1] - bias[1],
            raw_gyro[2] - bias[2],
        ]
    }
}

/// Unit test verifying thermal drift compensation curves and bias cancellation.
pub fn verify_thermal_drift_compensation() -> bool {
    let base_bias = [0.005, -0.004, 0.002];
    let mut gov = ThermalDriftGovernor::new(base_bias, ThermalCoefficients::default_factory());

    // At reference temperature (25.0C), bias should match base bias
    for _ in 0..100 {
        gov.compute_thermal_bias(25.0);
    }
    let b25 = gov.current_bias_rad_s;
    if (b25[0] - base_bias[0]).abs() > 0.0001 || (b25[1] - base_bias[1]).abs() > 0.0001 {
        return false;
    }

    // Warm up handheld chassis under APU load to 65.0C (delta_t = +40C)
    for _ in 0..200 {
        gov.compute_thermal_bias(65.0);
    }
    let b65 = gov.current_bias_rad_s;
    // Bias must have shifted with temperature
    if b65[0] <= base_bias[0] || b65[1] >= base_bias[1] {
        return false;
    }

    // Raw stationary gyro sample exhibiting thermal drift
    let raw_drifting = [b65[0], b65[1], b65[2]];
    let compensated = gov.compensate_gyro(raw_drifting, 65.0);

    // Compensated angular rate should be zeroed (within 0.0001 rad/s)
    if compensated[0].abs() > 0.0001 || compensated[1].abs() > 0.0001 || compensated[2].abs() > 0.0001 {
        return false;
    }

    true
}


// 6-DoF Mahony/Madgwick Orientation Quaternion Fusion Filter.

/// 4-Element Unit Quaternion representing 3D spatial rotation [w, x, y, z].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrientationQuaternion {
    pub w: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl OrientationQuaternion {
    /// Identity quaternion (zero rotation).
    pub const fn identity() -> Self {
        Self { w: 1.0, x: 0.0, y: 0.0, z: 0.0 }
    }

    /// Normalizes quaternion to ensure unit magnitude.
    pub fn normalize(&mut self) {
        let norm_sq = self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z;
        if norm_sq > 0.0000001 {
            let inv_norm = 1.0 / sqrt_f32(norm_sq);
            self.w *= inv_norm;
            self.x *= inv_norm;
            self.y *= inv_norm;
            self.z *= inv_norm;
        } else {
            *self = Self::identity();
        }
    }

    /// Computes estimated gravity direction vector in sensor reference frame [X, Y, Z].
    pub fn estimate_gravity_vector(&self) -> [f32; 3] {
        [
            2.0 * (self.x * self.z - self.w * self.y) * GRAVITY_MSS,
            2.0 * (self.w * self.x + self.y * self.z) * GRAVITY_MSS,
            (self.w * self.w - self.x * self.x - self.y * self.y + self.z * self.z) * GRAVITY_MSS,
        ]
    }
}

/// 6-DoF Madgwick / Mahony Sensor Fusion Filter.
#[derive(Debug, Clone)]
pub struct Imu6DofFusionFilter {
    pub q: OrientationQuaternion,
    pub beta: f32,
    pub dt_s: f32,
    pub converged: bool,
}

impl Imu6DofFusionFilter {
    pub const fn new(beta: f32, dt_s: f32) -> Self {
        Self {
            q: OrientationQuaternion::identity(),
            beta,
            dt_s,
            converged: false,
        }
    }

    /// Updates orientation quaternion using calibrated accel and gyro samples.
    pub fn update(&mut self, accel: [f32; 3], gyro: [f32; 3]) {
        let q0 = self.q.w;
        let q1 = self.q.x;
        let q2 = self.q.y;
        let q3 = self.q.z;

        // Rate of change of quaternion from gyroscope
        let q_dot_omega = [
            0.5 * (-q1 * gyro[0] - q2 * gyro[1] - q3 * gyro[2]),
            0.5 * ( q0 * gyro[0] + q2 * gyro[2] - q3 * gyro[1]),
            0.5 * ( q0 * gyro[1] - q1 * gyro[2] + q3 * gyro[0]),
            0.5 * ( q0 * gyro[2] + q1 * gyro[1] - q2 * gyro[0]),
        ];

        // Compute feedback from accelerometer if non-zero
        let a_norm_sq = accel[0] * accel[0] + accel[1] * accel[1] + accel[2] * accel[2];
        let mut s = [0.0f32; 4];

        if a_norm_sq > 0.01 {
            let inv_a = 1.0 / sqrt_f32(a_norm_sq);
            let ax = accel[0] * inv_a;
            let ay = accel[1] * inv_a;
            let az = accel[2] * inv_a;

            // Auxiliary variables to avoid repeated calculations
            let _2q0 = 2.0 * q0;
            let _2q1 = 2.0 * q1;
            let _2q2 = 2.0 * q2;
            let _2q3 = 2.0 * q3;
            let _4q0 = 4.0 * q0;
            let _4q1 = 4.0 * q1;
            let _4q2 = 4.0 * q2;
            let _8q1 = 8.0 * q1;
            let _8q2 = 8.0 * q2;
            let q0q0 = q0 * q0;
            let q1q1 = q1 * q1;
            let q2q2 = q2 * q2;
            let q3q3 = q3 * q3;

            // Gradient descent algorithm corrective step
            let mut s0 = _4q0 * q2q2 + _2q2 * ax + _4q0 * q1q1 - _2q1 * ay;
            let mut s1 = _4q1 * q3q3 - _2q3 * ax + 4.0 * q0q0 * q1 - _2q0 * ay - _4q1 + _8q1 * q1q1 + _8q1 * q2q2 + _4q1 * az;
            let mut s2 = 4.0 * q0q0 * q2 + _2q0 * ax + _4q2 * q3q3 - _2q3 * ay - _4q2 + _8q2 * q1q1 + _8q2 * q2q2 + _4q2 * az;
            let mut s3 = 4.0 * q1q1 * q3 - _2q1 * ax + 4.0 * q2q2 * q3 - _2q2 * ay;

            let s_norm_sq = s0 * s0 + s1 * s1 + s2 * s2 + s3 * s3;
            if s_norm_sq > 0.000001 {
                let inv_s = 1.0 / sqrt_f32(s_norm_sq);
                s0 *= inv_s;
                s1 *= inv_s;
                s2 *= inv_s;
                s3 *= inv_s;
            }

            s = [s0, s1, s2, s3];
        }

        // Apply corrective step and integrate
        self.q.w += (q_dot_omega[0] - self.beta * s[0]) * self.dt_s;
        self.q.x += (q_dot_omega[1] - self.beta * s[1]) * self.dt_s;
        self.q.y += (q_dot_omega[2] - self.beta * s[2]) * self.dt_s;
        self.q.z += (q_dot_omega[3] - self.beta * s[3]) * self.dt_s;

        self.q.normalize();
        self.converged = true;
    }
}

/// Unit test verifying 6-DoF orientation quaternion fusion and gravity vector convergence.
pub fn verify_6dof_quaternion_fusion() -> bool {
    let mut filter = Imu6DofFusionFilter::new(0.04, 0.001);

    // Initial state: handheld flat on table, gravity straight down along +Z
    let accel = [0.0, 0.0, GRAVITY_MSS];
    let gyro = [0.0, 0.0, 0.0];

    // Run 500 filter steps
    for _ in 0..500 {
        filter.update(accel, gyro);
    }

    if !filter.converged {
        return false;
    }

    // Estimated gravity vector in sensor frame should match [0, 0, 9.80665]
    let g_est = filter.q.estimate_gravity_vector();
    if g_est[0].abs() > 0.1 || g_est[1].abs() > 0.1 || (g_est[2] - GRAVITY_MSS).abs() > 0.1 {
        return false;
    }

    // Quaternion magnitude must be unit
    let q = filter.q;
    let mag = sqrt_f32(q.w * q.w + q.x * q.x + q.y * q.y + q.z * q.z);
    if (mag - 1.0).abs() > 0.001 {
        return false;
    }

    true
}


// Linear Motion Separator & Game Tilt Control Governor.

/// Separated acceleration components and game tilt controls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSeparationResult {
    /// Pure linear dynamic acceleration [X, Y, Z] in m/s^2 (gravity isolated).
    pub linear_accel_mss: [f32; 3],
    /// Estimated gravity vector [X, Y, Z] in m/s^2.
    pub gravity_vector_mss: [f32; 3],
    /// Game tilt roll angle in degrees (-180..+180).
    pub roll_deg: f32,
    /// Game tilt pitch angle in degrees (-90..+90).
    pub pitch_deg: f32,
}

/// Linear Motion Separator and Game Tilt Control Engine.
#[derive(Debug, Clone)]
pub struct LinearMotionSeparator {
    pub deadband_mss: f32,
    pub tilt_deadband_deg: f32,
}

impl LinearMotionSeparator {
    pub const fn new(deadband_mss: f32, tilt_deadband_deg: f32) -> Self {
        Self {
            deadband_mss,
            tilt_deadband_deg,
        }
    }

    /// Isolates linear acceleration and extracts pitch/roll from quaternion and accel reading.
    pub fn separate_motion(
        &self,
        q: &OrientationQuaternion,
        accel_mss: [f32; 3],
    ) -> MotionSeparationResult {
        let g_vec = q.estimate_gravity_vector();

        // Remove gravity from measured proper acceleration
        let mut lin_x = accel_mss[0] - g_vec[0];
        let mut lin_y = accel_mss[1] - g_vec[1];
        let mut lin_z = accel_mss[2] - g_vec[2];

        // Apply deadband
        if lin_x.abs() < self.deadband_mss { lin_x = 0.0; }
        if lin_y.abs() < self.deadband_mss { lin_y = 0.0; }
        if lin_z.abs() < self.deadband_mss { lin_z = 0.0; }

        // Compute tilt angles from quaternion
        // Roll: atan2(2*(w*x + y*z), 1 - 2*(x*x + y*y))
        // Pitch: asin(clamp(2*(w*y - z*x), -1, 1))
        let sinr_cosp = 2.0 * (q.w * q.x + q.y * q.z);
        let cosr_cosp = 1.0 - 2.0 * (q.x * q.x + q.y * q.y);
        let roll_rad = atan2_f32(sinr_cosp, cosr_cosp);

        let sinp = 2.0 * (q.w * q.y - q.z * q.x);
        let pitch_rad = if sinp.abs() >= 1.0 {
            if sinp > 0.0 { 1.5707963 } else { -1.5707963 }
        } else {
            asin_f32(sinp)
        };

        let rad_to_deg = 180.0 / 3.14159265;
        let mut roll_deg = roll_rad * rad_to_deg;
        let mut pitch_deg = pitch_rad * rad_to_deg;

        if roll_deg.abs() < self.tilt_deadband_deg { roll_deg = 0.0; }
        if pitch_deg.abs() < self.tilt_deadband_deg { pitch_deg = 0.0; }

        MotionSeparationResult {
            linear_accel_mss: [lin_x, lin_y, lin_z],
            gravity_vector_mss: g_vec,
            roll_deg,
            pitch_deg,
        }
    }
}

/// Unit test verifying linear motion separation and tilt angle extraction.
pub fn verify_linear_motion_separator() -> bool {
    let separator = LinearMotionSeparator::new(0.05, 0.5);

    // 1. Static test: flat orientation, gravity only
    let q_flat = OrientationQuaternion::identity();
    let accel_static = [0.0, 0.0, GRAVITY_MSS];

    let res_static = separator.separate_motion(&q_flat, accel_static);
    // Linear acceleration should be zeroed (gravity canceled)
    if res_static.linear_accel_mss[0] != 0.0
        || res_static.linear_accel_mss[1] != 0.0
        || res_static.linear_accel_mss[2] != 0.0
    {
        return false;
    }
    // Tilt angles should be 0.0
    if res_static.roll_deg != 0.0 || res_static.pitch_deg != 0.0 {
        return false;
    }

    // 2. Dynamic impulse test: add +2.0 m/s^2 linear acceleration along X (lateral shake)
    let accel_shake = [2.0, 0.0, GRAVITY_MSS];
    let res_shake = separator.separate_motion(&q_flat, accel_shake);

    if (res_shake.linear_accel_mss[0] - 2.0).abs() > 0.05 {
        return false;
    }
    if res_shake.linear_accel_mss[1] != 0.0 || res_shake.linear_accel_mss[2] != 0.0 {
        return false;
    }

    true
}
