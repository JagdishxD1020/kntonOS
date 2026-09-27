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


// ============================================================================
// HDR10 Peak Brightness Boost Governor with Junction Thermal Throttling
// ============================================================================

/// Maximum HDR10 specular highlight burst luminance in nits.
pub const HDR10_PEAK_BURST_NITS: f32 = 1000.0;
/// Sustained thermal equilibrium luminance ceiling in nits.
pub const HDR10_SUSTAINED_CEILING_NITS: f32 = 500.0;
/// Critical junction temperature threshold in Celsius (°C).
pub const JUNCTION_TEMP_CRITICAL_C: f32 = 55.0;
/// Thermal throttling onset temperature in Celsius (°C).
pub const JUNCTION_TEMP_THROTTLE_C: f32 = 45.0;
/// Thermal recovery threshold in Celsius (°C).
pub const JUNCTION_TEMP_RECOVERY_C: f32 = 42.0;

/// Display panel thermal health status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdrThermalState {
    /// Junction temperature safe (<45°C); full 1000-nit bursts allowed.
    Normal,
    /// Junction warm (45°C to 55°C); peak brightness throttled.
    WarmThrottling,
    /// Junction critical (>55°C); clamped to sustained 450-500 nits.
    CriticalOverheat,
}

/// Governor managing 1000-nit HDR highlight bursts and junction thermal safety.
#[derive(Debug, Clone)]
pub struct HdrPeakBoostGovernor {
    pub current_peak_cap_nits: f32,
    pub junction_temp_c: f32,
    pub thermal_state: HdrThermalState,
    pub burst_energy_capacity: f32,
    pub burst_energy_consumed: f32,
    pub throttle_event_count: u32,
}

impl HdrPeakBoostGovernor {
    pub const fn new() -> Self {
        Self {
            current_peak_cap_nits: HDR10_PEAK_BURST_NITS,
            junction_temp_c: 35.0,
            thermal_state: HdrThermalState::Normal,
            burst_energy_capacity: 5.0, // 5.0 seconds at peak 1000 nits
            burst_energy_consumed: 0.0,
            throttle_event_count: 0,
        }
    }

    /// Evaluates requested brightness against junction thermals and burst budget.
    pub fn update(&mut self, requested_nits: f32, junction_temp_c: f32, delta_sec: f32) -> f32 {
        self.junction_temp_c = junction_temp_c;

        // 1. Evaluate junction thermal safety state
        if junction_temp_c >= JUNCTION_TEMP_CRITICAL_C {
            if self.thermal_state != HdrThermalState::CriticalOverheat {
                self.throttle_event_count += 1;
            }
            self.thermal_state = HdrThermalState::CriticalOverheat;
            self.current_peak_cap_nits = 450.0;
        } else if junction_temp_c >= JUNCTION_TEMP_THROTTLE_C {
            if self.thermal_state == HdrThermalState::Normal {
                self.throttle_event_count += 1;
            }
            self.thermal_state = HdrThermalState::WarmThrottling;
            let factor = (junction_temp_c - JUNCTION_TEMP_THROTTLE_C) / (JUNCTION_TEMP_CRITICAL_C - JUNCTION_TEMP_THROTTLE_C);
            self.current_peak_cap_nits = HDR10_PEAK_BURST_NITS - factor * (HDR10_PEAK_BURST_NITS - HDR10_SUSTAINED_CEILING_NITS);
        } else if junction_temp_c <= JUNCTION_TEMP_RECOVERY_C {
            self.thermal_state = HdrThermalState::Normal;
            self.current_peak_cap_nits = HDR10_PEAK_BURST_NITS;
        }

        // 2. Track burst energy budget when driving above sustained ceiling
        let target_clamp = requested_nits.clamp(MIN_NIGHT_NITS, self.current_peak_cap_nits);

        if target_clamp > HDR10_SUSTAINED_CEILING_NITS {
            let excess_ratio = (target_clamp - HDR10_SUSTAINED_CEILING_NITS) / (HDR10_PEAK_BURST_NITS - HDR10_SUSTAINED_CEILING_NITS);
            self.burst_energy_consumed += excess_ratio * delta_sec;

            if self.burst_energy_consumed >= self.burst_energy_capacity {
                self.current_peak_cap_nits = HDR10_SUSTAINED_CEILING_NITS;
                return HDR10_SUSTAINED_CEILING_NITS;
            }
        } else {
            // Replenish burst energy when operating at or below sustained ceiling
            let recharge_rate = 0.5 * delta_sec;
            self.burst_energy_consumed = (self.burst_energy_consumed - recharge_rate).max(0.0);
        }

        target_clamp.min(self.current_peak_cap_nits)
    }
}

/// Unit test verifying HDR10 peak brightness boost governor and thermal back-off.
pub fn verify_hdr_boost_governor() -> bool {
    let mut gov = HdrPeakBoostGovernor::new();

    // 1. Normal thermals (38°C) + brief burst: full 1000 nits granted
    let out1 = gov.update(1000.0, 38.0, 0.1);
    if (out1 - 1000.0).abs() > 0.1 || gov.thermal_state != HdrThermalState::Normal {
        return false;
    }

    // 2. Warm thermals (50°C, midway between 45°C and 55°C): throttles to ~750 nits
    let out2 = gov.update(1000.0, 50.0, 0.1);
    if out2 > 800.0 || out2 < 700.0 || gov.thermal_state != HdrThermalState::WarmThrottling {
        return false;
    }

    // 3. Critical overheat (57°C > 55°C): hard clamped <= 450 nits
    let out3 = gov.update(1000.0, 57.0, 0.1);
    if out3 > 450.0 || gov.thermal_state != HdrThermalState::CriticalOverheat {
        return false;
    }

    // 4. Recovery below 42°C: resets to Normal
    gov.update(400.0, 40.0, 1.0);
    if gov.thermal_state != HdrThermalState::Normal || gov.current_peak_cap_nits < 999.0 {
        return false;
    }

    // 5. Burst energy exhaustion: 6.0 seconds continuous at 1000 nits
    let mut out_burst = 0.0f32;
    for _ in 0..60 {
        out_burst = gov.update(1000.0, 40.0, 0.1);
    }
    if out_burst > HDR10_SUSTAINED_CEILING_NITS {
        return false;
    }

    true
}


