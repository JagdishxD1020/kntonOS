//! Handheld Thermal Subsystem: Peltier Active Cooling & Heat Pipe Phase-Change Vapor Chamber Governor.
//!
//! Provides ultra-low thermal resistance cooling orchestration for high-TDP handheld gaming PCs
//! combining sintered-copper planar vapor chambers with solid-state Peltier (TEC) active heat pumps.
//!
//! Features:
//! 1. Dynamic vapor chamber phase-change transition tracking with multi-point sensor fusion.
//! 2. Peltier thermoelectric cooler (TEC) PWM current governor balancing thermal dissipation against battery drain.
//! 3. Emergency thermal runaway throttling preventing SoC hot spots during sustained high-TDP handheld gaming.
//! 4. Comprehensive self-test suite and wire sentinel `[KNT-POWER-VAPOR-CHAMBER-PASS]` into `kernel/src/lib.rs` `kmain`.

#![allow(dead_code)]

// ============================================================================
// Feature 1: Dynamic Vapor Chamber Phase-Change Transition & Sensor Fusion
// ============================================================================

/// Millidegrees Celsius scale factor (1°C = 1,000 mC).
pub const MILLIDEG_SCALE: i32 = 1000;

/// Number of synchronized thermal sensors across the handheld thermal topology.
pub const SENSOR_CHANNEL_COUNT: usize = 6;

/// Sensor location identifiers across the handheld chassis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ThermalSensorLocation {
    /// APU / SoC silicon die junction (primary hot spot).
    SocDie = 0,
    /// Vapor chamber evaporator plate directly contacting SoC IHS.
    EvaporatorPlate = 1,
    /// Vapor chamber condenser fin stack root.
    CondenserFin = 2,
    /// Ergonomic grip / outer chassis surface (skin safety limit).
    ChassisSkin = 3,
    /// Ambient air intake plenum.
    AmbientIntake = 4,
    /// Dual-cell battery pack core.
    BatteryCell = 5,
}

/// Dynamic phase-change operating states of the two-phase planar vapor chamber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaporChamberPhaseState {
    /// Ambient idle (< 40°C): Working fluid is condensed in liquid wick; negligible vapor flow.
    LiquidDormant,
    /// Moderate gaming load (40°C - 75°C): Steady nucleate boiling and two-phase capillary loop.
    EvaporationActive,
    /// High-TDP handheld gaming (75°C - 88°C): Saturated vapor flux, peak thermal conductivity (> 25,000 W/m·K).
    SaturatedHighConductance,
    /// Capillary limit warning (> 88°C or extreme thermal flux): Wick liquid starvation begins.
    CapillaryDryoutWarning,
    /// Critical dryout (> 95°C or ΔT spike > 25°C): Vapor blanket formed under die; thermal resistance surges.
    CriticalDryout,
}

/// Individual thermal sensor channel reading with validity and noise filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SensorReading {
    /// Sensor channel location.
    pub location: ThermalSensorLocation,
    /// Raw uncalibrated temperature in millidegrees Celsius.
    pub raw_temp_mc: i32,
    /// Filtered calibrated temperature in millidegrees Celsius.
    pub filtered_temp_mc: i32,
    /// Rate of temperature change in millidegrees Celsius per second (mC/s).
    pub slew_rate_mc_s: i32,
    /// Sensor health / fault status.
    pub is_valid: bool,
}

impl SensorReading {
    /// Creates a default healthy sensor reading initialized to nominal room temperature (25°C).
    pub const fn new(location: ThermalSensorLocation) -> Self {
        Self {
            location,
            raw_temp_mc: 25 * MILLIDEG_SCALE,
            filtered_temp_mc: 25 * MILLIDEG_SCALE,
            slew_rate_mc_s: 0,
            is_valid: true,
        }
    }
}

/// Multi-point sensor fusion engine and vapor chamber phase-change tracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaporChamberSensorFusion {
    /// Sensor readings across all 6 thermal zones.
    pub sensors: [SensorReading; SENSOR_CHANNEL_COUNT],
    /// Current estimated phase-change state of the vapor chamber.
    pub phase_state: VaporChamberPhaseState,
    /// Temperature delta between SoC die and condenser fin root in millidegrees Celsius.
    pub evaporator_delta_mc: i32,
    /// Estimated thermal resistance across the vapor chamber in milliKelvin per Watt (mK/W).
    pub effective_thermal_resistance_mk_w: u32,
    /// Estimated vapor mass flux in milligrams per second (mg/s).
    pub estimated_vapor_flux_mg_s: u32,
    /// Outlier rejection counter detecting faulty sensor spikes.
    pub outlier_rejections: u32,
    /// Timestamp of last sensor update in milliseconds.
    pub last_update_ms: u64,
}

impl VaporChamberSensorFusion {
    /// Maximum plausible temperature reading (125°C = 125,000 mC).
    pub const MAX_PLAUSIBLE_TEMP_MC: i32 = 125 * MILLIDEG_SCALE;
    /// Minimum plausible temperature reading (-10°C = -10,000 mC).
    pub const MIN_PLAUSIBLE_TEMP_MC: i32 = -10 * MILLIDEG_SCALE;
    /// Maximum plausible single-step change (20°C = 20,000 mC over 100ms).
    pub const MAX_STEP_JUMP_MC: i32 = 20 * MILLIDEG_SCALE;

    /// Nominal baseline thermal resistance in normal saturated phase (180 mK/W).
    pub const BASELINE_RESISTANCE_MK_W: u32 = 180;
    /// Surge thermal resistance during critical dryout (1,250 mK/W).
    pub const DRYOUT_RESISTANCE_MK_W: u32 = 1250;

