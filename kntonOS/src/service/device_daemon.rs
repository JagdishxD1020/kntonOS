//! Handheld Device Hardware Daemon & Input Event Dispatcher for kntonOS.
//!
//! Provides the primary userland hardware supervisor service responsible for:
//! 1. Real-time battery fuel gauge sampling and low-power telemetry monitoring.
//! 2. USB-PD charging contract state machine and power profile negotiation.
//! 3. USB-C DisplayPort Alt Mode display hot-plug event dispatching and modesetting.
//! 4. Audio volume mixer channel management with acoustic dynamic range limiting.
//! 5. Handheld gamepad controller event queue and radial deadzone filter.

#![allow(dead_code)]

// ============================================================================
// Feature 1: Real-Time Battery Fuel Gauge Sampling & Telemetry Monitor
// ============================================================================

/// Operational charging and presence state of the battery pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryStatus {
    /// Battery is discharging under internal load.
    Discharging,
    /// Battery is actively charging from external USB-PD source.
    Charging,
    /// Battery pack is at 100% capacity with trickle charge or bypass.
    Full,
    /// Battery pack disconnected or communication failure.
    NotPresent,
}

/// Real-time battery fuel gauge telemetry and health monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryFuelGauge {
    /// Pack terminal voltage in millivolts (e.g. 7400 mV to 8400 mV for 2S Li-ion).
    pub voltage_mv: u32,
    /// Instantaneous pack current in milliamperes (negative = discharge, positive = charge).
    pub current_ma: i32,
    /// State of Charge (SoC) percentage (0 to 100%).
    pub state_of_charge_pct: u8,
    /// Battery cell temperature in deci-degrees Celsius (e.g. 325 = 32.5°C).
    pub temperature_deci_c: i16,
    /// State of Health (SoH) percentage based on impedance degradation.
    pub state_of_health_pct: u8,
    /// Total charge/discharge cycle count.
    pub cycle_count: u16,
    /// Current battery charging / discharging status.
    pub status: BatteryStatus,
    /// Flag indicating low battery notification threshold has been reached.
    pub low_battery_warned: bool,
    /// Flag indicating critical battery emergency shutdown threshold reached.
    pub critical_battery_triggered: bool,
}

impl BatteryFuelGauge {
    /// Low battery warning threshold (15%).
    pub const LOW_BATTERY_THRESHOLD_PCT: u8 = 15;
    /// Critical emergency hibernation / shutdown threshold (5%).
    pub const CRITICAL_BATTERY_THRESHOLD_PCT: u8 = 5;
    /// Maximum safe battery cell temperature before thermal throttling (50.0°C).
    pub const MAX_SAFE_TEMP_DECI_C: i16 = 500;

    /// Initializes a default healthy battery fuel gauge.
    pub const fn new() -> Self {
        Self {
            voltage_mv: 7600, // 7.6V nominal 2S
            current_ma: -1500, // -1.5A initial discharge
            state_of_charge_pct: 85,
            temperature_deci_c: 280, // 28.0°C
            state_of_health_pct: 98,
            cycle_count: 12,
            status: BatteryStatus::Discharging,
            low_battery_warned: false,
            critical_battery_triggered: false,
        }
    }

    /// Updates fuel gauge readings from hardware ADC / SMBus fuel gauge register reads.
    pub fn update_telemetry(
        &mut self,
        voltage_mv: u32,
        current_ma: i32,
        state_of_charge_pct: u8,
        temp_deci_c: i16,
    ) {
        self.voltage_mv = voltage_mv;
        self.current_ma = current_ma;
        self.state_of_charge_pct = state_of_charge_pct.min(100);
        self.temperature_deci_c = temp_deci_c;

        // Determine status
        if current_ma > 50 {
            if self.state_of_charge_pct >= 100 {
                self.status = BatteryStatus::Full;
            } else {
                self.status = BatteryStatus::Charging;
            }
        } else if current_ma < -50 {
            self.status = BatteryStatus::Discharging;
        } else if self.state_of_charge_pct >= 99 {
            self.status = BatteryStatus::Full;
        } else {
            self.status = BatteryStatus::Discharging;
        }

        // Evaluate warning thresholds
        self.low_battery_warned = self.state_of_charge_pct <= Self::LOW_BATTERY_THRESHOLD_PCT;
        self.critical_battery_triggered = self.state_of_charge_pct <= Self::CRITICAL_BATTERY_THRESHOLD_PCT;
    }

