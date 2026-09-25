//! Handheld Power Supply (PD 3.1) Passthrough Charging & Thermal Bypass Governor.

#![allow(dead_code)]

/// USB-PD 3.1 Power Data Object (PDO) supply type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdSupplyType {
    /// Fixed voltage supply (SPR <= 20V, EPR 28V, 36V, 48V).
    Fixed,
    /// Adjustable Voltage Supply (AVS) EPR profile (15V to 48V).
    AdjustableVoltageSupply,
    /// Variable supply (non-battery).
    Variable,
    /// Battery direct supply.
    Battery,
}

/// USB-PD 3.1 Extended Power Range (EPR) Power Data Object (PDO).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdPowerDataObject {
    /// Object position index (1 to 7).
    pub position: u8,
    /// Power supply classification.
    pub supply_type: PdSupplyType,
    /// Nominal voltage in millivolts (mV).
    pub voltage_mv: u32,
    /// Minimum voltage for AVS in millivolts (mV).
    pub min_voltage_mv: u32,
    /// Maximum voltage for AVS in millivolts (mV).
    pub max_voltage_mv: u32,
    /// Maximum allowable current in milliamperes (mA).
    pub max_current_ma: u32,
    /// Peak wattage capacity in Watts (W).
    pub max_power_w: f32,
}

impl PdPowerDataObject {
    /// Fixed supply PDO constructor.
    pub const fn new_fixed(position: u8, voltage_mv: u32, max_current_ma: u32) -> Self {
        let max_power_w = (voltage_mv as f32 * max_current_ma as f32) / 1_000_000.0;
        Self {
            position,
            supply_type: PdSupplyType::Fixed,
            voltage_mv,
            min_voltage_mv: voltage_mv,
            max_voltage_mv: voltage_mv,
            max_current_ma,
            max_power_w,
        }
    }

    /// AVS EPR supply PDO constructor (15V to 28V/36V/48V @ 5A).
    pub const fn new_avs(position: u8, min_voltage_mv: u32, max_voltage_mv: u32, max_current_ma: u32) -> Self {
        let max_power_w = (max_voltage_mv as f32 * max_current_ma as f32) / 1_000_000.0;
        Self {
            position,
            supply_type: PdSupplyType::AdjustableVoltageSupply,
            voltage_mv: max_voltage_mv,
            min_voltage_mv,
            max_voltage_mv,
            max_current_ma,
            max_power_w,
        }
    }

    /// Standard EPR 140W profile (28V / 5A).
    pub const fn epr_140w(position: u8) -> Self {
        Self::new_fixed(position, 28_000, 5_000)
    }

    /// Standard EPR 240W profile (48V / 5A).
    pub const fn epr_240w(position: u8) -> Self {
        Self::new_fixed(position, 48_000, 5_000)
    }
}

/// USB-PD 3.1 Request Data Object (RDO) representing a sink power contract request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdRequestDataObject {
    /// Selected PDO object position.
    pub object_position: u8,
    /// Requested operating voltage in millivolts (mV).
    pub requested_voltage_mv: u32,
    /// Operating current in milliamperes (mA).
    pub operating_current_ma: u32,
    /// Maximum current sink can draw in milliamperes (mA).
    pub max_current_ma: u32,
    /// Capability mismatch flag.
    pub capability_mismatch: bool,
    /// GiveBack support flag.
    pub give_back_flag: bool,
}

impl PdRequestDataObject {
    pub const fn empty() -> Self {
        Self {
            object_position: 0,
            requested_voltage_mv: 0,
            operating_current_ma: 0,
            max_current_ma: 0,
            capability_mismatch: false,
            give_back_flag: false,
        }
    }
}

/// USB-PD 3.1 EPR contract negotiation state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EprNegotiationState {
    /// No USB-PD power source attached.
    Unattached,
    /// Source Capabilities advertisement received from charger.
    SourceCapsReceived,
    /// EPR entry handshake and Request Data Object dispatched.
    ContractNegotiating,
    /// Explicit EPR power contract successfully established.
    ContractEstablished,
    /// Contract rejected or protocol error occurred.
    ContractFailed,
}

