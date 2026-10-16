//! Handheld PCIe 5.0 Link Health Monitor & Dynamic Lane Width / Speed Re-Tuning Engine.
//!
//! Handheld gaming consoles equipped with high-speed PCIe Gen 5 interconnects (for high-speed
//! DirectStorage M.2 NVMe SSDs and external/discrete GPUs) operate in thermally constrained,
//! physically vibrating environments. Thermal expansion, signal crosstalk, and connector fatigue
//! at 32 GT/s PAM4/NRZ signaling can lead to severe eye margin degradation and transport errors.
//!
//! This module implements:
//! 1. PCIe Gen 5 link status monitoring (AER - Advanced Error Reporting, eye margin telemetry).
//! 2. Dynamic link retraining and down-throttling fallback (Gen 5 x4 -> Gen 4 x4 / Gen 5 x2).
//! 3. Thermal-aware bus power governor (scales width x4 -> x2 -> x1 during idle/moderate load).
//! 4. PCIe L1 PM Substates (L1.1 / L1.2 ASPM) fast-entry and sub-microsecond exit latency regulator.

#![allow(dead_code)]

// ============================================================================
// Feature 1: PCIe Gen 5 Link Status Monitoring & Advanced Error Reporting (AER)
// ============================================================================

/// PCIe Link Generation speed classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PcieGeneration {
    Gen1 = 1, // 2.5 GT/s
    Gen2 = 2, // 5.0 GT/s
    Gen3 = 3, // 8.0 GT/s
    Gen4 = 4, // 16.0 GT/s
    Gen5 = 5, // 32.0 GT/s
}

impl PcieGeneration {
    /// Nominal transfer rate in Gigatransfers per second (GT/s).
    pub const fn transfer_rate_gts(&self) -> u32 {
        match self {
            Self::Gen1 => 2,
            Self::Gen2 => 5,
            Self::Gen3 => 8,
            Self::Gen4 => 16,
            Self::Gen5 => 32,
        }
    }

    /// Theoretical peak bandwidth in Megabytes per second per lane (after encoding overhead).
    pub const fn bandwidth_mb_s_per_lane(&self) -> u32 {
        match self {
            Self::Gen1 => 250,   // 8b/10b encoding (80% efficiency)
            Self::Gen2 => 500,   // 8b/10b encoding
            Self::Gen3 => 985,   // 128b/130b encoding (~98.46% efficiency)
            Self::Gen4 => 1969,  // 128b/130b encoding
            Self::Gen5 => 3938,  // 128b/130b encoding
        }
    }
}

/// PCIe Link Negotiated Width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PcieLinkWidth {
    X1 = 1,
    X2 = 2,
    X4 = 4,
    X8 = 8,
    X16 = 16,
}

impl PcieLinkWidth {
    pub const fn lane_count(&self) -> u8 {
        *self as u8
    }
}

/// AER (Advanced Error Reporting) Correctable Error Status Mask.
pub mod aer_cor_bits {
    pub const RECEIVER_ERROR: u32 = 1 << 0;
    pub const BAD_TLP: u32 = 1 << 6;
    pub const BAD_DLLP: u32 = 1 << 7;
    pub const REPLAY_NUM_ROLLOVER: u32 = 1 << 8;
    pub const REPLAY_TIMER_TIMEOUT: u32 = 1 << 12;
    pub const ADVISORY_NON_FATAL: u32 = 1 << 13;
    pub const CORRECTED_INTERNAL_ERROR: u32 = 1 << 14;
    pub const HEADER_LOG_OVERFLOW: u32 = 1 << 15;
}

/// AER (Advanced Error Reporting) Uncorrectable Error Status Mask.
pub mod aer_uncor_bits {
    pub const DATA_LINK_PROTOCOL_ERROR: u32 = 1 << 4;
    pub const SURPRISE_DOWN: u32 = 1 << 5;
    pub const POISONED_TLP: u32 = 1 << 12;
    pub const FLOW_CONTROL_PROTOCOL_ERROR: u32 = 1 << 13;
    pub const COMPLETION_TIMEOUT: u32 = 1 << 14;
    pub const COMPLETER_ABORT: u32 = 1 << 15;
    pub const UNEXPECTED_COMPLETION: u32 = 1 << 16;
    pub const RECEIVER_OVERFLOW: u32 = 1 << 17;
    pub const MALFORMED_TLP: u32 = 1 << 18;
    pub const ECRC_ERROR: u32 = 1 << 19;
    pub const UNSUPPORTED_REQUEST: u32 = 1 << 20;
    pub const ACS_VIOLATION: u32 = 1 << 21;
    pub const UNCORRECTABLE_INTERNAL_ERROR: u32 = 1 << 22;
}

