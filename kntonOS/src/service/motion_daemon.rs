//! SteamDeck Gyro & Accelerometer Motion Sensor Service (TASK-KNT-334).
//!
//! Provides dual Bosch BMI160 IMU 6-axis motion sensing, complementary filter
//! orientation fusion, sensor thermal bias auto-calibration, gyro-to-mouse /
//! right-stick aim assist with capacitive touch gating, and drop/free-fall emergency detection.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

// ============================================================================
// Feature 1: Dual Bosch BMI160 IMU 6-Axis Hardware Driver
// ============================================================================

/// IMU channel location on SteamDeck chassis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImuChannel {
    /// Primary IMU mounted near motherboard center of mass.
    PrimaryChassis = 0,
    /// Secondary reference IMU for differential vibration cancellation.
    SecondaryDisplay = 1,
}

impl ImuChannel {
    pub const COUNT: usize = 2;

    pub const fn index(&self) -> usize {
        *self as usize
    }
}

/// IMU operating power mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImuPowerMode {
    Suspend,
    LowPower,
    Normal,
    FastStartup,
}

/// Raw 16-bit 6-axis motion sensor readings.
#[derive(Debug, Clone, Copy, Default)]
pub struct RawImuFrame {
    pub accel_x: i16,
    pub accel_y: i16,
    pub accel_z: i16,
    pub gyro_x: i16,
    pub gyro_y: i16,
    pub gyro_z: i16,
    pub sensor_temp_raw: i16,
    pub timestamp_us: u64,
}

/// Physical unit motion sample (milli-g for accel, millidegrees/sec for gyro).
#[derive(Debug, Clone, Copy, Default)]
pub struct MotionSample {
    /// Linear acceleration in milli-g (1000 mg = 1.0g = 9.81 m/s^2).
    pub accel_mg: [i32; 3],
    /// Angular velocity in millidegrees per second (1000 mdeg/s = 1.0 °/s).
    pub gyro_mdeg_s: [i32; 3],
    /// Sensor silicon temperature in millidegrees Celsius.
    pub temp_mdeg_c: i32,
    /// Sample timestamp in microseconds.
    pub timestamp_us: u64,
}

/// Atomic storage for live motion telemetry.
pub struct AtomicMotionChannel {
    accel_x_mg: AtomicI32,
    accel_y_mg: AtomicI32,
    accel_z_mg: AtomicI32,
    gyro_x_mdeg_s: AtomicI32,
    gyro_y_mdeg_s: AtomicI32,
    gyro_z_mdeg_s: AtomicI32,
    temp_mdeg_c: AtomicI32,
    sample_count: AtomicU32,
    last_timestamp_us: AtomicU64,
}

impl AtomicMotionChannel {
    pub const fn new() -> Self {
        Self {
            accel_x_mg: AtomicI32::new(0),
            accel_y_mg: AtomicI32::new(0),
            accel_z_mg: AtomicI32::new(1000), // Default 1.0g gravity downward
            gyro_x_mdeg_s: AtomicI32::new(0),
            gyro_y_mdeg_s: AtomicI32::new(0),
            gyro_z_mdeg_s: AtomicI32::new(0),
            temp_mdeg_c: AtomicI32::new(35_000),
            sample_count: AtomicU32::new(0),
            last_timestamp_us: AtomicU64::new(0),
        }
    }

    pub fn store(&self, sample: &MotionSample) {
        self.accel_x_mg.store(sample.accel_mg[0], Ordering::Release);
        self.accel_y_mg.store(sample.accel_mg[1], Ordering::Release);
        self.accel_z_mg.store(sample.accel_mg[2], Ordering::Release);
        self.gyro_x_mdeg_s.store(sample.gyro_mdeg_s[0], Ordering::Release);
        self.gyro_y_mdeg_s.store(sample.gyro_mdeg_s[1], Ordering::Release);
        self.gyro_z_mdeg_s.store(sample.gyro_mdeg_s[2], Ordering::Release);
        self.temp_mdeg_c.store(sample.temp_mdeg_c, Ordering::Release);
        self.sample_count.fetch_add(1, Ordering::Relaxed);
        self.last_timestamp_us.store(sample.timestamp_us, Ordering::Release);
    }

    pub fn load(&self) -> MotionSample {
        MotionSample {
            accel_mg: [
                self.accel_x_mg.load(Ordering::Acquire),
                self.accel_y_mg.load(Ordering::Acquire),
                self.accel_z_mg.load(Ordering::Acquire),
            ],
            gyro_mdeg_s: [
                self.gyro_x_mdeg_s.load(Ordering::Acquire),
                self.gyro_y_mdeg_s.load(Ordering::Acquire),
                self.gyro_z_mdeg_s.load(Ordering::Acquire),
            ],
            temp_mdeg_c: self.temp_mdeg_c.load(Ordering::Acquire),
            timestamp_us: self.last_timestamp_us.load(Ordering::Acquire),
        }
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count.load(Ordering::Acquire)
    }
}

