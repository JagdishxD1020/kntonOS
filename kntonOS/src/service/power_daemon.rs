//! Handheld Userspace Power Management & Thermal Profile Daemon (TASK-KNT-321).
//!
//! Provides intelligent battery charging governance, dynamic TDP thermal profiles,
//! ACPI platform fan curve acoustic balancing, and sleep/resume coordination.

#![allow(dead_code)]

// ============================================================================
// Feature 1: Battery Charge Governor, Health Telemetry & Low-Battery Alerts
// ============================================================================

/// Battery charging limit policy to protect cell longevity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeLimitPolicy {
    /// Full charge to 100% capacity (standard handheld gaming).
    Standard,
    /// Stop charging at 80% to maximize cycle life during stationary/docked play.
    BatteryProtect80,
    /// Maintain 50% charge for prolonged device storage.
    Storage50,
}

impl ChargeLimitPolicy {
    /// Returns the maximum allowed State of Charge percentage for this policy.
    pub const fn target_pct(&self) -> u8 {
        match self {
            Self::Standard => 100,
            Self::BatteryProtect80 => 80,
            Self::Storage50 => 50,
        }
    }
}

/// Low battery warning alert level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LowBatteryAlert {
    /// Battery level is sufficient (> 15%).
    Normal,
    /// Battery level has dropped below 15% (notify player / dim HUD).
    LowWarning,
    /// Battery level has dropped below 5% (emergency save / suspend impending).
    CriticalShutdown,
}

/// Intelligent battery charging governor and health telemetry monitor.
#[derive(Debug, Clone, Copy)]
pub struct BatteryChargeGovernor {
    /// Active charging limit policy.
    pub policy: ChargeLimitPolicy,
    /// Current battery State of Charge percentage (0 to 100%).
    pub state_of_charge_pct: u8,
    /// Whether external power adapter is currently providing charging current.
    pub is_charging: bool,
    /// Battery State of Health percentage based on internal impedance.
    pub state_of_health_pct: u8,
    /// Battery pack temperature in deci-degrees Celsius (e.g. 350 = 35.0 C).
    pub temperature_deci_c: i16,
    /// Total lifetime charge/discharge cycles logged.
    pub cycle_count: u32,
    /// Active low battery alert status.
    pub alert_status: LowBatteryAlert,
    /// Bypass pass-through mode enabled (powering board directly without cycling cell).
    pub bypass_mode: bool,
}

impl BatteryChargeGovernor {
    /// Initializes battery charge governor with default balanced settings.
    pub const fn new() -> Self {
        Self {
            policy: ChargeLimitPolicy::Standard,
            state_of_charge_pct: 75,
            is_charging: false,
            state_of_health_pct: 98,
            temperature_deci_c: 320,
            cycle_count: 24,
            alert_status: LowBatteryAlert::Normal,
            bypass_mode: false,
        }
    }

    /// Configures the charging limit policy.
    pub fn set_policy(&mut self, policy: ChargeLimitPolicy) {
        self.policy = policy;
        self.evaluate_charging_state();
    }

    /// Updates real-time telemetry from fuel gauge sensor.
    pub fn update_telemetry(&mut self, soc_pct: u8, is_charging: bool, temp_deci_c: i16) {
        self.state_of_charge_pct = soc_pct.min(100);
        self.is_charging = is_charging;
        self.temperature_deci_c = temp_deci_c;

        // Update alert level
        if self.state_of_charge_pct <= 5 {
            self.alert_status = LowBatteryAlert::CriticalShutdown;
        } else if self.state_of_charge_pct <= 15 {
            self.alert_status = LowBatteryAlert::LowWarning;
        } else {
            self.alert_status = LowBatteryAlert::Normal;
        }

        self.evaluate_charging_state();
    }

    /// Determines if charging should be terminated or bypassed based on policy.
    pub fn should_stop_charging(&self) -> bool {
        self.state_of_charge_pct >= self.policy.target_pct()
    }

    /// Internal evaluation of bypass charging mode.
    fn evaluate_charging_state(&mut self) {
        if self.is_charging && self.should_stop_charging() {
            // Cut charging current to cell and switch to direct AC bypass
            self.bypass_mode = true;
        } else {
            self.bypass_mode = false;
        }
    }
}

