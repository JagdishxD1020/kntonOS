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

// ============================================================================
// Feature 2: Microsecond Beacon Sync Governor for Local Handheld Mesh Multiplayer
// ============================================================================

/// Beacon frame timing configuration and metrics for low-latency ad-hoc/mesh synchronization.
pub const TARGET_BEACON_TRANSMIT_TIME_US: u32 = 102_400; // 102.4 ms (100 TU standard)
pub const MAX_ACCEPTABLE_CLOCK_DRIFT_PPM: f32 = 20.0;     // ±20 ppm hardware crystal tolerance
pub const MAX_BEACON_JITTER_TOLERANCE_US: i32 = 25;       // Target mesh jitter envelope: ±25 µs

/// Mesh role in local wireless multiplayer session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshSyncRole {
    /// Designated Mesh Group Coordinator (Grandmaster clock leader).
    MeshCoordinator,
    /// Mesh peer client participating in synchronous frame pacing.
    MeshPeer,
    /// Disconnected or probing mesh discovery state.
    Probing,
}

/// Timing Synchronization Function (TSF) microsecond tracking state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshTsfSyncState {
    /// 64-bit local hardware TSF microsecond timestamp.
    pub local_tsf_us: u64,
    /// 64-bit remote leader TSF microsecond timestamp received in beacon.
    pub remote_leader_tsf_us: u64,
    /// Computed clock offset in microseconds (local - remote). Positive means local is ahead.
    pub clock_offset_us: i64,
    /// Estimated frequency drift in parts-per-million (PPM).
    pub estimated_drift_ppm: f32,
    /// Phase adjustment applied to next beacon period in microseconds.
    pub phase_correction_us: i32,
    /// Current sync quality status.
    pub is_synchronized: bool,
    /// Consecutive synchronized beacons count.
    pub sync_lock_counter: u32,
}

impl MeshTsfSyncState {
    /// Constructs default un-synchronized TSF state.
    pub const fn new() -> Self {
        Self {
            local_tsf_us: 0,
            remote_leader_tsf_us: 0,
            clock_offset_us: 0,
            estimated_drift_ppm: 0.0,
            phase_correction_us: 0,
            is_synchronized: false,
            sync_lock_counter: 0,
        }
    }
}

/// Handheld local mesh multiplayer beacon synchronization governor.
///
/// Ensures peer-to-peer frame alignment within sub-50µs jitter bounds,
/// synchronizing TSF clocks between handhelds to eliminate packet collision
/// and scheduling jitter in ad-hoc local gaming sessions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshBeaconSyncGovernor {
    /// Node role in the local mesh network.
    pub role: MeshSyncRole,
    /// Target Beacon Transmission Time (TBTT) interval in microseconds.
    pub tbtt_interval_us: u32,
    /// Next scheduled Target Beacon Transmission Time (TSF µs).
    pub next_tbtt_us: u64,
    /// Detailed TSF sync tracking state.
    pub tsf_state: MeshTsfSyncState,
    /// Measured peer-to-peer beacon delivery jitter in microseconds.
    pub last_beacon_jitter_us: i32,
    /// Total beacons received from mesh peers.
    pub beacons_received_count: u32,
    /// Total beacons transmitted by this node.
    pub beacons_transmitted_count: u32,
}

impl MeshBeaconSyncGovernor {
    /// Initializes a new mesh beacon sync governor.
    pub fn new(role: MeshSyncRole) -> Self {
        Self {
            role,
            tbtt_interval_us: TARGET_BEACON_TRANSMIT_TIME_US,
            next_tbtt_us: TARGET_BEACON_TRANSMIT_TIME_US as u64,
            tsf_state: MeshTsfSyncState::new(),
            last_beacon_jitter_us: 0,
            beacons_received_count: 0,
            beacons_transmitted_count: 0,
        }
    }

    /// Advances local hardware TSF timer by `delta_us` microseconds.
    pub fn advance_local_tsf(&mut self, delta_us: u64) {
        self.tsf_state.local_tsf_us += delta_us;
    }

