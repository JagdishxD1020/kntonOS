//! Handheld USB-PD 3.1 Extended Power Range (EPR 240W) Fast Charge & Real-Time Impedance Tracker.
//!
//! Provides:
//! 1. USB-PD 3.1 EPR contract negotiation (28V, 36V, 48V up to 5A / 240W) and AVS (Adjustable Voltage Supply in 100mV increments).
//! 2. Real-time cable impedance and contact resistance tracking (delta-V / delta-I calculation, thermal contact warning, oxidation detection).
//! 3. Dynamic peak gaming boost charging governor balancing battery charge rate vs APU TDP draw.
//! 4. Hardware over-voltage (OVP), over-current (OCP), and thermal runaway emergency cutoff protection state machine.
//! 5. Self-test suite verifying the subsystem with sentinel `[KNT-POWER-USB-PD-EPR-PASS]`.

#![allow(dead_code)]

// ============================================================================
// Feature 1: USB-PD 3.1 EPR Contract Negotiation & AVS Stepping Engine
// ============================================================================

/// USB Power Delivery Revision & Power Range tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbPdRange {
    /// Standard Power Range (SPR): Fixed voltages 5V, 9V, 15V, 20V up to 5A (max 100W).
    StandardPowerRange100W,
    /// Extended Power Range (EPR 140W): 28V @ 5A.
    ExtendedPowerRange140W,
    /// Extended Power Range (EPR 180W): 36V @ 5A.
    ExtendedPowerRange180W,
    /// Extended Power Range (EPR 240W): 48V @ 5A (maximum USB-PD 3.1 specification limit).
    ExtendedPowerRange240W,
}

/// Power Data Object (PDO) type for USB-PD 3.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerDataObjectKind {
    /// Fixed supply voltage PDO (e.g. 28V, 36V, 48V).
    FixedSupply,
    /// Adjustable Voltage Supply (AVS) PDO: 15V to 48V continuously adjustable in 100mV steps.
    AdjustableVoltageSupplyAvs,
    /// Programmable Power Supply (PPS) SPR: 3.3V to 21V in 20mV steps.
    ProgrammablePowerSupplyPps,
}

/// Negotiated USB-PD 3.1 EPR Operating Contract.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsbPdEprContract {
    /// Active power range tier.
    pub range_tier: UsbPdRange,
    /// Active PDO classification.
    pub pdo_kind: PowerDataObjectKind,
    /// Negotiated operating voltage in millivolts (mV) (e.g. 28,000 mV, 36,000 mV, 48,000 mV).
    pub operating_voltage_mv: u32,
    /// Negotiated operating current limit in milliamperes (mA) (up to 5,000 mA).
    pub current_limit_ma: u32,
    /// Maximum negotiated contract power in Watts (W).
    pub max_contract_power_w: u16,
    /// Whether EPR Mode Entry handshake has been acknowledged by both source and sink (EPR_MODE_ENTER_ACK).
    pub is_epr_handshake_complete: bool,
    /// Whether 100mV AVS voltage stepping is active.
    pub is_avs_stepping_active: bool,
}

/// USB-PD 3.1 EPR Contract Negotiator & AVS Stepping Engine.
///
/// Handles BMC (Biphase Mark Coding) protocol handshakes over USB Type-C CC (Configuration Channel):
/// 1. Discovers Source Capabilities including SPR (5V..20V) and EPR (28V, 36V, 48V) PDOs.
/// 2. Verifies 50V / 5A EPR-capable e-marker cable presence.
/// 3. Executes `ENTER_EPR` control message sequence.
/// 4. Adjusts voltage via AVS in 100mV increments (15,000 mV to 48,000 mV) to minimize handheld buck-boost conversion losses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsbPd31EprNegotiator {
    /// Current active contract.
    pub active_contract: UsbPdEprContract,
    /// Cable e-marker 50V/5A EPR capability flag.
    pub cable_supports_epr_240w: bool,
    /// Source maximum supported voltage in mV.
    pub source_max_voltage_mv: u32,
    /// Source maximum supported current in mA.
    pub source_max_current_ma: u32,
    /// Negotiation cycle counter.
    pub negotiation_count: u32,
}