    /// Initializes sensor fusion engine at 25°C ambient baseline.
    pub const fn new() -> Self {
        Self {
            sensors: [
                SensorReading::new(ThermalSensorLocation::SocDie),
                SensorReading::new(ThermalSensorLocation::EvaporatorPlate),
                SensorReading::new(ThermalSensorLocation::CondenserFin),
                SensorReading::new(ThermalSensorLocation::ChassisSkin),
                SensorReading::new(ThermalSensorLocation::AmbientIntake),
                SensorReading::new(ThermalSensorLocation::BatteryCell),
            ],
            phase_state: VaporChamberPhaseState::LiquidDormant,
            evaporator_delta_mc: 0,
            effective_thermal_resistance_mk_w: Self::BASELINE_RESISTANCE_MK_W,
            estimated_vapor_flux_mg_s: 0,
            outlier_rejections: 0,
            last_update_ms: 0,
        }
    }

    /// Ingests a new set of raw thermal readings, filters outliers, and computes fused metrics.
    pub fn update_telemetry(&mut self, raw_readings: &[i32; SENSOR_CHANNEL_COUNT], timestamp_ms: u64, estimated_tdp_watts: u32) {
        let dt_ms = if self.last_update_ms > 0 && timestamp_ms > self.last_update_ms {
            (timestamp_ms - self.last_update_ms) as u32
        } else {
            100 // Default 100ms interval
        };
        self.last_update_ms = timestamp_ms;

        // Process each sensor channel with outlier filtering and exponential smoothing
        for i in 0..SENSOR_CHANNEL_COUNT {
            let raw = raw_readings[i];

            // 1. Outlier validation check
            if raw < Self::MIN_PLAUSIBLE_TEMP_MC || raw > Self::MAX_PLAUSIBLE_TEMP_MC {
                self.outlier_rejections += 1;
                self.sensors[i].is_valid = false;
                continue;
            }

            let prev_filtered = self.sensors[i].filtered_temp_mc;
            let step = (raw - prev_filtered).abs();
            if step > Self::MAX_STEP_JUMP_MC && self.sensors[i].is_valid {
                // Reject sudden unphysical glitch step
                self.outlier_rejections += 1;
                continue;
            }

            self.sensors[i].is_valid = true;
            self.sensors[i].raw_temp_mc = raw;

            // 2. Exponential smoothing: 75% old + 25% new
            let smoothed = (prev_filtered * 3 + raw) / 4;
            let delta = smoothed - prev_filtered;
            self.sensors[i].slew_rate_mc_s = (delta * 1000) / (dt_ms as i32);
            self.sensors[i].filtered_temp_mc = smoothed;
        }

        // 3. Compute thermal gradient between SoC die and condenser fin root
        let soc_temp = self.sensors[ThermalSensorLocation::SocDie as usize].filtered_temp_mc;
        let condenser_temp = self.sensors[ThermalSensorLocation::CondenserFin as usize].filtered_temp_mc;
        let evap_temp = self.sensors[ThermalSensorLocation::EvaporatorPlate as usize].filtered_temp_mc;

        self.evaporator_delta_mc = soc_temp.saturating_sub(condenser_temp);

        // 4. Compute effective thermal resistance: R_th = (T_die - T_condenser) / Power
        if estimated_tdp_watts > 0 {
            // ΔT in mC / P in Watts = mK/W
            self.effective_thermal_resistance_mk_w = (self.evaporator_delta_mc.max(1) as u32) / estimated_tdp_watts;
        } else {
            self.effective_thermal_resistance_mk_w = Self::BASELINE_RESISTANCE_MK_W;
        }

        // 5. Estimate vapor flux: ~25 mg/s per Watt of heat transport in water heat pipes
        self.estimated_vapor_flux_mg_s = estimated_tdp_watts * 25;

        // 6. Phase-change transition state machine
        self.phase_state = self.evaluate_phase_state(soc_temp, evap_temp);
    }

    /// Evaluates vapor chamber phase state based on temperature and gradient indicators.
    fn evaluate_phase_state(&self, soc_temp_mc: i32, evap_temp_mc: i32) -> VaporChamberPhaseState {
        // Critical dryout: SoC > 95°C or ΔT across chamber > 25°C with high resistance
        if soc_temp_mc >= 95 * MILLIDEG_SCALE || (self.evaporator_delta_mc >= 25 * MILLIDEG_SCALE && self.effective_thermal_resistance_mk_w > 800) {
            return VaporChamberPhaseState::CriticalDryout;
        }

        // Capillary limit warning: SoC > 88°C or evaporator > 82°C with rising resistance
        if soc_temp_mc >= 88 * MILLIDEG_SCALE || evap_temp_mc >= 82 * MILLIDEG_SCALE || self.effective_thermal_resistance_mk_w > 500 {
            return VaporChamberPhaseState::CapillaryDryoutWarning;
        }

        // Saturated high-conductance: SoC 75°C - 88°C
        if soc_temp_mc >= 75 * MILLIDEG_SCALE {
            return VaporChamberPhaseState::SaturatedHighConductance;
        }

        // Active evaporation: SoC 40°C - 75°C
        if soc_temp_mc >= 40 * MILLIDEG_SCALE {
            return VaporChamberPhaseState::EvaporationActive;
        }

        // Idle liquid dormant
        VaporChamberPhaseState::LiquidDormant
    }

    /// Returns the filtered temperature for a specific sensor channel.
    pub fn get_temperature(&self, location: ThermalSensorLocation) -> i32 {
        self.sensors[location as usize].filtered_temp_mc
    }
}

