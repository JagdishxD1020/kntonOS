//! Handheld Wi-Fi 7 Multi-Link Operation (MLO) & Deterministic Game Packet Pacer.
//!
//! Provides simultaneous 5GHz + 6GHz dual-link aggregation, sub-2ms jitter buffering,
//! DSCP 46 Expedited Forwarding (EF) QoS priority queuing for multiplayer netcode,
//! and sub-5ms fast link failover for uninterrupted handheld gaming.

#![allow(dead_code)]

// ============================================================================
// Feature 1: Dual-Link 5GHz + 6GHz Aggregation & Bandwidth Allocator
// ============================================================================

/// Identifies the physical radio frequency band in Wi-Fi 7 MLO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MloBand {
    /// 5 GHz band (160 MHz channel, higher penetration/range).
    Band5GHz = 0,
    /// 6 GHz band (320 MHz channel, ultra-low contention & sub-2ms latency).
    Band6GHz = 1,
}

/// Operational state and RF metrics for an individual Wi-Fi 7 radio link.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MloLinkState {
    /// Radio band identifier.
    pub band: MloBand,
    /// Operating channel frequency in MHz.
    pub channel_freq_mhz: u16,
    /// Channel bandwidth in MHz (160 MHz or 320 MHz).
    pub channel_width_mhz: u16,
    /// Received Signal Strength Indicator in dBm (-90 .. -30 dBm).
    pub rssi_dbm: i8,
    /// Signal-to-Noise Ratio in dB (0 .. 60 dB).
    pub snr_db: u8,
    /// Moving average Packet Error Rate percentage (0.0 .. 100.0).
    pub per_pct: f32,
    /// Estimated link throughput capacity in Mbps.
    pub capacity_mbps: u32,
    /// Baseline one-way transit latency in microseconds.
    pub baseline_latency_us: u32,
    /// Whether this radio link is associated and active.
    pub is_active: bool,
    /// Total transmitted packets on this link.
    pub tx_packets: u64,
    /// Total received packets on this link.
    pub rx_packets: u64,
}

impl MloLinkState {
    /// Constructs default parameters for a 5 GHz MLO link.
    pub const fn new_5ghz() -> Self {
        Self {
            band: MloBand::Band5GHz,
            channel_freq_mhz: 5500,
            channel_width_mhz: 160,
            rssi_dbm: -55,
            snr_db: 35,
            per_pct: 0.5,
            capacity_mbps: 1200,
            baseline_latency_us: 4500, // 4.5ms
            is_active: true,
            tx_packets: 0,
            rx_packets: 0,
        }
    }

    /// Constructs default parameters for a 6 GHz MLO link.
    pub const fn new_6ghz() -> Self {
        Self {
            band: MloBand::Band6GHz,
            channel_freq_mhz: 6200,
            channel_width_mhz: 320,
            rssi_dbm: -50,
            snr_db: 42,
            per_pct: 0.1,
            capacity_mbps: 2400,
            baseline_latency_us: 1200, // 1.2ms (sub-2ms)
            is_active: true,
            tx_packets: 0,
            rx_packets: 0,
        }
    }

    /// Returns a calculated health/quality score [0.0 .. 1.0].
    pub fn quality_score(&self) -> f32 {
        if !self.is_active || self.per_pct >= 25.0 || self.rssi_dbm <= -85 {
            return 0.0;
        }

        // Higher SNR and lower PER yield higher quality
        let snr_factor = (self.snr_db as f32 / 50.0).clamp(0.1, 1.0);
        let per_penalty = (1.0 - (self.per_pct / 25.0)).clamp(0.0, 1.0);
        snr_factor * per_penalty
    }
}

/// Link allocation strategy for outbound traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MloAllocationStrategy {
    /// Route game netcode to fastest link (6GHz) and background bulk to high-capacity link.
    AdaptiveLatencyAware,
    /// Balance packet load proportionally according to relative bandwidth capacity.
    ProportionalBandwidth,
    /// Transmit duplicate frames simultaneously on both links for zero-loss mission-critical ticks.
    SeamlessDuplication,
}