impl UsbPd31EprNegotiator {
    /// Minimum allowed AVS voltage (15.0V).
    pub const AVS_MIN_VOLTAGE_MV: u32 = 15_000;
    /// Maximum allowed AVS voltage (48.0V).
    pub const AVS_MAX_VOLTAGE_MV: u32 = 48_000;
    /// AVS voltage step increment (100 mV).
    pub const AVS_STEP_MV: u32 = 100;

    /// Constructs an un-negotiated USB-PD EPR controller in standard 5V safe state.
    pub const fn new() -> Self {
        Self {
            active_contract: UsbPdEprContract {
                range_tier: UsbPdRange::StandardPowerRange100W,
                pdo_kind: PowerDataObjectKind::FixedSupply,
                operating_voltage_mv: 5_000,
                current_limit_ma: 3_000,
                max_contract_power_w: 15,
                is_epr_handshake_complete: false,
                is_avs_stepping_active: false,
            },
            cable_supports_epr_240w: false,
            source_max_voltage_mv: 5_000,
            source_max_current_ma: 3_000,
            negotiation_count: 0,
        }
    }

    /// Negotiates an EPR contract up to 48V / 5A (240W) based on source capabilities and cable rating.
    pub fn negotiate_epr_contract(
        &mut self,
        source_max_mv: u32,
        source_max_ma: u32,
        cable_epr_rated: bool,
        prefer_avs: bool,
    ) -> Result<UsbPdEprContract, &'static str> {
        self.negotiation_count += 1;
        self.source_max_voltage_mv = source_max_mv;
        self.source_max_current_ma = source_max_ma;
        self.cable_supports_epr_240w = cable_epr_rated;

        // If cable does not have EPR e-marker (50V/5A rated), cap at SPR 20V/5A (100W)
        if !cable_epr_rated || source_max_mv < 28_000 {
            let safe_v = source_max_mv.min(20_000);
            let safe_i = source_max_ma.min(5_000);
            self.active_contract = UsbPdEprContract {
                range_tier: UsbPdRange::StandardPowerRange100W,
                pdo_kind: PowerDataObjectKind::FixedSupply,
                operating_voltage_mv: safe_v,
                current_limit_ma: safe_i,
                max_contract_power_w: ((safe_v as u64 * safe_i as u64) / 1_000_000) as u16,
                is_epr_handshake_complete: false,
                is_avs_stepping_active: false,
            };
            return Ok(self.active_contract);
        }

        // Determine highest supported EPR voltage tier
        let (range, target_v) = if source_max_mv >= 48_000 {
            (UsbPdRange::ExtendedPowerRange240W, 48_000)
        } else if source_max_mv >= 36_000 {
            (UsbPdRange::ExtendedPowerRange180W, 36_000)
        } else {
            (UsbPdRange::ExtendedPowerRange140W, 28_000)
        };

        let target_i = source_max_ma.min(5_000);
        let pdo_kind = if prefer_avs {
            PowerDataObjectKind::AdjustableVoltageSupplyAvs
        } else {
            PowerDataObjectKind::FixedSupply
        };

        self.active_contract = UsbPdEprContract {
            range_tier: range,
            pdo_kind,
            operating_voltage_mv: target_v,
            current_limit_ma: target_i,
            max_contract_power_w: ((target_v as u64 * target_i as u64) / 1_000_000) as u16,
            is_epr_handshake_complete: true,
            is_avs_stepping_active: prefer_avs,
        };

        Ok(self.active_contract)
    }

    /// Dynamically steps AVS operating voltage by `step_100mv_delta` (positive or negative).
    /// Enforces bounds between 15,000 mV and 48,000 mV in exact 100mV increments.
    pub fn step_avs_voltage(&mut self, step_100mv_delta: i16) -> Result<u32, &'static str> {
        if !self.active_contract.is_epr_handshake_complete || !self.active_contract.is_avs_stepping_active {
            return Err("AVS stepping requires active EPR AVS contract");
        }

        let delta_mv = (step_100mv_delta as i32) * (Self::AVS_STEP_MV as i32);
        let new_mv = (self.active_contract.operating_voltage_mv as i32) + delta_mv;

        let clamped_mv = if new_mv < Self::AVS_MIN_VOLTAGE_MV as i32 {
            Self::AVS_MIN_VOLTAGE_MV
        } else if new_mv > Self::AVS_MAX_VOLTAGE_MV as i32 {
            Self::AVS_MAX_VOLTAGE_MV
        } else {
            // Round to nearest 100mV
            ((new_mv / Self::AVS_STEP_MV as i32) * Self::AVS_STEP_MV as i32) as u32
        };

        self.active_contract.operating_voltage_mv = clamped_mv;
        self.active_contract.max_contract_power_w =
            ((clamped_mv as u64 * self.active_contract.current_limit_ma as u64) / 1_000_000) as u16;

        Ok(clamped_mv)
    }
}

