//! SteamDeck USB-C Power Delivery & DisplayPort Alt-Mode Dock Daemon (TASK-KNT-331).
//!
//! Provides USB Type-C CC line detection and role resolution, USB-PD 3.0 45W contract
//! negotiation, DisplayPort Alt-Mode pin configuration, Steam Deck Dock hub setup,
//! and automated external display/audio hotplug orchestration.

#![allow(dead_code)]

// ============================================================================
// Feature 1: USB Type-C CC Pin Orientation & Role Negotiation State Machine
// ============================================================================

/// Physical orientation of the reversible USB Type-C connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CcPinOrientation {
    Unattached,
    Cc1Normal,
    Cc2Flipped,
    AudioOrDebugAccessory,
}

/// Power and data roles negotiated over the CC channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerDataRole {
    /// Consumer of power, Upstream Facing Port (UFP) for data (Deck default when docked).
    SinkUfp,
    /// Provider of power, Downstream Facing Port (DFP) for data (Deck powering flash drive).
    SourceDfp,
    /// Dual Role Power / Data capable port.
    DualRole,
}

/// USB Type-C CC connection state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CcConnectionState {
    Detached,
    AttachWait,
    AttachedSink,
    AttachedSource,
}

/// USB Type-C physical controller and CC logic engine.
#[derive(Debug, Clone, Copy)]
pub struct UsbTypeCController {
    pub orientation: CcPinOrientation,
    pub role: PowerDataRole,
    pub state: CcConnectionState,
    pub cc1_mv: u16,
    pub cc2_mv: u16,
    pub vconn_active: bool,
}

impl UsbTypeCController {
    /// Initializes Type-C controller in unattached Sink/UFP mode.
    pub const fn new() -> Self {
        Self {
            orientation: CcPinOrientation::Unattached,
            role: PowerDataRole::SinkUfp,
            state: CcConnectionState::Detached,
            cc1_mv: 0,
            cc2_mv: 0,
            vconn_active: false,
        }
    }

    /// Evaluates analog CC1 and CC2 pin sensing voltages (in millivolts).
    /// As a Sink (pull-down Rd), an attached Source pulls up CC with Rp (yielding ~400mV to ~2600mV).
    pub fn poll_cc_pins(&mut self, cc1_mv: u16, cc2_mv: u16) -> CcConnectionState {
        self.cc1_mv = cc1_mv;
        self.cc2_mv = cc2_mv;

        let cc1_detected = cc1_mv >= 300 && cc1_mv <= 2600;
        let cc2_detected = cc2_mv >= 300 && cc2_mv <= 2600;

        if cc1_detected && cc2_detected {
            // Both pins active indicate debug or audio adapter accessory
            self.orientation = CcPinOrientation::AudioOrDebugAccessory;
            self.state = CcConnectionState::AttachedSink;
            self.vconn_active = false;
        } else if cc1_detected {
            // CC1 has Rp pull-up => Standard normal orientation
            self.orientation = CcPinOrientation::Cc1Normal;
            self.state = CcConnectionState::AttachedSink;
            self.vconn_active = true; // VCONN supplied on CC2
        } else if cc2_detected {
            // CC2 has Rp pull-up => Flipped orientation
            self.orientation = CcPinOrientation::Cc2Flipped;
            self.state = CcConnectionState::AttachedSink;
            self.vconn_active = true; // VCONN supplied on CC1
        } else {
            self.orientation = CcPinOrientation::Unattached;
            self.state = CcConnectionState::Detached;
            self.vconn_active = false;
        }

        self.state
    }

    /// Explicitly switches power data role.
    pub fn set_role(&mut self, role: PowerDataRole) {
        self.role = role;
    }
}

/// Verification test for USB Type-C CC pin orientation and role negotiation.
pub fn verify_usb_typec_cc_state_machine() -> bool {
    let mut ctrl = UsbTypeCController::new();

    // 1. Initial detached state
    if ctrl.orientation != CcPinOrientation::Unattached || ctrl.state != CcConnectionState::Detached {
        return false;
    }
    if ctrl.vconn_active || ctrl.role != PowerDataRole::SinkUfp {
        return false;
    }

    // 2. Plug in standard dock cable: CC1 = 1650mV (3.0A Rp advertisement), CC2 = 0mV
    let st1 = ctrl.poll_cc_pins(1650, 0);
    if st1 != CcConnectionState::AttachedSink || ctrl.orientation != CcPinOrientation::Cc1Normal {
        return false;
    }
    if !ctrl.vconn_active {
        return false;
    }

    // 3. Flipped cable orientation: CC1 = 0mV, CC2 = 1650mV
    let st2 = ctrl.poll_cc_pins(0, 1650);
    if st2 != CcConnectionState::AttachedSink || ctrl.orientation != CcPinOrientation::Cc2Flipped {
        return false;
    }
    if !ctrl.vconn_active {
        return false;
    }

    // 4. Disconnect cable
    let st3 = ctrl.poll_cc_pins(0, 0);
    if st3 != CcConnectionState::Detached || ctrl.orientation != CcPinOrientation::Unattached {
        return false;
    }
    if ctrl.vconn_active {
        return false;
    }

    true
}