/// USB-PD 3.1 EPR Power Contract Negotiator.
#[derive(Debug, Clone)]
pub struct EprContractNegotiator {
    pub state: EprNegotiationState,
    pub source_caps: [Option<PdPowerDataObject>; 7],
    pub active_contract_rdo: PdRequestDataObject,
    pub negotiated_voltage_mv: u32,
    pub negotiated_current_ma: u32,
    pub negotiated_power_w: f32,
    pub epr_mode_active: bool,
}

impl EprContractNegotiator {
    pub const fn new() -> Self {
        Self {
            state: EprNegotiationState::Unattached,
            source_caps: [None; 7],
            active_contract_rdo: PdRequestDataObject::empty(),
            negotiated_voltage_mv: 0,
            negotiated_current_ma: 0,
            negotiated_power_w: 0.0,
            epr_mode_active: false,
        }
    }

    /// Stores source capabilities received from USB-C CC line communication.
    pub fn receive_source_capabilities(&mut self, caps: &[PdPowerDataObject]) -> Result<usize, &'static str> {
        if caps.is_empty() || caps.len() > 7 {
            return Err("Invalid PDO count");
        }
        for (i, cap) in caps.iter().enumerate() {
            self.source_caps[i] = Some(*cap);
        }
        self.state = EprNegotiationState::SourceCapsReceived;
        Ok(caps.len())
    }

    /// Selects best EPR/SPR profile and requests explicit power contract.
    pub fn negotiate_optimal_contract(&mut self, target_watts: f32) -> Result<PdRequestDataObject, &'static str> {
        if self.state != EprNegotiationState::SourceCapsReceived {
            return Err("Source capabilities not received");
        }

        self.state = EprNegotiationState::ContractNegotiating;

        let mut best_pdo: Option<PdPowerDataObject> = None;
        for cap_opt in self.source_caps.iter() {
            if let Some(pdo) = cap_opt {
                if pdo.max_power_w >= target_watts {
                    if let Some(current_best) = best_pdo {
                        if pdo.voltage_mv > current_best.voltage_mv {
                            best_pdo = Some(*pdo);
                        }
                    } else {
                        best_pdo = Some(*pdo);
                    }
                }
            }
        }

        let selected = match best_pdo {
            Some(p) => p,
            None => {
                let mut max_p: Option<PdPowerDataObject> = None;
                for cap_opt in self.source_caps.iter() {
                    if let Some(pdo) = cap_opt {
                        if let Some(cur) = max_p {
                            if pdo.max_power_w > cur.max_power_w {
                                max_p = Some(*pdo);
                            }
                        } else {
                            max_p = Some(*pdo);
                        }
                    }
                }
                match max_p {
                    Some(p) => p,
                    None => return Err("No valid PDOs available"),
                }
            }
        };

        let rdo = PdRequestDataObject {
            object_position: selected.position,
            requested_voltage_mv: selected.voltage_mv,
            operating_current_ma: selected.max_current_ma,
            max_current_ma: selected.max_current_ma,
            capability_mismatch: selected.max_power_w < target_watts,
            give_back_flag: false,
        };

        self.active_contract_rdo = rdo;
        self.negotiated_voltage_mv = selected.voltage_mv;
        self.negotiated_current_ma = selected.max_current_ma;
        self.negotiated_power_w = (selected.voltage_mv as f32 * selected.max_current_ma as f32) / 1_000_000.0;
        self.epr_mode_active = selected.voltage_mv > 20_000;
        self.state = EprNegotiationState::ContractEstablished;

        Ok(rdo)
    }

    /// Detaches power supply and resets contract state.
    pub fn on_detach(&mut self) {
        self.state = EprNegotiationState::Unattached;
        self.source_caps = [None; 7];
        self.active_contract_rdo = PdRequestDataObject::empty();
        self.negotiated_voltage_mv = 0;
        self.negotiated_current_ma = 0;
        self.negotiated_power_w = 0.0;
        self.epr_mode_active = false;
    }
}

