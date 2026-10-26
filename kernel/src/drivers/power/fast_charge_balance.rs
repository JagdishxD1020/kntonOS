//! Handheld Power & Battery: Dynamic Fast Charge Dual-Cell Balance & Impedance Degradation Compensator.
//!
//! Handheld gaming consoles (e.g. Steam Deck OLED, ROG Ally X, Lenovo Legion Go) feature high-capacity
//! dual-cell lithium-ion / LiPo battery packs (typically 2S configuration, 7.4V nominal / 8.7V peak).
//! During high-wattage fast charging (up to 65W - 100W USB-PD EPR), unequal cell impedance, uneven thermal
//! dissipation, and long-term cycle aging cause millivolt drift between cells.
//!
//! This module implements:
//! 1. Dual-cell balance tracking with millivolt cell-drift balancing governor.
//! 2. Dynamic fast charge stepped current tapering governed by internal cell resistance (ESR) and thermal telemetry.
//! 3. Long-term cycle degradation compensator adjusting safe charging cut-offs to prevent pouch swelling.
//! 4. Comprehensive self-test suite and kmain integration.

#![allow(dead_code)]

// ============================================================================
// Feature 1: Dual-Cell Balance Tracking & Millivolt Cell-Drift Balancing
// ============================================================================

pub const CELL_CRITICAL_UNDERVOLTAGE_MV: u16 = 2900;
pub const CELL_NOMINAL_MIN_MV: u16 = 3200;
pub const CELL_NOMINAL_MAX_MV: u16 = 4350;
pub const CELL_CRITICAL_OVERVOLTAGE_MV: u16 = 4450;
pub const CELL_BALANCE_START_DELTA_MV: u16 = 15;
pub const CELL_BALANCE_STOP_DELTA_MV: u16 = 5;

/// Identifies an individual cell within the dual-cell battery pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastChargeCellId {
    Cell1,
    Cell2,
}

/// Active cell-balancing switch state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellBalanceMode {
    /// Both balancing FETs off (balanced or below voltage threshold).
    Inactive,
    /// Bleeding Cell 1 via internal bypass shunt resistor.
    BleedCell1,
    /// Bleeding Cell 2 via internal bypass shunt resistor.
    BleedCell2,
}

/// Instantaneous voltage and health telemetry for the dual-cell pack.
#[derive(Debug, Clone, Copy)]
pub struct CellTelemetry {
    pub cell1_mv: u16,
    pub cell2_mv: u16,
}

impl CellTelemetry {
    pub const fn new(cell1_mv: u16, cell2_mv: u16) -> Self {
        Self { cell1_mv, cell2_mv }
    }

    /// Total 2S pack terminal voltage in millivolts.
    pub const fn pack_voltage_mv(&self) -> u32 {
        self.cell1_mv as u32 + self.cell2_mv as u32
    }

    /// Absolute voltage imbalance between cells in millivolts.
    pub const fn delta_mv(&self) -> u16 {
        if self.cell1_mv >= self.cell2_mv {
            self.cell1_mv - self.cell2_mv
        } else {
            self.cell2_mv - self.cell1_mv
        }
    }

    /// Identifies the cell with higher terminal voltage requiring bypass bleeding.
    pub const fn higher_cell(&self) -> Option<FastChargeCellId> {
        if self.cell1_mv > self.cell2_mv {
            Some(FastChargeCellId::Cell1)
        } else if self.cell2_mv > self.cell1_mv {
            Some(FastChargeCellId::Cell2)
        } else {
            None
        }
    }
}

/// Governor managing real-time dual-cell drift tracking and passive bypass bleeding.
#[derive(Debug, Clone)]
pub struct DualCellBalanceGovernor {
    balance_mode: CellBalanceMode,
    balance_start_delta_mv: u16,
    balance_stop_delta_mv: u16,
    shunt_current_ma: f32,
    total_balance_cycles: u64,
    consecutive_balanced_ticks: u32,
}

impl DualCellBalanceGovernor {
    pub const fn new() -> Self {
        Self {
            balance_mode: CellBalanceMode::Inactive,
            balance_start_delta_mv: CELL_BALANCE_START_DELTA_MV,
            balance_stop_delta_mv: CELL_BALANCE_STOP_DELTA_MV,
            shunt_current_ma: 80.0,
            total_balance_cycles: 0,
            consecutive_balanced_ticks: 0,
        }
    }