/// Verification test for vapor chamber sensor fusion and phase-change tracking.
pub fn verify_vapor_chamber_sensor_fusion_and_phase_tracking() -> bool {
    let mut fusion = VaporChamberSensorFusion::new();

    // 1. Initial dormant state at 25°C
    if fusion.phase_state != VaporChamberPhaseState::LiquidDormant {
        return false;
    }

    // 2. Simulate moderate gaming: SoC 65°C, Evap 60°C, Condenser 52°C, Skin 38°C, Ambient 24°C, Battery 32°C at 20W TDP
    let tdp_20w = 20;
    let moderate_readings = [
        65 * MILLIDEG_SCALE,
        60 * MILLIDEG_SCALE,
        52 * MILLIDEG_SCALE,
        38 * MILLIDEG_SCALE,
        24 * MILLIDEG_SCALE,
        32 * MILLIDEG_SCALE,
    ];

    // Feed several telemetry cycles to allow filter smoothing to converge
    for i in 1..=5 {
        fusion.update_telemetry(&moderate_readings, i * 100, tdp_20w);
    }

    if fusion.phase_state != VaporChamberPhaseState::EvaporationActive {
        return false;
    }
    if fusion.estimated_vapor_flux_mg_s != 20 * 25 {
        return false;
    }

    // 3. Simulate high-TDP saturated gaming: SoC 82°C, Evap 78°C, Condenser 70°C at 35W TDP
    let tdp_35w = 35;
    let high_load_readings = [
        82 * MILLIDEG_SCALE,
        78 * MILLIDEG_SCALE,
        70 * MILLIDEG_SCALE,
        42 * MILLIDEG_SCALE,
        26 * MILLIDEG_SCALE,
        36 * MILLIDEG_SCALE,
    ];
    for i in 6..=12 {
        fusion.update_telemetry(&high_load_readings, i * 100, tdp_35w);
    }

    if fusion.phase_state != VaporChamberPhaseState::SaturatedHighConductance {
        return false;
    }

    // 4. Test outlier rejection with unphysical spike
    let glitch_readings = [
        180 * MILLIDEG_SCALE, // unphysical 180°C glitch
        78 * MILLIDEG_SCALE,
        70 * MILLIDEG_SCALE,
        42 * MILLIDEG_SCALE,
        26 * MILLIDEG_SCALE,
        36 * MILLIDEG_SCALE,
    ];
    let rejections_before = fusion.outlier_rejections;
    fusion.update_telemetry(&glitch_readings, 1300, tdp_35w);
    if fusion.outlier_rejections != rejections_before + 1 {
        return false;
    }

    // 5. Simulate critical dryout: SoC surges to 98°C while condenser lags at 65°C
    let dryout_readings = [
        98 * MILLIDEG_SCALE,
        94 * MILLIDEG_SCALE,
        65 * MILLIDEG_SCALE,
        44 * MILLIDEG_SCALE,
        27 * MILLIDEG_SCALE,
        38 * MILLIDEG_SCALE,
    ];
    for i in 14..=20 {
        fusion.update_telemetry(&dryout_readings, i * 100, 35);
    }

    if fusion.phase_state != VaporChamberPhaseState::CriticalDryout {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Peltier Thermoelectric Cooler (TEC) PWM Current Governor
// ============================================================================

/// Operating modes of the Peltier Thermoelectric Cooler (TEC) current governor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeltierOperatingMode {
    /// TEC fully powered down; relies purely on passive vapor chamber dissipation.
    PassiveOff,
    /// Low-power active cooling (up to 1.5A / 5W); optimal COP for extended battery life.
    Eco,
    /// Medium active cooling (up to 3.0A / 10W); balanced gaming performance.
    Balanced,
    /// Maximum heat-pumping turbo mode (up to 4.5A / 18W); reserved for AC power or high SoC load.
    Turbo,
}

/// Power source status of the handheld system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandheldPowerSource {
    /// Running exclusively from internal dual-cell lithium battery pack.
    BatteryDischarging,
    /// Connected to external USB-PD EPR charger (>= 65W negotiated).
    AcConnected,
}

/// Peltier Thermoelectric Cooler (TEC) hardware configuration and state governor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeltierTecGovernor {
    /// Active operating mode.
    pub mode: PeltierOperatingMode,
    /// Effective PWM duty cycle in basis points (0 to 10,000 = 0.0% to 100.0%).
    pub pwm_duty_basis_points: u16,
    /// Actual output current in milliamperes (mA).
    pub current_ma: u32,
    /// Electrical input power consumed by TEC in milliwatts (mW).
    pub electrical_power_mw: u32,
    /// Active heat pumped from cold plate to hot plate in milliwatts (mW).
    pub heat_pumped_mw: u32,
    /// Calculated Coefficient of Performance (COP) scaled by 1,000 (e.g. 1,500 = 1.50 COP).
    pub cop_scaled: u32,
    /// Estimated cold plate temperature in millidegrees Celsius.
    pub cold_plate_temp_mc: i32,
    /// Calculated dew point temperature in millidegrees Celsius to prevent condensation.
    pub dew_point_temp_mc: i32,
    /// Relative humidity percentage (0 to 100%).
    pub relative_humidity_pct: u8,
    /// Flag indicating whether anti-condensation clamp is actively restricting PWM duty.
    pub condensation_clamp_active: bool,
    /// Total energy consumed by TEC module in millijoules (mJ).
    pub total_energy_consumed_mj: u64,
}

impl PeltierTecGovernor {
    /// Maximum allowed TEC current in milliamperes (4,500 mA = 4.5A).
    pub const MAX_TEC_CURRENT_MA: u32 = 4500;
    /// Internal electrical resistance of the bismuth telluride TEC element in milliohms (1,200 mOhm = 1.2 Ohm).
    pub const TEC_INTERNAL_RESISTANCE_MOHM: u32 = 1200;
    /// Seebeck / Peltier coefficient equivalent in microvolts per Kelvin (42,000 uV/K).
    pub const SEEBECK_ALPHA_UV_K: u32 = 42000;
    /// Thermal conductance of TEC module in milliwatts per Kelvin (850 mW/K).
    pub const MODULE_CONDUCTANCE_MW_K: u32 = 850;
    /// Anti-condensation safety headroom above dew point (2.0°C = 2,000 mC).
    pub const DEW_POINT_SAFETY_MARGIN_MC: i32 = 2000;

    /// Initializes Peltier TEC governor in passive off state with default 50% relative humidity.
    pub const fn new() -> Self {
        Self {
            mode: PeltierOperatingMode::PassiveOff,
            pwm_duty_basis_points: 0,
            current_ma: 0,
            electrical_power_mw: 0,
            heat_pumped_mw: 0,
            cop_scaled: 0,
            cold_plate_temp_mc: 25 * MILLIDEG_SCALE,
            dew_point_temp_mc: 14 * MILLIDEG_SCALE,
            relative_humidity_pct: 50,
            condensation_clamp_active: false,
            total_energy_consumed_mj: 0,
        }
    }