/// Dual Bosch BMI160 hardware IMU manager.
pub struct DualBmi160Driver {
    power_mode: AtomicU32, // ImuPowerMode as u32
    channels: [AtomicMotionChannel; ImuChannel::COUNT],
    is_initialized: AtomicBool,
}

impl DualBmi160Driver {
    pub const fn new() -> Self {
        Self {
            power_mode: AtomicU32::new(0),
            channels: [
                AtomicMotionChannel::new(),
                AtomicMotionChannel::new(),
            ],
            is_initialized: AtomicBool::new(false),
        }
    }

    /// Initializes dual BMI160 hardware channels into Normal operating mode (200 Hz ODR).
    pub fn init(&self) -> bool {
        self.power_mode.store(ImuPowerMode::Normal as u32, Ordering::Release);
        self.is_initialized.store(true, Ordering::Release);
        true
    }

    /// Converts raw BMI160 frame (±4g, ±2000 dps full-scale) into physical units.
    pub fn convert_frame(&self, raw: &RawImuFrame) -> MotionSample {
        // ±4g range: 8192 LSB per 1g (1000 mg) -> mg = (raw * 1000) / 8192
        let ax = ((raw.accel_x as i64 * 1000) / 8192) as i32;
        let ay = ((raw.accel_y as i64 * 1000) / 8192) as i32;
        let az = ((raw.accel_z as i64 * 1000) / 8192) as i32;

        // ±2000 dps range: 16.4 LSB per 1 °/s (1000 mdeg/s) -> mdeg_s = (raw * 10000) / 164
        let gx = ((raw.gyro_x as i64 * 10000) / 164) as i32;
        let gy = ((raw.gyro_y as i64 * 10000) / 164) as i32;
        let gz = ((raw.gyro_z as i64 * 10000) / 164) as i32;

        // Silicon temp: 23°C + raw / 512 in mdeg C
        let temp = 23_000 + ((raw.sensor_temp_raw as i32 * 1000) / 512);

        MotionSample {
            accel_mg: [ax, ay, az],
            gyro_mdeg_s: [gx, gy, gz],
            temp_mdeg_c: temp,
            timestamp_us: raw.timestamp_us,
        }
    }

    /// Ingests a new raw motion frame from hardware SPI/I2C/USB endpoint.
    pub fn ingest_frame(&self, channel: ImuChannel, raw: &RawImuFrame) {
        let sample = self.convert_frame(raw);
        self.channels[channel.index()].store(&sample);
    }

    pub fn get_latest_sample(&self, channel: ImuChannel) -> MotionSample {
        self.channels[channel.index()].load()
    }

    pub fn is_operational(&self) -> bool {
        self.is_initialized.load(Ordering::Acquire)
            && self.power_mode.load(Ordering::Acquire) == ImuPowerMode::Normal as u32
    }
}

pub static IMU_DRIVER: DualBmi160Driver = DualBmi160Driver::new();


// ============================================================================
// Feature 2: Complementary Filter Orientation Fusion & Thermal Bias Calibration
// ============================================================================

/// Euler angle orientation representation in millidegrees (-180,000 to +180,000 mdeg).
#[derive(Debug, Clone, Copy, Default)]
pub struct OrientationEuler {
    pub roll_mdeg: i32,
    pub pitch_mdeg: i32,
    pub yaw_mdeg: i32,
    pub is_calibrated: bool,
}

/// Zero-rate gyroscope bias tracker and stationary stillness detector.
pub struct ThermalBiasCalibrator {
    bias_x_mdeg_s: AtomicI32,
    bias_y_mdeg_s: AtomicI32,
    bias_z_mdeg_s: AtomicI32,
    still_samples_count: AtomicU32,
    accum_x: AtomicI32,
    accum_y: AtomicI32,
    accum_z: AtomicI32,
    is_calibrated: AtomicBool,
    last_cal_temp_mdeg_c: AtomicI32,
}

impl ThermalBiasCalibrator {
    pub const fn new() -> Self {
        Self {
            bias_x_mdeg_s: AtomicI32::new(0),
            bias_y_mdeg_s: AtomicI32::new(0),
            bias_z_mdeg_s: AtomicI32::new(0),
            still_samples_count: AtomicU32::new(0),
            accum_x: AtomicI32::new(0),
            accum_y: AtomicI32::new(0),
            accum_z: AtomicI32::new(0),
            is_calibrated: AtomicBool::new(false),
            last_cal_temp_mdeg_c: AtomicI32::new(35_000),
        }
    }