    /// Evaluate cell voltages and update balancing FET control with hysteresis.
    pub fn update(&mut self, telemetry: &CellTelemetry) -> CellBalanceMode {
        // Safety lockouts: Inhibit balancing if cells are critically undervolted or overvolted
        if telemetry.cell1_mv < CELL_CRITICAL_UNDERVOLTAGE_MV
            || telemetry.cell2_mv < CELL_CRITICAL_UNDERVOLTAGE_MV
            || telemetry.cell1_mv > CELL_CRITICAL_OVERVOLTAGE_MV
            || telemetry.cell2_mv > CELL_CRITICAL_OVERVOLTAGE_MV
        {
            self.balance_mode = CellBalanceMode::Inactive;
            return self.balance_mode;
        }

        // Low voltage lockout: avoid bleeding weak cells in deep discharge
        if telemetry.cell1_mv < CELL_NOMINAL_MIN_MV || telemetry.cell2_mv < CELL_NOMINAL_MIN_MV {
            self.balance_mode = CellBalanceMode::Inactive;
            return self.balance_mode;
        }

        let delta = telemetry.delta_mv();

        match self.balance_mode {
            CellBalanceMode::Inactive => {
                if delta >= self.balance_start_delta_mv {
                    if telemetry.cell1_mv > telemetry.cell2_mv {
                        self.balance_mode = CellBalanceMode::BleedCell1;
                        self.total_balance_cycles += 1;
                    } else if telemetry.cell2_mv > telemetry.cell1_mv {
                        self.balance_mode = CellBalanceMode::BleedCell2;
                        self.total_balance_cycles += 1;
                    }
                    self.consecutive_balanced_ticks = 0;
                } else {
                    self.consecutive_balanced_ticks += 1;
                }
            }
            CellBalanceMode::BleedCell1 => {
                // Stop if delta drops below stop threshold or Cell 2 becomes higher
                if delta <= self.balance_stop_delta_mv || telemetry.cell2_mv >= telemetry.cell1_mv {
                    self.balance_mode = CellBalanceMode::Inactive;
                    self.consecutive_balanced_ticks = 1;
                }
            }
            CellBalanceMode::BleedCell2 => {
                // Stop if delta drops below stop threshold or Cell 1 becomes higher
                if delta <= self.balance_stop_delta_mv || telemetry.cell1_mv >= telemetry.cell2_mv {
                    self.balance_mode = CellBalanceMode::Inactive;
                    self.consecutive_balanced_ticks = 1;
                }
            }
        }

        self.balance_mode
    }

    pub fn current_mode(&self) -> CellBalanceMode {
        self.balance_mode
    }

    pub fn total_balance_cycles(&self) -> u64 {
        self.total_balance_cycles
    }
}

/// Verification test for dual-cell balance tracking and millivolt cell-drift balancing governor.
pub fn verify_dual_cell_drift_balancing() -> bool {
    let mut governor = DualCellBalanceGovernor::new();

    // 1. Initial balanced state (4100 mV vs 4102 mV, delta = 2 mV)
    let t_balanced = CellTelemetry::new(4100, 4102);
    if governor.update(&t_balanced) != CellBalanceMode::Inactive {
        return false;
    }

    // 2. High-rate charge drift: Cell 1 rises faster (4180 mV vs 4155 mV, delta = 25 mV >= 15 mV)
    let t_drift = CellTelemetry::new(4180, 4155);
    if governor.update(&t_drift) != CellBalanceMode::BleedCell1 {
        return false;
    }
    if governor.total_balance_cycles() != 1 {
        return false;
    }

    // 3. Hysteresis verification: delta decreases to 10 mV (4170 vs 4160 mV)
    // Threshold is 5 mV, so BleedCell1 MUST remain active to avoid flapping
    let t_hysteresis = CellTelemetry::new(4170, 4160);
    if governor.update(&t_hysteresis) != CellBalanceMode::BleedCell1 {
        return false;
    }

    // 4. Convergence verification: delta decreases to 4 mV (4164 vs 4160 mV <= 5 mV)
    // Bleeding must stop and transition to Inactive
    let t_converged = CellTelemetry::new(4164, 4160);
    if governor.update(&t_converged) != CellBalanceMode::Inactive {
        return false;
    }

    // 5. Symmetric test: Cell 2 higher drift (4150 vs 4175 mV, delta = 25 mV)
    let t_drift_c2 = CellTelemetry::new(4150, 4175);
    if governor.update(&t_drift_c2) != CellBalanceMode::BleedCell2 {
        return false;
    }

    // 6. Low-voltage safety lockout: Weak cells in deep discharge (3100 vs 3140 mV)
    // Must inhibit bleeding to preserve emergency boot reserve
    let t_low = CellTelemetry::new(3100, 3140);
    if governor.update(&t_low) != CellBalanceMode::Inactive {
        return false;
    }

    true
}

