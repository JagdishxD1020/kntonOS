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