/// Verification test for USB-PD 3.1 EPR contract negotiation and AVS stepping engine.
pub fn verify_usb_pd_epr_contract_negotiation() -> bool {
    let mut negotiator = UsbPd31EprNegotiator::new();

    // 1. Initial state: Safe 5V @ 3A SPR
    if negotiator.active_contract.operating_voltage_mv != 5_000
        || negotiator.active_contract.is_epr_handshake_complete
    {
        return false;
    }

    // 2. Negotiate EPR 240W (48V @ 5A) with AVS enabled and EPR-rated cable
    let contract_240w = negotiator
        .negotiate_epr_contract(48_000, 5_000, true, true)
        .expect("EPR negotiation failed");

    if contract_240w.range_tier != UsbPdRange::ExtendedPowerRange240W {
        return false;
    }
    if contract_240w.operating_voltage_mv != 48_000 || contract_240w.current_limit_ma != 5_000 {
        return false;
    }
    if contract_240w.max_contract_power_w != 240 || !contract_240w.is_epr_handshake_complete {
        return false;
    }
    if !contract_240w.is_avs_stepping_active {
        return false;
    }

    // 3. Step AVS voltage down by 5 x 100mV (-500 mV) -> 47,500 mV
    let v_step1 = negotiator.step_avs_voltage(-5).expect("AVS step failed");
    if v_step1 != 47_500 || negotiator.active_contract.operating_voltage_mv != 47_500 {
        return false;
    }

    // 4. Step AVS voltage up beyond max 48,000 mV (+20 x 100mV = +2000 mV) -> Clamped to 48,000 mV
    let v_step2 = negotiator.step_avs_voltage(20).expect("AVS step failed");
    if v_step2 != 48_000 || negotiator.active_contract.operating_voltage_mv != 48_000 {
        return false;
    }

    // 5. Fallback test: Non-EPR cable connected to 48V charger -> must cap at SPR 20V / 100W
    let contract_spr = negotiator
        .negotiate_epr_contract(48_000, 5_000, false, false)
        .expect("SPR fallback failed");
    if contract_spr.range_tier != UsbPdRange::StandardPowerRange100W {
        return false;
    }
    if contract_spr.operating_voltage_mv != 20_000 || contract_spr.is_epr_handshake_complete {
        return false;
    }
    if contract_spr.max_contract_power_w != 100 {
        return false;
    }

    true
}

// ============================================================================
// Feature 2: Real-Time Cable Impedance & Contact Resistance Tracking
// ============================================================================

/// Contact health diagnostic rating for USB Type-C receptacle and cable plug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CableContactHealth {
    /// Optimal contact resistance (< 120 mΩ); safe for maximum 5A EPR charging.
    OptimalCleanContact,
    /// Elevated contact resistance (120 mΩ .. 250 mΩ); moderate wear or dust, throttles current to 3A.
    ElevatedResistanceWarning,
    /// Severe oxidation / thermal degradation (> 250 mΩ); high risk of port melting, throttles to 1.5A / 5V.
    OxidizedSevereContactFault,
}