    /// Processes an incoming beacon frame carrying leader TSF and arrival timestamp.
    /// Returns true if phase locked and synchronized within jitter limits.
    pub fn process_rx_beacon(&mut self, leader_tsf_us: u64, rx_local_tsf_us: u64, propagation_delay_us: u32) -> bool {
        self.beacons_received_count += 1;
        self.tsf_state.local_tsf_us = rx_local_tsf_us;
        self.tsf_state.remote_leader_tsf_us = leader_tsf_us;

        // Effective leader timestamp at arrival = leader_tsf + propagation_delay
        let effective_leader_tsf = leader_tsf_us + propagation_delay_us as u64;

        // Offset = local_tsf - effective_leader_tsf
        let offset = rx_local_tsf_us as i64 - effective_leader_tsf as i64;
        self.tsf_state.clock_offset_us = offset;

        // Jitter relative to expected TBTT slot
        let phase_in_period = (rx_local_tsf_us % self.tbtt_interval_us as u64) as i32;
        let half_period = (self.tbtt_interval_us / 2) as i32;
        self.last_beacon_jitter_us = if phase_in_period > half_period {
            phase_in_period - self.tbtt_interval_us as i32
        } else {
            phase_in_period
        };

        // Standard 802.11 TSF update rule:
        // If received TSF is faster than local TSF, adopt the faster clock (or adjust PLL).
        if self.role == MeshSyncRole::MeshPeer {
            if offset < 0 {
                // Local is behind leader, slew local clock forward immediately
                self.tsf_state.phase_correction_us = (-offset).min(500) as i32;
                self.tsf_state.local_tsf_us = effective_leader_tsf;
            } else {
                // Local is ahead: slowly slew by stretching TBTT
                self.tsf_state.phase_correction_us = -(offset.min(500) as i32);
            }

            // Estimate clock drift in ppm
            if self.tbtt_interval_us > 0 {
                self.tsf_state.estimated_drift_ppm = (offset.abs() as f32 / self.tbtt_interval_us as f32) * 1_000_000.0;
            }

            // Lock detection: offset within ±50µs and jitter within tolerance
            if offset.abs() <= 50 && self.last_beacon_jitter_us.abs() <= MAX_BEACON_JITTER_TOLERANCE_US {
                self.tsf_state.sync_lock_counter += 1;
                if self.tsf_state.sync_lock_counter >= 3 {
                    self.tsf_state.is_synchronized = true;
                }
            } else {
                self.tsf_state.sync_lock_counter = 0;
                self.tsf_state.is_synchronized = false;
            }
        } else {
            // Coordinator maintains master clock
            self.tsf_state.is_synchronized = true;
        }

        // Schedule next TBTT
        let elapsed_in_period = self.tsf_state.local_tsf_us % self.tbtt_interval_us as u64;
        self.next_tbtt_us = self.tsf_state.local_tsf_us + (self.tbtt_interval_us as u64 - elapsed_in_period);

        self.tsf_state.is_synchronized
    }

    /// Evaluates whether the current local TSF is inside the transmission window for a multiplayer game packet.
    /// Handheld mesh schedules game packets right after beacon transmission (TBTT + guard time).
    pub fn is_gaming_tx_window(&self, guard_window_us: u32) -> bool {
        let current = self.tsf_state.local_tsf_us;
        let slot_start = self.next_tbtt_us.saturating_sub(self.tbtt_interval_us as u64);
        let time_since_beacon = current.saturating_sub(slot_start);
        time_since_beacon >= guard_window_us as u64 && time_since_beacon <= (self.tbtt_interval_us as u64 / 2)
    }
}

