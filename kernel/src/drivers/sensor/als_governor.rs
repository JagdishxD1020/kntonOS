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

/// Perceptual backlight governor using Stevens power-law and logarithmic PWM duty mapping.
#[derive(Debug, Clone, Copy)]
pub struct PerceptualBacklightGovernor {
    pub min_pwm_duty_percent: u8,
    pub max_pwm_duty_percent: u8,
    pub current_pwm_duty: u8,
    pub target_pwm_duty: u8,
}

impl PerceptualBacklightGovernor {
    /// Creates a new perceptual backlight governor with [5%..100%] PWM duty bounds.
    pub const fn new() -> Self {
        Self {
            min_pwm_duty_percent: 5,
            max_pwm_duty_percent: 100,
            current_pwm_duty: 50,
            target_pwm_duty: 50,
        }
    }

    /// Maps debounced ambient lux to a perceptual PWM duty cycle percentage.
    /// Uses Stevens power-law approximation: Brightness ~ Lux^0.33 mapped across human eye dynamic range.
    pub fn compute_target_duty(&mut self, lux: u32) -> u8 {
        // Lux thresholds:
        // 0..10 lux (pitch dark / bedside): 5% - 15%
        // 10..100 lux (dim indoor evening): 15% - 35%
        // 100..500 lux (normal office/living room): 35% - 65%
        // 500..2000 lux (bright indoor / overcast outdoor): 65% - 85%
        // 2000+ lux (direct sunlight / outdoor handheld gaming): 85% - 100%
        let duty = if lux <= 10 {
            5 + ((lux * 10) / 10) as u8 // 5% to 15%
        } else if lux <= 100 {
            15 + (((lux - 10) * 20) / 90) as u8 // 15% to 35%
        } else if lux <= 500 {
            35 + (((lux - 100) * 30) / 400) as u8 // 35% to 65%
        } else if lux <= 2000 {
            65 + (((lux - 500) * 20) / 1500) as u8 // 65% to 85%
        } else {
            let extra = (((lux - 2000) * 15) / 98000) as u8;
            (85 + extra).min(100)
        };

        self.target_pwm_duty = duty;
        self.target_pwm_duty
    }

    /// Smoothly slews current duty cycle toward target to prevent jarring jumps.
    pub fn step_slew(&mut self, max_step: u8) -> u8 {
        if self.current_pwm_duty < self.target_pwm_duty {
            self.current_pwm_duty = (self.current_pwm_duty + max_step).min(self.target_pwm_duty);
        } else if self.current_pwm_duty > self.target_pwm_duty {
            self.current_pwm_duty = self.current_pwm_duty.saturating_sub(max_step).max(self.target_pwm_duty);
        }
        self.current_pwm_duty
    }
}

/// Unit verification for perceptual backlight governor.
pub fn verify_perceptual_backlight_governor() -> bool {
    let mut gov = PerceptualBacklightGovernor::new();

    // Pitch dark (0 lux): should produce minimum 5%
    let dark_duty = gov.compute_target_duty(0);
    if dark_duty != 5 {
        return false;
    }

    // Normal indoor (300 lux): should produce ~50%
    let indoor_duty = gov.compute_target_duty(300);
    if indoor_duty < 45 || indoor_duty > 55 {
        return false;
    }

    // Direct sun (50,000 lux): should produce high duty >= 90%
    let sun_duty = gov.compute_target_duty(50_000);
    if sun_duty < 90 || sun_duty > 100 {
        return false;
    }

    // Slew verification
    gov.current_pwm_duty = 50;
    gov.target_pwm_duty = 80;
    let s1 = gov.step_slew(10);
    if s1 != 60 || gov.current_pwm_duty != 60 {
        return false;
    }

    true
}

/// Circadian night-mode color temperature range.
pub const COLOR_TEMP_DAYLIGHT_K: u32 = 6500;
pub const COLOR_TEMP_NIGHT_K: u32 = 3200;

/// Night-mode circadian white-point matrix adjustment and RGB gamma ramp engine.
#[derive(Debug, Clone, Copy)]
pub struct NightModeColorEngine {
    pub enabled: bool,
    pub current_temp_k: u32,
    pub target_temp_k: u32,
    pub red_gain_q8: u16,   // 256 = 1.0x
    pub green_gain_q8: u16, // 256 = 1.0x
    pub blue_gain_q8: u16,  // 256 = 1.0x
}

impl NightModeColorEngine {
    /// Creates a new circadian color engine initialized at 6500K daylight white-point.
    pub const fn new() -> Self {
        Self {
            enabled: true,
            current_temp_k: COLOR_TEMP_DAYLIGHT_K,
            target_temp_k: COLOR_TEMP_DAYLIGHT_K,
            red_gain_q8: 256,
            green_gain_q8: 256,
            blue_gain_q8: 256,
        }
    }

    /// Evaluates ambient light level to determine target color temperature (warm night mode in dark conditions).
    pub fn evaluate_ambient_lux(&mut self, lux: u32) -> u32 {
        if !self.enabled {
            self.target_temp_k = COLOR_TEMP_DAYLIGHT_K;
            self.compute_gains();
            return self.target_temp_k;
        }

        // Under 50 lux (bedside/dark gaming room): warm down toward 3200K
        // 50 to 500 lux: linear blend from 3200K to 6500K
        // >500 lux: full daylight 6500K
        let temp = if lux <= 50 {
            COLOR_TEMP_NIGHT_K
        } else if lux >= 500 {
            COLOR_TEMP_DAYLIGHT_K
        } else {
            let factor = (lux - 50) as u64;
            let span = (COLOR_TEMP_DAYLIGHT_K - COLOR_TEMP_NIGHT_K) as u64;
            (COLOR_TEMP_NIGHT_K as u64 + (factor * span) / 450) as u32
        };

        self.target_temp_k = temp;
        self.compute_gains();
        self.target_temp_k
    }

