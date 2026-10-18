//! Handheld USB-C Billboard Device & DisplayPort Alternate Mode Firmware Handshake Driver.
//!
//! In handheld gaming consoles (such as Steam Deck, ROG Ally, Legion Go), USB-C docks and external
//! monitors use USB Power Delivery (USB-PD) Structured Vendor Defined Messages (VDMs) to negotiate
//! DisplayPort (DP) Alternate Mode for high-refresh video output.
//! When a dock, monitor, or cable fails to negotiate DP Alt Mode (e.g. unpowered dock, inadequate wattage,
//! incompatible cable, or unsupported pin assignments), the USB-C Billboard Device Class 1.2.2 exposes
//! human-readable diagnostic status and failure codes to the host operating system.
//!
//! This module implements:
//! 1. USB Billboard Device Class 1.2.2 descriptor parsing and Alternate Mode status tracking.
//! 2. Firmware handshake state machine (handling DP Alt Mode enter/exit, VDM framing).
//! 3. User notification & diagnostic logging for unpowered docks, inadequate PD wattage (<45W), and non-compliant cables.
//! 4. Fast fallback modes: fall back to high-speed USB 2.0/3.2 data transfer if DP Alt Mode video channel negotiation fails.

#![allow(dead_code)]

// ============================================================================
// Feature 1: USB Billboard Device Class 1.2.2 Descriptors & Status Tracking
// ============================================================================

/// USB Descriptor Types for Billboard Device Class.
pub mod descriptor_types {
    pub const DEVICE_CAPABILITY: u8 = 0x10;
    pub const BILLBOARD_CAPABILITY: u8 = 0x0D;
    pub const BILLBOARD_ALT_MODE_CAPABILITY: u8 = 0x0F;
}

/// Standard Vendor IDs (SVIDs) for USB-C Alternate Modes.
pub mod alt_mode_svid {
    pub const DISPLAYPORT: u16 = 0xFF01;
    pub const THUNDERBOLT: u16 = 0x8087;
}

/// Maximum number of Alternate Modes supported per billboard endpoint.
pub const MAX_BILLBOARD_ALT_MODES: usize = 4;

/// Configuration state of an Alternate Mode as reported by the Billboard descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillboardAltModeState {
    /// Alternate Mode configuration not attempted.
    NotConfigured = 0,
    /// Configuration attempted but failed to establish video/audio link.
    ConfigurationFailed = 1,
    /// Successfully configured and operational in Alternate Mode.
    ConfiguredAndOperational = 2,
    /// Alternate Mode configuration encountered severe hardware/protocol error.
    ConfigurationError = 3,
}

impl BillboardAltModeState {
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::NotConfigured,
            1 => Self::ConfigurationFailed,
            2 => Self::ConfiguredAndOperational,
            _ => Self::ConfigurationError,
        }
    }
}

/// Vconn power requirements specified by Billboard Device Class 1.2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VconnPowerRequirement {
    Power1W = 0,
    Power1_5W = 1,
    Power2W = 2,
    Power3W = 3,
    Power4W = 4,
    Power5W = 5,
    Power6W = 6,
    PowerReserved = 7,
}

/// Parsed Alternate Mode entry from Billboard descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BillboardAltModeEntry {
    /// Standard or Vendor ID (e.g. 0xFF01 for DisplayPort).
    pub svid: u16,
    /// Alternate mode index within the SVID.
    pub alt_mode_index: u8,
    /// Operational configuration state.
    pub state: BillboardAltModeState,
    /// String descriptor index providing localized error/diagnostic text.
    pub string_index: u8,
}

impl BillboardAltModeEntry {
    pub const fn empty() -> Self {
        Self {
            svid: 0,
            alt_mode_index: 0,
            state: BillboardAltModeState::NotConfigured,
            string_index: 0,
        }
    }
}

