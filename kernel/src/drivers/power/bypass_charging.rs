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
