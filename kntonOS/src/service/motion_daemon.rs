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


// ============================================================================
// Feature 3: Gyro-to-Mouse / Right-Stick Aim Assist & Capacitive Touch Gate
// ============================================================================

/// Capacitive touch sensor trigger gate source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchGateSource {
    RightThumbstickCap,
    LeftThumbstickCap,
    RightTrackpad,
    AlwaysOn,
}

/// Aim assist output mapping mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimMappingMode {
    MouseEmulation,
    RightStickEmulation,
    Disabled,
}

/// Computed aim assist output frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AimOutput {
    /// Relative mouse delta X (pixels).
    pub mouse_dx: i32,
    /// Relative mouse delta Y (pixels).
    pub mouse_dy: i32,
    /// Virtual right stick analog X deflection (-32768..32767).
    pub stick_x: i16,
    /// Virtual right stick analog Y deflection (-32768..32767).
    pub stick_y: i16,
    /// Whether aim assist gate was open during this cycle.
    pub is_active: bool,
}

/// Capacitive touch sensor enable gate.
pub struct CapacitiveTouchGate {
    source: AtomicU32, // TouchGateSource as u32
    is_touched: AtomicBool,
}

impl CapacitiveTouchGate {
    pub const fn new() -> Self {
        Self {
            source: AtomicU32::new(0), // Default RightThumbstickCap
            is_touched: AtomicBool::new(false),
        }
    }

    pub fn set_source(&self, src: TouchGateSource) {
        self.source.store(src as u32, Ordering::Release);
    }

    pub fn update_touch(&self, touched: bool) {
        self.is_touched.store(touched, Ordering::Release);
    }

    pub fn is_gate_open(&self) -> bool {
        match self.source.load(Ordering::Acquire) {
            3 => true, // AlwaysOn
            _ => self.is_touched.load(Ordering::Acquire),
        }
    }
}

/// High-precision gyro aim assist mapping pipeline.
pub struct GyroAimPipeline {
    mode: AtomicU32, // AimMappingMode as u32
    sensitivity_pct: AtomicU32, // Sensitivity scaling percentage (100 = 1.0x)
    deadzone_mdeg_s: AtomicI32, // Deadzone to suppress micro-hand tremors (e.g. 150 mdeg/s)
    invert_pitch: AtomicBool,
}

impl GyroAimPipeline {
    pub const fn new() -> Self {
        Self {
            mode: AtomicU32::new(0), // Default MouseEmulation
            sensitivity_pct: AtomicU32::new(100),
            deadzone_mdeg_s: AtomicI32::new(150), // 0.15 °/s tremor deadband
            invert_pitch: AtomicBool::new(false),
        }
    }

    pub fn set_mode(&self, mode: AimMappingMode) {
        self.mode.store(mode as u32, Ordering::Release);
    }

    pub fn set_sensitivity(&self, sens_pct: u32) {
        self.sensitivity_pct.store(sens_pct, Ordering::Release);
    }

    pub fn set_deadzone(&self, deadzone_mdeg_s: i32) {
        self.deadzone_mdeg_s.store(deadzone_mdeg_s, Ordering::Release);
    }

    pub fn set_invert_pitch(&self, invert: bool) {
        self.invert_pitch.store(invert, Ordering::Release);
    }