/// Unit test verifying USB-PD 3.1 EPR contract negotiation and PDO/RDO selection.
pub fn verify_pd31_epr_negotiation() -> bool {
    let mut negotiator = EprContractNegotiator::new();
    if negotiator.state != EprNegotiationState::Unattached {
        return false;
    }

    // Standard gaming handheld PD 3.1 EPR charger offering 5V, 9V, 15V, 20V (100W), 28V (140W), 48V (240W)
    let caps = [
        PdPowerDataObject::new_fixed(1, 5_000, 3_000),   // 15W
        PdPowerDataObject::new_fixed(2, 9_000, 3_000),   // 27W
        PdPowerDataObject::new_fixed(3, 15_000, 3_000),  // 45W
        PdPowerDataObject::new_fixed(4, 20_000, 5_000),  // 100W SPR
        PdPowerDataObject::epr_140w(5),                  // 140W EPR (28V/5A)
        PdPowerDataObject::epr_240w(6),                  // 240W EPR (48V/5A)
        PdPowerDataObject::new_avs(7, 15_000, 48_000, 5_000), // AVS 15-48V
    ];

    if negotiator.receive_source_capabilities(&caps).is_err() {
        return false;
    }
    if negotiator.state != EprNegotiationState::SourceCapsReceived {
        return false;
    }

    // Request 120W gaming load: should pick 28V/5A (140W) or 48V/5A (240W) for low resistance
    let rdo = match negotiator.negotiate_optimal_contract(120.0) {
        Ok(r) => r,
        Err(_) => return false,
    };

    if negotiator.state != EprNegotiationState::ContractEstablished || !negotiator.epr_mode_active {
        return false;
    }
    if rdo.requested_voltage_mv != 48_000 && rdo.requested_voltage_mv != 28_000 {
        return false;
    }
    if negotiator.negotiated_power_w < 120.0 {
        return false;
    }

    negotiator.on_detach();
    if negotiator.state != EprNegotiationState::Unattached || negotiator.epr_mode_active {
        return false;
    }

    true
}


// ============================================================================
// Thermal Bypass Governor for High-Load APU Power Path Switching
// ============================================================================

/// Power path routing mode for motherboard and battery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BypassPowerMode {
    /// No AC supply connected; battery powers motherboard.
    BatteryDischarge,
    /// AC supply powers motherboard and concurrently charges battery pack.
    SimultaneousCharge,
    /// AC supply powers motherboard directly; battery charging FET isolated.
    DirectBypass,
}

/// Hardware power path MOSFET switch state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerPathSwitchState {
    /// AC input power FET closed (conducting).
    pub ac_input_closed: bool,
    /// Battery pack charge FET closed (conducting).
    pub batt_charge_closed: bool,
    /// Battery pack discharge FET closed (conducting).
    pub batt_discharge_closed: bool,
}

impl PowerPathSwitchState {
    pub const fn new() -> Self {
        Self {
            ac_input_closed: false,
            batt_charge_closed: false,
            batt_discharge_closed: true,
        }
    }
}

/// Governor evaluating APU TDP and battery thermals to engage direct bypass.
#[derive(Debug, Clone)]
pub struct ThermalBypassGovernor {
    /// High-load APU TDP threshold in Watts (e.g. 25.0W).
    pub apu_tdp_threshold_w: f32,
    /// Hysteresis recovery TDP threshold in Watts (e.g. 20.0W).
    pub apu_tdp_recovery_w: f32,
    /// Battery pack critical thermal threshold in Celsius (e.g. 42.0°C).
    pub batt_temp_threshold_c: f32,
    /// Battery thermal recovery threshold in Celsius (e.g. 38.0°C).
    pub batt_temp_recovery_c: f32,
    /// Current power path mode.
    pub current_mode: BypassPowerMode,
    /// Active switch configuration.
    pub switches: PowerPathSwitchState,
    /// Whether direct thermal bypass is currently engaged.
    pub bypass_engaged: bool,
    /// Consecutive tick counter for hysteresis recovery.
    pub recovery_tick_count: u32,
    /// Total bypass trigger events.
    pub bypass_trigger_count: u32,
}

impl ThermalBypassGovernor {
    pub const fn new() -> Self {
        Self {
            apu_tdp_threshold_w: 25.0,
            apu_tdp_recovery_w: 20.0,
            batt_temp_threshold_c: 42.0,
            batt_temp_recovery_c: 38.0,
            current_mode: BypassPowerMode::BatteryDischarge,
            switches: PowerPathSwitchState::new(),
            bypass_engaged: false,
            recovery_tick_count: 0,
            bypass_trigger_count: 0,
        }
    }

