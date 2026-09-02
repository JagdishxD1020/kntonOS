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
    report[2] = 0xE8; // LX = 1000 (0x03E8 in LE: [0xE8, 0x03])
    report[3] = 0x03;
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

/// LC3 audio frame duration mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lc3FrameDuration {
    Frame7_5ms,
    Frame10ms,
}

/// LE Audio Isochronous Stream (CIS/BIS) packet framer for LC3 gaming earbuds.
#[derive(Debug, Clone, Copy)]
pub struct LeAudioIsochronousStreamer {
    pub isochronous_handle: u16,
    pub frame_duration: Lc3FrameDuration,
    pub octets_per_codec_frame: u16, // 100 octets at 80kbps (48kHz/16-bit/10ms)
    pub sequence_number: u32,
    pub presentation_delay_us: u32,
    pub dropouts_prevented: u32,
}

impl LeAudioIsochronousStreamer {
    /// Creates a new LE Audio isochronous streamer configured for 10ms LC3 frames (48kHz, 80kbps).
    pub const fn new() -> Self {
        Self {
            isochronous_handle: 0x00E0,
            frame_duration: Lc3FrameDuration::Frame10ms,
            octets_per_codec_frame: 100, // 80 kbps LC3 frame
            sequence_number: 0,
            presentation_delay_us: 10_000, // 10ms presentation delay
            dropouts_prevented: 0,
        }
    }

    /// Encapsulates raw LC3 encoded audio payload into a BLE Isochronous Data Packet (HCI ISO Data Pkt).
    /// ISO Data Packet header:
    /// Bytes 0-1: Handle (12 bits) + PB flag (2 bits) + TS flag (2 bits)
    /// Bytes 2-3: ISO SDU Length (14 bits)
    /// Bytes 4-7: Time Stamp (32 bits, if TS flag set)
    /// Bytes 8-9: Packet Sequence Number (16 bits)
    /// Bytes 10-11: SDU Length (12 bits)
    pub fn frame_isochronous_packet(&mut self, lc3_payload: &[u8], out_buffer: &mut [u8]) -> usize {
        if out_buffer.len() < 12 + lc3_payload.len() {
            self.dropouts_prevented += 1;
            return 0;
        }

        self.sequence_number = self.sequence_number.wrapping_add(1);

        // Header: Handle 0x00E0 | PB=00 (start of SDU) | TS=1 (timestamp present)
        let handle_flags = (self.isochronous_handle & 0x0FFF) | (1 << 14); // TS = 1
        let hf_bytes = handle_flags.to_le_bytes();
        out_buffer[0] = hf_bytes[0];
        out_buffer[1] = hf_bytes[1];

        let total_iso_len = (8 + lc3_payload.len()) as u16; // 4 ts + 2 seq + 2 sdu_len + payload
        let iso_len_bytes = total_iso_len.to_le_bytes();
        out_buffer[2] = iso_len_bytes[0];
        out_buffer[3] = iso_len_bytes[1];

        // 32-bit presentation timestamp in us
        let ts = self.sequence_number * 10_000;
        let ts_bytes = ts.to_le_bytes();
        out_buffer[4..8].copy_from_slice(&ts_bytes);

        // 16-bit sequence number
        let seq_bytes = (self.sequence_number as u16).to_le_bytes();
        out_buffer[8] = seq_bytes[0];
        out_buffer[9] = seq_bytes[1];

        // 12-bit SDU length
        let sdu_bytes = (lc3_payload.len() as u16).to_le_bytes();
        out_buffer[10] = sdu_bytes[0];
        out_buffer[11] = sdu_bytes[1];

        out_buffer[12..12 + lc3_payload.len()].copy_from_slice(lc3_payload);
        12 + lc3_payload.len()
    }
}

/// Unit verification for LE Audio isochronous streamer.
pub fn verify_le_audio_streamer() -> bool {
    let mut streamer = LeAudioIsochronousStreamer::new();
    let mut out_pkt = [0u8; 128];
    let mock_lc3 = [0xAA; 100]; // 100 octets 10ms frame

    let framed_len = streamer.frame_isochronous_packet(&mock_lc3, &mut out_pkt);
    if framed_len != 112 {
        return false;
    }
    if streamer.sequence_number != 1 || streamer.dropouts_prevented != 0 {
        return false;
    }
    if out_pkt[12] != 0xAA || out_pkt[111] != 0xAA {
        return false;
    }

    true
}

