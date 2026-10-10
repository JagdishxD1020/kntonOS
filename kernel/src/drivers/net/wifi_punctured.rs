//! Handheld Wi-Fi 7 Punctured Channel Pacing & Ultra-Low-Latency Beacon Synchronization.
//!
//! Provides:
//! 1. Wi-Fi 7 preamble puncturing engine navigating dynamic DFS radar channels.
//! 2. Microsecond beacon sync governor for local handheld mesh multiplayer.
//! 3. Antenna diversity switch governor detecting hand grip RF attenuation.
//! 4. Low-latency UDP gaming socket packet classifier.
//! 5. Self-test suite verifying the subsystem with sentinel `[KNT-NET-WIFI-PUNCTURE-PASS]`.

#![allow(dead_code)]

// ============================================================================
// Feature 1: Wi-Fi 7 Preamble Puncturing Engine Navigating Dynamic DFS Radar Channels
// ============================================================================

/// Channel bandwidth in MHz for Wi-Fi 7 (802.11be).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wifi7Bandwidth {
    /// 80 MHz channel (comprising 4 x 20 MHz subchannels).
    Width80MHz = 80,
    /// 160 MHz channel (comprising 8 x 20 MHz subchannels).
    Width160MHz = 160,
    /// 320 MHz channel (comprising 16 x 20 MHz subchannels).
    Width320MHz = 320,
}

/// Dynamic Frequency Selection (DFS) radar event detection state for a 20 MHz subband.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DfsRadarState {
    /// Subchannel is clear of radar and co-channel interference.
    Clear,
    /// Radar chirp pulse detected; subchannel is locked out during In-Service Monitoring (ISM).
    RadarDetected,
    /// Non-Occupancy Period (NOP) timer active (mandatory 30-minute quiet period before re-clearing).
    NonOccupancyPeriod,
}

/// Represents the status of each 20 MHz subchannel within a wide 80/160/320 MHz channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubchannelStatus {
    /// Center frequency of this 20 MHz subchannel in MHz (e.g. 5260, 5280, 5500, etc.).
    pub center_freq_mhz: u16,
    /// DFS Radar occupancy state.
    pub radar_state: DfsRadarState,
    /// Noise floor in dBm (typically -98 dBm to -80 dBm).
    pub noise_floor_dbm: i8,
    /// Measured interference / co-channel utilization percentage (0.0 .. 100.0).
    pub interference_pct: f32,
    /// Remaining Non-Occupancy Period time in seconds (0 if Clear).
    pub nop_remaining_s: u32,
    /// Whether this subchannel is marked as punctured (notched out) in the 802.11be PHY preamble.
    pub is_punctured: bool,
}

impl SubchannelStatus {
    /// Creates a clear 20 MHz subchannel at the specified frequency.
    pub const fn new(center_freq_mhz: u16) -> Self {
        Self {
            center_freq_mhz,
            radar_state: DfsRadarState::Clear,
            noise_floor_dbm: -95,
            interference_pct: 0.0,
            nop_remaining_s: 0,
            is_punctured: false,
        }
    }
}

/// Wi-Fi 7 (802.11be) Preamble Puncturing Engine.
///
/// In 802.11be, preamble puncturing allows transmitters to notch out specific 20 MHz
/// subchannels where radar chirps or heavy narrowband interference are detected,
/// without having to fall back to a narrow 20 or 40 MHz channel.
/// Primary 20 MHz channel cannot be punctured. Secondary channels can be punctured
/// according to 802.11be valid puncturing patterns (16-bit puncturing bitmap).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wifi7PunctureEngine {
    /// Operating bandwidth mode (80 MHz, 160 MHz, or 320 MHz).
    pub bandwidth: Wifi7Bandwidth,
    /// Primary 20 MHz subchannel index (0..15). Cannot be punctured.
    pub primary_subchannel_idx: u8,
    /// Number of 20 MHz subchannels (4 for 80M, 8 for 160M, 16 for 320M).
    pub num_subchannels: u8,
    /// Subchannel status array (max 16 subchannels for 320 MHz).
    pub subchannels: [SubchannelStatus; 16],
    /// 16-bit Preamble Puncture Bitmap: bit n is 1 if subchannel n is active, 0 if punctured.
    pub puncture_bitmap: u16,
    /// Total operational active bandwidth in MHz remaining after puncturing.
    pub active_bandwidth_mhz: u16,
    /// Cumulative radar detection events logged.
    pub radar_detections_count: u32,
}

impl Wifi7PunctureEngine {
    /// Constructs a Wi-Fi 7 preamble puncturing engine for the given channel bandwidth.
    pub fn new(bandwidth: Wifi7Bandwidth, primary_freq_mhz: u16) -> Self {
        let num_sub = match bandwidth {
            Wifi7Bandwidth::Width80MHz => 4,
            Wifi7Bandwidth::Width160MHz => 8,
            Wifi7Bandwidth::Width320MHz => 16,
        };

        let mut subchannels = [SubchannelStatus::new(0); 16];
        for i in 0..num_sub {
            subchannels[i as usize] = SubchannelStatus::new(primary_freq_mhz + (i as u16 * 20));
        }

        let mut engine = Self {
            bandwidth,
            primary_subchannel_idx: 0,
            num_subchannels: num_sub,
            subchannels,
            puncture_bitmap: 0,
            active_bandwidth_mhz: 0,
            radar_detections_count: 0,
        };

        engine.recalculate_puncture_mask();
        engine
    }

