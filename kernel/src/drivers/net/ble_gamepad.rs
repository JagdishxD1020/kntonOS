//! Handheld Bluetooth Low Energy (BLE) Gamepad & Low-Latency Audio Transceiver Engine.

#![allow(dead_code)]

/// Supported wireless gamepad profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamepadProfileType {
    XboxWireless,
    DualSense,
    SwitchPro,
    GenericBleHid,
}

/// BLE L2CAP CID definitions.
pub const L2CAP_CID_ATT: u16 = 0x0004;
pub const L2CAP_CID_SIGNALING: u16 = 0x0005;
pub const L2CAP_CID_LE_AUDIO: u16 = 0x007A;

/// BLE HCI packet header parser and L2CAP channel manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BleHciPacketParser {
    pub connection_handle: u16,
    pub pb_flag: u8,
    pub bc_flag: u8,
    pub payload_len: u16,
    pub l2cap_cid: u16,
    pub detected_profile: GamepadProfileType,
}

impl BleHciPacketParser {
    /// Creates a new BLE HCI packet parser.
    pub const fn new() -> Self {
        Self {
            connection_handle: 0,
            pb_flag: 0,
            bc_flag: 0,
            payload_len: 0,
            l2cap_cid: 0,
            detected_profile: GamepadProfileType::GenericBleHid,
        }
    }

    /// Parses an incoming raw HCI ACL data packet:
    /// Bytes 0-1: Handle (12 bits) + PB flag (2 bits) + BC flag (2 bits)
    /// Bytes 2-3: Total Data Length (16 bits)
    /// Bytes 4-5: L2CAP Length (16 bits)
    /// Bytes 6-7: L2CAP Channel ID (16 bits)
    pub fn parse_hci_acl_packet(&mut self, packet: &[u8]) -> bool {
        if packet.len() < 8 {
            return false;
        }

        let handle_flags = u16::from_le_bytes([packet[0], packet[1]]);
        self.connection_handle = handle_flags & 0x0FFF;
        self.pb_flag = ((handle_flags >> 12) & 0x03) as u8;
        self.bc_flag = ((handle_flags >> 14) & 0x03) as u8;

        self.payload_len = u16::from_le_bytes([packet[2], packet[3]]);
        let l2cap_len = u16::from_le_bytes([packet[4], packet[5]]);
        self.l2cap_cid = u16::from_le_bytes([packet[6], packet[7]]);

        if packet.len() < 8 + l2cap_len as usize {
            return false;
        }

        // Inspect ATT payload if CID == 0x0004
        if self.l2cap_cid == L2CAP_CID_ATT && packet.len() >= 11 {
            let att_opcode = packet[8];
            // Handle Value Notification (0x1B) or Read Response (0x0B)
            if att_opcode == 0x1B {
                let report_handle = u16::from_le_bytes([packet[9], packet[10]]);
                self.detected_profile = match report_handle {
                    0x002B => GamepadProfileType::XboxWireless,
                    0x003A => GamepadProfileType::DualSense,
                    0x0014 => GamepadProfileType::SwitchPro,
                    _ => GamepadProfileType::GenericBleHid,
                };
            }
        }

        true
    }
}

/// Unit verification for BLE HCI packet parser and gamepad profile detection.
pub fn verify_ble_hci_parser() -> bool {
    let mut parser = BleHciPacketParser::new();

    // Construct mock HCI ACL packet with ATT notification from Xbox Wireless Controller handle 0x002B
    let mut mock_packet = [0u8; 16];
    mock_packet[0] = 0x42; // Handle 0x0042, PB=0, BC=0
    mock_packet[1] = 0x00;
    mock_packet[2] = 0x0C; // Payload length 12
    mock_packet[3] = 0x00;
    mock_packet[4] = 0x08; // L2CAP length 8
    mock_packet[5] = 0x00;
    mock_packet[6] = 0x04; // CID 0x0004 (ATT)
    mock_packet[7] = 0x00;
    mock_packet[8] = 0x1B; // ATT Handle Value Notification
    mock_packet[9] = 0x2B; // Handle 0x002B (Xbox)
    mock_packet[10] = 0x00;

    if !parser.parse_hci_acl_packet(&mock_packet) {
        return false;
    }
    if parser.connection_handle != 0x0042 || parser.l2cap_cid != L2CAP_CID_ATT {
        return false;
    }
    if parser.detected_profile != GamepadProfileType::XboxWireless {
        return false;
    }

    true
}