/// Verification test for battery charge governor and alert monitoring.
pub fn verify_battery_charge_governor() -> bool {
    let mut gov = BatteryChargeGovernor::new();

    // 1. Initial healthy state check
    if gov.state_of_charge_pct != 75 || gov.alert_status != LowBatteryAlert::Normal || gov.bypass_mode {
        return false;
    }

    // 2. Test BatteryProtect80 policy: at 85% SoC charging on AC, should halt charging & engage bypass
    gov.set_policy(ChargeLimitPolicy::BatteryProtect80);
    gov.update_telemetry(85, true, 330);
    if !gov.should_stop_charging() || !gov.bypass_mode {
        return false;
    }

    // 3. Test Storage50 policy
    gov.set_policy(ChargeLimitPolicy::Storage50);
    gov.update_telemetry(45, true, 310);
    if gov.should_stop_charging() || gov.bypass_mode {
        return false; // Still under 50%, charging allowed
    }
    gov.update_telemetry(50, true, 310);
    if !gov.should_stop_charging() || !gov.bypass_mode {
        return false; // Hit 50%, bypass engaged
    }

    // 4. Low battery alert testing
    gov.update_telemetry(12, false, 300);
    if gov.alert_status != LowBatteryAlert::LowWarning {
        return false;
    }

    gov.update_telemetry(4, false, 290);
    if gov.alert_status != LowBatteryAlert::CriticalShutdown {
        return false;
    }

    true
}

// ============================================================================
// Feature 2: Handheld TDP Thermal Profiles & Dynamic Governor State Machine
// ============================================================================

/// Handheld thermal and package power profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TdpProfile {
    /// Quiet Profile: 7W PL1 sustained / 10W PL2 burst (extended battery life & silent fan).
    Quiet,
    /// Balanced Profile: 15W PL1 sustained / 20W PL2 burst (optimal handheld gaming balance).
    Balanced,
    /// Turbo Profile: 28W PL1 sustained / 35W PL2 burst (docked / high-framerate performance).
    Turbo,
    /// Custom user-selected sustained power in Watts (clamped between 4W and 35W).
    Custom(u32),
}

impl TdpProfile {
    /// Returns default sustained power limit (PL1) in Watts.
    pub const fn pl1_watts(&self) -> u32 {
        match self {
            Self::Quiet => 7,
            Self::Balanced => 15,
            Self::Turbo => 28,
            Self::Custom(w) => {
                if *w < 4 {
                    4
                } else if *w > 35 {
                    35
                } else {
                    *w
                }
            }
        }
    }

    /// Returns default short burst power limit (PL2) in Watts.
    pub const fn pl2_watts(&self) -> u32 {
        match self {
            Self::Quiet => 10,
            Self::Balanced => 20,
            Self::Turbo => 35,
            Self::Custom(_) => {
                let sustained = self.pl1_watts();
                let burst = sustained + 5;
                if burst > 40 { 40 } else { burst }
            }
        }
    }
}

/// Dynamic TDP governor state machine coordinating power envelopes.
#[derive(Debug, Clone, Copy)]
pub struct TdpGovernor {
    /// Currently requested user profile.
    pub requested_profile: TdpProfile,
    /// Currently effective active profile after power source & thermal constraints.
    pub effective_profile: TdpProfile,
    /// Active sustained package power limit in Watts (PL1).
    pub active_pl1_watts: u32,
    /// Active short burst power limit in Watts (PL2).
    pub active_pl2_watts: u32,
    /// Short burst time window in milliseconds (Tau).
    pub burst_duration_ms: u32,
    /// External AC power connected.
    pub on_ac_power: bool,
    /// Current battery State of Charge percentage.
    pub battery_soc_pct: u8,
    /// Flag indicating low battery power-saving override is active.
    pub low_battery_throttled: bool,
}

impl TdpGovernor {
    /// Initializes TDP governor with Balanced 15W default profile.
    pub const fn new() -> Self {
        Self {
            requested_profile: TdpProfile::Balanced,
            effective_profile: TdpProfile::Balanced,
            active_pl1_watts: 15,
            active_pl2_watts: 20,
            burst_duration_ms: 28000, // 28s Tau standard
            on_ac_power: false,
            battery_soc_pct: 100,
            low_battery_throttled: false,
        }
    }