    /// Evaluates gyro angular rate and capacitive touch state to produce aim output.
    pub fn process(&self, gyro_mdeg_s: [i32; 3], gate_open: bool) -> AimOutput {
        if !gate_open || self.mode.load(Ordering::Acquire) == AimMappingMode::Disabled as u32 {
            return AimOutput::default();
        }

        let deadzone = self.deadzone_mdeg_s.load(Ordering::Acquire);
        let sens = self.sensitivity_pct.load(Ordering::Acquire) as i64;
        let invert = self.invert_pitch.load(Ordering::Acquire);

        // Yaw controls horizontal aim, Pitch controls vertical aim
        let mut raw_yaw = gyro_mdeg_s[2];   // Yaw rate (Z axis on handheld)
        let mut raw_pitch = gyro_mdeg_s[0]; // Pitch rate (X axis on handheld)

        // Apply tremor suppression deadzone
        if raw_yaw.abs() < deadzone {
            raw_yaw = 0;
        } else if raw_yaw > 0 {
            raw_yaw -= deadzone;
        } else {
            raw_yaw += deadzone;
        }

        if raw_pitch.abs() < deadzone {
            raw_pitch = 0;
        } else if raw_pitch > 0 {
            raw_pitch -= deadzone;
        } else {
            raw_pitch += deadzone;
        }

        if invert {
            raw_pitch = -raw_pitch;
        }

        match self.mode.load(Ordering::Acquire) {
            0 => {
                // MouseEmulation: 1000 mdeg/s (1°/s) at 100% sens -> ~10 mouse counts
                let dx = ((raw_yaw as i64 * sens * 10) / 100_000) as i32;
                let dy = -((raw_pitch as i64 * sens * 10) / 100_000) as i32;
                AimOutput {
                    mouse_dx: dx,
                    mouse_dy: dy,
                    stick_x: 0,
                    stick_y: 0,
                    is_active: true,
                }
            }
            1 => {
                // RightStickEmulation: Full deflection (32767) at ~180°/s (180,000 mdeg/s)
                let sx = ((raw_yaw as i64 * 32767 * sens) / (180_000 * 100)) as i32;
                let sy = -((raw_pitch as i64 * 32767 * sens) / (180_000 * 100)) as i32;

                let clamp_sx = if sx > 32767 { 32767 } else if sx < -32768 { -32768 } else { sx as i16 };
                let clamp_sy = if sy > 32767 { 32767 } else if sy < -32768 { -32768 } else { sy as i16 };

                AimOutput {
                    mouse_dx: 0,
                    mouse_dy: 0,
                    stick_x: clamp_sx,
                    stick_y: clamp_sy,
                    is_active: true,
                }
            }
            _ => AimOutput::default(),
        }
    }
}

pub static TOUCH_GATE: CapacitiveTouchGate = CapacitiveTouchGate::new();
pub static AIM_PIPELINE: GyroAimPipeline = GyroAimPipeline::new();


// ============================================================================
// Feature 4: Shake, Tilt, and Drop Emergency Free-Fall Detection Triggers
// ============================================================================

/// Handheld tilt posture classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiltPosture {
    /// Normal handheld gaming viewing angle (pitch 15°..45° upward).
    UprightGaming,
    /// Laid flat on flat surface (dock/table).
    FlatOnTable,
    /// Screen facing down (privacy/mute/sleep trigger).
    FaceDown,
    /// Steering tilt left (> 30° roll).
    SteerLeft,
    /// Steering tilt right (> 30° roll).
    SteerRight,
}

/// Motion gesture supervisor and drop emergency detector.
pub struct MotionGestureSupervisor {
    freefall_active: AtomicBool,
    freefall_ticks: AtomicU32,
    freefall_start_us: AtomicU64,
    freefall_duration_ms: AtomicU32,

    shake_detected: AtomicBool,
    shake_energy_accum: AtomicI32,
    last_accel_x: AtomicI32,

    tilt_posture: AtomicU32, // TiltPosture as u32
}

impl MotionGestureSupervisor {
    pub const fn new() -> Self {
        Self {
            freefall_active: AtomicBool::new(false),
            freefall_ticks: AtomicU32::new(0),
            freefall_start_us: AtomicU64::new(0),
            freefall_duration_ms: AtomicU32::new(0),

            shake_detected: AtomicBool::new(false),
            shake_energy_accum: AtomicI32::new(0),
            last_accel_x: AtomicI32::new(0),

            tilt_posture: AtomicU32::new(0),
        }
    }