    /// Checks if battery has breached low-power warning boundary.
    pub fn is_low_battery(&self) -> bool {
        self.low_battery_warned
    }

    /// Checks if battery has breached emergency critical boundary.
    pub fn is_critical_battery(&self) -> bool {
        self.critical_battery_triggered
    }

    /// Checks if cell temperature exceeds safe thermal limits.
    pub fn is_overheating(&self) -> bool {
        self.temperature_deci_c >= Self::MAX_SAFE_TEMP_DECI_C
    }

    /// Calculates estimated remaining battery life in minutes based on current draw.
    pub fn estimate_remaining_minutes(&self, nominal_capacity_mah: u32) -> u32 {
        if self.current_ma >= 0 {
            // Charging or Full
            return 0;
        }

        let discharge_ma = (-self.current_ma) as u32;
        if discharge_ma == 0 {
            return 999;
        }

        let remaining_mah = (nominal_capacity_mah * (self.state_of_charge_pct as u32)) / 100;
        (remaining_mah * 60) / discharge_ma
    }
}

/// Verification test for battery fuel gauge sampling and telemetry monitor.
pub fn verify_battery_fuel_gauge() -> bool {
    let mut gauge = BatteryFuelGauge::new();

    // 1. Initial healthy state check
    if gauge.state_of_charge_pct != 85 || gauge.status != BatteryStatus::Discharging {
        return false;
    }
    if gauge.is_low_battery() || gauge.is_critical_battery() || gauge.is_overheating() {
        return false;
    }

    // 2. Simulate discharging to 12% (triggers low battery warning)
    gauge.update_telemetry(7200, -2200, 12, 340);
    if !gauge.is_low_battery() || gauge.is_critical_battery() {
        return false;
    }
    if gauge.status != BatteryStatus::Discharging {
        return false;
    }

    // 3. Simulate discharging to 4% (triggers critical emergency)
    gauge.update_telemetry(6800, -1800, 4, 360);
    if !gauge.is_critical_battery() || !gauge.is_low_battery() {
        return false;
    }

    // 4. Simulate plugging in 45W charger (+3.5A charging current, SoC rising to 50%)
    gauge.update_telemetry(8000, 3500, 50, 310);
    if gauge.status != BatteryStatus::Charging {
        return false;
    }
    if gauge.is_low_battery() || gauge.is_critical_battery() {
        return false;
    }

    // 5. Test remaining minutes estimation: 5000 mAh pack, 50% SoC = 2500 mAh, discharge at 2500 mA = 60 mins
    gauge.update_telemetry(7400, -2500, 50, 300);
    let est = gauge.estimate_remaining_minutes(5000);
    if est != 60 {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: USB-PD Charging Contract State Machine & Power Profile Negotiator
// ============================================================================

/// Power Delivery contract profiles negotiated over USB-C CC lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbPdContract {
    /// No active PD contract; USB-C port disconnected.
    Disconnected,
    /// 5V @ 3A (15W baseline USB-C).
    Fix5V3A,
    /// 9V @ 3A (27W fast phone charger).
    Fix9V3A,
    /// 15V @ 3A (45W ultrabook charger).
    Fix15V3A,
    /// 20V @ 3.25A (65W standard handheld gaming dock).
    Fix20V3_25A,
    /// 20V @ 5A (100W high-power dock).
    Fix20V5A,
    /// 28V @ 5A (140W USB-PD 3.1 Extended Power Range EPR).
    Epr28V5A,
}

impl UsbPdContract {
    /// Returns maximum power in Watts delivered by this contract.
    pub const fn max_watts(&self) -> u16 {
        match self {
            Self::Disconnected => 0,
            Self::Fix5V3A => 15,
            Self::Fix9V3A => 27,
            Self::Fix15V3A => 45,
            Self::Fix20V3_25A => 65,
            Self::Fix20V5A => 100,
            Self::Epr28V5A => 140,
        }
    }

    /// Returns nominal voltage in millivolts for this contract.
    pub const fn voltage_mv(&self) -> u32 {
        match self {
            Self::Disconnected => 0,
            Self::Fix5V3A => 5000,
            Self::Fix9V3A => 9000,
            Self::Fix15V3A => 15000,
            Self::Fix20V3_25A | Self::Fix20V5A => 20000,
            Self::Epr28V5A => 28000,
        }
    }

    /// Returns maximum current limit in milliamperes for this contract.
    pub const fn current_ma(&self) -> u32 {
        match self {
            Self::Disconnected => 0,
            Self::Fix5V3A | Self::Fix9V3A | Self::Fix15V3A => 3000,
            Self::Fix20V3_25A => 3250,
            Self::Fix20V5A | Self::Epr28V5A => 5000,
        }
    }
}

/// Operational state of the USB-PD policy engine state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbPdState {
    /// No USB-C charger attached.
    Unattached,
    /// Power source detected on CC lines, awaiting capabilities broadcast.
    SourceAttached,
    /// Protocol handshake in progress.
    Negotiating,
    /// High-power PD contract actively negotiated and stable.
    ContractActive,
    /// Down-negotiated to lower power profile due to handheld thermal throttling.
    ThermalThrottled,
    /// Fault condition (over-voltage, over-current, reverse current).
    Fault,
}