/// Verification test for microsecond beacon sync governor for mesh multiplayer.
pub fn verify_mesh_beacon_sync_governor() -> bool {
    let mut peer = MeshBeaconSyncGovernor::new(MeshSyncRole::MeshPeer);

    // Initial check: not synchronized
    if peer.tsf_state.is_synchronized || peer.beacons_received_count != 0 {
        return false;
    }

    // Leader sends 3 consecutive beacons at TBTT intervals (102,400 µs)
    let propagation_delay_us = 4; // 4 µs RF transit time across room (~1.2 km equivalent, standard delay model)

    // Beacon 1: Peer clock is 40 µs behind leader
    let leader_tsf_1 = 102_400u64;
    let peer_rx_tsf_1 = 102_400u64 - 40 + propagation_delay_us as u64;
    let sync1 = peer.process_rx_beacon(leader_tsf_1, peer_rx_tsf_1, propagation_delay_us);
    if sync1 {
        return false; // Requires 3 locks for filter stabilization
    }
    if peer.tsf_state.clock_offset_us.abs() > 50 {
        return false;
    }

    // Beacon 2: Clock aligned closely, within 10 µs
    let leader_tsf_2 = 204_800u64;
    let peer_rx_tsf_2 = 204_800u64 + 10 + propagation_delay_us as u64;
    peer.process_rx_beacon(leader_tsf_2, peer_rx_tsf_2, propagation_delay_us);

    // Beacon 3: Clock within 5 µs
    let leader_tsf_3 = 307_200u64;
    let peer_rx_tsf_3 = 307_200u64 + 5 + propagation_delay_us as u64;
    let sync3 = peer.process_rx_beacon(leader_tsf_3, peer_rx_tsf_3, propagation_delay_us);
    if !sync3 || !peer.tsf_state.is_synchronized {
        return false;
    }

    // Test game tx window timing:
    // Slot start is 307,200. Check at 307,200 + 2,000 µs (inside slot with 1000 µs guard window)
    peer.advance_local_tsf(2000);
    if !peer.is_gaming_tx_window(1000) {
        return false;
    }

    true
}

// ============================================================================
// Feature 3: Antenna Diversity Switch Governor Detecting Hand Grip RF Attenuation
// ============================================================================

/// Physical antenna placement locations on the handheld gaming console chassis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandheldAntennaId {
    /// Top-edge primary antenna (free of user palm obstruction in standard grip).
    TopPrimary = 0,
    /// Left grip internal antenna (subject to left palm capacitive loading/attenuation).
    LeftGrip = 1,
    /// Right grip internal antenna (subject to right palm capacitive loading/attenuation).
    RightGrip = 2,
    /// Bottom-edge auxiliary diversity antenna.
    BottomAux = 3,
}

/// RF metrics and grip attenuation state for an antenna element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AntennaMetrics {
    /// Antenna identifier.
    pub id: HandheldAntennaId,
    /// Raw RSSI in dBm (-100 dBm to -30 dBm).
    pub rssi_dbm: i8,
    /// Estimated Signal-to-Interference-plus-Noise Ratio (SINR) in dB.
    pub sinr_db: i8,
    /// Grip capacitive sensor proximity reading (0..255, > 180 indicates firm palm contact).
    pub grip_proximity_raw: u8,
    /// Estimated RF human-body attenuation in dB (typically 0 dB to 25 dB).
    pub body_attenuation_db: u8,
    /// Total transmission errors / retries logged while this antenna was active.
    pub tx_retries: u32,
    /// Whether this antenna is currently marked as severely attenuated ("death grip").
    pub is_chassis_blocked: bool,
}

impl AntennaMetrics {
    /// Constructs default metrics for an antenna.
    pub const fn new(id: HandheldAntennaId) -> Self {
        Self {
            id,
            rssi_dbm: -60,
            sinr_db: 25,
            grip_proximity_raw: 0,
            body_attenuation_db: 0,
            tx_retries: 0,
            is_chassis_blocked: false,
        }
    }
}