/// Real-Time Cable Impedance and Contact Resistance Tracker.
///
/// Under 240W EPR (48V @ 5A), dirty, oxidized, or worn USB-C connector contacts
/// dissipate extreme I^2 * R heat (e.g. 5A through a 300 mΩ contact burns 7.5W of heat
/// inside the handheld charging port, melting plastics and scorching pins).
/// This engine performs active Kelvin-style impedance probing:
/// 1. Measures Vbus voltage droop between light load (0.5A) and heavy load (5.0A):
///    R_cable = (V_light - V_heavy) / (I_heavy - I_light)
/// 2. Tracks port thermal temperature sensor co-located on the USB-C PCB receptacle.
/// 3. Automatically triggers contact oxidation warnings and scales down charge current.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CableImpedanceTracker {
    /// Measured total round-trip cable and contact resistance in milliohms (mΩ).
    pub measured_resistance_mohm: u32,
    /// Baseline contact resistance when freshly mated in mΩ.
    pub baseline_resistance_mohm: u32,
    /// USB-C receptacle thermistor temperature in degrees Celsius (°C).
    pub port_temperature_c: f32,
    /// Contact health status.
    pub contact_health: CableContactHealth,
    /// Recommended maximum charging current in mA to prevent port overheating.
    pub recommended_current_cap_ma: u32,
    /// Total impedance measurement probes executed.
    pub probes_executed: u32,
    /// Thermal runaway warnings logged.
    pub thermal_warnings_count: u32,
}

impl CableImpedanceTracker {
    /// Optimal contact resistance threshold in milliohms (120 mΩ).
    pub const RESISTANCE_OPTIMAL_MAX_MOHM: u32 = 120;
    /// Elevated resistance warning threshold in milliohms (250 mΩ).
    pub const RESISTANCE_ELEVATED_MAX_MOHM: u32 = 250;
    /// Maximum safe USB-C receptacle temperature (°C).
    pub const MAX_SAFE_PORT_TEMP_C: f32 = 65.0;

    /// Constructs an impedance tracker with pristine default contact metrics.
    pub const fn new() -> Self {
        Self {
            measured_resistance_mohm: 80, // Pristine 80 mΩ
            baseline_resistance_mohm: 80,
            port_temperature_c: 28.0,
            contact_health: CableContactHealth::OptimalCleanContact,
            recommended_current_cap_ma: 5_000,
            probes_executed: 0,
            thermal_warnings_count: 0,
        }
    }

    /// Evaluates cable and contact impedance from delta-V and delta-I telemetry:
    /// `v_light_mv` @ `i_light_ma` and `v_heavy_mv` @ `i_heavy_ma`.
    pub fn probe_impedance(
        &mut self,
        v_light_mv: u32,
        i_light_ma: u32,
        v_heavy_mv: u32,
        i_heavy_ma: u32,
        port_temp_c: f32,
    ) -> CableContactHealth {
        self.probes_executed += 1;
        self.port_temperature_c = port_temp_c;

        if i_heavy_ma > i_light_ma && v_light_mv >= v_heavy_mv {
            let delta_v_mv = v_light_mv - v_heavy_mv;
            let delta_i_ma = i_heavy_ma - i_light_ma;

            // R (mΩ) = (delta_v_mv * 1000) / delta_i_ma
            let calculated_mohm = ((delta_v_mv as u64 * 1000) / (delta_i_ma as u64)) as u32;
            self.measured_resistance_mohm = calculated_mohm;
        }

        // Assess contact health and port temperature
        if self.port_temperature_c >= Self::MAX_SAFE_PORT_TEMP_C
            || self.measured_resistance_mohm > Self::RESISTANCE_ELEVATED_MAX_MOHM
        {
            self.contact_health = CableContactHealth::OxidizedSevereContactFault;
            self.recommended_current_cap_ma = 1_500; // Throttle to 1.5A safe limit
            self.thermal_warnings_count += 1;
        } else if self.measured_resistance_mohm > Self::RESISTANCE_OPTIMAL_MAX_MOHM {
            self.contact_health = CableContactHealth::ElevatedResistanceWarning;
            self.recommended_current_cap_ma = 3_000; // Throttle to 3.0A
        } else {
            self.contact_health = CableContactHealth::OptimalCleanContact;
            self.recommended_current_cap_ma = 5_000; // Full 5.0A EPR capability
        }

        self.contact_health
    }
}