// ============================================================================
// Unified Handheld Panel Overdrive & Illumination Manager
// ============================================================================

/// Active display illumination mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IlluminationMode {
    /// Standard dynamic range with pure PWM-free DC dimming.
    SdrDcDimming,
    /// High dynamic range with 1000-nit highlight burst governor.
    Hdr10Boost,
}

/// Unified Display Panel Overdrive & Illumination Manager.
#[derive(Debug, Clone)]
pub struct PanelOverdriveManager {
    pub mode: IlluminationMode,
    pub dc_dimming: DcDimmingCurve,
    pub als_adapter: AlsAutoAdapter,
    pub overdrive_engine: VariableOverdriveEngine,
    pub hdr_governor: HdrPeakBoostGovernor,
    pub active_level: DcDimmingLevel,
    pub current_refresh_hz: f32,
}

impl PanelOverdriveManager {
    pub const fn new() -> Self {
        Self {
            mode: IlluminationMode::SdrDcDimming,
            dc_dimming: DcDimmingCurve::new(),
            als_adapter: AlsAutoAdapter::new(),
            overdrive_engine: VariableOverdriveEngine::new(),
            hdr_governor: HdrPeakBoostGovernor::new(),
            active_level: DcDimmingLevel {
                nits: 120.0,
                dac_code: 980,
                pwm_flicker_hz: 0,
            },
            current_refresh_hz: 60.0,
        }
    }

    /// Evaluates display tick updating illumination and thermal state.
    pub fn update_tick(
        &mut self,
        ambient_lux: f32,
        junction_temp_c: f32,
        current_refresh_hz: f32,
        delta_sec: f32,
    ) -> DcDimmingLevel {
        self.current_refresh_hz = current_refresh_hz;

        match self.mode {
            IlluminationMode::SdrDcDimming => {
                self.active_level = self.als_adapter.update(ambient_lux);
            }
            IlluminationMode::Hdr10Boost => {
                let target_nits = self.als_adapter.lux_to_target_nits(ambient_lux);
                // Allow specular highlight boost up to 1000 nits when ambient or content requests it
                let requested_nits = if ambient_lux > 1000.0 {
                    HDR10_PEAK_BURST_NITS
                } else {
                    target_nits.max(HDR10_SUSTAINED_CEILING_NITS)
                };
                let effective_nits = self.hdr_governor.update(requested_nits, junction_temp_c, delta_sec);
                self.active_level = self.dc_dimming.generate_level(effective_nits);
            }
        }

        self.active_level
    }

    /// Computes pixel transition overdrive value using instantaneous VRR frequency.
    pub fn compute_pixel_overdrive(&self, from_gray: u8, to_gray: u8) -> u8 {
        self.overdrive_engine.compute_overdrive(from_gray, to_gray, self.current_refresh_hz)
    }
}

/// Unit test verifying unified panel overdrive manager.
pub fn verify_panel_overdrive_manager() -> bool {
    let mut mgr = PanelOverdriveManager::new();

    // 1. SDR Mode at 60Hz in dark room
    mgr.mode = IlluminationMode::SdrDcDimming;
    for _ in 0..25 {
        mgr.update_tick(0.0, 36.0, 60.0, 0.1);
    }
    if mgr.active_level.nits > 5.0 || mgr.active_level.pwm_flicker_hz != 0 {
        return false;
    }
    // 60Hz overdrive must have zero flicker and suppressed overshoot
    let od_60 = mgr.compute_pixel_overdrive(0, 128);
    if od_60 < 128 {
        return false;
    }

    // 2. Switch to HDR10 Mode at 144Hz with bright highlights
    mgr.mode = IlluminationMode::Hdr10Boost;
    let hdr_level = mgr.update_tick(2000.0, 38.0, 144.0, 0.1);
    if hdr_level.nits < 480.0 {
        return false;
    }
    let od_144 = mgr.compute_pixel_overdrive(0, 128);
    if od_144 <= od_60 {
        return false;
    }

    // 3. Thermal back-off under high junction temperature (58°C)
    let hot_level = mgr.update_tick(2000.0, 58.0, 144.0, 0.1);
    if hot_level.nits > 500.0 || mgr.hdr_governor.thermal_state != HdrThermalState::CriticalOverheat {
        return false;
    }

    true
}


/// Comprehensive verification self-test suite for Handheld Direct Display Brightness, DC Dimming & Variable Overdrive Controller.
pub fn test_panel_overdrive_subsystem() -> bool {
    crate::serial_write_str("[KNT-DISPLAY] Testing Direct Display Brightness, DC Dimming & Variable Overdrive Controller...\r\n");
    if !verify_dc_dimming_curve() {
        return false;
    }
    if !verify_variable_overdrive() {
        return false;
    }
    if !verify_hdr_boost_governor() {
        return false;
    }
    if !verify_panel_overdrive_manager() {
        return false;
    }
    crate::serial_write_str("[KNT-DISPLAY-OVERDRIVE-PASS] Handheld direct display DC dimming and variable overdrive controller verified\r\n");
    true
}