/// Dual-Link 5GHz + 6GHz Wi-Fi 7 MLO Aggregator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualLinkMloAggregator {
    /// 5 GHz secondary / fallback radio link.
    pub link_5ghz: MloLinkState,
    /// 6 GHz primary ultra-low-latency radio link.
    pub link_6ghz: MloLinkState,
    /// Current multi-link aggregation strategy.
    pub strategy: MloAllocationStrategy,
    /// Round-robin accumulator for proportional distribution.
    pub rr_counter: u32,
}

impl DualLinkMloAggregator {
    /// Constructs a dual-link MLO aggregator with both radios initialized.
    pub const fn new() -> Self {
        Self {
            link_5ghz: MloLinkState::new_5ghz(),
            link_6ghz: MloLinkState::new_6ghz(),
            strategy: MloAllocationStrategy::AdaptiveLatencyAware,
            rr_counter: 0,
        }
    }

    /// Updates link telemetry for the specified radio band.
    pub fn update_link_telemetry(
        &mut self,
        band: MloBand,
        rssi_dbm: i8,
        snr_db: u8,
        per_pct: f32,
        capacity_mbps: u32,
    ) {
        let link = match band {
            MloBand::Band5GHz => &mut self.link_5ghz,
            MloBand::Band6GHz => &mut self.link_6ghz,
        };
        link.rssi_dbm = rssi_dbm;
        link.snr_db = snr_db;
        link.per_pct = per_pct;
        link.capacity_mbps = capacity_mbps;
    }

    /// Selects the optimal transmission link for an outgoing packet.
    pub fn select_tx_link(&mut self, is_latency_critical: bool) -> Option<MloBand> {
        let score_5g = self.link_5ghz.quality_score();
        let score_6g = self.link_6ghz.quality_score();

        // If neither link is viable, transmission fails
        if score_5g <= 0.0 && score_6g <= 0.0 {
            return None;
        }

        // Single link active fallback
        if score_5g <= 0.0 {
            self.link_6ghz.tx_packets += 1;
            return Some(MloBand::Band6GHz);
        }
        if score_6g <= 0.0 {
            self.link_5ghz.tx_packets += 1;
            return Some(MloBand::Band5GHz);
        }

        match self.strategy {
            MloAllocationStrategy::AdaptiveLatencyAware => {
                // Latency critical packets strictly prefer 6 GHz when healthy
                if is_latency_critical {
                    self.link_6ghz.tx_packets += 1;
                    Some(MloBand::Band6GHz)
                } else {
                    // Bulk packets distribute according to capacity (6GHz has 2x capacity of 5GHz)
                    self.rr_counter = self.rr_counter.wrapping_add(1);
                    if self.rr_counter % 3 == 0 {
                        self.link_5ghz.tx_packets += 1;
                        Some(MloBand::Band5GHz)
                    } else {
                        self.link_6ghz.tx_packets += 1;
                        Some(MloBand::Band6GHz)
                    }
                }
            }
            MloAllocationStrategy::ProportionalBandwidth => {
                let total_cap = self.link_5ghz.capacity_mbps + self.link_6ghz.capacity_mbps;
                let ratio_5g = (self.link_5ghz.capacity_mbps * 10) / total_cap.max(1);

                self.rr_counter = self.rr_counter.wrapping_add(1);
                if (self.rr_counter % 10) < ratio_5g {
                    self.link_5ghz.tx_packets += 1;
                    Some(MloBand::Band5GHz)
                } else {
                    self.link_6ghz.tx_packets += 1;
                    Some(MloBand::Band6GHz)
                }
            }
            MloAllocationStrategy::SeamlessDuplication => {
                // Return primary link (6GHz), caller duplicates packet to both
                self.link_5ghz.tx_packets += 1;
                self.link_6ghz.tx_packets += 1;
                Some(MloBand::Band6GHz)
            }
        }
    }
}