    /// Evaluates system thermals and APU power to configure power path routing.
    pub fn evaluate(
        &mut self,
        ac_connected: bool,
        ac_power_w: f32,
        apu_tdp_w: f32,
        batt_temp_c: f32,
    ) -> BypassPowerMode {
        if !ac_connected || ac_power_w < 15.0 {
            self.current_mode = BypassPowerMode::BatteryDischarge;
            self.switches.ac_input_closed = false;
            self.switches.batt_charge_closed = false;
            self.switches.batt_discharge_closed = true;
            self.bypass_engaged = false;
            self.recovery_tick_count = 0;
            return self.current_mode;
        }

        self.switches.ac_input_closed = true;
        self.switches.batt_discharge_closed = false;

        let thermal_trip = batt_temp_c >= self.batt_temp_threshold_c;
        let tdp_trip = apu_tdp_w >= self.apu_tdp_threshold_w;

        if tdp_trip || thermal_trip {
            self.bypass_engaged = true;
            self.recovery_tick_count = 0;
            self.bypass_trigger_count += 1;
        } else if self.bypass_engaged {
            if apu_tdp_w <= self.apu_tdp_recovery_w && batt_temp_c <= self.batt_temp_recovery_c {
                self.recovery_tick_count += 1;
                if self.recovery_tick_count >= 3 {
                    self.bypass_engaged = false;
                    self.recovery_tick_count = 0;
                }
            } else {
                self.recovery_tick_count = 0;
            }
        }

        if self.bypass_engaged {
            self.current_mode = BypassPowerMode::DirectBypass;
            self.switches.batt_charge_closed = false;
        } else {
            self.current_mode = BypassPowerMode::SimultaneousCharge;
            self.switches.batt_charge_closed = true;
        }

        self.current_mode
    }
}

/// Unit test verifying thermal bypass governor under heavy APU gaming load.
pub fn verify_thermal_bypass_governor() -> bool {
    let mut gov = ThermalBypassGovernor::new();

    // 1. Unplugged: battery discharge mode
    let m0 = gov.evaluate(false, 0.0, 15.0, 30.0);
    if m0 != BypassPowerMode::BatteryDischarge || gov.bypass_engaged {
        return false;
    }

    // 2. Plugged in, light load (15W APU TDP, 32°C): simultaneous charging
    let m1 = gov.evaluate(true, 100.0, 15.0, 32.0);
    if m1 != BypassPowerMode::SimultaneousCharge || !gov.switches.batt_charge_closed {
        return false;
    }

    // 3. Heavy gaming load (28W APU TDP > 25W threshold): trigger direct bypass
    let m2 = gov.evaluate(true, 100.0, 28.0, 35.0);
    if m2 != BypassPowerMode::DirectBypass || !gov.bypass_engaged || gov.switches.batt_charge_closed {
        return false;
    }

    // 4. Hysteresis: TDP drops to 22W (between 20W and 25W): bypass remains engaged
    let m3 = gov.evaluate(true, 100.0, 22.0, 35.0);
    if m3 != BypassPowerMode::DirectBypass || !gov.bypass_engaged {
        return false;
    }

    // 5. Recovery: TDP drops to 18W (< 20W) for 3 ticks: transitions back to simultaneous charge
    gov.evaluate(true, 100.0, 18.0, 34.0);
    gov.evaluate(true, 100.0, 18.0, 34.0);
    let m_rec = gov.evaluate(true, 100.0, 18.0, 34.0);
    if m_rec != BypassPowerMode::SimultaneousCharge || gov.bypass_engaged || !gov.switches.batt_charge_closed {
        return false;
    }

    // 6. Thermal trip: low TDP (18W) but battery overheats (43°C >= 42°C threshold)
    let m_hot = gov.evaluate(true, 100.0, 18.0, 43.0);
    if m_hot != BypassPowerMode::DirectBypass || !gov.bypass_engaged {
        return false;
    }

    true
}