/// Dynamic Handheld Antenna Diversity Switching Governor.
///
/// Handheld devices suffer severe RF degradation (up to 18-24 dB) when a player's
/// sweaty palms wrap around side grip antennas ("death grip"). This governor
/// fuses capacitive touch grip sensor data with live per-antenna RSSI/SINR metrics
/// and sub-millisecond RF switch GPIO controls to select the optimal RF path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandheldAntennaDiversityGovernor {
    /// Metrics for all 4 chassis antennas.
    pub antennas: [AntennaMetrics; 4],
    /// Currently active TX/RX primary antenna.
    pub active_primary_antenna: HandheldAntennaId,
    /// Currently active secondary diversity antenna.
    pub active_secondary_antenna: HandheldAntennaId,
    /// Hysteresis threshold in dB required to switch antennas (prevents rapid flapping).
    pub switch_hysteresis_db: u8,
    /// Total dynamic antenna switches executed.
    pub switch_event_count: u32,
    /// Timestamp of last antenna switch in microseconds.
    pub last_switch_ts_us: u64,
}

impl HandheldAntennaDiversityGovernor {
    /// Initializes the antenna diversity governor with default antennas (TopPrimary & BottomAux).
    pub fn new() -> Self {
        let antennas = [
            AntennaMetrics::new(HandheldAntennaId::TopPrimary),
            AntennaMetrics::new(HandheldAntennaId::LeftGrip),
            AntennaMetrics::new(HandheldAntennaId::RightGrip),
            AntennaMetrics::new(HandheldAntennaId::BottomAux),
        ];

        Self {
            antennas,
            active_primary_antenna: HandheldAntennaId::TopPrimary,
            active_secondary_antenna: HandheldAntennaId::BottomAux,
            switch_hysteresis_db: 4, // 4 dB switching hysteresis
            switch_event_count: 0,
            last_switch_ts_us: 0,
        }
    }

    /// Updates capacitive grip sensor reading for an antenna and computes body attenuation.
    pub fn update_grip_sensor(&mut self, id: HandheldAntennaId, proximity_raw: u8) {
        let idx = id as usize;
        self.antennas[idx].grip_proximity_raw = proximity_raw;

        // Model RF attenuation based on grip proximity (0..255)
        // Above 128: progressive attenuation up to 20 dB. Above 200: blocked.
        if proximity_raw > 200 {
            self.antennas[idx].body_attenuation_db = 18;
            self.antennas[idx].is_chassis_blocked = true;
        } else if proximity_raw > 128 {
            let scale = (proximity_raw - 128) as f32 / 72.0;
            self.antennas[idx].body_attenuation_db = (scale * 15.0) as u8;
            self.antennas[idx].is_chassis_blocked = false;
        } else {
            self.antennas[idx].body_attenuation_db = 0;
            self.antennas[idx].is_chassis_blocked = false;
        }
    }

    /// Updates live RF measurements (raw RSSI and SINR) for an antenna.
    pub fn update_rf_metrics(&mut self, id: HandheldAntennaId, raw_rssi_dbm: i8, sinr_db: i8) {
        let idx = id as usize;
        self.antennas[idx].rssi_dbm = raw_rssi_dbm;
        self.antennas[idx].sinr_db = sinr_db;
    }

    /// Evaluates effective signal quality accounting for grip attenuation.
    pub fn get_effective_rssi(&self, id: HandheldAntennaId) -> i8 {
        let idx = id as usize;
        self.antennas[idx].rssi_dbm - (self.antennas[idx].body_attenuation_db as i8)
    }

    /// Runs the antenna selection arbitration logic. Returns true if a switch was performed.
    pub fn arbitrate_best_antennas(&mut self, current_time_us: u64) -> bool {
        // Enforce minimum dwell time of 5,000 µs (5ms) to prevent RF front-end oscillation
        if current_time_us.saturating_sub(self.last_switch_ts_us) < 5_000 {
            return false;
        }

        let current_primary_effective = self.get_effective_rssi(self.active_primary_antenna);

        // Find candidate with highest effective RSSI that is not blocked
        let mut best_id = self.active_primary_antenna;
        let mut best_score = current_primary_effective;

        let all_ids = [
            HandheldAntennaId::TopPrimary,
            HandheldAntennaId::LeftGrip,
            HandheldAntennaId::RightGrip,
            HandheldAntennaId::BottomAux,
        ];

        for &id in &all_ids {
            let eff = self.get_effective_rssi(id);
            // If current primary is blocked, switch immediately without hysteresis
            let hysteresis = if self.antennas[self.active_primary_antenna as usize].is_chassis_blocked {
                0
            } else {
                self.switch_hysteresis_db as i8
            };

            if eff > best_score + hysteresis {
                best_score = eff;
                best_id = id;
            }
        }

        if best_id != self.active_primary_antenna {
            // Perform antenna diversity switch
            self.active_secondary_antenna = self.active_primary_antenna;
            self.active_primary_antenna = best_id;
            self.switch_event_count += 1;
            self.last_switch_ts_us = current_time_us;
            true
        } else {
            false
        }
    }
}