/// USB-PD power profile negotiator and charging manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbPdNegotiator {
    /// Currently active PD contract.
    pub current_contract: UsbPdContract,
    /// Current state of the PD state machine.
    pub current_state: UsbPdState,
    /// Negotiated voltage in millivolts.
    pub negotiated_voltage_mv: u32,
    /// Negotiated current in milliamperes.
    pub negotiated_current_ma: u32,
    /// Negotiated maximum power in Watts.
    pub negotiated_max_watts: u16,
    /// Whether connected source supports USB-PD 3.1 Extended Power Range (EPR).
    pub is_epr_supported: bool,
    /// Flag indicating pass-through bypass charging is active (protecting battery).
    pub bypass_charging_engaged: bool,
}

impl UsbPdNegotiator {
    /// Initializes negotiator in unattached disconnected state.
    pub const fn new() -> Self {
        Self {
            current_contract: UsbPdContract::Disconnected,
            current_state: UsbPdState::Unattached,
            negotiated_voltage_mv: 0,
            negotiated_current_ma: 0,
            negotiated_max_watts: 0,
            is_epr_supported: false,
            bypass_charging_engaged: false,
        }
    }

    /// Handles USB-C cable insertion event.
    pub fn on_cable_attached(&mut self) {
        self.current_state = UsbPdState::SourceAttached;
    }

    /// Handles USB-C cable removal event.
    pub fn on_cable_detached(&mut self) {
        self.current_contract = UsbPdContract::Disconnected;
        self.current_state = UsbPdState::Unattached;
        self.negotiated_voltage_mv = 0;
        self.negotiated_current_ma = 0;
        self.negotiated_max_watts = 0;
        self.is_epr_supported = false;
        self.bypass_charging_engaged = false;
    }

    /// Selects and negotiates the optimal power profile offered by the attached source.
    pub fn negotiate_contract(&mut self, requested_watts: u16, offered_profiles: &[UsbPdContract]) -> bool {
        self.current_state = UsbPdState::Negotiating;

        // Find the highest offered profile that fits our requested target (or best available)
        let mut best_contract = UsbPdContract::Disconnected;
        for &profile in offered_profiles {
            if profile.max_watts() <= requested_watts && profile.max_watts() > best_contract.max_watts() {
                best_contract = profile;
            }
        }

        // If no profile was strictly <= requested, pick the highest available profile
        if best_contract == UsbPdContract::Disconnected && !offered_profiles.is_empty() {
            best_contract = offered_profiles[0];
            for &profile in offered_profiles {
                if profile.max_watts() > best_contract.max_watts() {
                    best_contract = profile;
                }
            }
        }

        if best_contract == UsbPdContract::Disconnected {
            self.current_state = UsbPdState::Fault;
            return false;
        }

        self.current_contract = best_contract;
        self.negotiated_voltage_mv = best_contract.voltage_mv();
        self.negotiated_current_ma = best_contract.current_ma();
        self.negotiated_max_watts = best_contract.max_watts();
        self.is_epr_supported = best_contract == UsbPdContract::Epr28V5A;
        self.current_state = UsbPdState::ContractActive;

        // Pass-through bypass charging can be engaged if we have >= 45W and battery is healthy
        self.bypass_charging_engaged = self.negotiated_max_watts >= 45;

        true
    }