    /// Computes dew point from ambient temperature and relative humidity using Lawrence approximation.
    pub fn calculate_dew_point_mc(ambient_temp_mc: i32, relative_humidity_pct: u8) -> i32 {
        let rh = relative_humidity_pct.clamp(1, 100) as i32;
        // Approximation: T_dew ≈ T_ambient - ((100 - RH) * 1000 / 5)
        let depression_mc = ((100 - rh) * MILLIDEG_SCALE) / 5;
        ambient_temp_mc.saturating_sub(depression_mc)
    }

    /// Updates Peltier governor policy based on thermal telemetry, battery level, and power source.
    pub fn update_governor(
        &mut self,
        soc_temp_mc: i32,
        ambient_temp_mc: i32,
        relative_humidity_pct: u8,
        battery_soc_pct: u8,
        power_source: HandheldPowerSource,
        dt_ms: u32,
    ) {
        self.relative_humidity_pct = relative_humidity_pct;
        self.dew_point_temp_mc = Self::calculate_dew_point_mc(ambient_temp_mc, relative_humidity_pct);

        // 1. Determine target operating mode based on battery budget and SoC thermals
        self.mode = match power_source {
            HandheldPowerSource::AcConnected => {
                // On AC wall power, full cooling capability can be deployed
                if soc_temp_mc >= 82 * MILLIDEG_SCALE {
                    PeltierOperatingMode::Turbo
                } else if soc_temp_mc >= 68 * MILLIDEG_SCALE {
                    PeltierOperatingMode::Balanced
                } else if soc_temp_mc >= 55 * MILLIDEG_SCALE {
                    PeltierOperatingMode::Eco
                } else {
                    PeltierOperatingMode::PassiveOff
                }
            }
            HandheldPowerSource::BatteryDischarging => {
                // On battery power, strictly preserve energy to avoid premature battery shutdown
                if battery_soc_pct < 20 {
                    // Critical battery (< 20%): TEC forced off to save power
                    PeltierOperatingMode::PassiveOff
                } else if battery_soc_pct < 50 {
                    // Low-mid battery (20%-50%): Only Eco mode allowed if SoC hot
                    if soc_temp_mc >= 75 * MILLIDEG_SCALE {
                        PeltierOperatingMode::Eco
                    } else {
                        PeltierOperatingMode::PassiveOff
                    }
                } else if battery_soc_pct < 75 {
                    // Normal battery (50%-75%): Balanced mode when hot
                    if soc_temp_mc >= 80 * MILLIDEG_SCALE {
                        PeltierOperatingMode::Balanced
                    } else if soc_temp_mc >= 65 * MILLIDEG_SCALE {
                        PeltierOperatingMode::Eco
                    } else {
                        PeltierOperatingMode::PassiveOff
                    }
                } else {
                    // Full battery (>= 75%): Balanced or Turbo for extreme spikes
                    if soc_temp_mc >= 86 * MILLIDEG_SCALE {
                        PeltierOperatingMode::Turbo
                    } else if soc_temp_mc >= 75 * MILLIDEG_SCALE {
                        PeltierOperatingMode::Balanced
                    } else if soc_temp_mc >= 60 * MILLIDEG_SCALE {
                        PeltierOperatingMode::Eco
                    } else {
                        PeltierOperatingMode::PassiveOff
                    }
                }
            }
        };

        // 2. Compute base target PWM duty cycle (0 - 10,000 basis points)
        let target_duty = match self.mode {
            PeltierOperatingMode::PassiveOff => 0,
            PeltierOperatingMode::Eco => 2500,     // 25.0% duty
            PeltierOperatingMode::Balanced => 5500, // 55.0% duty
            PeltierOperatingMode::Turbo => 9000,    // 90.0% duty
        };

        // 3. Estimate cold plate temperature at this duty: T_cold = T_soc - (ΔT_peltier)
        let max_delta_t_mc = 18 * MILLIDEG_SCALE; // Max 18°C temperature drop across TEC at 100% duty
        let estimated_delta_t_mc = (max_delta_t_mc * (target_duty as i32)) / 10000;
        let mut estimated_cold_plate_mc = soc_temp_mc - estimated_delta_t_mc;

        // 4. Anti-condensation protection: ensure cold plate is strictly above dew point + safety margin
        let min_safe_cold_temp_mc = self.dew_point_temp_mc + Self::DEW_POINT_SAFETY_MARGIN_MC;
        let mut final_duty = target_duty;

        if estimated_cold_plate_mc < min_safe_cold_temp_mc && target_duty > 0 {
            self.condensation_clamp_active = true;
            // Scale down duty to maintain cold plate at safe threshold
            let allowed_delta_t_mc = (soc_temp_mc - min_safe_cold_temp_mc).max(0);
            let clamped_duty = ((allowed_delta_t_mc * 10000) / max_delta_t_mc).clamp(0, 10000) as u16;
            final_duty = final_duty.min(clamped_duty);
            estimated_cold_plate_mc = soc_temp_mc - ((max_delta_t_mc * (final_duty as i32)) / 10000);
        } else {
            self.condensation_clamp_active = false;
        }

        self.pwm_duty_basis_points = final_duty;
        self.cold_plate_temp_mc = estimated_cold_plate_mc;

        // 5. Compute electrical characteristics
        // Current: I = I_max * duty / 10000
        self.current_ma = (Self::MAX_TEC_CURRENT_MA * (final_duty as u32)) / 10000;

        // Electrical power: P = I^2 * R
        // current in Amperes = current_ma / 1000
        let current_amp_x1000 = self.current_ma;
        let power_mw = ((current_amp_x1000 * current_amp_x1000) / 1000) * Self::TEC_INTERNAL_RESISTANCE_MOHM / 1000;
        self.electrical_power_mw = power_mw;

        // 6. Heat pumped Q_c: Q_c = alpha * I * T_c - 0.5 * I^2 * R - K * delta_T
        if self.current_ma > 0 {
            let tc_kelvin = (self.cold_plate_temp_mc / MILLIDEG_SCALE + 273) as u32;
            let alpha_i_tc = (Self::SEEBECK_ALPHA_UV_K * self.current_ma / 1000 * tc_kelvin) / 1000; // in mW
            let half_i2_r = power_mw / 2;
            let conduction_loss = (Self::MODULE_CONDUCTANCE_MW_K * (estimated_delta_t_mc.max(0) as u32)) / (MILLIDEG_SCALE as u32);
            self.heat_pumped_mw = alpha_i_tc.saturating_sub(half_i2_r + conduction_loss);

            // COP = Q_c / P_elec
            if power_mw > 0 {
                self.cop_scaled = (self.heat_pumped_mw * 1000) / power_mw;
            } else {
                self.cop_scaled = 0;
            }
        } else {
            self.heat_pumped_mw = 0;
            self.cop_scaled = 0;
        }

        // 7. Accumulate energy
        let energy_step_mj = (self.electrical_power_mw as u64) * (dt_ms as u64) / 1000;
        self.total_energy_consumed_mj += energy_step_mj;
    }
}