// ============================================================================
// Feature 2: USB Power Delivery 3.0 PDO Contract Negotiation
// ============================================================================

/// USB-PD Power Data Object (Fixed Supply).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerDeliveryObject {
    pub voltage_mv: u32,
    pub max_current_ma: u32,
}

impl PowerDeliveryObject {
    pub const fn fixed(voltage_mv: u32, max_current_ma: u32) -> Self {
        Self {
            voltage_mv,
            max_current_ma,
        }
    }

    pub const fn power_watts(&self) -> u32 {
        (self.voltage_mv * self.max_current_ma) / 1_000_000
    }
}

/// Power Delivery contract state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdContractStatus {
    Default5V,
    Negotiating,
    FastCharge45WActive,
    ContractActive(u32, u32), // Watts, Voltage in mV
    ContractFailed,
}

/// USB Power Delivery 3.0 negotiation engine targeting Steam Deck 45W fast charge.
#[derive(Debug, Clone, Copy)]
pub struct UsbPdProtocolEngine {
    pub status: PdContractStatus,
    pub negotiated_voltage_mv: u32,
    pub negotiated_current_ma: u32,
    pub negotiated_watts: u32,
    pub is_fast_charging_45w: bool,
}

impl UsbPdProtocolEngine {
    pub const fn new() -> Self {
        Self {
            status: PdContractStatus::Default5V,
            negotiated_voltage_mv: 5000,
            negotiated_current_ma: 1500,
            negotiated_watts: 7,
            is_fast_charging_45w: false,
        }
    }

    /// Evaluates Source Capabilities list and requests optimal contract.
    /// Prefers official Steam Deck native 15V@3A (45W) profile.
    pub fn evaluate_source_capabilities(&mut self, pdos: &[PowerDeliveryObject]) -> PdContractStatus {
        if pdos.is_empty() {
            self.status = PdContractStatus::ContractFailed;
            return self.status;
        }

        self.status = PdContractStatus::Negotiating;

        // 1. Check for exact 15V @ >=3000mA (45W) match
        for pdo in pdos {
            if pdo.voltage_mv == 15000 && pdo.max_current_ma >= 3000 {
                self.negotiated_voltage_mv = 15000;
                self.negotiated_current_ma = 3000;
                self.negotiated_watts = 45;
                self.is_fast_charging_45w = true;
                self.status = PdContractStatus::FastCharge45WActive;
                return self.status;
            }
        }

        // 2. Otherwise find highest power supply available
        let mut best_pdo: Option<PowerDeliveryObject> = None;
        let mut best_power = 0;

        for pdo in pdos {
            let pwr = pdo.power_watts();
            if pwr > best_power {
                best_power = pwr;
                best_pdo = Some(*pdo);
            }
        }

        if let Some(best) = best_pdo {
            self.negotiated_voltage_mv = best.voltage_mv;
            self.negotiated_current_ma = best.max_current_ma;
            self.negotiated_watts = best.power_watts();
            self.is_fast_charging_45w = false;
            self.status = PdContractStatus::ContractActive(self.negotiated_watts, self.negotiated_voltage_mv);
        } else {
            self.status = PdContractStatus::ContractFailed;
        }

        self.status
    }
}

/// Verification test for USB Power Delivery 3.0 PDO contract negotiation.
pub fn verify_usb_pd_contract_negotiation() -> bool {
    let mut engine = UsbPdProtocolEngine::new();

    // 1. Initial 5V default
    if engine.status != PdContractStatus::Default5V || engine.is_fast_charging_45w {
        return false;
    }

    // 2. Official Steam Deck 45W charger: offers [5V@3A, 9V@3A, 15V@3A]
    let steam_deck_ac = [
        PowerDeliveryObject::fixed(5000, 3000),
        PowerDeliveryObject::fixed(9000, 3000),
        PowerDeliveryObject::fixed(15000, 3000),
    ];
    let st = engine.evaluate_source_capabilities(&steam_deck_ac);
    if st != PdContractStatus::FastCharge45WActive || !engine.is_fast_charging_45w {
        return false;
    }
    if engine.negotiated_voltage_mv != 15000 || engine.negotiated_watts != 45 {
        return false;
    }

    // 3. Fallback: 65W laptop charger offering [5V@3A, 9V@3A, 15V@3A, 20V@3.25A]
    // Steam Deck engine should still select 15V@3A as official optimal charge curve
    let dock_ac = [
        PowerDeliveryObject::fixed(5000, 3000),
        PowerDeliveryObject::fixed(9000, 3000),
        PowerDeliveryObject::fixed(15000, 3000),
        PowerDeliveryObject::fixed(20000, 3250),
    ];
    let st2 = engine.evaluate_source_capabilities(&dock_ac);
    if st2 != PdContractStatus::FastCharge45WActive || !engine.is_fast_charging_45w {
        return false;
    }

    // 4. Low-power 18W phone charger offering [5V@3A, 9V@2A] -> fallback to 9V 18W
    let phone_ac = [
        PowerDeliveryObject::fixed(5000, 3000),
        PowerDeliveryObject::fixed(9000, 2000),
    ];
    let st3 = engine.evaluate_source_capabilities(&phone_ac);
    if st3 != PdContractStatus::ContractActive(18, 9000) || engine.is_fast_charging_45w {
        return false;
    }

    true
}