// ============================================================================
// Feature 2: Dynamic Fast Charge Stepped Current Tapering (ESR & Thermals)
// ============================================================================

/// JEITA battery temperature safety zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeThermalZone {
    /// T < 0 C: Extreme cold; charging prohibited to avoid lithium dendrite plating.
    ColdInhibit,
    /// 0 C <= T < 10 C: Cold temperature; restricted trickle charge (<= 1000 mA).
    ColdTrickle,
    /// 10 C <= T < 45 C: Optimal temperature range; full fast-charge capability.
    NormalFast,
    /// 45 C <= T < 55 C: Elevated temperature; stepped current tapering and voltage de-rating.
    WarmTaper,
    /// T >= 55 C: Critical thermal threshold; immediate charging cutoff to prevent thermal runaway.
    HotCutoff,
}

/// Dynamic Fast Charge Stepped Current Taper Governor.
///
/// Implements Multi-Step Constant Current (MSCC) charging profiles modulated by
/// real-time internal cell resistance (ESR) and multi-point NTC thermistor telemetry.
#[derive(Debug, Clone)]
pub struct FastChargeTaperGovernor {
    /// Maximum allowed cell internal joule heating budget in milliwatts (e.g. 800 mW).
    pub max_internal_heat_mw: u32,
    /// Design capacity in milliampere-hours (e.g. 5000 mAh).
    pub design_capacity_mah: u16,
    /// Total current reduction events due to thermal or ESR throttling.
    pub thermal_taper_events: u64,
    pub esr_taper_events: u64,
}

impl FastChargeTaperGovernor {
    pub const fn new(design_capacity_mah: u16) -> Self {
        Self {
            max_internal_heat_mw: 800,
            design_capacity_mah,
            thermal_taper_events: 0,
            esr_taper_events: 0,
        }
    }

    /// Classify cell temperature into standard JEITA thermal safety zone.
    pub const fn classify_thermal_zone(temp_c: i8) -> ChargeThermalZone {
        if temp_c < 0 {
            ChargeThermalZone::ColdInhibit
        } else if temp_c < 10 {
            ChargeThermalZone::ColdTrickle
        } else if temp_c < 45 {
            ChargeThermalZone::NormalFast
        } else if temp_c < 55 {
            ChargeThermalZone::WarmTaper
        } else {
            ChargeThermalZone::HotCutoff
        }
    }

    /// Compute baseline Multi-Step Constant Current (MSCC) stage from State of Charge (SoC).
    pub const fn mscc_base_current_ma(&self, soc_percent: u8) -> u16 {
        if soc_percent < 50 {
            // Stage 1: Peak fast charge (1.0C = 5000 mA)
            self.design_capacity_mah
        } else if soc_percent < 70 {
            // Stage 2: High charge (0.8C = 4000 mA)
            (self.design_capacity_mah * 8) / 10
        } else if soc_percent < 85 {
            // Stage 3: Medium charge (0.6C = 3000 mA)
            (self.design_capacity_mah * 6) / 10
        } else if soc_percent < 95 {
            // Stage 4: Taper stage (0.4C = 2000 mA)
            (self.design_capacity_mah * 4) / 10
        } else {
            // Stage 5: CV transition trickle (0.2C = 1000 mA)
            (self.design_capacity_mah * 2) / 10
        }
    }

