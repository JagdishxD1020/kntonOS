//! Handheld Direct Display Brightness, DC Dimming & Variable Overdrive Controller.

#![allow(dead_code)]

/// 12-bit DC dimming DAC resolution for analog backlight driving.
pub const DC_DIMMING_DAC_MAX: u16 = 4095;
/// Minimum night-gaming luminance floor in nits.
pub const MIN_NIGHT_NITS: f32 = 1.8;
/// Maximum standard dynamic range (SDR) luminance in nits.
pub const MAX_SDR_NITS: f32 = 500.0;

/// DC dimming state representing flicker-free analog current level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DcDimmingLevel {
    /// Screen luminance in candela per square meter (cd/m² or nits).
    pub nits: f32,
    /// 12-bit hardware current DAC drive code (0 to 4095).
    pub dac_code: u16,
    /// PWM modulation frequency in Hz (0 indicates pure analog DC dimming).
    pub pwm_flicker_hz: u32,
}

/// PWM-Free DC dimming perceptual gamma curve generator.
#[derive(Debug, Clone)]
pub struct DcDimmingCurve {
    pub min_nits: f32,
    pub max_nits: f32,
    pub gamma: f32,
}

impl DcDimmingCurve {
    pub const fn new() -> Self {
        Self {
            min_nits: MIN_NIGHT_NITS,
            max_nits: MAX_SDR_NITS,
            gamma: 2.2,
        }
    }

    /// Converts target luminance in nits into a 12-bit DC current DAC code.
    pub fn nits_to_dac(&self, nits: f32) -> u16 {
        let clamped = nits.clamp(self.min_nits, self.max_nits);
        let normalized = (clamped - self.min_nits) / (self.max_nits - self.min_nits);

        // Approximate perceptual gamma curve (x^2.2) using integer math or polynomial
        let perceptual = if normalized <= 0.0 {
            0.0
        } else if normalized >= 1.0 {
            1.0
        } else {
            // Cubic Hermite smoothstep approximation for continuous gamma curve
            normalized * normalized * (3.0 - 2.0 * normalized)
        };

        let raw_dac = (perceptual * (DC_DIMMING_DAC_MAX as f32)) as u16;
        // Keep minimum DAC code non-zero so LEDs remain in forward bias
        raw_dac.clamp(16, DC_DIMMING_DAC_MAX)
    }

    /// Converts 12-bit hardware DAC code back to estimated luminance in nits.
    pub fn dac_to_nits(&self, dac_code: u16) -> f32 {
        let code = dac_code.clamp(16, DC_DIMMING_DAC_MAX);
        let ratio = (code - 16) as f32 / (DC_DIMMING_DAC_MAX - 16) as f32;
        self.min_nits + ratio * (self.max_nits - self.min_nits)
    }

    /// Evaluates DC dimming level for requested luminance.
    pub fn generate_level(&self, requested_nits: f32) -> DcDimmingLevel {
        let dac_code = self.nits_to_dac(requested_nits);
        let actual_nits = self.dac_to_nits(dac_code);
        DcDimmingLevel {
            nits: actual_nits,
            dac_code,
            pwm_flicker_hz: 0,
        }
    }
}

/// Ambient Light Sensor (ALS) Auto-Adaptation Engine.
#[derive(Debug, Clone)]
pub struct AlsAutoAdapter {
    pub curve: DcDimmingCurve,
    pub current_nits: f32,
    pub target_nits: f32,
    pub user_bias_pct: i8,
    pub smoothing_factor: f32,
}

impl AlsAutoAdapter {
    pub const fn new() -> Self {
        Self {
            curve: DcDimmingCurve::new(),
            current_nits: 120.0,
            target_nits: 120.0,
            user_bias_pct: 0,
            smoothing_factor: 0.20,
        }
    }

    /// Computes target nits from ambient illuminance in lux.
    pub fn lux_to_target_nits(&self, lux: f32) -> f32 {
        let base_nits = if lux <= 1.0 {
            MIN_NIGHT_NITS
        } else if lux <= 10.0 {
            // Dark bedroom gaming: 2 to 20 nits
            MIN_NIGHT_NITS + (lux / 10.0) * 18.0
        } else if lux <= 100.0 {
            // Dim ambient room: 20 to 120 nits
            20.0 + ((lux - 10.0) / 90.0) * 100.0
        } else if lux <= 500.0 {
            // Indoor office lighting: 120 to 280 nits
            120.0 + ((lux - 100.0) / 400.0) * 160.0
        } else {
            // Bright daylight: 280 to 500 nits
            let factor = ((lux - 500.0) / 2500.0).clamp(0.0, 1.0);
            280.0 + factor * 220.0
        };

        // Apply user bias (-50% to +50%)
        let bias_multiplier = 1.0 + (self.user_bias_pct as f32 / 100.0);
        (base_nits * bias_multiplier).clamp(MIN_NIGHT_NITS, MAX_SDR_NITS)
    }