/// USB Billboard Device Class 1.2.2 Capability Descriptor.
#[derive(Debug, Clone)]
pub struct BillboardCapabilityDescriptor {
    /// Total length of descriptor in bytes.
    pub length: u8,
    /// Descriptor type (0x10 = Device Capability).
    pub descriptor_type: u8,
    /// Capability type (0x0D = Billboard Capability).
    pub capability_type: u8,
    /// String descriptor index for URL providing user-facing troubleshooting advice.
    pub additional_info_url_index: u8,
    /// Number of Alternate Modes supported by dock/device.
    pub number_of_alternate_modes: u8,
    /// Index of preferred Alternate Mode (0-indexed).
    pub preferred_alternate_mode: u8,
    /// Vconn power needed for adapter circuitry.
    pub vconn_power: VconnPowerRequirement,
    /// Version of Billboard specification (e.g. 0x0122 for 1.2.2).
    pub bcd_version: u16,
    /// Failure reason code (0 = Normal, 1 = Config failed, 2 = Power rejected).
    pub failure_code: u8,
    /// Parsed Alternate Mode status table.
    pub alt_modes: [BillboardAltModeEntry; MAX_BILLBOARD_ALT_MODES],
}

impl BillboardCapabilityDescriptor {
    /// Parses a raw USB 3.2 BOS (Binary Device Object Store) billboard capability slice.
    pub fn parse(raw_bytes: &[u8]) -> Result<Self, &'static str> {
        if raw_bytes.len() < 44 {
            return Err("Billboard descriptor slice too short (minimum 44 bytes)");
        }

        let length = raw_bytes[0];
        let descriptor_type = raw_bytes[1];
        let capability_type = raw_bytes[2];

        if descriptor_type != descriptor_types::DEVICE_CAPABILITY || capability_type != descriptor_types::BILLBOARD_CAPABILITY {
            return Err("Invalid Billboard Capability descriptor header");
        }

        let additional_info_url_index = raw_bytes[3];
        let number_of_alternate_modes = raw_bytes[4];
        let preferred_alternate_mode = raw_bytes[5];
        let vconn_raw = raw_bytes[6] & 0x07;
        let vconn_power = match vconn_raw {
            0 => VconnPowerRequirement::Power1W,
            1 => VconnPowerRequirement::Power1_5W,
            2 => VconnPowerRequirement::Power2W,
            3 => VconnPowerRequirement::Power3W,
            4 => VconnPowerRequirement::Power4W,
            5 => VconnPowerRequirement::Power5W,
            6 => VconnPowerRequirement::Power6W,
            _ => VconnPowerRequirement::PowerReserved,
        };

        // bmConfigured is a 32-bit field at offset 8 (bytes 8..12): 2 bits per mode
        let bm_configured = u32::from_le_bytes([raw_bytes[8], raw_bytes[9], raw_bytes[10], raw_bytes[11]]);
        let bcd_version = u16::from_le_bytes([raw_bytes[12], raw_bytes[13]]);
        let failure_code = raw_bytes[14];

        let mut alt_modes = [BillboardAltModeEntry::empty(); MAX_BILLBOARD_ALT_MODES];
        let modes_count = if (number_of_alternate_modes as usize) > MAX_BILLBOARD_ALT_MODES {
            MAX_BILLBOARD_ALT_MODES
        } else {
            number_of_alternate_modes as usize
        };

        // Mode entries start at offset 44 (each entry is 4 bytes: wSVID (2), bAlternateMode (1), iAlternateModeString (1))
        let mut offset = 44;
        for i in 0..modes_count {
            if offset + 4 <= raw_bytes.len() {
                let svid = u16::from_le_bytes([raw_bytes[offset], raw_bytes[offset + 1]]);
                let alt_mode_index = raw_bytes[offset + 2];
                let string_index = raw_bytes[offset + 3];
                let state_bits = ((bm_configured >> (i * 2)) & 0x03) as u8;
                let state = BillboardAltModeState::from_bits(state_bits);

                alt_modes[i] = BillboardAltModeEntry {
                    svid,
                    alt_mode_index,
                    state,
                    string_index,
                };
                offset += 4;
            }
        }

