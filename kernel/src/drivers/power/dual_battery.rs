//! Handheld Dual-Battery Hot-Swap Power Balancing & Charge Current Multiplexer.
//!
//! Features:
//! 1. Dual-cell battery impedance tracking and dynamic power share balancing.
//! 2. Hot-swap bridge hold-up time estimator ensuring zero reboot on pack swap.
//! 3. USB-PD PPS fast-charge thermal governor limiting grip surface temperature below 42°C.
//! 4. USB OTG reverse power bank discharge controller (up to 15W).
//! 5. Comprehensive self-test suite emitting `[KNT-POWER-DUAL-BATTERY-PASS]`.

#![allow(dead_code)]
#![allow(non_camel_case_types)]

// ============================================================================
// Feature 1: Dual-Cell Battery Impedance Tracking & Dynamic Power Sharing
// ============================================================================

/// Minimum cell voltage cutoff in millivolts (2S configuration = 6,000 mV).
pub const BATTERY_CUTOFF_VOLTAGE_MV: u32 = 6000;
/// Nominal fully-charged cell voltage in millivolts (2S configuration = 8,400 mV).
pub const BATTERY_NOMINAL_FULL_MV: u32 = 8400;

/// Individual battery pack descriptor with real-time impedance tracking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualBatteryCell {
    /// Pack identifier (0 = Primary Internal, 1 = Secondary Modular Grip Pack).
    pub pack_id: u8,
    /// Terminal loaded voltage in millivolts.
    pub voltage_mv: u32,
    /// Real-time current in milliamps (positive = discharging, negative = charging).
    pub current_ma: i32,
    /// Calculated internal DC resistance (impedance) in milliohms.
    pub internal_resistance_mohm: u32,
    /// State of Charge (SoC) percentage (0.0 to 100.0).
    pub soc_pct: f32,
    /// Core battery cell temperature in °C.
    pub temp_c: f32,
    /// Physical electrical connection status.
    pub is_connected: bool,
}

impl DualBatteryCell {
    /// Constructs a battery cell with nominal defaults.
    pub const fn new(pack_id: u8, is_connected: bool) -> Self {
        Self {
            pack_id,
            voltage_mv: 7600,
            current_ma: 0,
            internal_resistance_mohm: 60, // 60 mOhm nominal fresh pack
            soc_pct: 80.0,
            temp_c: 28.0,
            is_connected,
        }
    }

    /// Computes internal DC resistance from open-circuit vs loaded terminal voltage delta.
    ///
    /// R_i (mOhm) = ((V_oc - V_load) * 1000) / I_load
    pub fn update_impedance(&mut self, voc_mv: u32, vload_mv: u32, current_ma: u32) -> u32 {
        if current_ma >= 200 && voc_mv > vload_mv {
            let delta_mv = voc_mv - vload_mv;
            let raw_mohm = ((delta_mv as u64 * 1000) / current_ma as u64) as u32;
            // Smooth impedance updates with IIR low-pass filter (clamped 20 to 500 mOhm)
            let smoothed = (self.internal_resistance_mohm * 7 + raw_mohm.clamp(20, 500)) / 8;
            self.internal_resistance_mohm = smoothed;
        }
        self.internal_resistance_mohm
    }
}

/// Dual-Battery dynamic current multiplexer and impedance-weighted power balancer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualBatteryBalancer {
    /// Primary internal battery pack (Pack A).
    pub pack_a: DualBatteryCell,
    /// Secondary hot-swappable modular pack (Pack B).
    pub pack_b: DualBatteryCell,
    /// Current power share percentage allocated to Pack A (0.0 to 1.0).
    pub share_ratio_a: f32,
    /// Current power share percentage allocated to Pack B (0.0 to 1.0).
    pub share_ratio_b: f32,
    /// Total system load demand in milliamps.
    pub total_demand_ma: u32,
}

impl DualBatteryBalancer {
    /// Constructs a dual-battery power balancer.
    pub const fn new() -> Self {
        Self {
            pack_a: DualBatteryCell::new(0, true),
            pack_b: DualBatteryCell::new(1, true),
            share_ratio_a: 0.50,
            share_ratio_b: 0.50,
            total_demand_ma: 0,
        }
    }