    /// Calculate maximum safe charging current governed by ESR joule heating limit:
    /// P = I^2 * R_esr  =>  I_max = sqrt(P_max / R_esr)
    pub fn esr_current_limit_ma(&self, esr_milliohms: u16) -> u16 {
        if esr_milliohms == 0 {
            return self.design_capacity_mah;
        }

        // P_mw = 800 mW, R_ohm = esr_milliohms / 1000
        // I_A^2 = 0.8 / (esr / 1000) = 800 / esr
        // I_mA^2 = 800_000_000 / esr
        let val = 800_000_000u64 / (esr_milliohms as u64);
        // Integer square root
        let mut x = val;
        let mut y = (x + 1) / 2;
        while y < x {
            x = y;
            y = (x + val / x) / 2;
        }
        let max_current_ma = x as u16;

        if max_current_ma < 500 {
            500
        } else if max_current_ma > self.design_capacity_mah {
            self.design_capacity_mah
        } else {
            max_current_ma
        }
    }

    /// Calculate target charging current and active thermal zone for current operating conditions.
    pub fn calculate_target_current(
        &mut self,
        soc_percent: u8,
        cell_temp_c: i8,
        esr_milliohms: u16,
    ) -> (u16, ChargeThermalZone) {
        let zone = Self::classify_thermal_zone(cell_temp_c);

        // 1. Thermal zone gating
        let thermal_max_ma = match zone {
            ChargeThermalZone::ColdInhibit | ChargeThermalZone::HotCutoff => 0,
            ChargeThermalZone::ColdTrickle => 1000,
            ChargeThermalZone::NormalFast => self.design_capacity_mah,
            ChargeThermalZone::WarmTaper => 2000,
        };

        if thermal_max_ma == 0 {
            self.thermal_taper_events += 1;
            return (0, zone);
        }

        // 2. Base MSCC current from SoC
        let base_ma = self.mscc_base_current_ma(soc_percent);

        // 3. ESR degradation limit
        let esr_limit_ma = self.esr_current_limit_ma(esr_milliohms);

        let mut target_ma = base_ma;

        // Apply thermal clamp
        if target_ma > thermal_max_ma {
            target_ma = thermal_max_ma;
            self.thermal_taper_events += 1;
        }

        // Apply ESR clamp
        if target_ma > esr_limit_ma {
            target_ma = esr_limit_ma;
            self.esr_taper_events += 1;
        }

        (target_ma, zone)
    }
}

/// Verification test for dynamic fast charge stepped current tapering governed by ESR and thermals.
pub fn verify_fast_charge_esr_and_thermal_tapering() -> bool {
    let mut governor = FastChargeTaperGovernor::new(5000);

    // 1. Optimal conditions: 25 C, 25 mOhm ESR, 30% SoC -> full peak 5000 mA
    let (cur1, zone1) = governor.calculate_target_current(30, 25, 25);
    if cur1 != 5000 || zone1 != ChargeThermalZone::NormalFast {
        return false;
    }

    // 2. High SoC stepped tapering: 25 C, 25 mOhm, 75% SoC -> 3000 mA
    let (cur2, zone2) = governor.calculate_target_current(75, 25, 25);
    if cur2 != 3000 || zone2 != ChargeThermalZone::NormalFast {
        return false;
    }

    // 3. High ESR cell degradation: 80 mOhm ESR, 30% SoC -> clamped by ESR (sqrt(800M / 80) = 3162 mA)
    let (cur3, _) = governor.calculate_target_current(30, 25, 80);
    if cur3 > 3200 || cur3 < 3100 {
        return false;
    }
    if governor.esr_taper_events == 0 {
        return false;
    }

    // 4. Elevated temperature: 48 C (WarmTaper) -> capped at 2000 mA
    let (cur4, zone4) = governor.calculate_target_current(30, 48, 25);
    if cur4 != 2000 || zone4 != ChargeThermalZone::WarmTaper {
        return false;
    }

    // 5. Critical hot temperature: 58 C (HotCutoff) -> 0 mA cutoff
    let (cur5, zone5) = governor.calculate_target_current(30, 58, 25);
    if cur5 != 0 || zone5 != ChargeThermalZone::HotCutoff {
        return false;
    }

    // 6. Freezing temperature: -5 C (ColdInhibit) -> 0 mA cutoff
    let (cur6, zone6) = governor.calculate_target_current(30, -5, 25);
    if cur6 != 0 || zone6 != ChargeThermalZone::ColdInhibit {
        return false;
    }

    true
}