        Ok(Self {
            length,
            descriptor_type,
            capability_type,
            additional_info_url_index,
            number_of_alternate_modes,
            preferred_alternate_mode,
            vconn_power,
            bcd_version,
            failure_code,
            alt_modes,
        })
    }

    /// Finds DisplayPort Alternate Mode status if present.
    pub fn get_displayport_alt_mode(&self) -> Option<&BillboardAltModeEntry> {
        for mode in &self.alt_modes {
            if mode.svid == alt_mode_svid::DISPLAYPORT {
                return Some(mode);
            }
        }
        None
    }
}

/// Verification test for USB Billboard Device Class 1.2.2 descriptor parsing.
pub fn verify_billboard_descriptor_parsing_and_status() -> bool {
    // Construct synthetic 48-byte Billboard Capability Descriptor:
    // DisplayPort SVID 0xFF01 on mode 0, state = ConfigurationFailed (01b in bmConfigured)
    let mut raw_desc = [0u8; 48];
    raw_desc[0] = 48; // bLength
    raw_desc[1] = descriptor_types::DEVICE_CAPABILITY; // 0x10
    raw_desc[2] = descriptor_types::BILLBOARD_CAPABILITY; // 0x0D
    raw_desc[3] = 4;  // iAdditionalInfoURL index
    raw_desc[4] = 1;  // bNumberOfAlternateModes
    raw_desc[5] = 0;  // bPreferredAlternateMode
    raw_desc[6] = 1;  // VconnPower = 1.5W
    raw_desc[7] = 0;  // Reserved
    // bmConfigured: Mode 0 = 01b (Configuration Failed) -> 0x00000001
    raw_desc[8] = 0x01;
    raw_desc[9] = 0x00;
    raw_desc[10] = 0x00;
    raw_desc[11] = 0x00;
    // bcdVersion: 0x0122 (v1.2.2)
    raw_desc[12] = 0x22;
    raw_desc[13] = 0x01;
    // bAdditionalFailureInfo: 1 (Specified Alt Mode Failed)
    raw_desc[14] = 1;

    // Offset 44: wSVID = 0xFF01 (DisplayPort)
    raw_desc[44] = 0x01;
    raw_desc[45] = 0xFF;
    // bAlternateMode = 0
    raw_desc[46] = 0;
    // iAlternateModeString = 6
    raw_desc[47] = 6;

    let parsed = match BillboardCapabilityDescriptor::parse(&raw_desc) {
        Ok(p) => p,
        Err(_) => return false,
    };

    if parsed.length != 48 || parsed.number_of_alternate_modes != 1 {
        return false;
    }
    if parsed.bcd_version != 0x0122 || parsed.vconn_power != VconnPowerRequirement::Power1_5W {
        return false;
    }

    let dp_mode = match parsed.get_displayport_alt_mode() {
        Some(m) => m,
        None => return false,
    };

    if dp_mode.svid != alt_mode_svid::DISPLAYPORT {
        return false;
    }
    if dp_mode.state != BillboardAltModeState::ConfigurationFailed {
        return false;
    }
    if dp_mode.string_index != 6 {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: USB-PD VDM Message Framing & DisplayPort Alt Mode Handshake
// ============================================================================

/// USB-PD Structured VDM Command Codes (Table 6-29 USB PD Spec).
pub mod vdm_commands {
    pub const DISCOVER_IDENTITY: u8 = 0x01;
    pub const DISCOVER_SVIDS: u8 = 0x02;
    pub const DISCOVER_MODES: u8 = 0x03;
    pub const ENTER_MODE: u8 = 0x04;
    pub const EXIT_MODE: u8 = 0x05;
    pub const DP_STATUS_UPDATE: u8 = 0x10;
    pub const DP_CONFIGURE: u8 = 0x11;
}

/// USB-PD Structured VDM Command Types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VdmCommandType {
    InitiatorReq = 0,
    ResponderAck = 1,
    ResponderNak = 2,
    ResponderBusy = 3,
}

impl VdmCommandType {
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::InitiatorReq,
            1 => Self::ResponderAck,
            2 => Self::ResponderNak,
            _ => Self::ResponderBusy,
        }
    }
}