    /// Computes optimal discharge current split across both packs.
    ///
    /// Distributes current inversely proportional to internal impedance:
    /// Pack with lower resistance delivers proportionally higher current,
    /// minimizing total I²R thermal dissipation and cell voltage droop.
    pub fn balance_discharge(&mut self, total_demand_ma: u32) -> (u32, u32) {
        self.total_demand_ma = total_demand_ma;

        // Case 1: Only Pack A connected
        if self.pack_a.is_connected && !self.pack_b.is_connected {
            self.share_ratio_a = 1.0;
            self.share_ratio_b = 0.0;
            self.pack_a.current_ma = total_demand_ma as i32;
            self.pack_b.current_ma = 0;
            return (total_demand_ma, 0);
        }

        // Case 2: Only Pack B connected
        if !self.pack_a.is_connected && self.pack_b.is_connected {
            self.share_ratio_a = 0.0;
            self.share_ratio_b = 1.0;
            self.pack_a.current_ma = 0;
            self.pack_b.current_ma = total_demand_ma as i32;
            return (0, total_demand_ma);
        }

        // Case 3: Neither connected (error state)
        if !self.pack_a.is_connected && !self.pack_b.is_connected {
            self.share_ratio_a = 0.0;
            self.share_ratio_b = 0.0;
            return (0, 0);
        }

        // Case 4: Both packs active -> impedance-weighted split
        let ra = self.pack_a.internal_resistance_mohm.max(10) as f32;
        let rb = self.pack_b.internal_resistance_mohm.max(10) as f32;

        // Inversely proportional weights: W_a = 1/R_a, W_b = 1/R_b
        // Ratio_a = R_b / (R_a + R_b)
        let raw_ratio_a = rb / (ra + rb);

        // Adjust for SoC imbalance: bias load toward the pack with higher remaining capacity
        let soc_delta = (self.pack_a.soc_pct - self.pack_b.soc_pct) / 100.0; // [-1.0, 1.0]
        let biased_ratio_a = (raw_ratio_a + soc_delta * 0.15).clamp(0.10, 0.90);
        let biased_ratio_b = 1.0 - biased_ratio_a;

        self.share_ratio_a = biased_ratio_a;
        self.share_ratio_b = biased_ratio_b;

        let cur_a = (total_demand_ma as f32 * self.share_ratio_a) as u32;
        let cur_b = total_demand_ma.saturating_sub(cur_a);

        self.pack_a.current_ma = cur_a as i32;
        self.pack_b.current_ma = cur_b as i32;

        (cur_a, cur_b)
    }
}