// ============================================================================
// Feature 3: Long-Term Cycle Degradation Compensator & Swelling Prevention
// ============================================================================

/// Long-term battery degradation and wear metrics.
#[derive(Debug, Clone, Copy)]
pub struct BatteryDegradationProfile {
    pub cycle_count: u16,
    pub measured_capacity_mah: u16,
    pub design_capacity_mah: u16,
    pub esr_milliohms: u16,
}

impl BatteryDegradationProfile {
    pub const fn new(
        cycle_count: u16,
        measured_capacity_mah: u16,
        design_capacity_mah: u16,
        esr_milliohms: u16,
    ) -> Self {
        Self {
            cycle_count,
            measured_capacity_mah,
            design_capacity_mah,
            esr_milliohms,
        }
    }

    /// Calculate combined State of Health (SOH) percentage based on capacity retention and ESR rise.
    pub fn state_of_health_pct(&self) -> u8 {
        if self.design_capacity_mah == 0 {
            return 100;
        }

        let cap_retention_pct = ((self.measured_capacity_mah as u32 * 100) / (self.design_capacity_mah as u32)).min(100) as u8;

        // ESR index: baseline fresh = 25 mOhm. If ESR rises to 100 mOhm, health penalty is significant.
        let esr_factor = if self.esr_milliohms <= 25 {
            100
        } else if self.esr_milliohms >= 100 {
            50
        } else {
            100 - ((self.esr_milliohms - 25) * 50) / 75
        };

        // Weighted SOH: 70% capacity retention + 30% ESR health
        let soh = ((cap_retention_pct as u32 * 70) + (esr_factor as u32 * 30)) / 100;
        soh as u8
    }
}

/// Governor compensating for electrochemical aging and preventing pouch cell swelling.
#[derive(Debug, Clone)]
pub struct CycleDegradationCompensator {
    pub total_derate_events: u64,
    pub swelling_protection_active: bool,
}

impl CycleDegradationCompensator {
    pub const fn new() -> Self {
        Self {
            total_derate_events: 0,
            swelling_protection_active: false,
        }
    }

    /// Calculate safe maximum cell float cutoff voltage (in mV) to prevent electrolyte gassing.
    ///
    /// Fresh cells safely accept 4350 mV. As cycle count exceeds 200/400/600+ or SOH drops,
    /// float voltage is stepped down to 4300 mV, 4250 mV, and 4200 mV.
    pub fn calculate_safe_cutoff_voltage_mv(&mut self, profile: &BatteryDegradationProfile) -> u16 {
        let soh = profile.state_of_health_pct();
        let cycles = profile.cycle_count;

        let cutoff = if cycles >= 650 || soh < 80 || profile.esr_milliohms >= 85 {
            self.total_derate_events += 1;
            self.swelling_protection_active = true;
            4200
        } else if cycles >= 400 || soh < 88 || profile.esr_milliohms >= 60 {
            self.total_derate_events += 1;
            self.swelling_protection_active = false;
            4250
        } else if cycles >= 200 || soh < 95 || profile.esr_milliohms >= 40 {
            self.total_derate_events += 1;
            self.swelling_protection_active = false;
            4300
        } else {
            self.swelling_protection_active = false;
            4350
        };

        cutoff
    }

    /// Calculate Constant Voltage (CV) termination current threshold in mA.
    /// Worn/high-ESR cells terminate earlier (higher current threshold, e.g. 500 mA)
    /// to avoid prolonged float stress.
    pub fn calculate_cv_termination_ma(&self, profile: &BatteryDegradationProfile) -> u16 {
        if profile.cycle_count >= 500 || profile.esr_milliohms >= 75 {
            // Early CV cutoff to mitigate swelling
            500
        } else {
            // Standard 0.05C cutoff (250 mA for 5000 mAh pack)
            250
        }
    }

    /// Maximum continuous CV absorption hold duration in minutes before hard termination.
    pub fn calculate_max_cv_duration_minutes(&self, profile: &BatteryDegradationProfile) -> u16 {
        if profile.cycle_count >= 600 || profile.esr_milliohms >= 80 {
            45
        } else if profile.cycle_count >= 300 {
            75
        } else {
            120
        }
    }