    /// Changes requested TDP profile and recalculates active power limits.
    pub fn set_profile(&mut self, profile: TdpProfile) {
        self.requested_profile = profile;
        self.update_effective_limits();
    }

    /// Evaluates power supply telemetry and adjusts constraints.
    pub fn evaluate_power_source(&mut self, on_ac: bool, soc_pct: u8) {
        self.on_ac_power = on_ac;
        self.battery_soc_pct = soc_pct;
        self.update_effective_limits();
    }

    /// Recalculates effective PL1 and PL2 based on power rules.
    fn update_effective_limits(&mut self) {
        // Battery protection rules:
        // When running on battery with low capacity (<= 20%), clamp Turbo down to Balanced
        // If critical (<= 10%), clamp down to Quiet to prevent cell undervoltage cutoff.
        if !self.on_ac_power && self.battery_soc_pct <= 10 {
            self.effective_profile = TdpProfile::Quiet;
            self.low_battery_throttled = true;
        } else if !self.on_ac_power && self.battery_soc_pct <= 20 && self.requested_profile == TdpProfile::Turbo {
            self.effective_profile = TdpProfile::Balanced;
            self.low_battery_throttled = true;
        } else {
            self.effective_profile = self.requested_profile;
            self.low_battery_throttled = false;
        }

        self.active_pl1_watts = self.effective_profile.pl1_watts();
        self.active_pl2_watts = self.effective_profile.pl2_watts();
    }
}

/// Verification test for handheld TDP profiles and dynamic governor.
pub fn verify_tdp_governor() -> bool {
    let mut gov = TdpGovernor::new();

    // 1. Initial Balanced profile checks
    if gov.effective_profile != TdpProfile::Balanced || gov.active_pl1_watts != 15 || gov.active_pl2_watts != 20 {
        return false;
    }

    // 2. Select Turbo profile while connected to AC
    gov.evaluate_power_source(true, 90);
    gov.set_profile(TdpProfile::Turbo);
    if gov.effective_profile != TdpProfile::Turbo || gov.active_pl1_watts != 28 || gov.active_pl2_watts != 35 {
        return false;
    }

    // 3. Unplug AC, battery drops to 18% -> governor must throttle Turbo down to Balanced 15W
    gov.evaluate_power_source(false, 18);
    if gov.effective_profile != TdpProfile::Balanced || gov.active_pl1_watts != 15 || !gov.low_battery_throttled {
        return false;
    }

    // 4. Battery drops to 8% -> governor must force Quiet 7W to prevent brownout
    gov.evaluate_power_source(false, 8);
    if gov.effective_profile != TdpProfile::Quiet || gov.active_pl1_watts != 7 || !gov.low_battery_throttled {
        return false;
    }

    // 5. Plug AC back in -> governor restores requested Turbo profile
    gov.evaluate_power_source(true, 8);
    if gov.effective_profile != TdpProfile::Turbo || gov.active_pl1_watts != 28 || gov.low_battery_throttled {
        return false;
    }

    // 6. Test Custom profile clamping
    gov.set_profile(TdpProfile::Custom(50)); // Request 50W (over max 35W)
    if gov.active_pl1_watts != 35 {
        return false;
    }

    gov.set_profile(TdpProfile::Custom(2)); // Request 2W (under min 4W)
    if gov.active_pl1_watts != 4 {
        return false;
    }

    true
}

// ============================================================================
// Feature 3: ACPI Platform Fan Curve Controller & Thermal Throttling Backoff
// ============================================================================

/// Thermal safety status reflecting system temperatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalSafetyStatus {
    /// Safe operating temperatures (< 85 C).
    Normal,
    /// High thermal load; dynamic fan maxed and safety throttle active (85 C to 94 C).
    ThrottlingWarning,
    /// Critical emergency thermal cutoff; severe TDP drop to protect silicon (>= 95 C).
    CriticalEmergency,
}

/// Temperature trip point mapping degree Celsius to fan PWM duty cycle percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanTripPoint {
    pub temp_c: u8,
    pub pwm_pct: u8,
}

/// Total number of discrete points in the fan curve table.
pub const FAN_CURVE_POINTS: usize = 6;