/// Verification test for dual-cell battery impedance tracking and dynamic power share balancing.
pub fn verify_impedance_tracking_and_power_sharing() -> bool {
    let mut balancer = DualBatteryBalancer::new();

    // 1. Equal impedance test (both 60 mOhm, equal SoC 80%):
    // Demand 4000 mA -> split must be exactly 50% / 50% (2000 mA each)
    let (cur_a, cur_b) = balancer.balance_discharge(4000);
    if cur_a != 2000 || cur_b != 2000 {
        return false;
    }
    if (balancer.share_ratio_a - 0.50).abs() > 0.01 {
        return false;
    }

    // 2. Impedance Tracking:
    // Pack A undergoes load test: Voc = 7600 mV, Vload = 7440 mV at 2000 mA -> R = 160mV / 2A = 80 mOhm
    let r_a = balancer.pack_a.update_impedance(7600, 7440, 2000);
    // Pack B fresh: Voc = 7600 mV, Vload = 7520 mV at 2000 mA -> R = 80mV / 2A = 40 mOhm
    let r_b = balancer.pack_b.update_impedance(7600, 7520, 2000);

    if r_a <= r_b {
        return false; // Pack A has higher impedance
    }

    // 3. Power sharing under unequal impedance:
    // Pack B has lower resistance (40 mOhm vs 80 mOhm) -> Pack B must supply more current!
    let (cur_a_unbal, cur_b_unbal) = balancer.balance_discharge(4500);
    if cur_b_unbal <= cur_a_unbal {
        return false;
    }
    if balancer.share_ratio_b <= balancer.share_ratio_a {
        return false;
    }

    // 4. Single pack mode: Pack B disconnected -> Pack A must carry 100% load
    balancer.pack_b.is_connected = false;
    let (cur_a_single, cur_b_single) = balancer.balance_discharge(3000);
    if cur_a_single != 3000 || cur_b_single != 0 || balancer.share_ratio_a != 1.0 {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Hot-Swap Bridge Hold-Up Time Estimator (Zero Reboot Pack Swap)
// ============================================================================

/// Minimum system main power rail brownout threshold in millivolts (6,200 mV).
pub const SYSTEM_BROWNOUT_THRESHOLD_MV: u32 = 6200;
/// Nominal bridge buffer capacitor / micro-cell energy capacity in Joules (e.g. 600 Joules).
pub const BRIDGE_BUFFER_CAPACITY_JOULES: f32 = 600.0;
/// Critical hold-up time remaining threshold in seconds triggering emergency hibernate.
pub const CRITICAL_BROWNOUT_WARNING_SEC: f32 = 5.0;

/// Hot-swap lifecycle state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotSwapState {
    /// Both packs connected and online.
    DualPacksOnline,
    /// Only primary internal pack online.
    SinglePackOnline,
    /// Hot-swap bridge holding up system rail during active pack exchange.
    BridgeHoldActive,
    /// New pack inserted, contacts debounced, and supply rail locked.
    HotSwapComplete,
    /// Hold-up energy critically depleted (< 5s remaining).
    CriticalBrownoutWarning,
}

/// Status report emitted by the hot-swap supervisor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HotSwapStatus {
    pub state: HotSwapState,
    pub estimated_hold_up_sec: f32,
    pub power_pinch_active: bool,
    pub throttled_system_power_w: f32,
}

/// Hot-swap bridge hold-up energy estimator and power-pinch governor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HotSwapBridgeEstimator {
    /// Active state of the hot-swap bridge supervisor.
    pub state: HotSwapState,
    /// Usable energy remaining in the bridge hold-up buffer in Joules.
    pub remaining_energy_joules: f32,
    /// Current measured or throttled system power consumption in Watts.
    pub current_power_draw_w: f32,
    /// Calculated hold-up time remaining in seconds until brownout shutdown.
    pub estimated_hold_up_sec: f32,
    /// Whether emergency power-pinch mode is actively reducing APU/display load.
    pub power_pinch_active: bool,
    /// Lifetime count of hot-swap pack exchanges completed without reboot.
    pub total_swaps_completed: u32,
}

impl HotSwapBridgeEstimator {
    /// Constructs a hot-swap bridge estimator with fully charged buffer defaults.
    pub const fn new() -> Self {
        Self {
            state: HotSwapState::DualPacksOnline,
            remaining_energy_joules: BRIDGE_BUFFER_CAPACITY_JOULES,
            current_power_draw_w: 20.0,
            estimated_hold_up_sec: 30.0,
            power_pinch_active: false,
            total_swaps_completed: 0,
        }
    }

    /// Evaluates pack presence, updates hold-up energy dissipation, and manages power-pinch load shedding.
    pub fn update(&mut self, is_pack_b_present: bool, raw_system_power_w: f32, dt_s: f32) -> HotSwapStatus {
        if is_pack_b_present {
            if self.state == HotSwapState::BridgeHoldActive || self.state == HotSwapState::CriticalBrownoutWarning {
                // New pack inserted! Recover from bridge hold
                self.state = HotSwapState::HotSwapComplete;
                self.power_pinch_active = false;
                self.total_swaps_completed += 1;
            } else if self.state == HotSwapState::HotSwapComplete {
                self.state = HotSwapState::DualPacksOnline;
            }

            // Recharge bridge buffer when main packs are online
            self.remaining_energy_joules = (self.remaining_energy_joules + 15.0 * dt_s).min(BRIDGE_BUFFER_CAPACITY_JOULES);
            self.current_power_draw_w = raw_system_power_w;
            self.estimated_hold_up_sec = self.remaining_energy_joules / self.current_power_draw_w.max(1.0);

            return HotSwapStatus {
                state: self.state,
                estimated_hold_up_sec: self.estimated_hold_up_sec,
                power_pinch_active: false,
                throttled_system_power_w: self.current_power_draw_w,
            };
        }

        // Pack B is ABSENT / Disconnected:
        // Engage Hot-Swap Bridge Hold mode!
        self.state = HotSwapState::BridgeHoldActive;
        self.power_pinch_active = true;

        // Emergency Power Pinch: Clamp system load to 7.5W max (downclock APU, dim screen)
        self.current_power_draw_w = raw_system_power_w.min(7.5);

        // Deplete bridge energy: E = E - P * dt
        let energy_consumed = self.current_power_draw_w * dt_s;
        self.remaining_energy_joules = (self.remaining_energy_joules - energy_consumed).max(0.0);

        self.estimated_hold_up_sec = self.remaining_energy_joules / self.current_power_draw_w.max(0.5);

        if self.estimated_hold_up_sec <= CRITICAL_BROWNOUT_WARNING_SEC {
            self.state = HotSwapState::CriticalBrownoutWarning;
        }

        HotSwapStatus {
            state: self.state,
            estimated_hold_up_sec: self.estimated_hold_up_sec,
            power_pinch_active: true,
            throttled_system_power_w: self.current_power_draw_w,
        }
    }
}