/// Verification test for Wi-Fi 7 dual-link 5GHz + 6GHz aggregation and bandwidth allocator.
pub fn verify_dual_link_mlo_aggregation() -> bool {
    let mut aggregator = DualLinkMloAggregator::new();

    // 1. Initial link parameters verification
    if aggregator.link_5ghz.channel_width_mhz != 160 || aggregator.link_6ghz.channel_width_mhz != 320 {
        return false;
    }
    if aggregator.link_6ghz.baseline_latency_us > 2000 {
        return false;
    }

    // 2. Latency-critical packet routing should strictly select 6GHz
    for _ in 0..10 {
        let selected = aggregator.select_tx_link(true);
        if selected != Some(MloBand::Band6GHz) {
            return false;
        }
    }
    if aggregator.link_6ghz.tx_packets != 10 || aggregator.link_5ghz.tx_packets != 0 {
        return false;
    }

    // 3. Bulk traffic routing in Adaptive mode should distribute across both links
    for _ in 0..30 {
        let _ = aggregator.select_tx_link(false);
    }
    if aggregator.link_5ghz.tx_packets == 0 || aggregator.link_6ghz.tx_packets <= 10 {
        return false;
    }

    // 4. Degrade 6GHz RF metrics (high PER = 30%)
    aggregator.update_link_telemetry(MloBand::Band6GHz, -88, 5, 30.0, 100);
    // Latency-critical packets should now automatically route to healthy 5GHz link
    let fallback = aggregator.select_tx_link(true);
    if fallback != Some(MloBand::Band5GHz) {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Sub-2ms Jitter Buffer & Dynamic Link Re-balancing
// ============================================================================

/// Maximum playout jitter buffer deadline in microseconds (2.0 ms target).
pub const JITTER_BUFFER_MAX_HOLD_US: u64 = 2000;
/// Capacity of the jitter re-ordering buffer.
pub const JITTER_BUFFER_CAPACITY: usize = 32;

/// Packet descriptor transmitted and received across Wi-Fi 7 MLO links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MloPacket {
    /// Packet monotonic sequence number.
    pub seq_num: u32,
    /// Hardware timestamp in microseconds when packet was enqueued / received.
    pub timestamp_us: u64,
    /// Radio band on which packet was sent/received.
    pub band: MloBand,
    /// Differentiated Services Code Point (e.g. 46 for Expedited Forwarding).
    pub dscp: u8,
    /// Payload length in bytes.
    pub payload_len: u16,
    /// Whether this frame is an MLO duplicate transmission.
    pub is_duplicate: bool,
}

impl MloPacket {
    /// Constructs a standard gaming netcode packet.
    pub const fn new(seq_num: u32, timestamp_us: u64, band: MloBand, dscp: u8, payload_len: u16) -> Self {
        Self {
            seq_num,
            timestamp_us,
            band,
            dscp,
            payload_len,
            is_duplicate: false,
        }
    }
}

/// Sub-2ms Packet De-Jitter Buffer reordering out-of-order multi-link arrivals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MloJitterBuffer {
    /// Packet storage slots.
    pub slots: [Option<MloPacket>; JITTER_BUFFER_CAPACITY],
    /// Highest consecutive sequence number delivered to game engine.
    pub last_delivered_seq: u32,
    /// Playout target delay in microseconds (<= 2000 us).
    pub playout_delay_us: u64,
    /// Number of packets dropped due to late arrival past the sub-2ms deadline.
    pub dropped_late_packets: u32,
    /// Number of duplicate frames suppressed.
    pub duplicate_suppressed: u32,
}

impl MloJitterBuffer {
    /// Constructs a sub-2ms jitter buffer with zero initial state.
    pub const fn new() -> Self {
        Self {
            slots: [None; JITTER_BUFFER_CAPACITY],
            last_delivered_seq: 0,
            playout_delay_us: JITTER_BUFFER_MAX_HOLD_US,
            dropped_late_packets: 0,
            duplicate_suppressed: 0,
        }
    }