    /// Computes RGB white-point gains based on target color temperature.
    pub fn compute_gains(&mut self) {
        let temp = self.target_temp_k.min(COLOR_TEMP_DAYLIGHT_K).max(COLOR_TEMP_NIGHT_K);
        // Red remains 1.0x (256)
        self.red_gain_q8 = 256;

        // As temperature drops toward 3200K, green drops slightly (~0.85x = 217), blue drops strongly (~0.50x = 128)
        let ratio = (temp - COLOR_TEMP_NIGHT_K) as u32; // 0 (night) to 3300 (daylight)
        let g_drop = (39 * (3300 - ratio)) / 3300; // 0 to 39
        let b_drop = (128 * (3300 - ratio)) / 3300; // 0 to 128

        self.green_gain_q8 = 256 - g_drop as u16;
        self.blue_gain_q8 = 256 - b_drop as u16;
    }

    /// Transforms an RGB pixel via the circadian white-point matrix.
    pub fn transform_rgb(&self, r: u8, g: u8, b: u8) -> (u8, u8, u8) {
        let tr = ((r as u32 * self.red_gain_q8 as u32) / 256) as u8;
        let tg = ((g as u32 * self.green_gain_q8 as u32) / 256) as u8;
        let tb = ((b as u32 * self.blue_gain_q8 as u32) / 256) as u8;
        (tr, tg, tb)
    }
}

/// Unit verification for night-mode circadian color engine.
pub fn verify_night_mode_color_engine() -> bool {
    let mut engine = NightModeColorEngine::new();

    // Daylight (6500K, >500 lux): no attenuation
    engine.evaluate_ambient_lux(1000);
    if engine.target_temp_k != 6500 || engine.blue_gain_q8 != 256 {
        return false;
    }
    let (dr, dg, db) = engine.transform_rgb(200, 200, 200);
    if dr != 200 || dg != 200 || db != 200 {
        return false;
    }

    // Pitch dark (<50 lux): warm night mode (3200K)
    engine.evaluate_ambient_lux(10);
    if engine.target_temp_k != 3200 || engine.blue_gain_q8 != 128 {
        return false;
    }
    let (nr, ng, nb) = engine.transform_rgb(200, 200, 200);
    if nr != 200 || ng >= 200 || nb != 100 {
        return false;
    }

    true
}

/// Hysteresis stabilization governor with dual-threshold deadband to eliminate backlight flutter.
#[derive(Debug, Clone, Copy)]
pub struct HysteresisStabilizer {
    pub deadband_percent: u8,
    pub last_settled_duty: u8,
    pub flutter_count: u32,
}

impl HysteresisStabilizer {
    /// Creates a new hysteresis stabilizer with 5% deadband window.
    pub const fn new() -> Self {
        Self {
            deadband_percent: 5,
            last_settled_duty: 50,
            flutter_count: 0,
        }
    }

    /// Filters candidate PWM duty through the dual-threshold deadband.
    /// If candidate change is within +/- deadband_percent of last settled duty, the change is suppressed.
    pub fn filter_candidate_duty(&mut self, candidate_duty: u8) -> u8 {
        let diff = if candidate_duty > self.last_settled_duty {
            candidate_duty - self.last_settled_duty
        } else {
            self.last_settled_duty - candidate_duty
        };

        if diff >= self.deadband_percent {
            self.last_settled_duty = candidate_duty;
        } else {
            self.flutter_count += 1;
        }

        self.last_settled_duty
    }
}

/// Unit verification for hysteresis stabilization governor.
pub fn verify_hysteresis_stabilizer() -> bool {
    let mut stab = HysteresisStabilizer::new();
    stab.last_settled_duty = 50;

    // Small fluctuations (+/- 3%) inside 5% deadband must be suppressed
    let f1 = stab.filter_candidate_duty(52);
    let f2 = stab.filter_candidate_duty(48);
    let f3 = stab.filter_candidate_duty(53);
    if f1 != 50 || f2 != 50 || f3 != 50 || stab.flutter_count != 3 {
        return false;
    }

    // Significant ambient change (jump to 60%, diff = 10% >= 5%): updates settled duty
    let f4 = stab.filter_candidate_duty(60);
    if f4 != 60 || stab.last_settled_duty != 60 {
        return false;
    }

    true
}

/// Comprehensive verification self-test suite for Handheld Ambient Light Sensor (ALS) Dynamic Screen Dimming & Color Governor.
pub fn test_als_governor_subsystem() -> bool {
    crate::serial_write_str("[KNT-SENSOR] Testing Handheld ALS Dynamic Backlight & Color Temperature Governor...\r\n");

    if !verify_als_sensor_model() {
        crate::serial_write_str("[KNT-PANIC] ALS hardware sensor model verification failed!\r\n");
        return false;
    }
    if !verify_perceptual_backlight_governor() {
        crate::serial_write_str("[KNT-PANIC] Perceptual backlight governor verification failed!\r\n");
        return false;
    }
    if !verify_night_mode_color_engine() {
        crate::serial_write_str("[KNT-PANIC] Night-mode color temperature engine verification failed!\r\n");
        return false;
    }
    if !verify_hysteresis_stabilizer() {
        crate::serial_write_str("[KNT-PANIC] Hysteresis stabilization governor verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-SENSOR-ALS-PASS] Handheld ALS dynamic backlight and color temperature governor verified\r\n");
    true
}