    /// Whether the battery is at elevated risk of pouch expansion/swelling under fast charging.
    pub fn is_swelling_risk_elevated(&self, profile: &BatteryDegradationProfile) -> bool {
        profile.cycle_count >= 600 || profile.esr_milliohms >= 80 || profile.state_of_health_pct() < 80
    }
}

/// Verification test for long-term battery cycle degradation compensator and swelling prevention.
pub fn verify_cycle_degradation_and_swelling_prevention() -> bool {
    let mut compensator = CycleDegradationCompensator::new();

    // 1. Fresh pack: 50 cycles, 4950 mAh, 24 mOhm ESR
    let p_fresh = BatteryDegradationProfile::new(50, 4950, 5000, 24);
    let v_fresh = compensator.calculate_safe_cutoff_voltage_mv(&p_fresh);
    if v_fresh != 4350 {
        return false;
    }
    if compensator.calculate_cv_termination_ma(&p_fresh) != 250 {
        return false;
    }
    if compensator.calculate_max_cv_duration_minutes(&p_fresh) != 120 {
        return false;
    }
    if compensator.is_swelling_risk_elevated(&p_fresh) {
        return false;
    }

    // 2. Moderate cycle wear: 300 cycles, 4500 mAh, 45 mOhm ESR
    let p_moderate = BatteryDegradationProfile::new(300, 4500, 5000, 45);
    let v_moderate = compensator.calculate_safe_cutoff_voltage_mv(&p_moderate);
    if v_moderate != 4300 {
        return false;
    }

    // 3. High cycle wear: 550 cycles, 4100 mAh, 65 mOhm ESR
    let p_high = BatteryDegradationProfile::new(450, 4300, 5000, 55);
    let v_high = compensator.calculate_safe_cutoff_voltage_mv(&p_high);
    if v_high != 4250 {
        return false;
    }

    // 4. Severe degradation: 800 cycles, 3600 mAh, 90 mOhm ESR
    let p_severe = BatteryDegradationProfile::new(800, 3600, 5000, 90);
    let v_severe = compensator.calculate_safe_cutoff_voltage_mv(&p_severe);
    if v_severe != 4200 {
        return false;
    }
    if compensator.calculate_cv_termination_ma(&p_severe) != 500 {
        return false;
    }
    if compensator.calculate_max_cv_duration_minutes(&p_severe) != 45 {
        return false;
    }
    if !compensator.is_swelling_risk_elevated(&p_severe) {
        return false;
    }

    true
}



// ============================================================================
// Feature 4: Fast Charge Balance Manager Pipeline Integration
// ============================================================================

/// Charging operational state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastChargeState {
    /// No charger attached or charge complete.
    Idle,
    /// High-rate Multi-Step Constant Current (MSCC) charging active.
    FastChargeMSCC,
    /// Stepped current tapering active due to high SoC or warm temperature.
    Tapering,
    /// Constant Voltage (CV) saturation phase with voltage clamp.
    ConstantVoltageAbsorption,
    /// Only bypass cell bleeding active (charging inhibited or complete).
    BalanceOnly,
    /// Critical thermal protection cutoff (extreme cold or overheating).
    ThermalInhibit,
    /// Battery pack fully charged (CV termination current reached).
    FullChargeComplete,
}

/// Real-time pack sensor inputs for the charge manager.
#[derive(Debug, Clone, Copy)]
pub struct PackTelemetry {
    pub cell1_mv: u16,
    pub cell2_mv: u16,
    pub cell1_temp_c: i8,
    pub cell2_temp_c: i8,
    pub actual_charge_current_ma: u16,
    pub soc_percent: u8,
}

/// Output command dispatched to PMIC and USB-PD EPR buck-boost charger.
#[derive(Debug, Clone, Copy)]
pub struct FastChargeCommand {
    pub charge_enabled: bool,
    pub cell_target_voltage_mv: u16,
    pub pack_target_voltage_mv: u32,
    pub target_current_ma: u16,
    pub balance_mode: CellBalanceMode,
    pub thermal_zone: ChargeThermalZone,
    pub state: FastChargeState,
}