// ============================================================================
// Feature 3: DisplayPort Alternate Mode Pin Assignment & HPD Signaling
// ============================================================================

/// DisplayPort Alt-Mode pin assignments over Type-C connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum DpPinAssignment {
    Disabled,
    /// Pin Assignment C: Dedicated DP 4-lane (supports up to 4K120 / 8K, USB 2.0 data only).
    PinAssignmentC_4Lane,
    /// Pin Assignment D: DP 2-lane + USB 3.2 Gen2 10Gbps data (Steam Deck Docking default).
    PinAssignmentD_2LaneUsb3,
}

/// Hot-Plug Detect (HPD) electrical line state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HpdSignalState {
    Low,
    High,
    IrqPulse,
}

/// DisplayPort Alt-Mode signaling controller.
#[derive(Debug, Clone, Copy)]
pub struct DpAltModeController {
    pub pin_assignment: DpPinAssignment,
    pub hpd: HpdSignalState,
    pub lane_count: u8,
    pub link_rate_mbps: u32,
    pub usb3_concurrent: bool,
    pub mst_enabled: bool,
}

impl DpAltModeController {
    pub const fn new() -> Self {
        Self {
            pin_assignment: DpPinAssignment::Disabled,
            hpd: HpdSignalState::Low,
            lane_count: 0,
            link_rate_mbps: 0,
            usb3_concurrent: false,
            mst_enabled: false,
        }
    }

    /// Negotiates VDM pin assignment with dock partner.
    pub fn configure_pin_assignment(&mut self, assignment: DpPinAssignment) {
        self.pin_assignment = assignment;
        match assignment {
            DpPinAssignment::PinAssignmentD_2LaneUsb3 => {
                self.lane_count = 2;
                self.link_rate_mbps = 8100; // HBR3 8.1 Gbps per lane
                self.usb3_concurrent = true; // Enables USB 3.2 Gen2 hub on dock
                self.mst_enabled = true;      // Multi-Stream Transport for dual displays
            }
            DpPinAssignment::PinAssignmentC_4Lane => {
                self.lane_count = 4;
                self.link_rate_mbps = 8100; // HBR3 8.1 Gbps per lane
                self.usb3_concurrent = false; // All 4 differential pairs dedicated to DP
                self.mst_enabled = true;
            }
            DpPinAssignment::Disabled => {
                self.lane_count = 0;
                self.link_rate_mbps = 0;
                self.usb3_concurrent = false;
                self.mst_enabled = false;
            }
        }
    }

    /// Processes HPD state changes from external sink.
    pub fn update_hpd(&mut self, hpd: HpdSignalState) {
        self.hpd = hpd;
    }

    /// Checks if video scanout transmission is ready.
    pub const fn is_scanout_ready(&self) -> bool {
        matches!(self.hpd, HpdSignalState::High) && !matches!(self.pin_assignment, DpPinAssignment::Disabled)
    }
}

/// Verification test for DisplayPort Alt-Mode pin assignment and HPD signaling.
pub fn verify_dp_alt_mode_signaling() -> bool {
    let mut dp = DpAltModeController::new();

    // 1. Initial disabled state
    if dp.pin_assignment != DpPinAssignment::Disabled || dp.is_scanout_ready() {
        return false;
    }

    // 2. Configure Pin Assignment D (Steam Deck Dock default: 2-lane DP + USB 3.2 data)
    dp.configure_pin_assignment(DpPinAssignment::PinAssignmentD_2LaneUsb3);
    if dp.lane_count != 2 || !dp.usb3_concurrent || !dp.mst_enabled {
        return false;
    }
    // HPD is still Low, so scanout not ready yet
    if dp.is_scanout_ready() {
        return false;
    }

    // 3. Monitor asserts HPD High
    dp.update_hpd(HpdSignalState::High);
    if !dp.is_scanout_ready() || dp.hpd != HpdSignalState::High {
        return false;
    }

    // 4. Test HPD IRQ pulse (link retraining request)
    dp.update_hpd(HpdSignalState::IrqPulse);
    if dp.hpd != HpdSignalState::IrqPulse {
        return false;
    }

    // 5. Cable disconnect: HPD Low, Pin Assignment Disabled
    dp.update_hpd(HpdSignalState::Low);
    dp.configure_pin_assignment(DpPinAssignment::Disabled);
    if dp.is_scanout_ready() || dp.lane_count != 0 {
        return false;
    }

    true
}