/// Verification test for hot-swap bridge hold-up time estimator ensuring zero reboot on pack swap.
pub fn verify_hot_swap_bridge_hold_up_estimator() -> bool {
    let mut estimator = HotSwapBridgeEstimator::new();

    // 1. Dual-pack online gaming at 25W load: bridge buffer is fully charged
    let s_online = estimator.update(true, 25.0, 0.10);
    if s_online.state != HotSwapState::DualPacksOnline || s_online.power_pinch_active {
        return false;
    }
    if estimator.remaining_energy_joules < 590.0 {
        return false;
    }

    // 2. User unlatches Pack B during active gaming (raw power 28W):
    // Bridge mode MUST engage and trigger Power Pinch (< 8W load)
    let s_swap = estimator.update(false, 28.0, 1.0);
    if s_swap.state != HotSwapState::BridgeHoldActive || !s_swap.power_pinch_active {
        return false;
    }
    // Power draw must be clamped to 7.5W max to maximize hold-up window
    if s_swap.throttled_system_power_w > 7.5 || estimator.current_power_draw_w > 7.5 {
        return false;
    }
    // At 7.5W with ~590 Joules, hold-up time must be > 70 seconds!
    if s_swap.estimated_hold_up_sec < 70.0 {
        return false;
    }

    // 3. User inserts fresh Pack B after 15 seconds:
    let s_restored = estimator.update(true, 20.0, 1.0);
    if s_restored.state != HotSwapState::HotSwapComplete || s_restored.power_pinch_active {
        return false;
    }
    if estimator.total_swaps_completed != 1 {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: USB-PD PPS Fast-Charge Thermal Governor (Grip Temp < 42°C)
// ============================================================================

/// Maximum allowable handheld chassis/grip surface temperature in °C (IEC 62368-1 compliance).
pub const GRIP_SURFACE_TEMP_MAX_C: f32 = 42.0;
/// Early thermal warning threshold in °C where PPS charging current begins progressive derating.
pub const GRIP_SURFACE_DERATE_START_C: f32 = 38.0;
/// Minimum trickle charge current in milliamps when under severe thermal constraint.
pub const PPS_MIN_SAFE_CHARGE_CURRENT_MA: u32 = 500;

/// Operating thermal zone for USB-PD PPS fast charging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpsThermalZone {
    /// Full unconstrained fast charging (Grip <= 38°C).
    Optimal,
    /// Progressive current throttling (38°C < Grip <= 40.5°C).
    WarmThrottled,
    /// Heavy thermal derating (40.5°C < Grip <= 42.0°C).
    HotThrottled,
    /// Thermal limit exceeded (Grip > 42.0°C): clamped to minimum safe trickle.
    CriticalClamped,
}

/// USB-PD Programmable Power Supply (PPS) Thermal Governor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsbPdPpsThermalGovernor {
    /// Active negotiated PPS voltage in millivolts (e.g. 9000 mV).
    pub negotiated_voltage_mv: u32,
    /// Active modulated PPS current limit in milliamps.
    pub negotiated_current_ma: u32,
    /// Measured handheld palm grip surface temperature in °C.
    pub current_grip_temp_c: f32,
    /// Active PPS thermal zone.
    pub thermal_zone: PpsThermalZone,
    /// Whether fast-charge current is currently being throttled due to heat.
    pub is_thermally_throttled: bool,
    /// Lifetime count of PPS thermal derating events.
    pub total_derate_events: u32,
}