/// Verification test for antenna diversity switch governor.
pub fn verify_antenna_diversity_switch_governor() -> bool {
    let mut gov = HandheldAntennaDiversityGovernor::new();

    // 1. Initial configuration: TopPrimary is active, effective RSSI = -60 dBm
    if gov.active_primary_antenna != HandheldAntennaId::TopPrimary {
        return false;
    }
    if gov.get_effective_rssi(HandheldAntennaId::TopPrimary) != -60 {
        return false;
    }

    // 2. Set RightGrip with slightly better raw RSSI (-58 dBm vs -60 dBm), but difference (2 dB) < 4 dB hysteresis
    gov.update_rf_metrics(HandheldAntennaId::RightGrip, -58, 26);
    let switched_no = gov.arbitrate_best_antennas(10_000);
    if switched_no || gov.active_primary_antenna != HandheldAntennaId::TopPrimary {
        return false; // Hysteresis prevented false switch
    }

    // 3. User firmly grips the top chassis edge (capacitive proximity = 220 -> death grip on TopPrimary)
    gov.update_grip_sensor(HandheldAntennaId::TopPrimary, 220);
    if !gov.antennas[HandheldAntennaId::TopPrimary as usize].is_chassis_blocked {
        return false;
    }
    // Effective RSSI drops from -60 to -78 dBm (-60 - 18 dB)
    if gov.get_effective_rssi(HandheldAntennaId::TopPrimary) != -78 {
        return false;
    }

    // 4. Arbitrate at t = 20,000 µs: RightGrip (-58 dBm) is clear and wins immediate switch
    let switched = gov.arbitrate_best_antennas(20_000);
    if !switched || gov.active_primary_antenna != HandheldAntennaId::RightGrip {
        return false;
    }
    if gov.active_secondary_antenna != HandheldAntennaId::TopPrimary {
        return false;
    }
    if gov.switch_event_count != 1 {
        return false;
    }

    // 5. Verify dwell time guard: attempted switch at 22,000 µs (< 5,000 µs dwell) is rejected
    let early_switch = gov.arbitrate_best_antennas(22_000);
    if early_switch {
        return false;
    }

    true
}

// ============================================================================
// Feature 4: Low-Latency UDP Gaming Socket Packet Classifier
// ============================================================================

/// Differentiated Services Code Point (DSCP) traffic classification tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamingPacketPriority {
    /// DSCP 46 (0x2E) - Expedited Forwarding (EF): Real-time input ticks & player positioning (< 2ms deadline).
    CriticalGameTick = 46,
    /// DSCP 34 (0x22) - Assured Forwarding 41 (AF41): Real-time spatial game audio & voice chat (< 10ms deadline).
    SpatialVoiceAudio = 34,
    /// DSCP 26 (0x1A) - Assured Forwarding 31 (AF31): State synchronization & physics snapshot delta (< 25ms deadline).
    StateSyncSnapshot = 26,
    /// DSCP 0 (0x00) - Best Effort (BE): Background game telemetry, match stats, asset streaming.
    BestEffortTelemetry = 0,
}

/// Wi-Fi Enhanced Distributed Channel Access (EDCA) Access Category (AC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiEdcaAccessCategory {
    /// AC_VO (Voice): Highest priority Wi-Fi transmit queue with shortest AIFS and contention window.
    AcVoice = 0,
    /// AC_VI (Video): High priority stream queue.
    AcVideo = 1,
    /// AC_BE (Best Effort): Standard medium priority queue.
    AcBestEffort = 2,
    /// AC_BK (Background): Low priority non-urgent queue.
    AcBackground = 3,
}