    /// Resets the jitter buffer.
    pub fn reset(&mut self) {
        self.slots = [None; JITTER_BUFFER_CAPACITY];
        self.last_delivered_seq = 0;
        self.dropped_late_packets = 0;
        self.duplicate_suppressed = 0;
    }

    /// Ingests an incoming packet received from either 5GHz or 6GHz link.
    pub fn push_packet(&mut self, pkt: MloPacket, _now_us: u64) -> bool {
        // Discard duplicates or already-delivered sequence numbers
        if self.last_delivered_seq > 0 && pkt.seq_num <= self.last_delivered_seq {
            self.duplicate_suppressed += 1;
            return false;
        }

        // Check for duplicate in current buffer
        for slot in self.slots.iter_mut() {
            if let Some(existing) = slot {
                if existing.seq_num == pkt.seq_num {
                    self.duplicate_suppressed += 1;
                    return false;
                }
            }
        }

        // Find empty slot
        for slot in self.slots.iter_mut() {
            if slot.is_none() {
                *slot = Some(pkt);
                return true;
            }
        }

        // Buffer overflow: drop oldest
        self.dropped_late_packets += 1;
        false
    }

    /// Pops the next playable in-order packet or expires overdue gaps.
    pub fn pop_packet(&mut self, now_us: u64) -> Option<MloPacket> {
        let expected_seq = self.last_delivered_seq + 1;

        // 1. Direct in-order hit check
        for slot in self.slots.iter_mut() {
            if let Some(pkt) = slot {
                if pkt.seq_num == expected_seq {
                    let res = *pkt;
                    *slot = None;
                    self.last_delivered_seq = res.seq_num;
                    return Some(res);
                }
            }
        }

        // 2. Deadline expiration check: find oldest buffered packet
        let mut min_seq: Option<(usize, u32, u64)> = None;
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(pkt) = slot {
                match min_seq {
                    None => min_seq = Some((i, pkt.seq_num, pkt.timestamp_us)),
                    Some((_, best_seq, _)) => {
                        if pkt.seq_num < best_seq {
                            min_seq = Some((i, pkt.seq_num, pkt.timestamp_us));
                        }
                    }
                }
            }
        }

        if let Some((idx, seq, ts)) = min_seq {
            // If the oldest buffered packet has waited longer than the sub-2ms playout deadline
            if now_us.saturating_sub(ts) >= self.playout_delay_us {
                let res = self.slots[idx].take();
                self.last_delivered_seq = seq;
                return res;
            }
        }

        None
    }
}

/// Real-time Jitter Monitor and Link Re-balancer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MloLinkRebalancer {
    /// Estimated moving average jitter on 5 GHz link (microseconds).
    pub jitter_5ghz_us: u32,
    /// Estimated moving average jitter on 6 GHz link (microseconds).
    pub jitter_6ghz_us: u32,
    /// Previous packet transit latency on 5 GHz link.
    pub prev_transit_5ghz_us: u32,
    /// Previous packet transit latency on 6 GHz link.
    pub prev_transit_6ghz_us: u32,
    /// Whether 6 GHz link jitter exceeded sub-2ms bound, forcing traffic shift.
    pub force_shift_to_5ghz: bool,
}

impl MloLinkRebalancer {
    pub const fn new() -> Self {
        Self {
            jitter_5ghz_us: 1000,
            jitter_6ghz_us: 300,
            prev_transit_5ghz_us: 4500,
            prev_transit_6ghz_us: 1200,
            force_shift_to_5ghz: false,
        }
    }

