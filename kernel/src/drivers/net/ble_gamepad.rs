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

/// Unpacked standard gamepad HID input state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GamepadInputState {
    pub buttons: u16,       // A, B, X, Y, LB, RB, LT, RT, View, Menu, LSB, RSB
    pub left_stick_x: i16,
    pub left_stick_y: i16,
    pub right_stick_x: i16,
    pub right_stick_y: i16,
    pub trigger_left: u16,
    pub trigger_right: u16,
}

/// Sub-4ms low-latency connection interval negotiator and HID report framer.
#[derive(Debug, Clone, Copy)]
pub struct LowLatencyInputFramer {
    pub connection_interval_slots: u16, // 1 slot = 1.25ms; 3 slots = 3.75ms (<4ms)
    pub supervision_timeout_ms: u16,
    pub slave_latency: u16,
    pub last_state: GamepadInputState,
    pub packets_processed: u64,
}

impl LowLatencyInputFramer {
    /// Creates a new input framer configured for sub-4ms BLE connection interval (3 slots = 3.75ms).
    pub const fn new() -> Self {
        Self {
            connection_interval_slots: 3, // 3.75ms
            supervision_timeout_ms: 1000,
            slave_latency: 0,
            last_state: GamepadInputState {
                buttons: 0,
                left_stick_x: 0,
                left_stick_y: 0,
                right_stick_x: 0,
                right_stick_y: 0,
                trigger_left: 0,
                trigger_right: 0,
            },
            packets_processed: 0,
        }
    }

    /// Verifies if connection parameters meet competitive gaming sub-4ms latency budget.
    pub fn is_sub_4ms(&self) -> bool {
        // Interval time in ms: slots * 1.25ms. For 3 slots: 3.75ms <= 4.0ms
        self.connection_interval_slots <= 3 && self.slave_latency == 0
    }

    /// Unpacks raw 16-byte HID gamepad report into normalized input state.
    pub fn unpack_hid_report(&mut self, report: &[u8]) -> Option<GamepadInputState> {
        if report.len() < 12 {
            return None;
        }

        self.packets_processed += 1;
        let buttons = u16::from_le_bytes([report[0], report[1]]);
        let lx = i16::from_le_bytes([report[2], report[3]]);
        let ly = i16::from_le_bytes([report[4], report[5]]);
        let rx = i16::from_le_bytes([report[6], report[7]]);
        let ry = i16::from_le_bytes([report[8], report[9]]);
        let lt = u16::from_le_bytes([report[10], (report[10] >> 4)]); // 10-bit trigger
        let rt = if report.len() >= 12 { report[11] as u16 * 4 } else { 0 };

        self.last_state = GamepadInputState {
            buttons,
            left_stick_x: lx,
            left_stick_y: ly,
            right_stick_x: rx,
            right_stick_y: ry,
            trigger_left: lt,
            trigger_right: rt,
        };

        Some(self.last_state)
    }
}

/// Unit verification for sub-4ms input report framer.
pub fn verify_low_latency_input_framer() -> bool {
    let mut framer = LowLatencyInputFramer::new();
    if !framer.is_sub_4ms() {
        return false;
    }

    let mut report = [0u8; 16];
    report[0] = 0x01; // Button A pressed
    report[1] = 0x00;
    report[2] = 0x00; // LX = 1000
    report[3] = 0x03; // (0x03E8 = 1000)
    report[4] = 0x00; // LY = 0
    report[5] = 0x00;

    let state = framer.unpack_hid_report(&report);
    if state.is_none() {
        return false;
    }
    let s = state.unwrap();
    if s.buttons != 0x0001 || s.left_stick_x != 1000 || framer.packets_processed != 1 {
        return false;
    }

    true
}