/// Link quality grade based on Received Signal Strength Indication (RSSI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleLinkQuality {
    Excellent, // > -60 dBm
    Good,      // -60 to -75 dBm
    Poor,      // -75 to -88 dBm
    Critical,  // < -88 dBm
}

/// Dynamic connection quality monitor, adaptive retransmission, and health telemetry.
#[derive(Debug, Clone, Copy)]
pub struct BleLinkQualityMonitor {
    pub current_rssi_dbm: i8,
    pub packet_loss_rate_ppm: u32, // Parts per million
    pub retransmissions_requested: u32,
    pub link_quality: BleLinkQuality,
}

impl BleLinkQualityMonitor {
    /// Creates a new BLE link quality monitor.
    pub const fn new() -> Self {
        Self {
            current_rssi_dbm: -65,
            packet_loss_rate_ppm: 0,
            retransmissions_requested: 0,
            link_quality: BleLinkQuality::Good,
        }
    }

    /// Ingests a new RSSI measurement from HCI and updates quality grade.
    pub fn update_rssi(&mut self, rssi_dbm: i8) -> BleLinkQuality {
        self.current_rssi_dbm = rssi_dbm;
        self.link_quality = if rssi_dbm > -60 {
            BleLinkQuality::Excellent
        } else if rssi_dbm >= -75 {
            BleLinkQuality::Good
        } else if rssi_dbm >= -88 {
            BleLinkQuality::Poor
        } else {
            BleLinkQuality::Critical
        };
        self.link_quality
    }

    /// Evaluates whether an immediate packet retransmission should be requested based on ACK timeout / CRC error.
    pub fn record_packet_event(&mut self, success: bool) -> bool {
        if !success {
            self.retransmissions_requested += 1;
            self.packet_loss_rate_ppm = (self.packet_loss_rate_ppm + 10_000).min(1_000_000);
            true
        } else {
            self.packet_loss_rate_ppm = self.packet_loss_rate_ppm.saturating_sub(1_000);
            false
        }
    }
}

/// Unit verification for RSSI link quality monitor.
pub fn verify_ble_link_quality_monitor() -> bool {
    let mut mon = BleLinkQualityMonitor::new();

    if mon.update_rssi(-55) != BleLinkQuality::Excellent {
        return false;
    }
    if mon.update_rssi(-70) != BleLinkQuality::Good {
        return false;
    }
    if mon.update_rssi(-80) != BleLinkQuality::Poor {
        return false;
    }
    if mon.update_rssi(-92) != BleLinkQuality::Critical {
        return false;
    }

    // Packet loss recording
    if !mon.record_packet_event(false) || mon.retransmissions_requested != 1 {
        return false;
    }
    if mon.record_packet_event(true) {
        return false;
    }

    true
}

/// Comprehensive verification self-test suite for Handheld Bluetooth Low Energy (BLE) Gamepad & Low-Latency Audio Transceiver Engine.
pub fn test_ble_gamepad_subsystem() -> bool {
    crate::serial_write_str("[KNT-NET] Testing Handheld BLE Gamepad & LE Audio Transceiver...\r\n");

    if !verify_ble_hci_parser() {
        crate::serial_write_str("[KNT-PANIC] BLE HCI packet parser verification failed!\r\n");
        return false;
    }
    if !verify_low_latency_input_framer() {
        crate::serial_write_str("[KNT-PANIC] Low-latency input framer verification failed!\r\n");
        return false;
    }
    if !verify_le_audio_streamer() {
        crate::serial_write_str("[KNT-PANIC] LE Audio isochronous streamer verification failed!\r\n");
        return false;
    }
    if !verify_ble_link_quality_monitor() {
        crate::serial_write_str("[KNT-PANIC] BLE link quality monitor verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-BLE-GAMEPAD-PASS] Handheld BLE gamepad and LE Audio transceiver verified\r\n");
    true
}