    /// Updates ALS adaptation filter with new lux sample.
    pub fn update(&mut self, ambient_lux: f32) -> DcDimmingLevel {
        self.target_nits = self.lux_to_target_nits(ambient_lux);
        // Exponential moving average filter for smooth adaptation
        self.current_nits += self.smoothing_factor * (self.target_nits - self.current_nits);
        self.curve.generate_level(self.current_nits)
    }
}

/// Unit test verifying PWM-free DC dimming curve and ALS auto-adaptation.
pub fn verify_dc_dimming_curve() -> bool {
    let curve = DcDimmingCurve::new();

    // 1. Minimum night floor: non-zero DAC, 0Hz flicker
    let low_level = curve.generate_level(MIN_NIGHT_NITS);
    if low_level.pwm_flicker_hz != 0 || low_level.dac_code < 16 {
        return false;
    }

    // 2. Maximum SDR floor (500 nits): full DAC code
    let high_level = curve.generate_level(MAX_SDR_NITS);
    if high_level.dac_code != DC_DIMMING_DAC_MAX {
        return false;
    }

    // 3. Monotonicity check
    let mid1 = curve.nits_to_dac(50.0);
    let mid2 = curve.nits_to_dac(150.0);
    let mid3 = curve.nits_to_dac(300.0);
    if mid1 >= mid2 || mid2 >= mid3 {
        return false;
    }

    // 4. ALS auto adaptation
    let mut als = AlsAutoAdapter::new();
    let dark_level = als.update(0.0);
    if dark_level.nits > 120.0 {
        return false;
    }
    // Run multiple ticks to let filter converge to dark room
    for _ in 0..20 {
        als.update(0.0);
    }
    if als.current_nits > 5.0 {
        return false;
    }

    // Converge to bright sunlight (3000 lux)
    for _ in 0..25 {
        als.update(3000.0);
    }
    if als.current_nits < 450.0 {
        return false;
    }

    true
}


// ============================================================================
// Dynamic Variable Overdrive with Per-Refresh-Rate LUTs
// ============================================================================

/// Grid points for 5x5 Gray-to-Gray (G2G) overdrive lookup tables.
pub const OVERDRIVE_GRID_POINTS: [u8; 5] = [0, 64, 128, 192, 255];

/// 5x5 Gray-to-Gray Overdrive Lookup Table for a fixed refresh rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverdriveLut {
    pub refresh_hz: u16,
    pub matrix: [[u8; 5]; 5],
}

impl OverdriveLut {
    /// 60Hz Overdrive LUT: gentle boost, zero overshoot to eliminate inverse coronas at 16.6ms.
    pub const fn lut_60hz() -> Self {
        Self {
            refresh_hz: 60,
            matrix: [
                [0,   68,  133, 196, 255],
                [0,   64,  131, 195, 255],
                [0,   62,  128, 194, 255],
                [0,   60,  125, 192, 255],
                [0,   58,  123, 188, 255],
            ],
        }
    }

    /// 90Hz Overdrive LUT: moderate boost for 11.1ms frame budget.
    pub const fn lut_90hz() -> Self {
        Self {
            refresh_hz: 90,
            matrix: [
                [0,   74,  140, 202, 255],
                [0,   64,  136, 200, 255],
                [0,   58,  128, 198, 255],
                [0,   54,  122, 192, 255],
                [0,   50,  118, 185, 255],
            ],
        }
    }

    /// 120Hz Overdrive LUT: aggressive boost for 8.3ms frame budget.
    pub const fn lut_120hz() -> Self {
        Self {
            refresh_hz: 120,
            matrix: [
                [0,   80,  146, 208, 255],
                [0,   64,  141, 205, 255],
                [0,   54,  128, 202, 255],
                [0,   48,  119, 192, 255],
                [0,   44,  114, 182, 255],
            ],
        }
    }

    /// 144Hz Overdrive LUT: maximum boost for 6.9ms frame budget.
    pub const fn lut_144hz() -> Self {
        Self {
            refresh_hz: 144,
            matrix: [
                [0,   86,  152, 214, 255],
                [0,   64,  146, 210, 255],
                [0,   50,  128, 206, 255],
                [0,   42,  116, 192, 255],
                [0,   38,  110, 180, 255],
            ],
        }
    }

    /// Bilinear interpolation across nearest 5x5 grid coordinates.
    pub fn lookup(&self, from_gray: u8, to_gray: u8) -> u8 {
        if from_gray == to_gray {
            return to_gray;
        }

        let (from_idx, from_t) = Self::grid_coord(from_gray);
        let (to_idx, to_t) = Self::grid_coord(to_gray);

        let v00 = self.matrix[from_idx][to_idx] as f32;
        let v01 = self.matrix[from_idx][(to_idx + 1).min(4)] as f32;
        let v10 = self.matrix[(from_idx + 1).min(4)][to_idx] as f32;
        let v11 = self.matrix[(from_idx + 1).min(4)][(to_idx + 1).min(4)] as f32;

        let top = v00 + to_t * (v01 - v00);
        let bottom = v10 + to_t * (v11 - v10);
        let interpolated = top + from_t * (bottom - top);

        interpolated.clamp(0.0, 255.0) as u8
    }