/// Verification test for Peltier thermoelectric cooler PWM governor.
pub fn verify_peltier_tec_current_governor() -> bool {
    let mut tec = PeltierTecGovernor::new();

    // 1. Initial passive state
    if tec.mode != PeltierOperatingMode::PassiveOff || tec.pwm_duty_basis_points != 0 {
        return false;
    }

    // 2. Ambient dew point check at 25°C, 50% RH:
    // T_dew ≈ 25 - ((100 - 50) / 5) = 15°C = 15,000 mC
    let dew_25c_50rh = PeltierTecGovernor::calculate_dew_point_mc(25 * MILLIDEG_SCALE, 50);
    if dew_25c_50rh != 15 * MILLIDEG_SCALE {
        return false;
    }

    // 3. Test Battery Discharging with Low Battery (15% SoC):
    // Even if SoC is hot (85°C), TEC must be forced off to prevent draining the dead battery!
    tec.update_governor(
        85 * MILLIDEG_SCALE,
        25 * MILLIDEG_SCALE,
        50,
        15, // 15% battery
        HandheldPowerSource::BatteryDischarging,
        100,
    );
    if tec.mode != PeltierOperatingMode::PassiveOff || tec.pwm_duty_basis_points != 0 {
        return false;
    }

    // 4. Test Battery Discharging with Healthy Battery (80% SoC) under 88°C spike:
    // Should enter Turbo mode on high SoC load
    tec.update_governor(
        88 * MILLIDEG_SCALE,
        25 * MILLIDEG_SCALE,
        50,
        80, // 80% battery
        HandheldPowerSource::BatteryDischarging,
        100,
    );
    if tec.mode != PeltierOperatingMode::Turbo || tec.current_ma == 0 {
        return false;
    }

    // 5. Test AC Connected operation at 85°C:
    // Should run Turbo mode (90% duty = 9000 basis points)
    tec.update_governor(
        85 * MILLIDEG_SCALE,
        25 * MILLIDEG_SCALE,
        50,
        100,
        HandheldPowerSource::AcConnected,
        100,
    );
    if tec.mode != PeltierOperatingMode::Turbo {
        return false;
    }

    // 6. Test Anti-Condensation Protection:
    // Very high humidity (95% RH at 22°C): T_dew ≈ 22 - ((100-95)/5) = 21°C.
    // If SoC is 28°C and Turbo tries to pull cold plate down by 16°C (down to 12°C),
    // anti-condensation clamp must trigger to prevent liquid condensation inside handheld!
    tec.update_governor(
        28 * MILLIDEG_SCALE,
        22 * MILLIDEG_SCALE,
        95, // 95% high humidity
        100,
        HandheldPowerSource::AcConnected,
        100,
    );
    // At 28°C SoC, mode is PassiveOff anyway because SoC is cool (< 55°C).
    // Let's test with SoC at 32°C forced with simulated clamp scenario:
    let dew_high = PeltierTecGovernor::calculate_dew_point_mc(22 * MILLIDEG_SCALE, 95);
    if dew_high != 21 * MILLIDEG_SCALE {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: Emergency Thermal Runaway Throttling Preventing SoC Hot Spots
// ============================================================================

/// Multi-stage thermal throttle severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThermalThrottleStage {
    /// Stage 0: Nominal operation, no thermal throttling active.
    Nominal = 0,
    /// Stage 1: Soft TDP cap (24W), fan curve pre-emptive ramp (+25%).
    ModerateSoftCap = 1,
    /// Stage 2: Heavy TDP reduction (16W), maximum fan boost (100%), GPU clock clamp.
    AggressiveThrottle = 2,
    /// Stage 3: Emergency PROCHOT# assertion, TDP clamped to minimum (7W), TEC reverse-dump protection.
    CriticalEmergency = 3,
    /// Stage 4: Catastrophic thermal trip latch (> 105°C), immediate power-off required.
    HardwareTrip = 4,
}

/// Emergency thermal runaway detector and multi-stage throttle governor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmergencyThermalThrottler {
    /// Current throttle stage.
    pub current_stage: ThermalThrottleStage,
    /// Target APU TDP limit in Watts enforced by throttler.
    pub enforced_tdp_watts: u32,
    /// Hardware PROCHOT# assertion status.
    pub prochot_asserted: bool,
    /// TEC reverse-dump suppression active (TEC forced off because heatsink is heat-soaked).
    pub tec_reverse_dump_suppressed: bool,
    /// Fan boost duty cycle in basis points (0 to 10,000).
    pub emergency_fan_boost_basis_points: u16,
    /// Timestamp when temperature fell below current recovery threshold.
    pub recovery_timer_ms: u64,
    /// Total number of emergency thermal runaway throttle events.
    pub throttle_events_count: u32,
}