/// ACPI platform fan curve acoustic governor and thermal safety monitor.
#[derive(Debug, Clone, Copy)]
pub struct PlatformFanController {
    /// Ordered trip points from coolest to hottest.
    pub curve: [FanTripPoint; FAN_CURVE_POINTS],
    /// Last sampled temperature in degrees Celsius.
    pub last_temp_c: u8,
    /// Current calculated fan PWM duty cycle (0 to 100%).
    pub target_pwm_pct: u8,
    /// Estimated fan rotational speed in RPM (0 to 6000 RPM).
    pub current_rpm: u16,
    /// Acoustic hysteresis deadband to prevent rapid cycling.
    pub hysteresis_c: u8,
    /// Active thermal safety status.
    pub safety_status: ThermalSafetyStatus,
    /// TDP power backoff deduction in Watts applied to active profile.
    pub throttle_backoff_watts: u32,
}

impl PlatformFanController {
    /// Maximum fan RPM at 100% PWM.
    pub const MAX_RPM: u32 = 6000;

    /// Initializes default acoustic fan curve for handheld platform.
    pub const fn new() -> Self {
        Self {
            curve: [
                FanTripPoint { temp_c: 40, pwm_pct: 0   }, // Silent below 40C
                FanTripPoint { temp_c: 50, pwm_pct: 25  }, // Quiet whisper
                FanTripPoint { temp_c: 65, pwm_pct: 45  }, // Moderate gaming load
                FanTripPoint { temp_c: 75, pwm_pct: 70  }, // Heavy gaming
                FanTripPoint { temp_c: 85, pwm_pct: 90  }, // Aggressive cooling
                FanTripPoint { temp_c: 95, pwm_pct: 100 }, // Full 100% emergency
            ],
            last_temp_c: 45,
            target_pwm_pct: 25,
            current_rpm: 1500,
            hysteresis_c: 3,
            safety_status: ThermalSafetyStatus::Normal,
            throttle_backoff_watts: 0,
        }
    }

    /// Evaluates temperature reading, computes target PWM and thermal safety backoff.
    pub fn update_temperature(&mut self, temp_c: u8) -> u8 {
        // Hysteresis deadband: if cooling down, require dropping by at least hysteresis_c
        if temp_c < self.last_temp_c && (self.last_temp_c - temp_c) < self.hysteresis_c {
            return self.target_pwm_pct;
        }

        self.last_temp_c = temp_c;

        // Determine thermal safety status and throttle backoff
        if temp_c >= 95 {
            self.safety_status = ThermalSafetyStatus::CriticalEmergency;
            self.throttle_backoff_watts = 15; // Cut 15W immediately
        } else if temp_c >= 85 {
            self.safety_status = ThermalSafetyStatus::ThrottlingWarning;
            self.throttle_backoff_watts = 5;  // Cut 5W backoff
        } else {
            self.safety_status = ThermalSafetyStatus::Normal;
            self.throttle_backoff_watts = 0;
        }

        // Interpolate fan PWM
        self.target_pwm_pct = self.interpolate_pwm(temp_c);
        self.current_rpm = ((self.target_pwm_pct as u32 * Self::MAX_RPM) / 100) as u16;
        self.target_pwm_pct
    }

    /// Interpolates target PWM between curve trip points.
    fn interpolate_pwm(&self, temp_c: u8) -> u8 {
        if temp_c <= self.curve[0].temp_c {
            return self.curve[0].pwm_pct;
        }
        let last_idx = FAN_CURVE_POINTS - 1;
        if temp_c >= self.curve[last_idx].temp_c {
            return self.curve[last_idx].pwm_pct;
        }

        for i in 0..last_idx {
            let p0 = &self.curve[i];
            let p1 = &self.curve[i + 1];
            if temp_c >= p0.temp_c && temp_c <= p1.temp_c {
                let temp_range = (p1.temp_c - p0.temp_c) as u32;
                let pwm_range = (p1.pwm_pct - p0.pwm_pct) as u32;
                let temp_offset = (temp_c - p0.temp_c) as u32;
                let interpolated = p0.pwm_pct as u32 + (temp_offset * pwm_range) / temp_range;
                return interpolated as u8;
            }
        }

        100
    }
}