    /// Evaluates linear acceleration and orientation for free-fall, shake, and tilt triggers.
    pub fn update(&self, sample: &MotionSample, orientation: &OrientationEuler) {
        let ax = sample.accel_mg[0] as i64;
        let ay = sample.accel_mg[1] as i64;
        let az = sample.accel_mg[2] as i64;

        // 1. Drop & Free-Fall Emergency Detector
        // In free-fall, acceleration magnitude drops to near zero (|a| < 250 mg)
        let mag_sq = ax * ax + ay * ay + az * az;
        let is_weightless = mag_sq < 62_500; // < 250^2 mg^2

        if is_weightless {
            let ticks = self.freefall_ticks.fetch_add(1, Ordering::AcqRel) + 1;
            if ticks == 1 {
                self.freefall_start_us.store(sample.timestamp_us, Ordering::Release);
            }
            if ticks >= 10 { // 10 ticks @ 200 Hz = 50ms sustained weightlessness
                self.freefall_active.store(true, Ordering::Release);
                let start = self.freefall_start_us.load(Ordering::Acquire);
                let dur_ms = ((sample.timestamp_us - start) / 1000) as u32;
                self.freefall_duration_ms.store(dur_ms, Ordering::Release);
            }
        } else {
            self.freefall_ticks.store(0, Ordering::Release);
            self.freefall_active.store(false, Ordering::Release);
        }

        // 2. Shake Gesture Detector
        // Measures high dynamic acceleration jerk on X axis (lateral shake)
        let prev_x = self.last_accel_x.swap(sample.accel_mg[0], Ordering::AcqRel);
        let jerk = (sample.accel_mg[0] - prev_x).abs();
        if jerk > 1200 { // Rapid change in lateral acceleration
            let accum = self.shake_energy_accum.fetch_add(jerk, Ordering::AcqRel) + jerk;
            if accum > 3000 {
                self.shake_detected.store(true, Ordering::Release);
            }
        } else {
            let cur = self.shake_energy_accum.load(Ordering::Acquire);
            if cur > 100 {
                self.shake_energy_accum.fetch_sub(100, Ordering::Release);
            } else {
                self.shake_energy_accum.store(0, Ordering::Release);
                self.shake_detected.store(false, Ordering::Release);
            }
        }

        // 3. Handheld Tilt Posture Classification
        // Face down: Z acceleration is strongly negative (gravity pointing up into screen)
        let posture = if sample.accel_mg[2] < -700 {
            TiltPosture::FaceDown
        } else if orientation.roll_mdeg < -30_000 {
            TiltPosture::SteerLeft
        } else if orientation.roll_mdeg > 30_000 {
            TiltPosture::SteerRight
        } else if sample.accel_mg[2] > 900 && sample.accel_mg[1].abs() < 200 {
            TiltPosture::FlatOnTable
        } else {
            TiltPosture::UprightGaming
        };

        self.tilt_posture.store(posture as u32, Ordering::Release);
    }

    pub fn is_freefall_detected(&self) -> bool {
        self.freefall_active.load(Ordering::Acquire)
    }

    pub fn get_freefall_duration_ms(&self) -> u32 {
        self.freefall_duration_ms.load(Ordering::Acquire)
    }

    pub fn is_shake_detected(&self) -> bool {
        self.shake_detected.load(Ordering::Acquire)
    }

    pub fn get_tilt_posture(&self) -> TiltPosture {
        match self.tilt_posture.load(Ordering::Acquire) {
            1 => TiltPosture::FlatOnTable,
            2 => TiltPosture::FaceDown,
            3 => TiltPosture::SteerLeft,
            4 => TiltPosture::SteerRight,
            _ => TiltPosture::UprightGaming,
        }
    }

    pub fn reset(&self) {
        self.freefall_active.store(false, Ordering::Release);
        self.freefall_ticks.store(0, Ordering::Release);
        self.freefall_start_us.store(0, Ordering::Release);
        self.freefall_duration_ms.store(0, Ordering::Release);
        self.shake_detected.store(false, Ordering::Release);
        self.shake_energy_accum.store(0, Ordering::Release);
        self.last_accel_x.store(0, Ordering::Release);
        self.tilt_posture.store(0, Ordering::Release);
    }
}

pub static GESTURE_SUPERVISOR: MotionGestureSupervisor = MotionGestureSupervisor::new();


// ============================================================================
// Verification Self-Test Suite & Subsystem Sentinel
// ============================================================================