    /// Evaluates if the handheld is stationary to accumulate and calibrate zero-rate bias.
    pub fn process_sample(&self, sample: &MotionSample) -> bool {
        // Check gyro magnitude < 1.5 °/s (1500 mdeg/s) on all axes
        let gx = sample.gyro_mdeg_s[0].abs();
        let gy = sample.gyro_mdeg_s[1].abs();
        let gz = sample.gyro_mdeg_s[2].abs();

        // Check accel magnitude within 1g ± 0.1g (900 to 1100 mg)
        let ax = sample.accel_mg[0] as i64;
        let ay = sample.accel_mg[1] as i64;
        let az = sample.accel_mg[2] as i64;
        let accel_sq = ax * ax + ay * ay + az * az;
        let is_1g = accel_sq >= 810_000 && accel_sq <= 1_210_000; // ~900^2 .. 1100^2

        if gx < 1500 && gy < 1500 && gz < 1500 && is_1g {
            self.accum_x.fetch_add(sample.gyro_mdeg_s[0], Ordering::Relaxed);
            self.accum_y.fetch_add(sample.gyro_mdeg_s[1], Ordering::Relaxed);
            self.accum_z.fetch_add(sample.gyro_mdeg_s[2], Ordering::Relaxed);
            let count = self.still_samples_count.fetch_add(1, Ordering::AcqRel) + 1;

            if count >= 30 {
                // Calibrate bias
                self.bias_x_mdeg_s.store(self.accum_x.load(Ordering::Relaxed) / count as i32, Ordering::Release);
                self.bias_y_mdeg_s.store(self.accum_y.load(Ordering::Relaxed) / count as i32, Ordering::Release);
                self.bias_z_mdeg_s.store(self.accum_z.load(Ordering::Relaxed) / count as i32, Ordering::Release);
                self.is_calibrated.store(true, Ordering::Release);
                self.last_cal_temp_mdeg_c.store(sample.temp_mdeg_c, Ordering::Release);

                // Reset accumulation
                self.accum_x.store(0, Ordering::Relaxed);
                self.accum_y.store(0, Ordering::Relaxed);
                self.accum_z.store(0, Ordering::Relaxed);
                self.still_samples_count.store(0, Ordering::Release);
                return true;
            }
        } else {
            // Motion detected; reset still window
            self.accum_x.store(0, Ordering::Relaxed);
            self.accum_y.store(0, Ordering::Relaxed);
            self.accum_z.store(0, Ordering::Relaxed);
            self.still_samples_count.store(0, Ordering::Release);
        }

        self.is_calibrated.load(Ordering::Acquire)
    }

    /// Subtracts calibrated zero-rate bias from raw gyro readings.
    pub fn correct_gyro(&self, raw_gyro_mdeg_s: [i32; 3]) -> [i32; 3] {
        [
            raw_gyro_mdeg_s[0] - self.bias_x_mdeg_s.load(Ordering::Acquire),
            raw_gyro_mdeg_s[1] - self.bias_y_mdeg_s.load(Ordering::Acquire),
            raw_gyro_mdeg_s[2] - self.bias_z_mdeg_s.load(Ordering::Acquire),
        ]
    }

    pub fn is_calibrated(&self) -> bool {
        self.is_calibrated.load(Ordering::Acquire)
    }

    pub fn set_bias(&self, bx: i32, by: i32, bz: i32) {
        self.bias_x_mdeg_s.store(bx, Ordering::Release);
        self.bias_y_mdeg_s.store(by, Ordering::Release);
        self.bias_z_mdeg_s.store(bz, Ordering::Release);
        self.is_calibrated.store(true, Ordering::Release);
    }
}

/// Fixed-point arctangent approximation returning millidegrees (-180,000 to +180,000).
fn approx_atan2_mdeg(y: i32, x: i32) -> i32 {
    if x == 0 && y == 0 {
        return 0;
    }
    if x == 0 {
        return if y > 0 { 90_000 } else { -90_000 };
    }

    let abs_y = y.abs() as i64;
    let abs_x = x.abs() as i64;
    let ratio = if abs_x >= abs_y {
        (abs_y * 1000) / abs_x
    } else {
        (abs_x * 1000) / abs_y
    };

    // Polynomial approximation of atan(r): r - r^3 / 3 in milliradians
    let r = ratio; // 0..1000
    let r3 = (r * r * r) / 1_000_000;
    let mrad = r - r3 / 3;
    // 1 mrad = 57.2957795 millidegrees (57296 / 1000)
    let mut deg = ((mrad * 57296) / 1000) as i32;
    if abs_x < abs_y {
        deg = 90_000 - deg;
    }

    if x < 0 && y >= 0 {
        180_000 - deg
    } else if x < 0 && y < 0 {
        -(180_000 - deg)
    } else if x >= 0 && y < 0 {
        -deg
    } else {
        deg
    }
}

