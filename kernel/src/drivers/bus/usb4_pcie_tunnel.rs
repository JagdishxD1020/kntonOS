//! Handheld USB4 / Thunderbolt 4 PCIe Tunneling & Dynamic DMA Hot-Plug Arbiter.
//!
//! Provides bare-metal transport layer encapsulation and high-speed PCIe tunneling
//! for handheld gaming console external GPU (eGPU) docking stations and NVMe docks:
//! 1. USB4 Router transport layer packet framing with PCIe Protocol Adapter Layer (PAL).
//! 2. Dynamic PCIe DMA hot-plug arbiter handling eGPU docking and seamless link re-training.
//! 3. Bandwidth credit-based QoS governor arbitrating DisplayPort tunneling and PCIe traffic.
//! 4. Comprehensive self-test suite and kmain integration.

#![allow(dead_code)]

// ============================================================================
// Feature 1: USB4 Transport Packet Framing & PCIe Protocol Adapter Layer (PAL)
// ============================================================================

pub const USB4_MAX_PAYLOAD_BYTES: usize = 256;
pub const USB4_PCIE_TUNNEL_HOP_ID: u16 = 8;
pub const USB4_DP_TUNNEL_HOP_ID: u16 = 9;

/// PCIe Transaction Layer Packet (TLP) type code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PcieTlpType {
    MemoryRead32 = 0x00,
    MemoryRead64 = 0x20,
    MemoryWrite32 = 0x40,
    MemoryWrite64 = 0x60,
    CompletionWithData = 0x4A,
    CompletionWithoutData = 0x0A,
    Message = 0x30,
}

/// PCIe TLP header representation for tunneling across USB4 transport fabric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcieTlpHeader {
    pub tlp_type: PcieTlpType,
    pub length_dw: u16,
    pub requester_id: u16,
    pub tag: u8,
    pub address: u64,
}

impl PcieTlpHeader {
    pub const fn new(tlp_type: PcieTlpType, length_dw: u16, requester_id: u16, tag: u8, address: u64) -> Self {
        Self {
            tlp_type,
            length_dw,
            requester_id,
            tag,
            address,
        }
    }
}

/// USB4 Transport Layer Packet (TLP) carrying tunneled protocol payloads.
#[derive(Debug, Clone, Copy)]
pub struct Usb4TransportPacket {
    pub target_adapter_id: u8,
    pub hop_id: u16,
    pub sequence_number: u8,
    pub pdf: u8,
    pub payload_length: u16,
    pub header_crc: u8,
    pub payload: [u8; USB4_MAX_PAYLOAD_BYTES],
}

impl Usb4TransportPacket {
    pub const fn empty() -> Self {
        Self {
            target_adapter_id: 0,
            hop_id: 0,
            sequence_number: 0,
            pdf: 0,
            payload_length: 0,
            header_crc: 0,
            payload: [0u8; USB4_MAX_PAYLOAD_BYTES],
        }
    }