/// DisplayPort Pin Assignment configurations over USB Type-C.
pub mod dp_pin_assignments {
    /// Pin Assignment C: All 4 high-speed lanes dedicated to DisplayPort video (up to 8K / 4K 144Hz, no USB 3.x data).
    pub const PIN_C: u8 = 1 << 2;
    /// Pin Assignment D: 2 lanes DisplayPort video + 2 lanes SuperSpeed USB 3.2 Gen 2 (up to 4K 60Hz + 10Gbps USB data).
    pub const PIN_D: u8 = 1 << 3;
    /// Pin Assignment E: 4 lanes DisplayPort video on DP plug/receptacle.
    pub const PIN_E: u8 = 1 << 4;
}

/// 32-bit Structured VDM Header (USB-PD Spec Section 6.4.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructuredVdmHeader {
    /// Standard or Vendor ID (e.g. 0xFF01 for DisplayPort).
    pub svid: u16,
    /// VDM Type: true for Structured VDM.
    pub structured: bool,
    /// SVDM Version: 0 = 1.0, 1 = 2.0.
    pub version: u8,
    /// Object Position (1-7 for mode index).
    pub obj_pos: u8,
    /// Command Type: InitiatorReq, ResponderAck, ResponderNak, ResponderBusy.
    pub cmd_type: VdmCommandType,
    /// Command: Discover Identity, SVIDs, Modes, Enter Mode, Exit Mode, DP Status, DP Configure.
    pub command: u8,
}

impl StructuredVdmHeader {
    pub const fn new(svid: u16, obj_pos: u8, cmd_type: VdmCommandType, command: u8) -> Self {
        Self {
            svid,
            structured: true,
            version: 1, // SVDM v2.0
            obj_pos,
            cmd_type,
            command,
        }
    }

    /// Packs into 32-bit little-endian wire word.
    pub const fn to_u32(&self) -> u32 {
        let svid_field = (self.svid as u32) << 16;
        let struct_bit = if self.structured { 1 << 15 } else { 0 };
        let ver_field = ((self.version & 0x03) as u32) << 13;
        let pos_field = ((self.obj_pos & 0x07) as u32) << 8;
        let type_field = ((self.cmd_type as u8 & 0x03) as u32) << 6;
        let cmd_field = (self.command & 0x1F) as u32;

        svid_field | struct_bit | ver_field | pos_field | type_field | cmd_field
    }

    /// Unpacks from 32-bit wire word.
    pub const fn from_u32(raw: u32) -> Self {
        let svid = (raw >> 16) as u16;
        let structured = (raw & (1 << 15)) != 0;
        let version = ((raw >> 13) & 0x03) as u8;
        let obj_pos = ((raw >> 8) & 0x07) as u8;
        let cmd_type = VdmCommandType::from_bits(((raw >> 6) & 0x03) as u8);
        let command = (raw & 0x1F) as u8;

        Self {
            svid,
            structured,
            version,
            obj_pos,
            cmd_type,
            command,
        }
    }
}