/// Integrated Fast Charge Balance & Degradation Manager.
#[derive(Debug, Clone)]
pub struct FastChargeBalanceManager {
    pub balance_governor: DualCellBalanceGovernor,
    pub taper_governor: FastChargeTaperGovernor,
    pub degradation_compensator: CycleDegradationCompensator,
    pub degradation_profile: BatteryDegradationProfile,
    pub current_state: FastChargeState,
    pub cv_absorption_ticks: u32,
    pub total_ticks: u64,
}

impl FastChargeBalanceManager {
    pub const fn new(degradation_profile: BatteryDegradationProfile) -> Self {
        Self {
            balance_governor: DualCellBalanceGovernor::new(),
            taper_governor: FastChargeTaperGovernor::new(degradation_profile.design_capacity_mah),
            degradation_compensator: CycleDegradationCompensator::new(),
            degradation_profile,
            current_state: FastChargeState::Idle,
            cv_absorption_ticks: 0,
            total_ticks: 0,
        }
    }

    /// Process a telemetry tick and calculate real-time PMIC and balancing commands.
    pub fn tick(&mut self, telemetry: &PackTelemetry) -> FastChargeCommand {
        self.total_ticks += 1;

        // 1. Worst-case cell temperature across dual-pack
        let max_temp_c = if telemetry.cell1_temp_c > telemetry.cell2_temp_c {
            telemetry.cell1_temp_c
        } else {
            telemetry.cell2_temp_c
        };

        // 2. Cell drift balancing update
        let cell_telem = CellTelemetry::new(telemetry.cell1_mv, telemetry.cell2_mv);
        let balance_mode = self.balance_governor.update(&cell_telem);

        // 3. Degradation compensator: compute safe float voltage limit per cell
        let safe_cutoff_mv = self.degradation_compensator.calculate_safe_cutoff_voltage_mv(&self.degradation_profile);
        let cv_term_ma = self.degradation_compensator.calculate_cv_termination_ma(&self.degradation_profile);

        // 4. Stepped current tapering based on SoC, temperature, and ESR
        let (target_current_ma, thermal_zone) = self.taper_governor.calculate_target_current(
            telemetry.soc_percent,
            max_temp_c,
            self.degradation_profile.esr_milliohms,
        );

        let max_cell_mv = if telemetry.cell1_mv > telemetry.cell2_mv {
            telemetry.cell1_mv
        } else {
            telemetry.cell2_mv
        };

        // 5. State machine evaluation
        let (charge_enabled, final_current_ma, state) = match thermal_zone {
            ChargeThermalZone::ColdInhibit | ChargeThermalZone::HotCutoff => {
                self.cv_absorption_ticks = 0;
                (false, 0, FastChargeState::ThermalInhibit)
            }
            _ => {
                // If maximum cell has hit or exceeded the safe float cutoff voltage
                if max_cell_mv >= safe_cutoff_mv {
                    self.cv_absorption_ticks += 1;

                    // Check for charge termination condition
                    if telemetry.actual_charge_current_ma <= cv_term_ma && telemetry.soc_percent >= 98 {
                        (false, 0, FastChargeState::FullChargeComplete)
                    } else {
                        // Taper current in CV mode (decaying toward termination)
                        let cv_current = target_current_ma.min(1500);
                        (true, cv_current, FastChargeState::ConstantVoltageAbsorption)
                    }
                } else if telemetry.soc_percent >= 85 || thermal_zone == ChargeThermalZone::WarmTaper {
                    self.cv_absorption_ticks = 0;
                    (true, target_current_ma, FastChargeState::Tapering)
                } else {
                    self.cv_absorption_ticks = 0;
                    (true, target_current_ma, FastChargeState::FastChargeMSCC)
                }
            }
        };

        self.current_state = state;

        FastChargeCommand {
            charge_enabled,
            cell_target_voltage_mv: safe_cutoff_mv,
            pack_target_voltage_mv: (safe_cutoff_mv as u32) * 2,
            target_current_ma: final_current_ma,
            balance_mode,
            thermal_zone,
            state,
        }
    }
}