/// Fixed-point integer square root.
fn isqrt(val: i64) -> i64 {
    if val <= 0 {
        return 0;
    }
    let mut x0 = val / 2;
    if x0 == 0 {
        return 1;
    }
    let mut x1 = (x0 + val / x0) / 2;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + val / x0) / 2;
    }
    x0
}

/// Complementary orientation fusion filter fusing gyro integration and gravity vector.
pub struct ComplementaryFilter {
    roll_mdeg: AtomicI32,
    pitch_mdeg: AtomicI32,
    yaw_mdeg: AtomicI32,
    last_timestamp_us: AtomicU64,
}

impl ComplementaryFilter {
    pub const fn new() -> Self {
        Self {
            roll_mdeg: AtomicI32::new(0),
            pitch_mdeg: AtomicI32::new(0),
            yaw_mdeg: AtomicI32::new(0),
            last_timestamp_us: AtomicU64::new(0),
        }
    }

    /// Fuses gyro angular rates and accelerometer gravity vector.
    /// Alpha = 0.98 (gyro) / 0.02 (accel).
    pub fn update(&self, sample: &MotionSample, corrected_gyro: [i32; 3]) -> OrientationEuler {
        let prev_time = self.last_timestamp_us.swap(sample.timestamp_us, Ordering::AcqRel);
        let dt_us = if sample.timestamp_us > prev_time && prev_time > 0 {
            (sample.timestamp_us - prev_time) as i64
        } else {
            5_000 // Default 5ms (200 Hz)
        };

        let prev_roll = self.roll_mdeg.load(Ordering::Acquire);
        let prev_pitch = self.pitch_mdeg.load(Ordering::Acquire);
        let prev_yaw = self.yaw_mdeg.load(Ordering::Acquire);

        // Gyro integration: d_angle = (rate * dt_us) / 1_000_000
        let gyro_roll = prev_roll + ((corrected_gyro[0] as i64 * dt_us) / 1_000_000) as i32;
        let gyro_pitch = prev_pitch + ((corrected_gyro[1] as i64 * dt_us) / 1_000_000) as i32;
        let gyro_yaw = prev_yaw + ((corrected_gyro[2] as i64 * dt_us) / 1_000_000) as i32;

        // Accelerometer tilt reference
        let ay = sample.accel_mg[1] as i64;
        let az = sample.accel_mg[2] as i64;

        let accel_roll = approx_atan2_mdeg(sample.accel_mg[1], sample.accel_mg[2]);
        let yz_norm = isqrt(ay * ay + az * az) as i32;
        let accel_pitch = approx_atan2_mdeg(-sample.accel_mg[0], yz_norm);

        // Complementary blend: 98% gyro + 2% accel
        let fused_roll = ((gyro_roll as i64 * 98 + accel_roll as i64 * 2) / 100) as i32;
        let fused_pitch = ((gyro_pitch as i64 * 98 + accel_pitch as i64 * 2) / 100) as i32;
        let fused_yaw = gyro_yaw; // Yaw has no gravity vector reference, relies on gyro integration

        self.roll_mdeg.store(fused_roll, Ordering::Release);
        self.pitch_mdeg.store(fused_pitch, Ordering::Release);
        self.yaw_mdeg.store(fused_yaw, Ordering::Release);

        OrientationEuler {
            roll_mdeg: fused_roll,
            pitch_mdeg: fused_pitch,
            yaw_mdeg: fused_yaw,
            is_calibrated: true,
        }
    }

    pub fn get_orientation(&self) -> OrientationEuler {
        OrientationEuler {
            roll_mdeg: self.roll_mdeg.load(Ordering::Acquire),
            pitch_mdeg: self.pitch_mdeg.load(Ordering::Acquire),
            yaw_mdeg: self.yaw_mdeg.load(Ordering::Acquire),
            is_calibrated: true,
        }
    }

    pub fn reset(&self) {
        self.roll_mdeg.store(0, Ordering::Release);
        self.pitch_mdeg.store(0, Ordering::Release);
        self.yaw_mdeg.store(0, Ordering::Release);
        self.last_timestamp_us.store(0, Ordering::Release);
    }
}

pub static BIAS_CALIBRATOR: ThermalBiasCalibrator = ThermalBiasCalibrator::new();
pub static ORIENTATION_FUSION: ComplementaryFilter = ComplementaryFilter::new();