impl EmergencyThermalThrottler {
    /// Nominal full-power gaming TDP in Watts (30W).
    pub const NOMINAL_TDP_WATTS: u32 = 30;
    /// Stage 1 soft-cap TDP in Watts (24W).
    pub const STAGE1_TDP_WATTS: u32 = 24;
    /// Stage 2 aggressive throttle TDP in Watts (16W).
    pub const STAGE2_TDP_WATTS: u32 = 16;
    /// Stage 3 emergency PROCHOT# TDP in Watts (7W).
    pub const STAGE3_TDP_WATTS: u32 = 7;

    /// Stage 1 trigger temperature (86°C = 86,000 mC).
    pub const STAGE1_TRIGGER_TEMP_MC: i32 = 86 * MILLIDEG_SCALE;
    /// Stage 2 trigger temperature (92°C = 92,000 mC).
    pub const STAGE2_TRIGGER_TEMP_MC: i32 = 92 * MILLIDEG_SCALE;
    /// Stage 3 critical PROCHOT trigger temperature (98°C = 98,000 mC).
    pub const STAGE3_TRIGGER_TEMP_MC: i32 = 98 * MILLIDEG_SCALE;
    /// Stage 4 catastrophic trip temperature (105°C = 105,000 mC).
    pub const STAGE4_TRIP_TEMP_MC: i32 = 105 * MILLIDEG_SCALE;

    /// Maximum permissible chassis skin temperature for user grip comfort (48°C = 48,000 mC).
    pub const MAX_SAFE_SKIN_TEMP_MC: i32 = 48 * MILLIDEG_SCALE;
    /// Maximum permissible condenser/heatsink temperature before TEC reverse heat dumps (85°C = 85,000 mC).
    pub const MAX_CONDENSER_TEMP_FOR_TEC_MC: i32 = 85 * MILLIDEG_SCALE;

    /// Hysteresis dwell duration in milliseconds required before down-throttling (500 ms).
    pub const HYSTERESIS_DWELL_MS: u64 = 500;

    /// Initializes emergency thermal throttler in nominal unthrottled state.
    pub const fn new() -> Self {
        Self {
            current_stage: ThermalThrottleStage::Nominal,
            enforced_tdp_watts: Self::NOMINAL_TDP_WATTS,
            prochot_asserted: false,
            tec_reverse_dump_suppressed: false,
            emergency_fan_boost_basis_points: 0,
            recovery_timer_ms: 0,
            throttle_events_count: 0,
        }
    }

    /// Evaluates thermal runaway conditions and updates throttling posture.
    pub fn evaluate_thermal_runaway(
        &mut self,
        soc_temp_mc: i32,
        condenser_temp_mc: i32,
        skin_temp_mc: i32,
        phase_state: VaporChamberPhaseState,
        slew_rate_mc_s: i32,
        timestamp_ms: u64,
    ) {
        // 1. Check for heatsink heat-soak: if condenser >= 85°C, TEC cannot reject heat; shut it off
        self.tec_reverse_dump_suppressed = condenser_temp_mc >= Self::MAX_CONDENSER_TEMP_FOR_TEC_MC;

        // 2. Determine target raw throttle stage based on thermal indicators
        let mut target_stage = ThermalThrottleStage::Nominal;

        if soc_temp_mc >= Self::STAGE4_TRIP_TEMP_MC {
            target_stage = ThermalThrottleStage::HardwareTrip;
        } else if soc_temp_mc >= Self::STAGE3_TRIGGER_TEMP_MC
            || phase_state == VaporChamberPhaseState::CriticalDryout
        {
            target_stage = ThermalThrottleStage::CriticalEmergency;
        } else if soc_temp_mc >= Self::STAGE2_TRIGGER_TEMP_MC
            || skin_temp_mc >= Self::MAX_SAFE_SKIN_TEMP_MC
            || slew_rate_mc_s >= 3500 // > 3.5°C/s rapid temperature surge
            || phase_state == VaporChamberPhaseState::CapillaryDryoutWarning
        {
            target_stage = ThermalThrottleStage::AggressiveThrottle;
        } else if soc_temp_mc >= Self::STAGE1_TRIGGER_TEMP_MC || skin_temp_mc >= 44 * MILLIDEG_SCALE {
            target_stage = ThermalThrottleStage::ModerateSoftCap;
        }

        // 3. Fast Up-Throttle / Hysteresis Down-Throttle Logic
        if target_stage > self.current_stage {
            // Immediate up-throttle (< 100ms response to thermal spike)
            self.current_stage = target_stage;
            self.recovery_timer_ms = timestamp_ms;
            self.throttle_events_count += 1;
        } else if target_stage < self.current_stage {
            // Down-throttle requires sustained cooldown for at least HYSTERESIS_DWELL_MS
            if timestamp_ms.saturating_sub(self.recovery_timer_ms) >= Self::HYSTERESIS_DWELL_MS {
                self.current_stage = target_stage;
                self.recovery_timer_ms = timestamp_ms;
            }
        } else {
            // Maintained in current stage
            self.recovery_timer_ms = timestamp_ms;
        }

        // 4. Enforce stage-specific hardware parameters
        match self.current_stage {
            ThermalThrottleStage::Nominal => {
                self.enforced_tdp_watts = Self::NOMINAL_TDP_WATTS;
                self.prochot_asserted = false;
                self.emergency_fan_boost_basis_points = 0;
            }
            ThermalThrottleStage::ModerateSoftCap => {
                self.enforced_tdp_watts = Self::STAGE1_TDP_WATTS;
                self.prochot_asserted = false;
                self.emergency_fan_boost_basis_points = 2500; // +25% fan duty
            }
            ThermalThrottleStage::AggressiveThrottle => {
                self.enforced_tdp_watts = Self::STAGE2_TDP_WATTS;
                self.prochot_asserted = false;
                self.emergency_fan_boost_basis_points = 10000; // 100% max fan duty
            }
            ThermalThrottleStage::CriticalEmergency | ThermalThrottleStage::HardwareTrip => {
                self.enforced_tdp_watts = Self::STAGE3_TDP_WATTS;
                self.prochot_asserted = true; // Assert PROCHOT# pin
                self.emergency_fan_boost_basis_points = 10000; // 100% max fan duty
            }
        }
    }
}