    /// Down-negotiates power profile during emergency thermal throttling.
    pub fn on_thermal_throttle(&mut self) {
        if self.current_state == UsbPdState::ContractActive {
            // Drop power to 27W or 15W to reduce charging heat dissipation
            if self.current_contract.max_watts() > 27 {
                self.current_contract = UsbPdContract::Fix9V3A;
                self.negotiated_voltage_mv = UsbPdContract::Fix9V3A.voltage_mv();
                self.negotiated_current_ma = UsbPdContract::Fix9V3A.current_ma();
                self.negotiated_max_watts = UsbPdContract::Fix9V3A.max_watts();
                self.current_state = UsbPdState::ThermalThrottled;
                self.bypass_charging_engaged = false;
            }
        }
    }
}

/// Verification test for USB-PD charging contracts and power negotiation.
pub fn verify_usb_pd_charging_contracts() -> bool {
    let mut pd = UsbPdNegotiator::new();

    // 1. Initial state
    if pd.current_state != UsbPdState::Unattached || pd.current_contract != UsbPdContract::Disconnected {
        return false;
    }

    // 2. Attach cable
    pd.on_cable_attached();
    if pd.current_state != UsbPdState::SourceAttached {
        return false;
    }

    // 3. Negotiate 65W target from a 100W dock offering [5V, 9V, 15V, 20V/3.25A, 20V/5A]
    let dock_profiles = [
        UsbPdContract::Fix5V3A,
        UsbPdContract::Fix9V3A,
        UsbPdContract::Fix15V3A,
        UsbPdContract::Fix20V3_25A,
        UsbPdContract::Fix20V5A,
    ];
    let ok = pd.negotiate_contract(65, &dock_profiles);
    if !ok || pd.current_contract != UsbPdContract::Fix20V3_25A {
        return false;
    }
    if pd.negotiated_max_watts != 65 || pd.negotiated_voltage_mv != 20000 || !pd.bypass_charging_engaged {
        return false;
    }
    if pd.current_state != UsbPdState::ContractActive {
        return false;
    }

    // 4. Test thermal throttle downgrade
    pd.on_thermal_throttle();
    if pd.current_state != UsbPdState::ThermalThrottled || pd.negotiated_max_watts != 27 {
        return false;
    }

    // 5. Test detachment
    pd.on_cable_detached();
    if pd.current_state != UsbPdState::Unattached || pd.current_contract != UsbPdContract::Disconnected {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: USB-C DP Alt Mode Display Hot-Plug Detector & Modesetting Events
// ============================================================================

/// Hot-Plug Detect (HPD) pin status for USB-C DisplayPort Alternate Mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayHpdStatus {
    /// No external display connected to USB-C port.
    Disconnected,
    /// Stable external display monitor detected and powered.
    Connected,
    /// Short HPD pulse (0.5ms - 1.0ms) requesting DisplayPort link retraining / IRQ.
    HpdPulseInterrupt,
}

/// Active DisplayPort video timing and link configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayPortMode {
    /// Number of active USB-C differential lanes allocated to DisplayPort (2 or 4).
    pub lanes: u8,
    /// Link rate per lane in megabits per second (e.g. 5400 for HBR2, 8100 for HBR3).
    pub link_rate_mbps: u32,
    /// Horizontal pixel resolution (e.g. 1920, 2560, 3840).
    pub width: u16,
    /// Vertical pixel resolution (e.g. 1080, 1440, 2160).
    pub height: u16,
    /// Vertical refresh rate in Hertz (e.g. 60, 120, 144).
    pub refresh_rate_hz: u16,
    /// High Dynamic Range (HDR10) metadata transmission active.
    pub hdr_enabled: bool,
}

impl DisplayPortMode {
    /// Standard 1080p @ 120Hz gaming dock profile.
    pub const fn fhd_120hz() -> Self {
        Self {
            lanes: 4,
            link_rate_mbps: 5400, // HBR2
            width: 1920,
            height: 1080,
            refresh_rate_hz: 120,
            hdr_enabled: true,
        }
    }

    /// 4K @ 60Hz high-resolution desktop profile.
    pub const fn uhd_60hz() -> Self {
        Self {
            lanes: 4,
            link_rate_mbps: 8100, // HBR3
            width: 3840,
            height: 2160,
            refresh_rate_hz: 60,
            hdr_enabled: true,
        }
    }

    /// Calculates required uncompressed pixel bandwidth in megabits per second.
    pub fn required_bandwidth_mbps(&self) -> u32 {
        let bpp = if self.hdr_enabled { 30 } else { 24 }; // 10-bit vs 8-bit color
        let pixels_per_sec = (self.width as u64) * (self.height as u64) * (self.refresh_rate_hz as u64);
        let bps = pixels_per_sec * bpp;
        // Add 20% blanking overhead
        ((bps * 12) / 10 / 1_000_000) as u32
    }
}

