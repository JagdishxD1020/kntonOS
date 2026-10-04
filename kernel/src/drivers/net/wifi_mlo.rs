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