/// Verification test for real-time cable impedance and contact resistance tracking.
pub fn verify_cable_impedance_tracking() -> bool {
    let mut tracker = CableImpedanceTracker::new();

    // 1. Initial state: Optimal 80 mΩ, 5A cap
    if tracker.contact_health != CableContactHealth::OptimalCleanContact {
        return false;
    }
    if tracker.recommended_current_cap_ma != 5_000 {
        return false;
    }

    // 2. Clean probe: Light load (48,000 mV @ 500 mA), Heavy load (47,595 mV @ 5000 mA)
    // Delta V = 405 mV, Delta I = 4500 mA -> R = 405,000 / 4500 = 90 mΩ (< 120 mΩ optimal)
    let health_clean = tracker.probe_impedance(48_000, 500, 47_595, 5_000, 35.0);
    if health_clean != CableContactHealth::OptimalCleanContact {
        return false;
    }
    if tracker.measured_resistance_mohm != 90 || tracker.recommended_current_cap_ma != 5_000 {
        return false;
    }

    // 3. Elevated resistance test (worn cable): Delta V = 720 mV, Delta I = 4500 mA -> R = 160 mΩ
    let health_elevated = tracker.probe_impedance(48_000, 500, 47_280, 5_000, 42.0);
    if health_elevated != CableContactHealth::ElevatedResistanceWarning {
        return false;
    }
    if tracker.measured_resistance_mohm != 160 || tracker.recommended_current_cap_ma != 3_000 {
        return false;
    }

    // 4. Severe contact oxidation / thermal warning: Delta V = 1350 mV @ 4500 mA -> R = 300 mΩ or Temp = 68°C
    let health_severe = tracker.probe_impedance(48_000, 500, 46_650, 5_000, 68.0);
    if health_severe != CableContactHealth::OxidizedSevereContactFault {
        return false;
    }
    if tracker.measured_resistance_mohm != 300 || tracker.recommended_current_cap_ma != 1_500 {
        return false;
    }
    if tracker.thermal_warnings_count != 1 {
        return false;
    }

    true
}

// ============================================================================
// Feature 3: Dynamic Peak Gaming Boost Charging Governor
// ============================================================================

/// Operating power state for handheld APU and battery fast-charge power splitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamingBoostPowerState {
    /// Pure Battery Power: AC charger disconnected.
    BatteryDischargeOnly,
    /// Balanced Charging: Charger provides power for APU and full battery charging.
    BalancedCharging,
    /// Peak Gaming Boost: APU draw exceeds sustained rating; battery charge rate throttled to 0W (pass-through).
    PeakGamingPassThrough,
    /// Battery Assist Boost: APU demand exceeds charger contract capacity; battery assists charger simultaneously.
    HybridBatteryAssist,
}

/// Dynamic Peak Gaming Boost Charging Governor.
///
/// Handheld gaming APUs frequently spike from 15W sustained TDP to 35W-55W transient burst TDP.
/// Under high load, concurrent fast charging can push the total handheld thermal dissipation
/// past the chassis cooling limit (causing uncomfortably hot palm grips > 43°C or fan noise).
/// This governor dynamically balances:
/// P_charger = P_apu_draw + P_battery_charge
///
/// 1. Prioritizes APU gaming performance to prevent frame-rate drops.
/// 2. Scales down battery charge current during heavy GPU/CPU scenes.
/// 3. Enables "Hybrid Battery Assist" if APU power exceeds the negotiated USB-PD contract.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeakGamingBoostGovernor {
    /// Total charger power delivery budget in Watts (from USB-PD contract, e.g. 65W, 100W, 140W, 240W).
    pub charger_budget_w: u16,
    /// Instantaneous APU package electrical power in Watts.
    pub apu_power_draw_w: u16,
    /// Handheld chassis palm grip temperature in °C.
    pub grip_temp_c: f32,
    /// Allocated battery charging power in Watts.
    pub allocated_battery_charge_w: u16,
    /// Battery discharge assist power in Watts (0 if not assisting).
    pub battery_assist_draw_w: u16,
    /// Active boost power mode.
    pub power_state: GamingBoostPowerState,
    /// Maximum allowable palm grip temperature (42.0°C).
    pub max_grip_temp_c: f32,
}