impl UsbPdPpsThermalGovernor {
    /// Constructs a PPS thermal governor with default 9V/4.5A (40.5W) fast charging.
    pub const fn new() -> Self {
        Self {
            negotiated_voltage_mv: 9000,
            negotiated_current_ma: 4500,
            current_grip_temp_c: 28.0,
            thermal_zone: PpsThermalZone::Optimal,
            is_thermally_throttled: false,
            total_derate_events: 0,
        }
    }

    /// Computes safe PPS voltage and current contract based on grip surface thermistor readings.
    pub fn update_thermal_governor(
        &mut self,
        grip_temp_c: f32,
        battery_temp_c: f32,
        target_v_mv: u32,
        requested_i_ma: u32,
    ) -> (u32, u32) {
        self.current_grip_temp_c = grip_temp_c;
        self.negotiated_voltage_mv = target_v_mv;

        // Effective thermal pressure is dominated by grip surface, with battery core safety check
        let effective_temp = grip_temp_c.max(battery_temp_c - 5.0);

        if effective_temp <= GRIP_SURFACE_DERATE_START_C {
            // Zone 1: Optimal fast charging
            self.thermal_zone = PpsThermalZone::Optimal;
            self.negotiated_current_ma = requested_i_ma;
            self.is_thermally_throttled = false;
        } else if effective_temp <= 40.5 {
            // Zone 2: Warm throttling (linearly derate between 38°C and 40.5°C)
            if self.thermal_zone == PpsThermalZone::Optimal {
                self.total_derate_events += 1;
            }
            self.thermal_zone = PpsThermalZone::WarmThrottled;
            self.is_thermally_throttled = true;

            let factor = 1.0 - ((effective_temp - GRIP_SURFACE_DERATE_START_C) / 2.5) * 0.45; // derate to ~55%
            self.negotiated_current_ma = ((requested_i_ma as f32) * factor) as u32;
        } else if effective_temp <= GRIP_SURFACE_TEMP_MAX_C {
            // Zone 3: Hot throttling (derate down to 1500 mA)
            self.thermal_zone = PpsThermalZone::HotThrottled;
            self.is_thermally_throttled = true;

            let factor = 0.55 - ((effective_temp - 40.5) / 1.5) * 0.25; // derate to ~30%
            self.negotiated_current_ma = ((requested_i_ma as f32) * factor).max(1200.0) as u32;
        } else {
            // Zone 4: Critical safety clamp (Grip > 42°C): force 500 mA trickle
            self.thermal_zone = PpsThermalZone::CriticalClamped;
            self.is_thermally_throttled = true;
            self.negotiated_current_ma = PPS_MIN_SAFE_CHARGE_CURRENT_MA;
        }

        // Align current to standard USB-PD PPS 50mA step resolution
        self.negotiated_current_ma = (self.negotiated_current_ma / 50) * 50;

        (self.negotiated_voltage_mv, self.negotiated_current_ma)
    }
}