    /// Records transit latency for an arrived packet and updates RFC 3550 style jitter.
    pub fn update_transit_sample(&mut self, band: MloBand, transit_time_us: u32) {
        match band {
            MloBand::Band5GHz => {
                let diff = if transit_time_us >= self.prev_transit_5ghz_us {
                    transit_time_us - self.prev_transit_5ghz_us
                } else {
                    self.prev_transit_5ghz_us - transit_time_us
                };
                self.prev_transit_5ghz_us = transit_time_us;
                self.jitter_5ghz_us = self.jitter_5ghz_us + ((diff as i32 - self.jitter_5ghz_us as i32) / 16) as u32;
            }
            MloBand::Band6GHz => {
                let diff = if transit_time_us >= self.prev_transit_6ghz_us {
                    transit_time_us - self.prev_transit_6ghz_us
                } else {
                    self.prev_transit_6ghz_us - transit_time_us
                };
                self.prev_transit_6ghz_us = transit_time_us;
                self.jitter_6ghz_us = self.jitter_6ghz_us + ((diff as i32 - self.jitter_6ghz_us as i32) / 16) as u32;

                // Re-balance trigger: if 6 GHz jitter exceeds sub-2ms bound (2000 us), shift traffic
                if self.jitter_6ghz_us >= JITTER_BUFFER_MAX_HOLD_US as u32 {
                    self.force_shift_to_5ghz = true;
                } else if self.jitter_6ghz_us <= 1200 {
                    self.force_shift_to_5ghz = false;
                }
            }
        }
    }
}