impl PeakGamingBoostGovernor {
    /// Constructs a peak gaming boost governor with a default 100W charger budget.
    pub const fn new() -> Self {
        Self {
            charger_budget_w: 100,
            apu_power_draw_w: 25,
            grip_temp_c: 32.0,
            allocated_battery_charge_w: 60,
            battery_assist_draw_w: 0,
            power_state: GamingBoostPowerState::BalancedCharging,
            max_grip_temp_c: 42.0,
        }
    }

    /// Evaluates and balances power sharing between APU and battery fast charge.
    pub fn arbitrate_power(
        &mut self,
        charger_w: u16,
        apu_w: u16,
        grip_c: f32,
        battery_target_w: u16,
    ) -> GamingBoostPowerState {
        self.charger_budget_w = charger_w;
        self.apu_power_draw_w = apu_w;
        self.grip_temp_c = grip_c;

        if charger_w == 0 {
            // Disconnected
            self.allocated_battery_charge_w = 0;
            self.battery_assist_draw_w = apu_w;
            self.power_state = GamingBoostPowerState::BatteryDischargeOnly;
            return self.power_state;
        }

        if apu_w > charger_w {
            // APU demands more than charger can physically deliver (e.g. 120W burst on 100W charger)
            // Enter Hybrid Battery Assist: Battery provides the difference
            self.allocated_battery_charge_w = 0;
            self.battery_assist_draw_w = apu_w - charger_w;
            self.power_state = GamingBoostPowerState::HybridBatteryAssist;
        } else {
            // Charger has surplus capacity above APU draw
            self.battery_assist_draw_w = 0;
            let surplus_w = charger_w - apu_w;

            // Thermal throttle: if palm grip temperature approaches or exceeds 42.0°C, throttle battery charge
            let thermal_derating = if grip_c >= self.max_grip_temp_c {
                0.0f32 // Complete thermal charge cutoff to cool down chassis
            } else if grip_c >= 38.0 {
                // Progressive linear derating from 38°C to 42°C
                (self.max_grip_temp_c - grip_c) / (self.max_grip_temp_c - 38.0)
            } else {
                1.0f32
            };

            let allowable_charge_w = (surplus_w.min(battery_target_w) as f32 * thermal_derating) as u16;
            self.allocated_battery_charge_w = allowable_charge_w;

            if allowable_charge_w == 0 {
                self.power_state = GamingBoostPowerState::PeakGamingPassThrough;
            } else {
                self.power_state = GamingBoostPowerState::BalancedCharging;
            }
        }

        self.power_state
    }
}

/// Verification test for dynamic peak gaming boost charging governor.
pub fn verify_peak_gaming_boost_governor() -> bool {
    let mut governor = PeakGamingBoostGovernor::new();

    // 1. Balanced charging: 100W charger, 25W APU draw, cool grip (32°C), target 65W battery charge
    let s1 = governor.arbitrate_power(100, 25, 32.0, 65);
    if s1 != GamingBoostPowerState::BalancedCharging {
        return false;
    }
    if governor.allocated_battery_charge_w != 65 || governor.battery_assist_draw_w != 0 {
        return false;
    }

    // 2. High gaming load: 60W APU draw, 100W charger, surplus = 40W
    let s2 = governor.arbitrate_power(100, 60, 35.0, 65);
    if s2 != GamingBoostPowerState::BalancedCharging {
        return false;
    }
    if governor.allocated_battery_charge_w != 40 || governor.battery_assist_draw_w != 0 {
        return false;
    }

    // 3. Thermal throttling: Grip temp hits 42.5°C (> 42.0°C limit) -> Charge cut to 0W (Pass-Through mode)
    let s3 = governor.arbitrate_power(100, 35, 42.5, 65);
    if s3 != GamingBoostPowerState::PeakGamingPassThrough {
        return false;
    }
    if governor.allocated_battery_charge_w != 0 {
        return false;
    }

    // 4. Hybrid Battery Assist: Heavy 120W burst on a 100W charger -> Battery assists with 20W
    let s4 = governor.arbitrate_power(100, 120, 38.0, 65);
    if s4 != GamingBoostPowerState::HybridBatteryAssist {
        return false;
    }
    if governor.battery_assist_draw_w != 20 || governor.allocated_battery_charge_w != 0 {
        return false;
    }

    true
}
