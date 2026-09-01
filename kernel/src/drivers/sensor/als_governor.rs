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