/// Verification test for sub-2ms jitter buffer and dynamic link rebalancing.
pub fn verify_mlo_jitter_buffer_and_rebalance() -> bool {
    let mut jbuf = MloJitterBuffer::new();

    // 1. Ingest out-of-order packets: [Seq 1, Seq 3, Seq 2, Seq 4]
    let t0 = 10_000_000u64;
    let p1 = MloPacket::new(1, t0, MloBand::Band5GHz, 46, 64);
    let p3 = MloPacket::new(3, t0 + 200, MloBand::Band6GHz, 46, 64);
    let p2 = MloPacket::new(2, t0 + 400, MloBand::Band5GHz, 46, 64);
    let p4 = MloPacket::new(4, t0 + 600, MloBand::Band6GHz, 46, 64);

    jbuf.push_packet(p1, t0);
    jbuf.push_packet(p3, t0 + 200);
    jbuf.push_packet(p2, t0 + 400);
    jbuf.push_packet(p4, t0 + 600);

    // Verify packets are popped in strictly monotonic sequence [1, 2, 3, 4]
    let out1 = jbuf.pop_packet(t0 + 500);
    if out1.map(|p| p.seq_num) != Some(1) {
        return false;
    }
    let out2 = jbuf.pop_packet(t0 + 500);
    if out2.map(|p| p.seq_num) != Some(2) {
        return false;
    }
    let out3 = jbuf.pop_packet(t0 + 500);
    if out3.map(|p| p.seq_num) != Some(3) {
        return false;
    }
    let out4 = jbuf.pop_packet(t0 + 700);
    if out4.map(|p| p.seq_num) != Some(4) {
        return false;
    }

    // 2. Test Duplicate Suppression
    let dup_p4 = MloPacket::new(4, t0 + 800, MloBand::Band5GHz, 46, 64);
    if jbuf.push_packet(dup_p4, t0 + 800) {
        return false;
    }
    if jbuf.duplicate_suppressed == 0 {
        return false;
    }

    // 3. Test Playout Timeout on Lost Packet
    // Packet 5 lost. Packet 6 arrives at t = 10_001_000
    let p6 = MloPacket::new(6, 10_001_000, MloBand::Band6GHz, 46, 64);
    jbuf.push_packet(p6, 10_001_000);

    // At t + 1000us (< 2000us), packet 6 should NOT pop yet (waiting for packet 5)
    if jbuf.pop_packet(10_002_000).is_some() {
        return false;
    }

    // At t + 2500us (>= 2000us deadline), deadline expires and packet 6 must pop
    let out6 = jbuf.pop_packet(10_003_500);
    if out6.map(|p| p.seq_num) != Some(6) {
        return false;
    }

    // 4. Test Jitter Monitor and Link Rebalancing
    let mut rebalancer = MloLinkRebalancer::new();
    if rebalancer.force_shift_to_5ghz {
        return false;
    }

    // Inject high transit latency fluctuations on 6 GHz link
    for i in 0..40 {
        let transit = if i % 2 == 0 { 10_000 } else { 1_000 };
        rebalancer.update_transit_sample(MloBand::Band6GHz, transit);
    }

    // Jitter on 6GHz should exceed 2000 us and trigger shift
    if !rebalancer.force_shift_to_5ghz || rebalancer.jitter_6ghz_us < 2000 {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: DSCP 46 Expedited Forwarding (EF) QoS Priority Queue
// ============================================================================

/// DSCP 46 (Expedited Forwarding, 0x2E): Critical Multiplayer Game Netcode.
pub const DSCP_EXPEDITED_FORWARDING: u8 = 46;
/// DSCP 34 (Assured Forwarding AF41, 0x22): Real-Time Voice Chat.
pub const DSCP_VOICE_CHAT: u8 = 34;
/// DSCP 26 (Assured Forwarding AF31, 0x1A): Video Streaming Slices.
pub const DSCP_VIDEO_STREAMING: u8 = 26;
/// DSCP 0 (Best Effort, 0x00): Default Background Downloads.
pub const DSCP_BEST_EFFORT: u8 = 0;

/// Internal Quality of Service priority tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QosPriorityTier {
    /// Strict highest priority: game ticks, inputs, hit registration.
    ExpeditedForwarding = 0,
    /// Second priority: voice communications.
    Voice = 1,
    /// Third priority: display video frames.
    Video = 2,
    /// Lowest priority: background bulk downloads.
    BestEffort = 3,
}

impl QosPriorityTier {
    /// Maps a 6-bit DSCP code point into the corresponding QoS priority tier.
    pub fn from_dscp(dscp: u8) -> Self {
        match dscp {
            DSCP_EXPEDITED_FORWARDING => Self::ExpeditedForwarding,
            DSCP_VOICE_CHAT => Self::Voice,
            DSCP_VIDEO_STREAMING => Self::Video,
            _ => Self::BestEffort,
        }
    }
}

/// Fixed-capacity ring buffer for an individual QoS tier.
pub const QOS_QUEUE_TIER_CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QosFifoRing {
    pub buffer: [Option<MloPacket>; QOS_QUEUE_TIER_CAPACITY],
    pub head: usize,
    pub tail: usize,
    pub count: usize,
}

impl QosFifoRing {
    pub const fn new() -> Self {
        Self {
            buffer: [None; QOS_QUEUE_TIER_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, pkt: MloPacket) -> bool {
        if self.count >= QOS_QUEUE_TIER_CAPACITY {
            return false; // Queue full
        }
        self.buffer[self.tail] = Some(pkt);
        self.tail = (self.tail + 1) % QOS_QUEUE_TIER_CAPACITY;
        self.count += 1;
        true
    }

    pub fn pop(&mut self) -> Option<MloPacket> {
        if self.count == 0 {
            return None;
        }
        let item = self.buffer[self.head].take();
        self.head = (self.head + 1) % QOS_QUEUE_TIER_CAPACITY;
        self.count -= 1;
        item
    }
}

/// 4-Tier Strict-Priority QoS Scheduler with DSCP 46 Expedited Forwarding preemption.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DscpPriorityQueue {
    /// Tier 0: DSCP 46 EF game netcode queue.
    pub ef_queue: QosFifoRing,
    /// Tier 1: DSCP 34 voice chat queue.
    pub voice_queue: QosFifoRing,
    /// Tier 2: DSCP 26 video streaming queue.
    pub video_queue: QosFifoRing,
    /// Tier 3: DSCP 0 best effort queue.
    pub be_queue: QosFifoRing,
    /// Total packets processed across all queues.
    pub total_enqueued: u64,
    /// Total packets dropped due to tier capacity saturation.
    pub total_dropped: u32,
}

impl DscpPriorityQueue {
    /// Constructs a strict-priority QoS queue.
    pub const fn new() -> Self {
        Self {
            ef_queue: QosFifoRing::new(),
            voice_queue: QosFifoRing::new(),
            video_queue: QosFifoRing::new(),
            be_queue: QosFifoRing::new(),
            total_enqueued: 0,
            total_dropped: 0,
        }
    }

    /// Enqueues an outgoing packet according to its DSCP code point.
    pub fn enqueue(&mut self, pkt: MloPacket) -> bool {
        let tier = QosPriorityTier::from_dscp(pkt.dscp);
        let success = match tier {
            QosPriorityTier::ExpeditedForwarding => self.ef_queue.push(pkt),
            QosPriorityTier::Voice => self.voice_queue.push(pkt),
            QosPriorityTier::Video => self.video_queue.push(pkt),
            QosPriorityTier::BestEffort => self.be_queue.push(pkt),
        };

        if success {
            self.total_enqueued += 1;
            true
        } else {
            self.total_dropped += 1;
            false
        }
    }

    /// Dequeues the highest-priority available packet with strict EF preemption.
    pub fn dequeue_highest_priority(&mut self) -> Option<MloPacket> {
        // Strict priority order: EF (46) > Voice (34) > Video (26) > BE (0)
        if let Some(pkt) = self.ef_queue.pop() {
            return Some(pkt);
        }
        if let Some(pkt) = self.voice_queue.pop() {
            return Some(pkt);
        }
        if let Some(pkt) = self.video_queue.pop() {
            return Some(pkt);
        }
        self.be_queue.pop()
    }
}

/// Verification test for DSCP 46 Expedited Forwarding QoS priority queue.
pub fn verify_dscp46_qos_priority_queue() -> bool {
    let mut qos = DscpPriorityQueue::new();
    let now = 20_000_000u64;

    // 1. Enqueue 5 bulk best effort packets (DSCP 0)
    for i in 1..=5 {
        let be_pkt = MloPacket::new(i, now, MloBand::Band5GHz, DSCP_BEST_EFFORT, 1400);
        if !qos.enqueue(be_pkt) {
            return false;
        }
    }

    // 2. Enqueue a video packet (DSCP 26) and a voice packet (DSCP 34)
    let vid_pkt = MloPacket::new(6, now, MloBand::Band6GHz, DSCP_VIDEO_STREAMING, 1200);
    let voice_pkt = MloPacket::new(7, now, MloBand::Band6GHz, DSCP_VOICE_CHAT, 120);
    qos.enqueue(vid_pkt);
    qos.enqueue(voice_pkt);

    // 3. Enqueue a critical multiplayer game netcode tick (DSCP 46 Expedited Forwarding)
    let ef_pkt = MloPacket::new(99, now, MloBand::Band6GHz, DSCP_EXPEDITED_FORWARDING, 64);
    qos.enqueue(ef_pkt);

    // 4. Strict priority dequeue verification:
    // First dequeued MUST be the DSCP 46 EF packet (Seq 99), preempting all prior bulk packets
    let first = qos.dequeue_highest_priority();
    if first.map(|p| p.seq_num) != Some(99) || first.map(|p| p.dscp) != Some(DSCP_EXPEDITED_FORWARDING) {
        return false;
    }

    // Second dequeued MUST be Voice (Seq 7, DSCP 34)
    let second = qos.dequeue_highest_priority();
    if second.map(|p| p.seq_num) != Some(7) || second.map(|p| p.dscp) != Some(DSCP_VOICE_CHAT) {
        return false;
    }

    // Third dequeued MUST be Video (Seq 6, DSCP 26)
    let third = qos.dequeue_highest_priority();
    if third.map(|p| p.seq_num) != Some(6) || third.map(|p| p.dscp) != Some(DSCP_VIDEO_STREAMING) {
        return false;
    }

    // Fourth dequeued MUST be the first Best Effort packet (Seq 1, DSCP 0)
    let fourth = qos.dequeue_highest_priority();
    if fourth.map(|p| p.seq_num) != Some(1) || fourth.map(|p| p.dscp) != Some(DSCP_BEST_EFFORT) {
        return false;
    }

    true
}