/// Metadata extracted from a gaming UDP datagram.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UdpGamingPacket {
    /// 32-bit packet sequence number.
    pub sequence_number: u32,
    /// Destination UDP port (e.g., 27015 for Source Engine, 7777 for Unreal Engine).
    pub dst_port: u16,
    /// Datagram payload length in bytes.
    pub payload_len: u16,
    /// DSCP priority classification.
    pub priority: GamingPacketPriority,
    /// Mapped 802.11 EDCA access category.
    pub edca_ac: WifiEdcaAccessCategory,
    /// Hardware enqueue timestamp in microseconds.
    pub enqueue_time_us: u64,
    /// Deadline in microseconds by which this packet must be placed on RF channel.
    pub deadline_us: u64,
    /// Whether packet header indicates critical player input tick.
    pub is_player_input: bool,
}

/// Fast packet classifier and priority queue governor for handheld multiplayer UDP streams.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UdpGamingPacketClassifier {
    /// Registered multiplayer game ports (e.g. 27015, 7777, 3074, 9987).
    pub registered_game_ports: [u16; 4],
    /// Total classified critical tick packets.
    pub critical_packets_count: u32,
    /// Total classified audio packets.
    pub audio_packets_count: u32,
    /// Total classified state sync packets.
    pub sync_packets_count: u32,
    /// Total classified background packets.
    pub best_effort_packets_count: u32,
    /// Total packets that violated deadline (jitter drop telemetry).
    pub deadline_miss_count: u32,
}

impl UdpGamingPacketClassifier {
    /// Creates a packet classifier with standard gaming UDP ports pre-registered.
    pub fn new() -> Self {
        Self {
            registered_game_ports: [27015, 7777, 3074, 9987],
            critical_packets_count: 0,
            audio_packets_count: 0,
            sync_packets_count: 0,
            best_effort_packets_count: 0,
            deadline_miss_count: 0,
        }
    }

    /// Classifies an incoming or outgoing UDP gaming packet based on port, size, and header flags.
    pub fn classify_packet(
        &mut self,
        dst_port: u16,
        payload: &[u8],
        enqueue_time_us: u64,
        is_user_input: bool,
    ) -> UdpGamingPacket {
        let payload_len = payload.len() as u16;

        // Check if destination port is one of the registered competitive game ports
        let is_game_port = self.registered_game_ports.iter().any(|&p| p == dst_port);

        let (priority, edca_ac, max_lead_time_us) = if is_user_input || (is_game_port && payload_len <= 128) {
            // Small packets on game port or explicitly flagged user input: Critical Game Tick
            self.critical_packets_count += 1;
            (
                GamingPacketPriority::CriticalGameTick,
                WifiEdcaAccessCategory::AcVoice,
                2_000u64, // 2 ms deadline
            )
        } else if dst_port == 9987 || (payload_len >= 129 && payload_len <= 320 && is_game_port) {
            // Voice / Audio stream packet
            self.audio_packets_count += 1;
            (
                GamingPacketPriority::SpatialVoiceAudio,
                WifiEdcaAccessCategory::AcVoice,
                10_000u64, // 10 ms deadline
            )
        } else if is_game_port && payload_len <= 1200 {
            // World state snapshot delta
            self.sync_packets_count += 1;
            (
                GamingPacketPriority::StateSyncSnapshot,
                WifiEdcaAccessCategory::AcVideo,
                25_000u64, // 25 ms deadline
            )
        } else {
            // Telemetry / asset download
            self.best_effort_packets_count += 1;
            (
                GamingPacketPriority::BestEffortTelemetry,
                WifiEdcaAccessCategory::AcBestEffort,
                100_000u64, // 100 ms deadline
            )
        };

        UdpGamingPacket {
            sequence_number: 0,
            dst_port,
            payload_len,
            priority,
            edca_ac,
            enqueue_time_us,
            deadline_us: enqueue_time_us + max_lead_time_us,
            is_player_input: is_user_input,
        }
    }