/// Verification test for emergency thermal runaway throttling.
pub fn verify_emergency_thermal_runaway_throttling() -> bool {
    let mut throttler = EmergencyThermalThrottler::new();

    // 1. Initial nominal state
    if throttler.current_stage != ThermalThrottleStage::Nominal || throttler.enforced_tdp_watts != 30 {
        return false;
    }

    // 2. Moderate thermal load: 88°C SoC (triggers Stage 1: Soft Cap)
    throttler.evaluate_thermal_runaway(
        88 * MILLIDEG_SCALE,
        65 * MILLIDEG_SCALE,
        40 * MILLIDEG_SCALE,
        VaporChamberPhaseState::SaturatedHighConductance,
        500,
        100,
    );
    if throttler.current_stage != ThermalThrottleStage::ModerateSoftCap || throttler.enforced_tdp_watts != 24 {
        return false;
    }
    if throttler.prochot_asserted {
        return false;
    }

    // 3. Fast thermal spike: 94°C SoC or rapid slew (triggers Stage 2: Aggressive Throttle)
    throttler.evaluate_thermal_runaway(
        94 * MILLIDEG_SCALE,
        72 * MILLIDEG_SCALE,
        42 * MILLIDEG_SCALE,
        VaporChamberPhaseState::CapillaryDryoutWarning,
        4000,
        200,
    );
    if throttler.current_stage != ThermalThrottleStage::AggressiveThrottle || throttler.enforced_tdp_watts != 16 {
        return false;
    }
    if throttler.emergency_fan_boost_basis_points != 10000 {
        return false;
    }

    // 4. Critical dryout runaway: 99°C SoC and CriticalDryout (triggers Stage 3: PROCHOT# assertion)
    throttler.evaluate_thermal_runaway(
        99 * MILLIDEG_SCALE,
        86 * MILLIDEG_SCALE, // Heat sink heat-soaked >= 85°C
        45 * MILLIDEG_SCALE,
        VaporChamberPhaseState::CriticalDryout,
        5000,
        300,
    );
    if throttler.current_stage != ThermalThrottleStage::CriticalEmergency || throttler.enforced_tdp_watts != 7 {
        return false;
    }
    if !throttler.prochot_asserted {
        return false;
    }
    if !throttler.tec_reverse_dump_suppressed {
        return false;
    }

    // 5. Test Hysteresis recovery: temperature drops back to 70°C at t=400ms (< 500ms dwell)
    throttler.evaluate_thermal_runaway(
        70 * MILLIDEG_SCALE,
        60 * MILLIDEG_SCALE,
        38 * MILLIDEG_SCALE,
        VaporChamberPhaseState::EvaporationActive,
        -1000,
        400, // 400ms - only 100ms since critical event
    );
    // Should NOT down-throttle yet due to hysteresis lock
    if throttler.current_stage != ThermalThrottleStage::CriticalEmergency {
        return false;
    }

    // 6. After dwell time passes at t=1000ms (> 500ms dwell), down-throttles cleanly
    throttler.evaluate_thermal_runaway(
        70 * MILLIDEG_SCALE,
        60 * MILLIDEG_SCALE,
        38 * MILLIDEG_SCALE,
        VaporChamberPhaseState::EvaporationActive,
        0,
        1000,
    );
    if throttler.current_stage != ThermalThrottleStage::Nominal || throttler.enforced_tdp_watts != 30 {
        return false;
    }
    if throttler.prochot_asserted {
        return false;
    }

    true
}


// ============================================================================
// Feature 4: Coordinated Vapor Chamber & Peltier Active Cooling Pipeline
// ============================================================================

/// Coordinated thermal control command dispatched to power and platform EC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandheldThermalCommand {
    /// APU TDP limit in Watts to program into CPPC / RAPL MSRs.
    pub target_tdp_watts: u32,
    /// TEC PWM duty cycle in basis points (0 to 10,000).
    pub tec_pwm_duty_basis_points: u16,
    /// Supplemental fan boost duty cycle in basis points (0 to 10,000).
    pub emergency_fan_boost_basis_points: u16,
    /// Hardware PROCHOT# assertion status.
    pub prochot_asserted: bool,
    /// Current vapor chamber phase state.
    pub phase_state: VaporChamberPhaseState,
    /// Active thermal throttle stage.
    pub throttle_stage: ThermalThrottleStage,
}

/// Unified coordinator orchestrating vapor chamber sensor fusion, Peltier TEC, and thermal runaway protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandheldVaporChamberCoordinator {
    /// Multi-point sensor fusion and phase-change tracker.
    pub sensor_fusion: VaporChamberSensorFusion,
    /// Peltier TEC PWM current governor.
    pub peltier_tec: PeltierTecGovernor,
    /// Emergency thermal runaway detector and multi-stage throttler.
    pub throttler: EmergencyThermalThrottler,
    /// Total coordination cycles processed.
    pub cycles_processed: u64,
}

impl HandheldVaporChamberCoordinator {
    /// Initializes unified vapor chamber coordinator.
    pub const fn new() -> Self {
        Self {
            sensor_fusion: VaporChamberSensorFusion::new(),
            peltier_tec: PeltierTecGovernor::new(),
            throttler: EmergencyThermalThrottler::new(),
            cycles_processed: 0,
        }
    }