/// DisplayPort Alt Mode Firmware Handshake Operational State.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpAltModeState {
    /// No dock or display attached to USB-C port.
    Detached,
    /// Discovering device capabilities via USB-PD VDM.
    DiscoveringCapabilities,
    /// Querying supported SVIDs (confirming 0xFF01 presence).
    DiscoveringSvids,
    /// SVID confirmed; discovering DisplayPort mode VDOs.
    DiscoveringModes,
    /// Sending Enter Mode request for DisplayPort.
    NegotiatingEnterMode,
    /// Sending DP Configure VDO (selecting Pin Assignment C or D).
    ConfiguringDp,
    /// DisplayPort Alternate Mode active; HPD high and video streaming.
    ActiveDpAltMode,
    /// Teardown of Alternate Mode in progress (sending Exit Mode).
    ExitModeInProgress,
    /// Handshake rejected by responder or cable mismatch; fallback required.
    NegotiationFailed,
}

/// Firmware Handshake State Machine Controller for DisplayPort Alternate Mode.
#[derive(Debug, Clone)]
pub struct DpAltModeHandshakeController {
    /// Current state machine step.
    pub state: DpAltModeState,
    /// Target pin assignment (Pin C = 4-lane DP, Pin D = 2-lane DP + USB 3 data).
    pub selected_pin_assignment: u8,
    /// Whether host prefers concurrent USB 3 data multiplexing (Pin D).
    pub multi_function_preferred: bool,
    /// DisplayPort Hot Plug Detect (HPD) status line.
    pub hpd_high: bool,
    /// Count of IRQ_HPD interrupt pulses (e.g. for display resolution renegotiation).
    pub hpd_irq_count: u32,
    /// Count of VDM message transactions exchanged.
    pub vdm_transaction_count: u32,
    /// Last VDM command sent or processed.
    pub last_vdm_command: u8,
    /// Error reason if negotiation failed.
    pub failure_reason: Option<&'static str>,
}

impl DpAltModeHandshakeController {
    pub const fn new(prefer_multi_function: bool) -> Self {
        let pin = if prefer_multi_function {
            dp_pin_assignments::PIN_D
        } else {
            dp_pin_assignments::PIN_C
        };

        Self {
            state: DpAltModeState::Detached,
            selected_pin_assignment: pin,
            multi_function_preferred: prefer_multi_function,
            hpd_high: false,
            hpd_irq_count: 0,
            vdm_transaction_count: 0,
            last_vdm_command: 0,
            failure_reason: None,
        }
    }

    /// Initiates USB-PD VDM discovery handshake upon physical USB-C cable attach.
    pub fn on_cable_attach(&mut self) -> StructuredVdmHeader {
        self.state = DpAltModeState::DiscoveringCapabilities;
        self.vdm_transaction_count += 1;
        self.last_vdm_command = vdm_commands::DISCOVER_IDENTITY;
        StructuredVdmHeader::new(0xFF00, 0, VdmCommandType::InitiatorReq, vdm_commands::DISCOVER_IDENTITY)
    }

    /// Processes responder VDM packet and drives handshake forward.
    /// Returns next VDM request header if sequence continues.
    pub fn process_vdm_response(
        &mut self,
        header: &StructuredVdmHeader,
        payload_vdo: u32,
    ) -> Result<Option<StructuredVdmHeader>, &'static str> {
        self.vdm_transaction_count += 1;

        // If responder NAKed our request, abort to failed state
        if header.cmd_type == VdmCommandType::ResponderNak {
            self.state = DpAltModeState::NegotiationFailed;
            self.failure_reason = Some("Responder rejected VDM request with NAK");
            return Ok(None);
        }