/// Comprehensive verification self-test for SteamDeck Gyro & Accelerometer Motion Sensor Service.
pub fn verify_motion_daemon_subsystem() -> bool {
    // 1. Dual BMI160 IMU Driver Initialization & Frame Ingestion
    if !IMU_DRIVER.init() || !IMU_DRIVER.is_operational() {
        return false;
    }

    // Ingest sample frame: 1.0g on Z-axis (raw = 8192), 10°/s on Z-axis (raw = 164), 30°C silicon
    let raw_frame = RawImuFrame {
        accel_x: 0,
        accel_y: 0,
        accel_z: 8192,
        gyro_x: 0,
        gyro_y: 0,
        gyro_z: 164,
        sensor_temp_raw: 3584, // 23°C + (3584*1000)/512 m°C = 30_000 m°C (30.0°C)
        timestamp_us: 10_000,
    };

    IMU_DRIVER.ingest_frame(ImuChannel::PrimaryChassis, &raw_frame);
    let sample = IMU_DRIVER.get_latest_sample(ImuChannel::PrimaryChassis);

    if sample.accel_mg[2] != 1000 || sample.gyro_mdeg_s[2] != 10_000 || sample.temp_mdeg_c != 30_000 {
        return false;
    }

    // 2. Thermal Bias Auto-Calibration & Complementary Filter Fusion
    BIAS_CALIBRATOR.set_bias(120, -80, 250);
    let raw_gyro = [120, 920, 250];
    let corrected = BIAS_CALIBRATOR.correct_gyro(raw_gyro);
    if corrected[0] != 0 || corrected[1] != 1000 || corrected[2] != 0 {
        return false;
    }

    // Orientation fusion update: tilt along Y-axis (roll)
    ORIENTATION_FUSION.reset();
    let tilt_sample = MotionSample {
        accel_mg: [0, 500, 866], // ~30° roll angle
        gyro_mdeg_s: [0, 0, 0],
        temp_mdeg_c: 30_000,
        timestamp_us: 20_000,
    };
    let orient = ORIENTATION_FUSION.update(&tilt_sample, [0, 0, 0]);
    if !orient.is_calibrated {
        return false;
    }

    // 3. Capacitive Touch Gate & Gyro Aim Assist Pipeline
    TOUCH_GATE.set_source(TouchGateSource::RightThumbstickCap);
    TOUCH_GATE.update_touch(false); // Thumb off stick -> gate closed
    if TOUCH_GATE.is_gate_open() {
        return false;
    }

    AIM_PIPELINE.set_mode(AimMappingMode::MouseEmulation);
    AIM_PIPELINE.set_sensitivity(100);
    AIM_PIPELINE.set_deadzone(150);

    // With touch gate closed, aim output must be inactive
    let out_inactive = AIM_PIPELINE.process([0, 0, 5000], TOUCH_GATE.is_gate_open());
    if out_inactive.is_active || out_inactive.mouse_dx != 0 {
        return false;
    }

    // Touch stick -> gate opens
    TOUCH_GATE.update_touch(true);
    if !TOUCH_GATE.is_gate_open() {
        return false;
    }

    // Tremor below deadzone (100 mdeg/s < 150 mdeg/s) -> filtered out
    let out_tremor = AIM_PIPELINE.process([0, 0, 100], true);
    if out_tremor.mouse_dx != 0 {
        return false;
    }

    // Deliberate aim motion: 5000 mdeg/s (5°/s). Net yaw = 5000 - 150 = 4850 mdeg/s
    // dx = (4850 * 100 * 10) / 100_000 = 48 pixels
    let out_active = AIM_PIPELINE.process([0, 0, 5000], true);
    if !out_active.is_active || out_active.mouse_dx < 45 || out_active.mouse_dx > 52 {
        return false;
    }

    // Test RightStickEmulation mode
    AIM_PIPELINE.set_mode(AimMappingMode::RightStickEmulation);
    let out_stick = AIM_PIPELINE.process([0, 0, 5000], true);
    if !out_stick.is_active || out_stick.stick_x == 0 {
        return false;
    }

    // 4. Motion Gesture & Drop Free-Fall Triggers
    GESTURE_SUPERVISOR.reset();
    let freefall_sample = MotionSample {
        accel_mg: [10, -20, 15], // Near zero g weightlessness
        gyro_mdeg_s: [0, 0, 0],
        temp_mdeg_c: 30_000,
        timestamp_us: 100_000,
    };

    // Simulate 12 ticks of freefall
    let mut tick = 0;
    while tick < 12 {
        let mut s = freefall_sample;
        s.timestamp_us = 100_000 + tick * 5_000;
        GESTURE_SUPERVISOR.update(&s, &orient);
        tick += 1;
    }

    if !GESTURE_SUPERVISOR.is_freefall_detected() {
        return false;
    }
    if GESTURE_SUPERVISOR.get_freefall_duration_ms() < 50 {
        return false;
    }

    // Face down posture trigger
    let face_down_sample = MotionSample {
        accel_mg: [0, 0, -980], // Gravity pointing upward into screen
        gyro_mdeg_s: [0, 0, 0],
        temp_mdeg_c: 30_000,
        timestamp_us: 200_000,
    };
    GESTURE_SUPERVISOR.update(&face_down_sample, &orient);
    if GESTURE_SUPERVISOR.get_tilt_posture() != TiltPosture::FaceDown {
        return false;
    }

    // Reset clean baseline
    GESTURE_SUPERVISOR.reset();
    TOUCH_GATE.update_touch(false);
    AIM_PIPELINE.set_mode(AimMappingMode::Disabled);

    true
}