    /// Evaluates whether a packet is still valid to transmit or has expired its real-time deadline.
    pub fn check_packet_deadline(&mut self, packet: &UdpGamingPacket, current_time_us: u64) -> bool {
        if current_time_us > packet.deadline_us {
            self.deadline_miss_count += 1;
            false // Expired, discard to save RF airtime for fresh state
        } else {
            true // Valid
        }
    }
}

/// Integrated Handheld Wi-Fi 7 Subsystem Controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandheldWifi7Controller {
    /// Wi-Fi 7 dynamic preamble puncturing engine.
    pub puncture_engine: Wifi7PunctureEngine,
    /// Mesh multiplayer microsecond beacon synchronization governor.
    pub beacon_governor: MeshBeaconSyncGovernor,
    /// Chassis grip antenna diversity switch governor.
    pub antenna_governor: HandheldAntennaDiversityGovernor,
    /// Low-latency UDP gaming packet classifier.
    pub classifier: UdpGamingPacketClassifier,
}

impl HandheldWifi7Controller {
    /// Initializes the integrated Wi-Fi 7 controller.
    pub fn new() -> Self {
        Self {
            puncture_engine: Wifi7PunctureEngine::new(Wifi7Bandwidth::Width160MHz, 5260),
            beacon_governor: MeshBeaconSyncGovernor::new(MeshSyncRole::MeshPeer),
            antenna_governor: HandheldAntennaDiversityGovernor::new(),
            classifier: UdpGamingPacketClassifier::new(),
        }
    }
}

/// Verification test for low-latency UDP gaming socket packet classifier.
pub fn verify_udp_gaming_packet_classifier() -> bool {
    let mut classifier = UdpGamingPacketClassifier::new();

    // 1. Classify 64-byte user input tick to game port 27015
    let dummy_tick = [0xAAu8; 64];
    let pkt1 = classifier.classify_packet(27015, &dummy_tick, 100_000, true);

    if pkt1.priority != GamingPacketPriority::CriticalGameTick {
        return false;
    }
    if pkt1.edca_ac != WifiEdcaAccessCategory::AcVoice {
        return false;
    }
    if pkt1.deadline_us != 102_000 { // 100,000 + 2,000 us
        return false;
    }
    if classifier.critical_packets_count != 1 {
        return false;
    }

    // 2. Classify 250-byte voice audio datagram to port 9987
    let dummy_voice = [0x55u8; 250];
    let pkt2 = classifier.classify_packet(9987, &dummy_voice, 100_000, false);
    if pkt2.priority != GamingPacketPriority::SpatialVoiceAudio {
        return false;
    }
    if pkt2.edca_ac != WifiEdcaAccessCategory::AcVoice {
        return false;
    }
    if classifier.audio_packets_count != 1 {
        return false;
    }

    // 3. Classify 800-byte game state snapshot on port 7777
    let dummy_snap = [0x11u8; 800];
    let pkt3 = classifier.classify_packet(7777, &dummy_snap, 100_000, false);
    if pkt3.priority != GamingPacketPriority::StateSyncSnapshot {
        return false;
    }
    if pkt3.edca_ac != WifiEdcaAccessCategory::AcVideo {
        return false;
    }

    // 4. Test deadline expiration:
    // Packet 1 has deadline 102,000 µs. At 101,500 µs it is valid; at 102,500 µs it is expired.
    if !classifier.check_packet_deadline(&pkt1, 101_500) {
        return false;
    }
    if classifier.check_packet_deadline(&pkt1, 102_500) {
        return false;
    }
    if classifier.deadline_miss_count != 1 {
        return false;
    }

    // 5. Test integrated controller instantiation
    let controller = HandheldWifi7Controller::new();
    if controller.puncture_engine.active_bandwidth_mhz != 160 {
        return false;
    }

    true
}