        match self.state {
            DpAltModeState::DiscoveringCapabilities => {
                // Identity acknowledged, now discover supported SVIDs
                self.state = DpAltModeState::DiscoveringSvids;
                self.last_vdm_command = vdm_commands::DISCOVER_SVIDS;
                Ok(Some(StructuredVdmHeader::new(0xFF00, 0, VdmCommandType::InitiatorReq, vdm_commands::DISCOVER_SVIDS)))
            }
            DpAltModeState::DiscoveringSvids => {
                // SVIDs discovered, query modes for DisplayPort SVID (0xFF01)
                self.state = DpAltModeState::DiscoveringModes;
                self.last_vdm_command = vdm_commands::DISCOVER_MODES;
                Ok(Some(StructuredVdmHeader::new(alt_mode_svid::DISPLAYPORT, 0, VdmCommandType::InitiatorReq, vdm_commands::DISCOVER_MODES)))
            }
            DpAltModeState::DiscoveringModes => {
                // Modes discovered: payload contains supported pin assignments
                // Check if responder supports our desired pin assignment
                let supported_pins = ((payload_vdo >> 8) & 0xFF) as u8;
                if (supported_pins & self.selected_pin_assignment) == 0 {
                    // Try fallback to Pin C if Pin D unsupported, or vice versa
                    if (supported_pins & dp_pin_assignments::PIN_C) != 0 {
                        self.selected_pin_assignment = dp_pin_assignments::PIN_C;
                    } else if (supported_pins & dp_pin_assignments::PIN_D) != 0 {
                        self.selected_pin_assignment = dp_pin_assignments::PIN_D;
                    } else {
                        self.state = DpAltModeState::NegotiationFailed;
                        self.failure_reason = Some("No compatible DisplayPort pin assignments supported");
                        return Ok(None);
                    }
                }

                // Send Enter Mode request for Mode 1
                self.state = DpAltModeState::NegotiatingEnterMode;
                self.last_vdm_command = vdm_commands::ENTER_MODE;
                Ok(Some(StructuredVdmHeader::new(alt_mode_svid::DISPLAYPORT, 1, VdmCommandType::InitiatorReq, vdm_commands::ENTER_MODE)))
            }
            DpAltModeState::NegotiatingEnterMode => {
                // Enter Mode ACK received, now configure DisplayPort parameters
                self.state = DpAltModeState::ConfiguringDp;
                self.last_vdm_command = vdm_commands::DP_CONFIGURE;
                Ok(Some(StructuredVdmHeader::new(alt_mode_svid::DISPLAYPORT, 1, VdmCommandType::InitiatorReq, vdm_commands::DP_CONFIGURE)))
            }
            DpAltModeState::ConfiguringDp => {
                // Configuration ACK received; Alternate Mode is now active!
                self.state = DpAltModeState::ActiveDpAltMode;
                self.hpd_high = true;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Handles Hot Plug Detect (HPD) status updates from dock or monitor.
    pub fn on_hpd_update(&mut self, hpd_high: bool, is_irq: bool) {
        self.hpd_high = hpd_high;
        if is_irq {
            self.hpd_irq_count += 1;
        }
        if !hpd_high && self.state == DpAltModeState::ActiveDpAltMode {
            // Monitor unplugged on downstream dock port: tear down Alt Mode
            self.state = DpAltModeState::ExitModeInProgress;
        }
    }

    /// Requests graceful teardown of DisplayPort Alternate Mode.
    pub fn initiate_exit_mode(&mut self) -> StructuredVdmHeader {
        self.state = DpAltModeState::ExitModeInProgress;
        self.last_vdm_command = vdm_commands::EXIT_MODE;
        self.vdm_transaction_count += 1;
        StructuredVdmHeader::new(alt_mode_svid::DISPLAYPORT, 1, VdmCommandType::InitiatorReq, vdm_commands::EXIT_MODE)
    }
}

/// Verification test for USB-PD VDM message framing and DisplayPort Alt Mode handshake.
pub fn verify_vdm_framing_and_dp_altmode_handshake() -> bool {
    // 1. Verify 32-bit VDM header wire framing
    let req = StructuredVdmHeader::new(alt_mode_svid::DISPLAYPORT, 1, VdmCommandType::InitiatorReq, vdm_commands::ENTER_MODE);
    let wire = req.to_u32();
    let unpacked = StructuredVdmHeader::from_u32(wire);

    if unpacked.svid != alt_mode_svid::DISPLAYPORT || unpacked.command != vdm_commands::ENTER_MODE {
        return false;
    }
    if unpacked.obj_pos != 1 || unpacked.cmd_type != VdmCommandType::InitiatorReq {
        return false;
    }

    // 2. Drive full handshake sequence from cable attach through ActiveDpAltMode
    let mut ctrl = DpAltModeHandshakeController::new(true); // Prefer Pin D (DP + USB 3 data)
    if ctrl.state != DpAltModeState::Detached {
        return false;
    }

    let h1 = ctrl.on_cable_attach();
    if ctrl.state != DpAltModeState::DiscoveringCapabilities || h1.command != vdm_commands::DISCOVER_IDENTITY {
        return false;
    }

    // Step 1: ACK to Discover Identity
    let ack_id = StructuredVdmHeader::new(0xFF00, 0, VdmCommandType::ResponderAck, vdm_commands::DISCOVER_IDENTITY);
    let h2 = match ctrl.process_vdm_response(&ack_id, 0) {
        Ok(Some(h)) => h,
        _ => return false,
    };
    if ctrl.state != DpAltModeState::DiscoveringSvids || h2.command != vdm_commands::DISCOVER_SVIDS {
        return false;
    }

    // Step 2: ACK to Discover SVIDs
    let ack_svid = StructuredVdmHeader::new(0xFF00, 0, VdmCommandType::ResponderAck, vdm_commands::DISCOVER_SVIDS);
    let h3 = match ctrl.process_vdm_response(&ack_svid, 0) {
        Ok(Some(h)) => h,
        _ => return false,
    };
    if ctrl.state != DpAltModeState::DiscoveringModes || h3.command != vdm_commands::DISCOVER_MODES {
        return false;
    }

    // Step 3: ACK to Discover Modes: return payload with Pin C (0x04) and Pin D (0x08) supported -> (0x0C << 8)
    let ack_modes = StructuredVdmHeader::new(alt_mode_svid::DISPLAYPORT, 0, VdmCommandType::ResponderAck, vdm_commands::DISCOVER_MODES);
    let h4 = match ctrl.process_vdm_response(&ack_modes, ((dp_pin_assignments::PIN_C | dp_pin_assignments::PIN_D) as u32) << 8) {
        Ok(Some(h)) => h,
        _ => return false,
    };
    if ctrl.state != DpAltModeState::NegotiatingEnterMode || h4.command != vdm_commands::ENTER_MODE {
        return false;
    }

    // Step 4: ACK to Enter Mode
    let ack_enter = StructuredVdmHeader::new(alt_mode_svid::DISPLAYPORT, 1, VdmCommandType::ResponderAck, vdm_commands::ENTER_MODE);
    let h5 = match ctrl.process_vdm_response(&ack_enter, 0) {
        Ok(Some(h)) => h,
        _ => return false,
    };
    if ctrl.state != DpAltModeState::ConfiguringDp || h5.command != vdm_commands::DP_CONFIGURE {
        return false;
    }

    // Step 5: ACK to DP Configure
    let ack_cfg = StructuredVdmHeader::new(alt_mode_svid::DISPLAYPORT, 1, VdmCommandType::ResponderAck, vdm_commands::DP_CONFIGURE);
    let res = ctrl.process_vdm_response(&ack_cfg, 0);
    if res.is_err() || res.unwrap().is_some() {
        return false;
    }
    if ctrl.state != DpAltModeState::ActiveDpAltMode || !ctrl.hpd_high {
        return false;
    }

    // Step 6: Test HPD IRQ event
    ctrl.on_hpd_update(true, true);
    if ctrl.hpd_irq_count != 1 {
        return false;
    }

    // Step 7: Test downstream display disconnect (HPD low)
    ctrl.on_hpd_update(false, false);
    if ctrl.state != DpAltModeState::ExitModeInProgress || ctrl.hpd_high {
        return false;
    }

    true
}