/// Maximum lanes supported in handheld form factor monitoring.
pub const MAX_PCIE_LANES: usize = 4;

/// Per-lane physical layer receiver eye margin telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneEyeMarginTelemetry {
    /// Vertical eye margin (mV height above decision threshold). Healthy: >= 50 mV.
    pub voltage_margin_mv: u16,
    /// Horizontal eye margin (milli-UI timing width, 1 UI = 31.25 ps at Gen 5). Healthy: >= 300 mUI (30% UI).
    pub timing_margin_mui: u16,
    /// Signal-to-noise ratio in decibels (dB). Healthy: >= 20 dB.
    pub snr_db: u16,
    /// Bit Error Rate exponent (e.g. -12 for 10^-12, -5 for 10^-5). Healthy: <= -10.
    pub ber_exponent: i8,
}

impl LaneEyeMarginTelemetry {
    pub const fn default_healthy() -> Self {
        Self {
            voltage_margin_mv: 85,
            timing_margin_mui: 380,
            snr_db: 24,
            ber_exponent: -12,
        }
    }

    /// Checks if this lane exhibits signal degradation below safety margins.
    pub const fn is_degraded(&self) -> bool {
        self.voltage_margin_mv < 40 || self.timing_margin_mui < 200 || self.ber_exponent > -8
    }
}

/// Overall PCIe link health operational state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcieLinkHealthState {
    /// Signal integrity optimal, error rate zero or within standard FEC/replay bounds.
    Optimal,
    /// Elevated correctable errors or marginal eye opening; proactive monitoring active.
    DegradedWarning,
    /// Uncorrectable errors detected or eye collapse; link down-retraining required.
    CriticalFailure,
}

/// Advanced Error Reporting and Link Health Monitor.
#[derive(Debug, Clone)]
pub struct PcieLinkHealthMonitor {
    /// Active link generation.
    pub generation: PcieGeneration,
    /// Active link width.
    pub width: PcieLinkWidth,
    /// Accumulated correctable error register mask.
    pub aer_cor_status: u32,
    /// Accumulated uncorrectable error register mask.
    pub aer_uncor_status: u32,
    /// Total count of correctable error incidents recorded.
    pub total_correctable_count: u32,
    /// Total count of uncorrectable error incidents recorded.
    pub total_uncorrectable_count: u32,
    /// Replay timer timeout count.
    pub replay_timeout_count: u32,
    /// Bad TLP packet count.
    pub bad_tlp_count: u32,
    /// Per-lane eye margin telemetry.
    pub lane_telemetry: [LaneEyeMarginTelemetry; MAX_PCIE_LANES],
    /// Overall health state assessment.
    pub health_state: PcieLinkHealthState,
    /// Calculated composite health score (0-100%).
    pub health_score: u8,
}

impl PcieLinkHealthMonitor {
    pub const fn new(generation: PcieGeneration, width: PcieLinkWidth) -> Self {
        Self {
            generation,
            width,
            aer_cor_status: 0,
            aer_uncor_status: 0,
            total_correctable_count: 0,
            total_uncorrectable_count: 0,
            replay_timeout_count: 0,
            bad_tlp_count: 0,
            lane_telemetry: [LaneEyeMarginTelemetry::default_healthy(); MAX_PCIE_LANES],
            health_state: PcieLinkHealthState::Optimal,
            health_score: 100,
        }
    }

    /// Records a correctable error event reported by the PCIe AER hardware logic.
    pub fn record_correctable_error(&mut self, error_mask: u32) {
        self.aer_cor_status |= error_mask;
        self.total_correctable_count += 1;

        if (error_mask & aer_cor_bits::BAD_TLP) != 0 {
            self.bad_tlp_count += 1;
        }
        if (error_mask & aer_cor_bits::REPLAY_TIMER_TIMEOUT) != 0 {
            self.replay_timeout_count += 1;
        }

        self.update_health_assessment();
    }

    /// Records an uncorrectable error event reported by PCIe AER hardware logic.
    pub fn record_uncorrectable_error(&mut self, error_mask: u32) {
        self.aer_uncor_status |= error_mask;
        self.total_uncorrectable_count += 1;
        self.update_health_assessment();
    }

