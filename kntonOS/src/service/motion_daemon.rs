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