    /// Triggers a dynamic DFS radar chirp detection event on a specific subchannel frequency.
    pub fn report_radar_detection(&mut self, subchannel_idx: usize) -> bool {
        if subchannel_idx >= self.num_subchannels as usize {
            return false;
        }

        // Primary subchannel cannot be punctured; if radar hits primary, full channel switch (CSA) is required.
        if subchannel_idx == self.primary_subchannel_idx as usize {
            self.radar_detections_count += 1;
            self.subchannels[subchannel_idx].radar_state = DfsRadarState::RadarDetected;
            self.subchannels[subchannel_idx].nop_remaining_s = 1800; // 30 min NOP
            return false; // Indicates primary hit (full channel switch needed)
        }

        // Secondary subchannel: apply preamble puncturing!
        self.radar_detections_count += 1;
        self.subchannels[subchannel_idx].radar_state = DfsRadarState::RadarDetected;
        self.subchannels[subchannel_idx].nop_remaining_s = 1800; // 30 min NOP
        self.subchannels[subchannel_idx].is_punctured = true;

        self.recalculate_puncture_mask();
        true
    }

    /// Clear or tick NOP timers for all subchannels.
    pub fn tick_nop_timers(&mut self, elapsed_s: u32) {
        for i in 0..(self.num_subchannels as usize) {
            if self.subchannels[i].nop_remaining_s > 0 {
                if self.subchannels[i].nop_remaining_s <= elapsed_s {
                    self.subchannels[i].nop_remaining_s = 0;
                    self.subchannels[i].radar_state = DfsRadarState::Clear;
                    self.subchannels[i].is_punctured = false;
                } else {
                    self.subchannels[i].nop_remaining_s -= elapsed_s;
                    self.subchannels[i].radar_state = DfsRadarState::NonOccupancyPeriod;
                }
            }
        }
        self.recalculate_puncture_mask();
    }

    /// Recalculates the 802.11be puncturing bitmap and active usable bandwidth.
    pub fn recalculate_puncture_mask(&mut self) {
        let mut bitmap: u16 = 0;
        let mut active_count = 0u16;

        for i in 0..(self.num_subchannels as usize) {
            if !self.subchannels[i].is_punctured && self.subchannels[i].radar_state == DfsRadarState::Clear {
                bitmap |= 1 << i;
                active_count += 1;
            }
        }

        self.puncture_bitmap = bitmap;
        self.active_bandwidth_mhz = active_count * 20;
    }

    /// Evaluates whether an outgoing frame transmission can proceed on the active punctured channel.
    pub fn can_transmit_frame(&self) -> bool {
        // Can transmit if primary channel is clear and at least 40 MHz active bandwidth remains
        let primary_clear = !self.subchannels[self.primary_subchannel_idx as usize].is_punctured
            && self.subchannels[self.primary_subchannel_idx as usize].radar_state == DfsRadarState::Clear;
        primary_clear && (self.active_bandwidth_mhz >= 40)
    }
}

/// Verification test for Wi-Fi 7 preamble puncturing engine.
pub fn verify_wifi7_preamble_puncturing() -> bool {
    // Test a 160 MHz channel (8 x 20 MHz subchannels) starting at 5260 MHz (DFS band UNII-2A)
    let mut engine = Wifi7PunctureEngine::new(Wifi7Bandwidth::Width160MHz, 5260);

    // Initial check: all 8 subchannels clear (active bandwidth = 160 MHz, bitmap = 0x00FF)
    if engine.active_bandwidth_mhz != 160 || engine.puncture_bitmap != 0x00FF {
        return false;
    }
    if !engine.can_transmit_frame() {
        return false;
    }

    // Secondary subchannel 2 detects weather radar chirp (5300 MHz)
    let punctured = engine.report_radar_detection(2);
    if !punctured {
        return false;
    }

    // Verify subchannel 2 is punctured (bitmap: bit 2 cleared -> 0x00FB, active BW: 140 MHz)
    if engine.puncture_bitmap != 0x00FB || engine.active_bandwidth_mhz != 140 {
        return false;
    }
    if !engine.subchannels[2].is_punctured || engine.subchannels[2].radar_state != DfsRadarState::RadarDetected {
        return false;
    }
    if !engine.can_transmit_frame() {
        return false;
    }

    // Subchannel 5 also detects radar (bitmap: bits 2 & 5 cleared -> 0x00DB, active BW: 120 MHz)
    let punctured2 = engine.report_radar_detection(5);
    if !punctured2 {
        return false;
    }
    if engine.puncture_bitmap != 0x00DB || engine.active_bandwidth_mhz != 120 {
        return false;
    }

    // Test primary channel (idx 0) radar hit -> must return false (requires Channel Switch Announcement CSA)
    let primary_hit = engine.report_radar_detection(0);
    if primary_hit {
        return false; // Should not allow puncture on primary
    }
    if engine.can_transmit_frame() {
        return false; // Primary is down, cannot transmit
    }

    true
}