/// USB-C DisplayPort Alt Mode display hot-plug detector and modesetting coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayHotPlugManager {
    /// Physical HPD line status.
    pub hpd_status: DisplayHpdStatus,
    /// Whether an external monitor is actively driving scanout.
    pub is_external_monitor_active: bool,
    /// Currently active video timing mode, if connected.
    pub active_mode: Option<DisplayPortMode>,
    /// Counter tracking connection / disconnection cycles.
    pub hotplug_event_counter: u32,
    /// Counter tracking modesetting changes.
    pub modeset_event_counter: u32,
    /// Counter tracking link retraining pulses.
    pub hpd_pulse_counter: u32,
}

impl DisplayHotPlugManager {
    /// Initializes hot-plug detector in disconnected state.
    pub const fn new() -> Self {
        Self {
            hpd_status: DisplayHpdStatus::Disconnected,
            is_external_monitor_active: false,
            active_mode: None,
            hotplug_event_counter: 0,
            modeset_event_counter: 0,
            hpd_pulse_counter: 0,
        }
    }

    /// Handles display connection event on USB-C port.
    pub fn on_hpd_asserted(&mut self, default_mode: DisplayPortMode) {
        self.hpd_status = DisplayHpdStatus::Connected;
        self.is_external_monitor_active = true;
        self.active_mode = Some(default_mode);
        self.hotplug_event_counter += 1;
        self.modeset_event_counter += 1;
    }

    /// Applies a new display modeset (e.g. user toggles 60Hz to 120Hz in Game Mode UI).
    pub fn apply_modeset(&mut self, new_mode: DisplayPortMode) -> bool {
        if !self.is_external_monitor_active {
            return false;
        }

        // Validate link capacity: lanes * link_rate must accommodate bandwidth
        let total_link_capacity_mbps = (new_mode.lanes as u32) * new_mode.link_rate_mbps;
        if new_mode.required_bandwidth_mbps() > total_link_capacity_mbps {
            return false; // Mode exceeds physical link capacity
        }

        self.active_mode = Some(new_mode);
        self.modeset_event_counter += 1;
        true
    }

    /// Handles HPD short pulse (link retraining / sink IRQ).
    pub fn on_hpd_pulse(&mut self) -> bool {
        if self.is_external_monitor_active {
            self.hpd_pulse_counter += 1;
            true
        } else {
            false
        }
    }

    /// Handles display disconnection event on USB-C port.
    pub fn on_hpd_deasserted(&mut self) {
        self.hpd_status = DisplayHpdStatus::Disconnected;
        self.is_external_monitor_active = false;
        self.active_mode = None;
        self.hotplug_event_counter += 1;
    }
}

/// Verification test for USB-C DP Alt Mode display hot-plug detector and modesetting.
pub fn verify_dp_alt_mode_hotplug() -> bool {
    let mut mgr = DisplayHotPlugManager::new();

    // 1. Initial state
    if mgr.hpd_status != DisplayHpdStatus::Disconnected || mgr.is_external_monitor_active {
        return false;
    }

    // 2. Connect external monitor (docking to 1080p 120Hz display)
    let fhd_mode = DisplayPortMode::fhd_120hz();
    mgr.on_hpd_asserted(fhd_mode);
    if !mgr.is_external_monitor_active || mgr.hpd_status != DisplayHpdStatus::Connected {
        return false;
    }
    if mgr.active_mode != Some(fhd_mode) || mgr.hotplug_event_counter != 1 {
        return false;
    }

    // 3. Switch modeset to 4K 60Hz
    let uhd_mode = DisplayPortMode::uhd_60hz();
    let modeset_ok = mgr.apply_modeset(uhd_mode);
    if !modeset_ok || mgr.active_mode != Some(uhd_mode) || mgr.modeset_event_counter != 2 {
        return false;
    }

    // 4. Test HPD pulse (link retraining)
    let pulse_ok = mgr.on_hpd_pulse();
    if !pulse_ok || mgr.hpd_pulse_counter != 1 {
        return false;
    }

    // 5. Disconnect monitor (undocking handheld)
    mgr.on_hpd_deasserted();
    if mgr.is_external_monitor_active || mgr.active_mode.is_some() || mgr.hotplug_event_counter != 2 {
        return false;
    }

    true
}