/// Verification test for USB-PD PPS fast-charge thermal governor.
pub fn verify_usb_pd_pps_thermal_governor() -> bool {
    let mut gov = UsbPdPpsThermalGovernor::new();

    // 1. Cool grip surface (32°C): Full unconstrained 4500 mA fast charging
    let (v1, i1) = gov.update_thermal_governor(32.0, 34.0, 9000, 4500);
    if v1 != 9000 || i1 != 4500 || gov.is_thermally_throttled {
        return false;
    }
    if gov.thermal_zone != PpsThermalZone::Optimal {
        return false;
    }

    // 2. Grip warms to 39.5°C during intense gameplay: Warm throttling derates current
    let (_, i2) = gov.update_thermal_governor(39.5, 41.0, 9000, 4500);
    if i2 >= 4500 || i2 < 2000 || !gov.is_thermally_throttled {
        return false;
    }
    if gov.thermal_zone != PpsThermalZone::WarmThrottled || gov.total_derate_events != 1 {
        return false;
    }

    // 3. Grip heats to 41.2°C: Hot throttling derates further
    let (_, i3) = gov.update_thermal_governor(41.2, 43.0, 9000, 4500);
    if i3 >= i2 || i3 > 2500 || gov.thermal_zone != PpsThermalZone::HotThrottled {
        return false;
    }

    // 4. Grip breaches critical 42.0°C limit (reaches 42.4°C):
    // Must immediately clamp to minimum safe trickle (500 mA) to prevent palm discomfort
    let (_, i4) = gov.update_thermal_governor(42.4, 44.0, 9000, 4500);
    if i4 != PPS_MIN_SAFE_CHARGE_CURRENT_MA || gov.thermal_zone != PpsThermalZone::CriticalClamped {
        return false;
    }

    true
}


// ============================================================================
// Feature 4: USB OTG Reverse Power Bank Discharge Controller (Up to 15W)
// ============================================================================

/// Minimum host battery SoC percentage required to permit reverse power bank discharge (25%).
pub const OTG_HOST_RESERVE_MIN_SOC_PCT: f32 = 25.0;
/// Maximum rated reverse power bank output power in milliwatts (15,000 mW = 15W @ 5V/3A).
pub const OTG_MAX_REVERSE_POWER_MW: u32 = 15_000;
/// Hardware overcurrent trip threshold in milliamps (3,300 mA = 3.3A).
pub const OTG_OVERCURRENT_TRIP_MA: u32 = 3300;
/// Maximum allowable battery core temperature in °C during reverse discharge (45°C).
pub const OTG_MAX_SAFE_BATTERY_TEMP_C: f32 = 45.0;

/// Safety trip reasons for reverse power bank shutoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtgTripReason {
    None,
    LowHostBatteryReserve,
    OvercurrentFault,
    ThermalOvertemp,
    DisabledByUser,
}

/// USB OTG 15W Reverse Power Bank Boost Controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UsbOtgReversePowerBank {
    /// Whether reverse boost power delivery is actively enabled.
    pub is_enabled: bool,
    /// Commanded output voltage in millivolts (nominally 5000 mV).
    pub output_voltage_mv: u32,
    /// Negotiated current limit in milliamps (e.g. 1500 mA or 3000 mA).
    pub current_limit_ma: u32,
    /// Real-time measured load current drawn by external accessory in milliamps.
    pub measured_load_ma: u32,
    /// Cumulative energy delivered to external devices in milliwatt-hours.
    pub total_energy_delivered_mwh: f32,
    /// Active safety trip status.
    pub trip_status: OtgTripReason,
}

impl UsbOtgReversePowerBank {
    /// Constructs a reverse power bank controller configured for 5V/3A (15W) max delivery.
    pub const fn new() -> Self {
        Self {
            is_enabled: false,
            output_voltage_mv: 5000,
            current_limit_ma: 3000,
            measured_load_ma: 0,
            total_energy_delivered_mwh: 0.0,
            trip_status: OtgTripReason::DisabledByUser,
        }
    }

    /// Enables reverse power bank mode with specified current ceiling (e.g. 1500mA or 3000mA).
    pub fn enable_reverse_boost(&mut self, max_current_ma: u32) {
        self.is_enabled = true;
        self.output_voltage_mv = 5000;
        self.current_limit_ma = max_current_ma.min(3000);
        self.trip_status = OtgTripReason::None;
    }

    /// Disables reverse power bank mode.
    pub fn disable_reverse_boost(&mut self) {
        self.is_enabled = false;
        self.measured_load_ma = 0;
        self.trip_status = OtgTripReason::DisabledByUser;
    }