    /// Calculate CRC-8 over transport header fields (polynomial x^8 + x^2 + x + 1).
    pub fn compute_header_crc(&self) -> u8 {
        let mut crc: u8 = 0;
        let bytes: [u8; 5] = [
            self.target_adapter_id,
            (self.hop_id & 0xFF) as u8,
            ((self.hop_id >> 8) & 0xFF) as u8,
            self.sequence_number,
            (self.payload_length & 0xFF) as u8,
        ];

        for &b in bytes.iter() {
            crc ^= b;
            for _ in 0..8 {
                if (crc & 0x80) != 0 {
                    crc = (crc << 1) ^ 0x07;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }

    /// Verify header CRC-8 integrity.
    pub fn is_header_valid(&self) -> bool {
        self.compute_header_crc() == self.header_crc
    }
}

/// PCIe Protocol Adapter Layer (PAL) for encapsulating / decapsulating PCIe TLPs.
#[derive(Debug, Clone)]
pub struct PcieProtocolAdapterLayer {
    pub total_tlps_encapsulated: u64,
    pub total_tlps_decapsulated: u64,
    pub crc_error_count: u64,
}

impl PcieProtocolAdapterLayer {
    pub const fn new() -> Self {
        Self {
            total_tlps_encapsulated: 0,
            total_tlps_decapsulated: 0,
            crc_error_count: 0,
        }
    }

    /// Encapsulate a PCIe TLP and its payload into a USB4 Transport Packet.
    pub fn encapsulate_pcie_tlp(
        &mut self,
        header: &PcieTlpHeader,
        payload_data: &[u8],
        hop_id: u16,
        target_adapter_id: u8,
        sequence_number: u8,
    ) -> Usb4TransportPacket {
        let mut pkt = Usb4TransportPacket::empty();
        pkt.target_adapter_id = target_adapter_id;
        pkt.hop_id = hop_id;
        pkt.sequence_number = sequence_number;
        pkt.pdf = 0x01; // End of TLP marker

        // Pack PCIe Header into first 16 bytes of payload
        // Byte 0: TLP Type
        pkt.payload[0] = header.tlp_type as u8;
        // Bytes 1..3: Length DW
        pkt.payload[1] = (header.length_dw & 0xFF) as u8;
        pkt.payload[2] = ((header.length_dw >> 8) & 0xFF) as u8;
        // Bytes 3..5: Requester ID
        pkt.payload[3] = (header.requester_id & 0xFF) as u8;
        pkt.payload[4] = ((header.requester_id >> 8) & 0xFF) as u8;
        // Byte 5: Tag
        pkt.payload[5] = header.tag;
        // Bytes 6..14: 64-bit Address
        let addr_bytes = header.address.to_le_bytes();
        let mut i = 0;
        while i < 8 {
            pkt.payload[6 + i] = addr_bytes[i];
            i += 1;
        }

        // Copy actual TLP payload starting at offset 16
        let copy_len = payload_data.len().min(USB4_MAX_PAYLOAD_BYTES - 16);
        let mut j = 0;
        while j < copy_len {
            pkt.payload[16 + j] = payload_data[j];
            j += 1;
        }

        pkt.payload_length = (16 + copy_len) as u16;
        pkt.header_crc = pkt.compute_header_crc();

        self.total_tlps_encapsulated += 1;
        pkt
    }

    /// Decapsulate a USB4 Transport Packet back into a PCIe TLP and payload.
    pub fn decapsulate_pcie_tlp(
        &mut self,
        pkt: &Usb4TransportPacket,
    ) -> Option<(PcieTlpHeader, [u8; USB4_MAX_PAYLOAD_BYTES - 16], usize)> {
        if !pkt.is_header_valid() || pkt.payload_length < 16 {
            self.crc_error_count += 1;
            return None;
        }

        let raw_type = pkt.payload[0];
        let tlp_type = match raw_type {
            0x00 => PcieTlpType::MemoryRead32,
            0x20 => PcieTlpType::MemoryRead64,
            0x40 => PcieTlpType::MemoryWrite32,
            0x60 => PcieTlpType::MemoryWrite64,
            0x4A => PcieTlpType::CompletionWithData,
            0x0A => PcieTlpType::CompletionWithoutData,
            0x30 => PcieTlpType::Message,
            _ => return None,
        };

        let length_dw = (pkt.payload[1] as u16) | ((pkt.payload[2] as u16) << 8);
        let requester_id = (pkt.payload[3] as u16) | ((pkt.payload[4] as u16) << 8);
        let tag = pkt.payload[5];

        let mut addr_bytes = [0u8; 8];
        let mut i = 0;
        while i < 8 {
            addr_bytes[i] = pkt.payload[6 + i];
            i += 1;
        }
        let address = u64::from_le_bytes(addr_bytes);

        let header = PcieTlpHeader::new(tlp_type, length_dw, requester_id, tag, address);

        let payload_len = (pkt.payload_length as usize) - 16;
        let mut data = [0u8; USB4_MAX_PAYLOAD_BYTES - 16];
        let mut j = 0;
        while j < payload_len {
            data[j] = pkt.payload[16 + j];
            j += 1;
        }

        self.total_tlps_decapsulated += 1;
        Some((header, data, payload_len))
    }
}

/// Verification test for USB4 transport packet framing and PCIe protocol adapter layer.
pub fn verify_usb4_router_packet_framing_and_pal() -> bool {
    let mut pal = PcieProtocolAdapterLayer::new();

    // 1. Create a 64-bit Memory Write TLP targeting eGPU VRAM buffer
    let tlp_hdr = PcieTlpHeader::new(
        PcieTlpType::MemoryWrite64,
        32,                 // 32 DW = 128 bytes
        0x0100,             // Bus 1, Dev 0, Func 0
        7,                  // Tag 7
        0x0000_000E_8000_0000, // eGPU BAR1 VRAM base
    );

    let mut test_payload = [0u8; 128];
    for i in 0..128 {
        test_payload[i] = (i ^ 0x5A) as u8;
    }

    // 2. Encapsulate into USB4 Transport Layer packet
    let packet = pal.encapsulate_pcie_tlp(
        &tlp_hdr,
        &test_payload,
        USB4_PCIE_TUNNEL_HOP_ID,
        1,  // PCIe Downstream Adapter ID
        3,  // Sequence number 3
    );

    // 3. Verify packet framing & CRC
    if !packet.is_header_valid() || packet.hop_id != USB4_PCIE_TUNNEL_HOP_ID || packet.payload_length != 144 {
        return false;
    }
    if pal.total_tlps_encapsulated != 1 {
        return false;
    }

    // 4. Decapsulate back and verify exact bit-for-bit reconstruction
    let (dec_hdr, dec_data, dec_len) = match pal.decapsulate_pcie_tlp(&packet) {
        Some(res) => res,
        None => return false,
    };

    if dec_hdr != tlp_hdr || dec_len != 128 {
        return false;
    }
    if dec_data[0..128] != test_payload[0..128] {
        return false;
    }

    // 5. Verify CRC corruption detection
    let mut corrupted = packet;
    corrupted.target_adapter_id ^= 0xFF; // Corrupt adapter ID without updating CRC
    if corrupted.is_header_valid() {
        return false;
    }
    if pal.decapsulate_pcie_tlp(&corrupted).is_some() {
        return false;
    }
    if pal.crc_error_count != 1 {
        return false;
    }

    true
}