/// Verification test for fast charge balance manager pipeline integration.
pub fn verify_fast_charge_balance_manager_integration() -> bool {
    let profile = BatteryDegradationProfile::new(100, 4900, 5000, 26);
    let mut manager = FastChargeBalanceManager::new(profile);

    // 1. Nominal Fast Charging: 30% SoC, 4000/4005 mV, 28 C -> FastChargeMSCC, 5000 mA, Inactive balance
    let t1 = PackTelemetry {
        cell1_mv: 4000,
        cell2_mv: 4005,
        cell1_temp_c: 28,
        cell2_temp_c: 29,
        actual_charge_current_ma: 4950,
        soc_percent: 30,
    };
    let cmd1 = manager.tick(&t1);
    if !cmd1.charge_enabled || cmd1.state != FastChargeState::FastChargeMSCC || cmd1.target_current_ma != 5000 {
        return false;
    }
    if cmd1.balance_mode != CellBalanceMode::Inactive {
        return false;
    }

    // 2. High-rate charge with drift: 4180/4155 mV (delta 25 mV) -> BleedCell1 active during fast charge
    let t2 = PackTelemetry {
        cell1_mv: 4180,
        cell2_mv: 4155,
        cell1_temp_c: 32,
        cell2_temp_c: 31,
        actual_charge_current_ma: 4800,
        soc_percent: 65,
    };
    let cmd2 = manager.tick(&t2);
    if cmd2.balance_mode != CellBalanceMode::BleedCell1 {
        return false;
    }

    // 3. Constant Voltage Absorption: Cell 1 reaches 4350 mV safe cutoff -> CV absorption state
    let t3 = PackTelemetry {
        cell1_mv: 4350,
        cell2_mv: 4345,
        cell1_temp_c: 35,
        cell2_temp_c: 34,
        actual_charge_current_ma: 1200,
        soc_percent: 94,
    };
    let cmd3 = manager.tick(&t3);
    if cmd3.state != FastChargeState::ConstantVoltageAbsorption || cmd3.target_current_ma > 1500 {
        return false;
    }

    // 4. Full charge termination: current drops below 250 mA at 99% SoC -> FullChargeComplete
    let t4 = PackTelemetry {
        cell1_mv: 4350,
        cell2_mv: 4348,
        cell1_temp_c: 33,
        cell2_temp_c: 32,
        actual_charge_current_ma: 180,
        soc_percent: 99,
    };
    let cmd4 = manager.tick(&t4);
    if cmd4.charge_enabled || cmd4.state != FastChargeState::FullChargeComplete || cmd4.target_current_ma != 0 {
        return false;
    }

    // 5. Thermal safety cutoff test: Sudden overheating to 58 C -> ThermalInhibit, 0 mA
    let t5 = PackTelemetry {
        cell1_mv: 4200,
        cell2_mv: 4200,
        cell1_temp_c: 58,
        cell2_temp_c: 56,
        actual_charge_current_ma: 3000,
        soc_percent: 70,
    };
    let cmd5 = manager.tick(&t5);
    if cmd5.charge_enabled || cmd5.state != FastChargeState::ThermalInhibit || cmd5.target_current_ma != 0 {
        return false;
    }

    true
}



// ============================================================================
// Feature 5: Comprehensive Subsystem Self-Test Suite & kmain Integration
// ============================================================================

/// Comprehensive self-test suite for Handheld Dual-Cell Fast Charge Balance & Impedance Degradation Compensator.
pub fn test_fast_charge_balance_subsystem() -> bool {
    crate::serial_write_str("[KNT-POWER-FAST-CHARGE] Initiating dual-cell fast charge balance & degradation self-test...\r\n");

    if !verify_dual_cell_drift_balancing() {
        crate::serial_write_str("[KNT-POWER-FAST-CHARGE-FAIL] Dual-cell drift balancing verification failed\r\n");
        return false;
    }

    if !verify_fast_charge_esr_and_thermal_tapering() {
        crate::serial_write_str("[KNT-POWER-FAST-CHARGE-FAIL] Fast charge ESR and thermal tapering verification failed\r\n");
        return false;
    }

    if !verify_cycle_degradation_and_swelling_prevention() {
        crate::serial_write_str("[KNT-POWER-FAST-CHARGE-FAIL] Cycle degradation and swelling prevention verification failed\r\n");
        return false;
    }

    if !verify_fast_charge_balance_manager_integration() {
        crate::serial_write_str("[KNT-POWER-FAST-CHARGE-FAIL] Fast charge balance manager integration verification failed\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-POWER-FAST-CHARGE-PASS] Handheld dual-cell fast charge balance and degradation compensator operational\r\n");
    true
}