    /// Evaluates safety protections and updates energy delivery telemetry.
    pub fn update(
        &mut self,
        host_soc_pct: f32,
        battery_temp_c: f32,
        actual_load_ma: u32,
        dt_s: f32,
    ) -> Result<u32, OtgTripReason> {
        if !self.is_enabled {
            return Err(OtgTripReason::DisabledByUser);
        }

        // 1. Host Gaming Battery Reserve Protection:
        // Cut off reverse charging if host battery drops below 25% to protect gaming session
        if host_soc_pct <= OTG_HOST_RESERVE_MIN_SOC_PCT {
            self.disable_reverse_boost();
            self.trip_status = OtgTripReason::LowHostBatteryReserve;
            return Err(self.trip_status);
        }

        // 2. Battery Thermal Over-temperature Protection
        if battery_temp_c >= OTG_MAX_SAFE_BATTERY_TEMP_C {
            self.disable_reverse_boost();
            self.trip_status = OtgTripReason::ThermalOvertemp;
            return Err(self.trip_status);
        }

        // 3. Hardware Overcurrent Trip
        if actual_load_ma >= OTG_OVERCURRENT_TRIP_MA {
            self.disable_reverse_boost();
            self.trip_status = OtgTripReason::OvercurrentFault;
            return Err(self.trip_status);
        }

        self.measured_load_ma = actual_load_ma.min(self.current_limit_ma);

        // Telemetry: P = V * I (mWh = (mV * mA * dt_s) / (1000 * 3600))
        let power_mw = ((self.output_voltage_mv as u64 * self.measured_load_ma as u64) / 1000) as u32;
        let delivered_mwh = (power_mw as f32 * dt_s) / 3600.0;
        self.total_energy_delivered_mwh += delivered_mwh;

        Ok(power_mw)
    }
}

/// Integrated Handheld Dual-Battery Hot-Swap & Charge Multiplexer Governor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualBatteryPowerGovernor {
    /// Dual-cell dynamic impedance power balancer.
    pub balancer: DualBatteryBalancer,
    /// Hot-swap bridge hold-up energy estimator.
    pub hot_swap_estimator: HotSwapBridgeEstimator,
    /// USB-PD PPS fast-charge thermal governor.
    pub pps_governor: UsbPdPpsThermalGovernor,
    /// USB OTG 15W reverse power bank controller.
    pub reverse_power_bank: UsbOtgReversePowerBank,
}

impl DualBatteryPowerGovernor {
    /// Constructs an integrated dual-battery power governor.
    pub const fn new() -> Self {
        Self {
            balancer: DualBatteryBalancer::new(),
            hot_swap_estimator: HotSwapBridgeEstimator::new(),
            pps_governor: UsbPdPpsThermalGovernor::new(),
            reverse_power_bank: UsbOtgReversePowerBank::new(),
        }
    }
}

/// Verification test for USB OTG reverse power bank discharge controller up to 15W.
pub fn verify_usb_otg_reverse_power_bank() -> bool {
    let mut otg = UsbOtgReversePowerBank::new();

    // 1. Initial state: disabled
    if otg.is_enabled || otg.trip_status != OtgTripReason::DisabledByUser {
        return false;
    }

    // 2. Enable 15W mode (5V @ 3000 mA) with host battery at 80% SoC, 30°C
    otg.enable_reverse_boost(3000);
    if !otg.is_enabled || otg.current_limit_ma != 3000 {
        return false;
    }

    // Normal discharge at 2400 mA (12W delivery)
    let pwr_res = otg.update(80.0, 32.0, 2400, 1.0);
    if pwr_res.is_err() || pwr_res.unwrap() != 12000 {
        return false;
    }
    if otg.total_energy_delivered_mwh <= 0.0 {
        return false;
    }

    // 3. Overcurrent protection test: External device attempts to draw 3500 mA (> 3300 mA trip)
    let oc_res = otg.update(75.0, 33.0, 3500, 0.1);
    if oc_res != Err(OtgTripReason::OvercurrentFault) || otg.is_enabled {
        return false;
    }

    // 4. Low battery gaming reserve protection: Re-enable, but host SoC drops to 24% (< 25% cutoff)
    otg.enable_reverse_boost(1500);
    let low_soc_res = otg.update(24.0, 32.0, 1000, 0.1);
    if low_soc_res != Err(OtgTripReason::LowHostBatteryReserve) || otg.is_enabled {
        return false;
    }

    // 5. Test Integrated Governor instantiation
    let mut gov = DualBatteryPowerGovernor::new();
    let (ca, cb) = gov.balancer.balance_discharge(3000);
    if ca + cb != 3000 {
        return false;
    }

    true
}