/// Verification test for ACPI platform fan curve and thermal throttling.
pub fn verify_fan_curve_and_throttling() -> bool {
    let mut fan = PlatformFanController::new();

    // 1. Below 40C: fan should stay silent at 0% PWM and 0 RPM
    let pwm = fan.update_temperature(38);
    if pwm != 0 || fan.current_rpm != 0 || fan.safety_status != ThermalSafetyStatus::Normal {
        return false;
    }

    // 2. Exactly at 50C trip point: 25% PWM, 1500 RPM
    let pwm50 = fan.update_temperature(50);
    if pwm50 != 25 || fan.current_rpm != 1500 {
        return false;
    }

    // 3. Moderate gaming at 70C (interpolated between 65C:45% and 75C:70%)
    // Offset = 5C out of 10C (50%), PWM = 45 + (5 * 25 / 10) = 45 + 12 = 57%
    let pwm70 = fan.update_temperature(70);
    if pwm70 != 57 || fan.safety_status != ThermalSafetyStatus::Normal || fan.throttle_backoff_watts != 0 {
        return false;
    }

    // 4. Thermal warning at 88C: fan should ramp up, warning flag set, 5W throttle backoff
    let pwm88 = fan.update_temperature(88);
    if pwm88 < 90 || fan.safety_status != ThermalSafetyStatus::ThrottlingWarning || fan.throttle_backoff_watts != 5 {
        return false;
    }

    // 5. Critical emergency at 98C: fan 100%, 6000 RPM, CriticalEmergency, 15W throttle backoff
    let pwm98 = fan.update_temperature(98);
    if pwm98 != 100 || fan.current_rpm != 6000 || fan.safety_status != ThermalSafetyStatus::CriticalEmergency || fan.throttle_backoff_watts != 15 {
        return false;
    }

    // 6. Test hysteresis: dropping from 98C to 97C (< 3C delta) retains target
    let pwm_hyst = fan.update_temperature(97);
    if pwm_hyst != 100 {
        return false;
    }

    true
}

// ============================================================================
// Feature 4: Handheld System Suspend / Sleep / Resume Coordination Protocol
// ============================================================================

/// System operational power phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendPhase {
    /// Active interactive gaming execution.
    Running,
    /// Broadcasting suspend preparation to audio, display, GPU, and input daemons.
    PreparingSleep,
    /// Low-power quiescent state (ACPI S0i3 Modern Standby / S3 sleep).
    Sleeping,
    /// Hardware unblanking, audio unmute, and thread unfreeze sequence.
    Waking,
}

/// Hardware trigger that initiated system wakeup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeReason {
    None,
    PowerButton,
    GamepadHome,
    UsbPowerConnected,
    RtcAlarm,
}

/// Bitmask identifiers for subsystems participating in coordinated suspend.
pub const SUBSYSTEM_AUDIO: u32   = 1 << 0;
pub const SUBSYSTEM_DISPLAY: u32 = 1 << 1;
pub const SUBSYSTEM_GPU: u32     = 1 << 2;
pub const SUBSYSTEM_INPUT: u32   = 1 << 3;
pub const SUBSYSTEM_STORAGE: u32 = 1 << 4;
pub const ALL_SUBSYSTEMS_READY: u32 = SUBSYSTEM_AUDIO
    | SUBSYSTEM_DISPLAY
    | SUBSYSTEM_GPU
    | SUBSYSTEM_INPUT
    | SUBSYSTEM_STORAGE;

/// Handheld instant sleep and fast-resume coordinator.
#[derive(Debug, Clone, Copy)]
pub struct SuspendCoordinator {
    /// Current power phase.
    pub phase: SuspendPhase,
    /// Subsystem readiness acknowledgment bitmask.
    pub prepared_mask: u32,
    /// Trigger that caused the latest wakeup.
    pub last_wake_reason: WakeReason,
    /// Monotonic timestamp in milliseconds when sleep was entered.
    pub sleep_enter_ms: u64,
    /// Monotonic timestamp in milliseconds when resume completed.
    pub resume_complete_ms: u64,
    /// Total lifetime suspend/resume cycles successfully handled.
    pub total_sleep_cycles: u32,
    /// Fast instant-resume bypass active.
    pub instant_resume_supported: bool,
}