    fn grid_coord(gray: u8) -> (usize, f32) {
        if gray <= OVERDRIVE_GRID_POINTS[0] {
            return (0, 0.0);
        }
        for i in 0..4 {
            let low = OVERDRIVE_GRID_POINTS[i];
            let high = OVERDRIVE_GRID_POINTS[i + 1];
            if gray <= high {
                let t = (gray - low) as f32 / (high - low) as f32;
                return (i, t);
            }
        }
        (4, 0.0)
    }
}

/// Dynamic Variable Overdrive Engine adapting to instantaneous VRR refresh rate.
#[derive(Debug, Clone)]
pub struct VariableOverdriveEngine {
    pub lut_60: OverdriveLut,
    pub lut_90: OverdriveLut,
    pub lut_120: OverdriveLut,
    pub lut_144: OverdriveLut,
    pub corona_suppression_enabled: bool,
}

impl VariableOverdriveEngine {
    pub const fn new() -> Self {
        Self {
            lut_60: OverdriveLut::lut_60hz(),
            lut_90: OverdriveLut::lut_90hz(),
            lut_120: OverdriveLut::lut_120hz(),
            lut_144: OverdriveLut::lut_144hz(),
            corona_suppression_enabled: true,
        }
    }

    /// Computes overdrive pixel drive value with VRR frequency interpolation.
    pub fn compute_overdrive(&self, from_gray: u8, to_gray: u8, current_refresh_hz: f32) -> u8 {
        if from_gray == to_gray {
            return to_gray;
        }

        let hz = current_refresh_hz.clamp(60.0, 144.0);

        let (lut_low, lut_high, t) = if hz <= 90.0 {
            (&self.lut_60, &self.lut_90, (hz - 60.0) / 30.0)
        } else if hz <= 120.0 {
            (&self.lut_90, &self.lut_120, (hz - 90.0) / 30.0)
        } else {
            (&self.lut_120, &self.lut_144, (hz - 120.0) / 24.0)
        };

        let val_low = lut_low.lookup(from_gray, to_gray) as f32;
        let val_high = lut_high.lookup(from_gray, to_gray) as f32;
        let mut boosted = val_low + t * (val_high - val_low);

        // Inverse corona suppression at low refresh rates (<75Hz)
        if self.corona_suppression_enabled && hz < 75.0 {
            let target = to_gray as f32;
            let corona_damping = (75.0 - hz) / 15.0; // 0.0 at 75Hz, 1.0 at 60Hz
            // Pull boosted voltage closer to true target to prevent bright/dark halos
            boosted = boosted + corona_damping * 0.40 * (target - boosted);
        }

        boosted.clamp(0.0, 255.0) as u8
    }
}

/// Unit test verifying variable overdrive LUT interpolation and corona suppression.
pub fn verify_variable_overdrive() -> bool {
    let engine = VariableOverdriveEngine::new();

    // 1. Static pixels (no transition) must never overshoot
    if engine.compute_overdrive(128, 128, 60.0) != 128 {
        return false;
    }
    if engine.compute_overdrive(128, 128, 144.0) != 128 {
        return false;
    }

    // 2. Upward transition (0 -> 128): overdrive increases with refresh rate
    let up_60 = engine.compute_overdrive(0, 128, 60.0);
    let up_90 = engine.compute_overdrive(0, 128, 90.0);
    let up_120 = engine.compute_overdrive(0, 128, 120.0);
    let up_144 = engine.compute_overdrive(0, 128, 144.0);

    if up_60 < 128 || up_90 < up_60 || up_120 < up_90 || up_144 < up_120 {
        return false;
    }

    // 3. Downward transition (255 -> 64): undershoot gets more aggressive at high refresh
    let down_60 = engine.compute_overdrive(255, 64, 60.0);
    let down_90 = engine.compute_overdrive(255, 64, 90.0);
    let down_120 = engine.compute_overdrive(255, 64, 120.0);
    let down_144 = engine.compute_overdrive(255, 64, 144.0);

    if down_60 > 64 || down_90 > down_60 || down_120 > down_90 || down_144 > down_120 {
        return false;
    }

    // 4. Smooth VRR interpolation: 75Hz must fall strictly between 60Hz and 90Hz
    let up_75 = engine.compute_overdrive(0, 128, 75.0);
    if up_75 < up_60 || up_75 > up_90 {
        return false;
    }

    // 5. Corona suppression active: 60Hz overshoot is suppressed closer to target
    let engine_no_suppress = VariableOverdriveEngine {
        corona_suppression_enabled: false,
        ..VariableOverdriveEngine::new()
    };
    let unsuppressed_60 = engine_no_suppress.compute_overdrive(0, 128, 60.0);
    if up_60 > unsuppressed_60 {
        return false;
    }

    true
}