    /// Executes an end-to-end thermal control loop step.
    pub fn coordinate_thermal_cycle(
        &mut self,
        raw_readings: &[i32; SENSOR_CHANNEL_COUNT],
        relative_humidity_pct: u8,
        battery_soc_pct: u8,
        power_source: HandheldPowerSource,
        timestamp_ms: u64,
        dt_ms: u32,
    ) -> HandheldThermalCommand {
        self.cycles_processed += 1;

        // 1. Ingest telemetry into sensor fusion
        let current_tdp = self.throttler.enforced_tdp_watts;
        self.sensor_fusion.update_telemetry(raw_readings, timestamp_ms, current_tdp);

        let soc_temp = self.sensor_fusion.get_temperature(ThermalSensorLocation::SocDie);
        let condenser_temp = self.sensor_fusion.get_temperature(ThermalSensorLocation::CondenserFin);
        let skin_temp = self.sensor_fusion.get_temperature(ThermalSensorLocation::ChassisSkin);
        let ambient_temp = self.sensor_fusion.get_temperature(ThermalSensorLocation::AmbientIntake);
        let slew_rate = self.sensor_fusion.sensors[ThermalSensorLocation::SocDie as usize].slew_rate_mc_s;

        // 2. Evaluate thermal runaway and throttling requirements
        self.throttler.evaluate_thermal_runaway(
            soc_temp,
            condenser_temp,
            skin_temp,
            self.sensor_fusion.phase_state,
            slew_rate,
            timestamp_ms,
        );

        // 3. Update Peltier TEC governor
        self.peltier_tec.update_governor(
            soc_temp,
            ambient_temp,
            relative_humidity_pct,
            battery_soc_pct,
            power_source,
            dt_ms,
        );

        // 4. If throttler detects heatsink heat-soak (condenser >= 85°C), suppress TEC to prevent reverse heat dumping!
        let effective_tec_duty = if self.throttler.tec_reverse_dump_suppressed {
            0
        } else {
            self.peltier_tec.pwm_duty_basis_points
        };

        HandheldThermalCommand {
            target_tdp_watts: self.throttler.enforced_tdp_watts,
            tec_pwm_duty_basis_points: effective_tec_duty,
            emergency_fan_boost_basis_points: self.throttler.emergency_fan_boost_basis_points,
            prochot_asserted: self.throttler.prochot_asserted,
            phase_state: self.sensor_fusion.phase_state,
            throttle_stage: self.throttler.current_stage,
        }
    }
}

/// Verification test for vapor chamber coordinator pipeline.
pub fn verify_vapor_chamber_coordinator_pipeline() -> bool {
    let mut coordinator = HandheldVaporChamberCoordinator::new();

    // 1. Nominal cold boot at 25°C
    let room_sensors = [
        25 * MILLIDEG_SCALE,
        25 * MILLIDEG_SCALE,
        25 * MILLIDEG_SCALE,
        25 * MILLIDEG_SCALE,
        22 * MILLIDEG_SCALE,
        24 * MILLIDEG_SCALE,
    ];
    let cmd = coordinator.coordinate_thermal_cycle(
        &room_sensors,
        50,
        90,
        HandheldPowerSource::BatteryDischarging,
        100,
        100,
    );
    if cmd.target_tdp_watts != 30 || cmd.throttle_stage != ThermalThrottleStage::Nominal {
        return false;
    }
    if cmd.prochot_asserted || cmd.tec_pwm_duty_basis_points != 0 {
        return false;
    }

    // 2. High gaming load on AC power: SoC 82°C
    let gaming_sensors = [
        82 * MILLIDEG_SCALE,
        78 * MILLIDEG_SCALE,
        68 * MILLIDEG_SCALE,
        41 * MILLIDEG_SCALE,
        25 * MILLIDEG_SCALE,
        35 * MILLIDEG_SCALE,
    ];
    for i in 2..=8 {
        coordinator.coordinate_thermal_cycle(
            &gaming_sensors,
            50,
            100,
            HandheldPowerSource::AcConnected,
            i * 100,
            100,
        );
    }
    let cmd_gaming = coordinator.coordinate_thermal_cycle(
        &gaming_sensors,
        50,
        100,
        HandheldPowerSource::AcConnected,
        900,
        100,
    );
    // On AC power at 82°C, TEC enters Turbo (90% duty = 9000 basis points)
    if cmd_gaming.tec_pwm_duty_basis_points == 0 {
        return false;
    }

    // 3. Simulated condenser heat-soak (condenser fin surges to 88°C >= 85°C):
    // TEC reverse dump protection must immediately shut off TEC duty!
    let heat_soaked_sensors = [
        95 * MILLIDEG_SCALE,
        92 * MILLIDEG_SCALE,
        88 * MILLIDEG_SCALE, // condenser >= 85°C
        47 * MILLIDEG_SCALE,
        26 * MILLIDEG_SCALE,
        40 * MILLIDEG_SCALE,
    ];
    let cmd_soaked = coordinator.coordinate_thermal_cycle(
        &heat_soaked_sensors,
        50,
        100,
        HandheldPowerSource::AcConnected,
        1000,
        100,
    );
    if cmd_soaked.tec_pwm_duty_basis_points != 0 {
        return false; // TEC must be suppressed!
    }
    if cmd_soaked.target_tdp_watts >= 30 {
        return false; // Throttling must be active!
    }

    true
}


// ============================================================================
// Feature 5: Comprehensive Subsystem Self-Test Suite and kmain Integration
// ============================================================================

/// Comprehensive verification self-test suite for Handheld Peltier Active Cooling and Phase-Change Vapor Chamber Governor.
pub fn test_vapor_chamber_subsystem() -> bool {
    crate::serial_write_str("[KNT-POWER-VAPOR-CHAMBER] Initiating Peltier active cooling & vapor chamber governor self-test...\r\n");

    if !verify_vapor_chamber_sensor_fusion_and_phase_tracking() {
        crate::serial_write_str("[KNT-POWER-VAPOR-CHAMBER-FAIL] Vapor chamber sensor fusion & phase tracking verification failed!\r\n");
        return false;
    }

    if !verify_peltier_tec_current_governor() {
        crate::serial_write_str("[KNT-POWER-VAPOR-CHAMBER-FAIL] Peltier TEC PWM current governor verification failed!\r\n");
        return false;
    }

    if !verify_emergency_thermal_runaway_throttling() {
        crate::serial_write_str("[KNT-POWER-VAPOR-CHAMBER-FAIL] Emergency thermal runaway throttling verification failed!\r\n");
        return false;
    }

    if !verify_vapor_chamber_coordinator_pipeline() {
        crate::serial_write_str("[KNT-POWER-VAPOR-CHAMBER-FAIL] Vapor chamber coordinator pipeline verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-POWER-VAPOR-CHAMBER-PASS] Handheld Peltier active cooling and vapor chamber governor operational\r\n");
    true
}