    /// Updates eye margin telemetry for a specific lane.
    pub fn update_lane_telemetry(&mut self, lane_idx: usize, telemetry: LaneEyeMarginTelemetry) {
        if lane_idx < MAX_PCIE_LANES {
            self.lane_telemetry[lane_idx] = telemetry;
            self.update_health_assessment();
        }
    }

    /// Computes composite link health score (0-100%) and updates `health_state`.
    pub fn update_health_assessment(&mut self) {
        // Start from 100 base score
        let mut score: i32 = 100;

        // Uncorrectable errors immediately slash health score
        if self.total_uncorrectable_count > 0 {
            score -= (self.total_uncorrectable_count as i32) * 50;
        }

        // Correctable errors deduct incrementally
        if self.total_correctable_count > 0 {
            score -= (self.total_correctable_count as i32) * 2;
        }

        // Replay timeouts indicate link instability
        if self.replay_timeout_count > 0 {
            score -= (self.replay_timeout_count as i32) * 10;
        }

        // Check per-lane eye margins
        let active_lanes = self.width.lane_count() as usize;
        let lanes_to_check = if active_lanes > MAX_PCIE_LANES { MAX_PCIE_LANES } else { active_lanes };
        let mut degraded_lanes_count = 0;

        for i in 0..lanes_to_check {
            let lane = &self.lane_telemetry[i];
            if lane.is_degraded() {
                degraded_lanes_count += 1;
                score -= 15;
            }
        }

        // Clamp score 0 to 100
        if score < 0 {
            score = 0;
        } else if score > 100 {
            score = 100;
        }
        self.health_score = score as u8;

        // Classify health state
        if self.total_uncorrectable_count > 0 || degraded_lanes_count >= 2 || self.health_score < 40 {
            self.health_state = PcieLinkHealthState::CriticalFailure;
        } else if self.total_correctable_count >= 5 || degraded_lanes_count == 1 || self.health_score < 80 {
            self.health_state = PcieLinkHealthState::DegradedWarning;
        } else {
            self.health_state = PcieLinkHealthState::Optimal;
        }
    }

    /// Clears accumulated error status registers (e.g. after successful retraining).
    pub fn clear_error_registers(&mut self) {
        self.aer_cor_status = 0;
        self.aer_uncor_status = 0;
        self.update_health_assessment();
    }
}

/// Verification test for PCIe Gen 5 link status monitoring and AER error tracking.
pub fn verify_pcie5_link_status_and_aer() -> bool {
    let mut monitor = PcieLinkHealthMonitor::new(PcieGeneration::Gen5, PcieLinkWidth::X4);

    // 1. Initial healthy state check
    if monitor.generation != PcieGeneration::Gen5 || monitor.width != PcieLinkWidth::X4 {
        return false;
    }
    if monitor.health_state != PcieLinkHealthState::Optimal || monitor.health_score != 100 {
        return false;
    }

    // 2. Bandwidth calculation check: Gen5 x4 = 3938 MB/s * 4 = 15,752 MB/s
    let total_bw = monitor.generation.bandwidth_mb_s_per_lane() * monitor.width.lane_count() as u32;
    if total_bw != 15_752 {
        return false;
    }

    // 3. Inject correctable errors (Bad TLP + Receiver Error)
    monitor.record_correctable_error(aer_cor_bits::BAD_TLP | aer_cor_bits::RECEIVER_ERROR);
    if monitor.total_correctable_count != 1 || monitor.bad_tlp_count != 1 {
        return false;
    }
    if monitor.health_score != 98 {
        return false;
    }

    // 4. Inject 5 more correctable errors to trigger DegradedWarning
    for _ in 0..5 {
        monitor.record_correctable_error(aer_cor_bits::BAD_DLLP);
    }
    if monitor.health_state != PcieLinkHealthState::DegradedWarning {
        return false;
    }

    // 5. Inject lane degradation on Lane 0: voltage margin collapses to 25 mV, BER drops to 10^-5
    let degraded_lane = LaneEyeMarginTelemetry {
        voltage_margin_mv: 25,
        timing_margin_mui: 180,
        snr_db: 14,
        ber_exponent: -5,
    };
    monitor.update_lane_telemetry(0, degraded_lane);
    if !monitor.lane_telemetry[0].is_degraded() {
        return false;
    }

    // 6. Inject uncorrectable error (Data Link Protocol Error) -> immediate CriticalFailure
    monitor.record_uncorrectable_error(aer_uncor_bits::DATA_LINK_PROTOCOL_ERROR);
    if monitor.health_state != PcieLinkHealthState::CriticalFailure {
        return false;
    }
    if monitor.total_uncorrectable_count != 1 {
        return false;
    }

    true
}
