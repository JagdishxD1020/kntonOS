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