impl SuspendCoordinator {
    /// Initializes suspend coordinator in fully operational Running state.
    pub const fn new() -> Self {
        Self {
            phase: SuspendPhase::Running,
            prepared_mask: 0,
            last_wake_reason: WakeReason::None,
            sleep_enter_ms: 0,
            resume_complete_ms: 0,
            total_sleep_cycles: 0,
            instant_resume_supported: true,
        }
    }

    /// Initiates graceful sleep sequence across all registered handheld subsystems.
    pub fn initiate_suspend(&mut self, timestamp_ms: u64) -> bool {
        if self.phase != SuspendPhase::Running {
            return false;
        }
        self.phase = SuspendPhase::PreparingSleep;
        self.prepared_mask = 0;
        self.sleep_enter_ms = timestamp_ms;
        true
    }

    /// Records acknowledgment from an individual subsystem (e.g. audio muted, display off).
    pub fn acknowledge_subsystem_ready(&mut self, subsystem_mask: u32) -> bool {
        if self.phase != SuspendPhase::PreparingSleep {
            return false;
        }
        self.prepared_mask |= subsystem_mask;

        // If all subsystems have acknowledged flush & mute, enter full low-power sleep
        if self.prepared_mask == ALL_SUBSYSTEMS_READY {
            self.phase = SuspendPhase::Sleeping;
        }
        true
    }

    /// Hardware interrupt handler to awaken the system.
    pub fn trigger_resume(&mut self, reason: WakeReason, timestamp_ms: u64) -> bool {
        if self.phase != SuspendPhase::Sleeping && self.phase != SuspendPhase::PreparingSleep {
            return false;
        }

        self.phase = SuspendPhase::Waking;
        self.last_wake_reason = reason;
        self.resume_complete_ms = timestamp_ms;
        self.total_sleep_cycles = self.total_sleep_cycles.saturating_add(1);

        // Hardware devices restored (display unblanked, audio unmuted, threads unfrozen)
        self.phase = SuspendPhase::Running;
        self.prepared_mask = 0;
        true
    }

    /// Checks if the handheld is running in fully active interactive mode.
    pub const fn is_fully_awake(&self) -> bool {
        matches!(self.phase, SuspendPhase::Running)
    }
}

/// Verification test for handheld system suspend sleep resume coordination.
pub fn verify_suspend_coordinator() -> bool {
    let mut coord = SuspendCoordinator::new();

    // 1. Initial awake state
    if !coord.is_fully_awake() || coord.phase != SuspendPhase::Running || coord.total_sleep_cycles != 0 {
        return false;
    }

    // 2. Initiate suspend sequence at t=1000ms
    let ok = coord.initiate_suspend(1000);
    if !ok || coord.phase != SuspendPhase::PreparingSleep {
        return false;
    }

    // 3. Acknowledge subsystems one by one
    coord.acknowledge_subsystem_ready(SUBSYSTEM_AUDIO);
    coord.acknowledge_subsystem_ready(SUBSYSTEM_DISPLAY);
    coord.acknowledge_subsystem_ready(SUBSYSTEM_GPU);
    coord.acknowledge_subsystem_ready(SUBSYSTEM_INPUT);
    if coord.phase != SuspendPhase::PreparingSleep {
        return false; // Storage subsystem still pending
    }

    coord.acknowledge_subsystem_ready(SUBSYSTEM_STORAGE);
    if coord.phase != SuspendPhase::Sleeping {
        return false; // Should now be in low-power sleeping state!
    }

    // 4. Trigger resume via Gamepad Home button at t=5000ms
    let resume_ok = coord.trigger_resume(WakeReason::GamepadHome, 5000);
    if !resume_ok || !coord.is_fully_awake() || coord.total_sleep_cycles != 1 {
        return false;
    }
    if coord.last_wake_reason != WakeReason::GamepadHome || coord.resume_complete_ms != 5000 {
        return false;
    }

    // 5. Test another cycle via PowerButton
    coord.initiate_suspend(6000);
    coord.acknowledge_subsystem_ready(ALL_SUBSYSTEMS_READY);
    if coord.phase != SuspendPhase::Sleeping {
        return false;
    }
    coord.trigger_resume(WakeReason::PowerButton, 9500);
    if !coord.is_fully_awake() || coord.total_sleep_cycles != 2 || coord.last_wake_reason != WakeReason::PowerButton {
        return false;
    }

    true
}

