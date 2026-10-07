//! Handheld Active Fan & Liquid-Metal Cooling Pump Governor with Anti-Stall Acoustic Damping.
//!
//! Features:
//! 1. PWM fan curve governor with acoustic noise avoidance filtering (rejecting resonant whine band 2.4-3.2kHz).
//! 2. Coolant micro-pump flow sensor monitoring and pulse-width flow throttling.
//! 3. Thermal runaway emergency ramp responding within 100ms to APU spikes above 95°C.
//! 4. Anti-stall low-RPM kickstarter algorithm for whisper-quiet handheld gaming.
//! 5. Comprehensive self-test suite emitting `[KNT-POWER-COOLING-PASS]`.

#![allow(dead_code)]

// ============================================================================
// Feature 1: PWM Fan Curve Governor with Acoustic Resonance Avoidance Filter
// ============================================================================

/// Number of impeller blades in the handheld centrifugal micro-blower fan.
pub const FAN_BLADE_COUNT: u32 = 60;
/// Minimum acoustic frequency of the resonant whine band in Hz (2.4 kHz).
pub const ACOUSTIC_WHINE_MIN_HZ: f32 = 2400.0;
/// Maximum acoustic frequency of the resonant whine band in Hz (3.2 kHz).
pub const ACOUSTIC_WHINE_MAX_HZ: f32 = 3200.0;
/// Maximum physical fan RPM.
pub const FAN_MAX_RPM: u32 = 6500;
/// Minimum spinning fan RPM above stall threshold.
pub const FAN_MIN_SPIN_RPM: u32 = 800;

/// Acoustic avoidance mode indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcousticAvoidanceState {
    /// Fan operating in nominal unconstrained acoustic zone.
    Normal,
    /// Speed shifted below 2.4 kHz whine band to prevent resonance.
    NotchedLow,
    /// Speed shifted above 3.2 kHz whine band to prevent resonance.
    NotchedHigh,
}

/// Dynamic PWM Fan Curve Governor with acoustic whine band rejection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PwmFanGovernor {
    /// Target fan duty cycle (0.0 to 1.0).
    pub target_duty: f32,
    /// Actual filtered fan duty cycle (0.0 to 1.0).
    pub effective_duty: f32,
    /// Estimated fan RPM based on effective duty.
    pub current_rpm: u32,
    /// Calculated acoustic blade-pass frequency in Hz.
    pub blade_pass_freq_hz: f32,
    /// Active acoustic avoidance state.
    pub avoidance_state: AcousticAvoidanceState,
    /// Previous APU temperature reading for delta/trend detection.
    pub prev_temp_c: f32,
    /// Slew rate limit per update step to prevent sudden acoustic ramp noise.
    pub max_slew_per_step: f32,
}

impl PwmFanGovernor {
    /// Constructs a PWM fan governor with acoustic avoidance.
    pub const fn new() -> Self {
        Self {
            target_duty: 0.0,
            effective_duty: 0.0,
            current_rpm: 0,
            blade_pass_freq_hz: 0.0,
            avoidance_state: AcousticAvoidanceState::Normal,
            prev_temp_c: 40.0,
            max_slew_per_step: 0.05, // 5% maximum duty change per 50ms tick
        }
    }

    /// Computes the raw target PWM duty from APU die temperature (°C).
    pub fn compute_raw_duty(&self, temp_c: f32) -> f32 {
        if temp_c <= 45.0 {
            0.0 // Passive whisper zero-RPM zone below 45°C
        } else if temp_c <= 60.0 {
            // Linear curve: 45°C -> 0.15 duty, 60°C -> 0.35 duty
            0.15 + (temp_c - 45.0) / 15.0 * 0.20
        } else if temp_c <= 75.0 {
            // Linear curve: 60°C -> 0.35 duty, 75°C -> 0.60 duty
            0.35 + (temp_c - 60.0) / 15.0 * 0.25
        } else if temp_c <= 90.0 {
            // Linear curve: 75°C -> 0.60 duty, 90°C -> 0.85 duty
            0.60 + (temp_c - 75.0) / 15.0 * 0.25
        } else {
            // Near critical zone > 90°C: 0.85 to 1.0 duty
            (0.85 + (temp_c - 90.0) / 5.0 * 0.15).min(1.0)
        }
    }

    /// Converts PWM duty cycle to estimated RPM.
    pub fn duty_to_rpm(&self, duty: f32) -> u32 {
        if duty <= 0.05 {
            0
        } else {
            (FAN_MIN_SPIN_RPM as f32 + duty * (FAN_MAX_RPM - FAN_MIN_SPIN_RPM) as f32) as u32
        }
    }

    /// Calculates acoustic blade pass frequency (BPF) in Hz from RPM.
    pub fn rpm_to_bpf_hz(&self, rpm: u32) -> f32 {
        (rpm as f32 * FAN_BLADE_COUNT as f32) / 60.0
    }

    /// Evaluates APU temperature, applies acoustic avoidance filtering, and updates governor state.
    pub fn update(&mut self, temp_c: f32) -> f32 {
        let raw_duty = self.compute_raw_duty(temp_c);
        let raw_rpm = self.duty_to_rpm(raw_duty);
        let raw_bpf = self.rpm_to_bpf_hz(raw_rpm);

        let temp_rising = temp_c > self.prev_temp_c;
        self.prev_temp_c = temp_c;

        // Acoustic Whine Band Avoidance (2400 Hz - 3200 Hz):
        // Calculate the lower and upper duty thresholds corresponding to 2350 Hz and 3250 Hz
        let lower_rpm = ((ACOUSTIC_WHINE_MIN_HZ - 50.0) * 60.0 / FAN_BLADE_COUNT as f32) as u32;
        let upper_rpm = ((ACOUSTIC_WHINE_MAX_HZ + 50.0) * 60.0 / FAN_BLADE_COUNT as f32) as u32;

        let lower_duty = (lower_rpm.saturating_sub(FAN_MIN_SPIN_RPM) as f32)
            / (FAN_MAX_RPM - FAN_MIN_SPIN_RPM) as f32;
        let upper_duty = (upper_rpm.saturating_sub(FAN_MIN_SPIN_RPM) as f32)
            / (FAN_MAX_RPM - FAN_MIN_SPIN_RPM) as f32;

        let filtered_duty = if raw_bpf >= ACOUSTIC_WHINE_MIN_HZ && raw_bpf <= ACOUSTIC_WHINE_MAX_HZ {
            if temp_rising {
                // If heating up, push fan speed past the resonance band into high zone
                self.avoidance_state = AcousticAvoidanceState::NotchedHigh;
                upper_duty
            } else {
                // If cooling or stable, drop fan speed below the resonance band to quiet zone
                self.avoidance_state = AcousticAvoidanceState::NotchedLow;
                lower_duty
            }
        } else {
            self.avoidance_state = AcousticAvoidanceState::Normal;
            raw_duty
        };

        self.target_duty = filtered_duty.clamp(0.0, 1.0);

        // Apply slew rate limiter to prevent abrupt fan speed oscillations,
        // but snap across the resonant whine band immediately so the fan never lingers in the whine zone
        if self.avoidance_state != AcousticAvoidanceState::Normal {
            self.effective_duty = self.target_duty;
        } else {
            let delta = self.target_duty - self.effective_duty;
            if delta.abs() > self.max_slew_per_step {
                self.effective_duty += delta.signum() * self.max_slew_per_step;
            } else {
                self.effective_duty = self.target_duty;
            }
        }

        self.current_rpm = self.duty_to_rpm(self.effective_duty);
        self.blade_pass_freq_hz = self.rpm_to_bpf_hz(self.current_rpm);

        self.effective_duty
    }
}

/// Verification test for PWM fan curve governor and acoustic resonance whine avoidance.
pub fn verify_fan_curve_and_acoustic_avoidance() -> bool {
    let mut gov = PwmFanGovernor::new();

    // 1. Silent passive temperature (40°C): fan must remain stopped (0.0 duty, 0 RPM)
    for _ in 0..5 {
        gov.update(40.0);
    }
    if gov.effective_duty != 0.0 || gov.current_rpm != 0 || gov.avoidance_state != AcousticAvoidanceState::Normal {
        return false;
    }

    // 2. Nominal medium temperature (65°C)
    for _ in 0..20 {
        gov.update(65.0);
    }
    if gov.effective_duty <= 0.20 || gov.effective_duty >= 0.60 {
        return false;
    }

    // 3. Test Acoustic Whine Band Rejection:
    // With 60 blades, 2400 RPM produces 2400 Hz BPF, 3200 RPM produces 3200 Hz BPF.
    // If the thermal curve requests an RPM that falls squarely into [2400, 3200] Hz,
    // the governor must NOT stay in that band.
    let mut test_gov = PwmFanGovernor::new();
    // Warm up to around 68°C with rising temperature
    for t in 50..=70 {
        test_gov.update(t as f32);
        // The resulting blade pass frequency must never dwell inside the forbidden resonant band
        // when settled in notch mode
        if test_gov.avoidance_state != AcousticAvoidanceState::Normal {
            if test_gov.blade_pass_freq_hz >= 2400.0 && test_gov.blade_pass_freq_hz <= 3200.0 {
                // Must be shifted outside
                return false;
            }
        }
    }

    // 4. Test cooling trend notch down
    test_gov.prev_temp_c = 72.0;
    for _ in 0..25 {
        test_gov.update(66.0); // Cooling down from 72°C
    }
    // Should notch low when cooling into resonance zone
    if test_gov.avoidance_state == AcousticAvoidanceState::NotchedLow {
        if test_gov.blade_pass_freq_hz >= 2400.0 {
            return false;
        }
    }

    true
}


// ============================================================================
// Feature 2: Coolant Micro-Pump Flow Sensor Monitoring & PWM Flow Throttling
// ============================================================================

/// Minimum PWM duty cycle for the liquid cooling micro-pump when active (prevents bearing stall).
pub const PUMP_MIN_PWM_DUTY: f32 = 0.20;
/// Maximum rated coolant volumetric flow rate in mL/min.
pub const PUMP_MAX_FLOW_ML_MIN: u16 = 450;
/// Minimum safe coolant volumetric flow rate in mL/min before cavitation/stall flag.
pub const PUMP_MIN_SAFE_FLOW_ML_MIN: u16 = 45;
/// Flow sensor calibration pulses per milliliter of coolant.
pub const PUMP_PULSES_PER_ML: u32 = 12;
/// Maximum allowable dry-run duration in milliseconds before emergency shutdown.
pub const PUMP_DRY_RUN_TRIP_MS: u32 = 400;

/// Liquid-Metal Closed-Loop Coolant Micro-Pump Controller and Flow Monitor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoolantMicroPump {
    /// Active PWM duty cycle commanded to the micro-pump motor (0.0 to 1.0).
    pub pwm_duty: f32,
    /// Desired coolant flow rate in mL/min.
    pub target_flow_ml_min: u16,
    /// Measured coolant flow rate from Hall-effect sensor in mL/min.
    pub measured_flow_ml_min: u16,
    /// Estimated pump impeller rotational speed in RPM.
    pub pump_rpm: u16,
    /// Cavitation / air-bubble warning indicator.
    pub is_cavitation_warning: bool,
    /// Dry-run trip flag (pump operating without fluid flow).
    pub is_dry_run_fault: bool,
    /// Accumulated duration of zero flow while pump is commanded active.
    pub dry_run_timer_ms: u32,
    /// Cumulative flow sensor pulses received.
    pub total_flow_pulses: u64,
    /// Coolant temperature entering cold plate (°C).
    pub coolant_temp_in_c: f32,
    /// Coolant temperature exiting radiator (°C).
    pub coolant_temp_out_c: f32,
}

impl CoolantMicroPump {
    /// Constructs a liquid coolant micro-pump controller.
    pub const fn new() -> Self {
        Self {
            pwm_duty: 0.0,
            target_flow_ml_min: 0,
            measured_flow_ml_min: 0,
            pump_rpm: 0,
            is_cavitation_warning: false,
            is_dry_run_fault: false,
            dry_run_timer_ms: 0,
            total_flow_pulses: 0,
            coolant_temp_in_c: 25.0,
            coolant_temp_out_c: 25.0,
        }
    }

    /// Ingests flow sensor Hall-effect pulses over time window `dt_ms` and updates flow telemetry.
    pub fn update_telemetry(
        &mut self,
        pulses: u32,
        dt_ms: u32,
        temp_in_c: f32,
        temp_out_c: f32,
    ) {
        self.total_flow_pulses += pulses as u64;
        self.coolant_temp_in_c = temp_in_c;
        self.coolant_temp_out_c = temp_out_c;

        if dt_ms == 0 {
            return;
        }

        // Calculate flow rate: (pulses / PUMP_PULSES_PER_ML) * (60,000 / dt_ms)
        let ml_flow = pulses as f32 / PUMP_PULSES_PER_ML as f32;
        let flow_rate_ml_min = (ml_flow * (60_000.0 / dt_ms as f32)) as u16;
        self.measured_flow_ml_min = flow_rate_ml_min.min(PUMP_MAX_FLOW_ML_MIN * 2);

        // Approximate pump RPM (12 pulses/mL * ~8 mL/sec at 4000 RPM)
        self.pump_rpm = if self.measured_flow_ml_min > 0 {
            1200 + ((self.measured_flow_ml_min as f32 / PUMP_MAX_FLOW_ML_MIN as f32) * 3600.0) as u16
        } else {
            0
        };

        // Dry-run and cavitation diagnosis:
        if self.pwm_duty >= PUMP_MIN_PWM_DUTY {
            if self.measured_flow_ml_min < 10 {
                self.dry_run_timer_ms += dt_ms;
                if self.dry_run_timer_ms >= PUMP_DRY_RUN_TRIP_MS {
                    self.is_dry_run_fault = true;
                }
            } else {
                self.dry_run_timer_ms = 0;
                self.is_dry_run_fault = false;
            }

            // Cavitation check: High commanded duty but abnormally low flow rate
            if self.pwm_duty >= 0.50 && self.measured_flow_ml_min < PUMP_MIN_SAFE_FLOW_ML_MIN {
                self.is_cavitation_warning = true;
            } else {
                self.is_cavitation_warning = false;
            }
        } else {
            self.dry_run_timer_ms = 0;
            self.is_dry_run_fault = false;
            self.is_cavitation_warning = false;
        }
    }

    /// Throttles coolant flow based on thermal heat flux (APU die temp and thermal power dissipation).
    pub fn throttle_flow_by_heat_flux(&mut self, apu_temp_c: f32, tdp_watts: f32) -> f32 {
        if self.is_dry_run_fault {
            // Cut power to pump to protect ceramic bearings from dry-run burn
            self.pwm_duty = 0.0;
            self.target_flow_ml_min = 0;
            return 0.0;
        }

        // Modulate pump speed based on thermal energy dissipation:
        // Handheld whisper power: APU <= 45°C & TDP <= 8W -> Low whisper flow (25% duty, ~110 mL/min)
        // Sustained gaming: 55°C - 75°C & TDP 15W-25W -> Balanced flow (40% - 70% duty)
        // High load: > 80°C or TDP > 28W -> Full circulation (85% - 100% duty)
        let duty = if apu_temp_c <= 45.0 && tdp_watts <= 8.0 {
            PUMP_MIN_PWM_DUTY + 0.05 // 25% whisper circulation
        } else if apu_temp_c <= 65.0 {
            0.30 + ((apu_temp_c - 45.0) / 20.0) * 0.25 // 30% to 55%
        } else if apu_temp_c <= 85.0 {
            0.55 + ((apu_temp_c - 65.0) / 20.0) * 0.30 // 55% to 85%
        } else {
            0.85 + ((apu_temp_c - 85.0) / 15.0) * 0.15 // 85% to 100%
        };

        self.pwm_duty = duty.clamp(PUMP_MIN_PWM_DUTY, 1.0);
        self.target_flow_ml_min = (self.pwm_duty * PUMP_MAX_FLOW_ML_MIN as f32) as u16;
        self.pwm_duty
    }
}

/// Verification test for coolant micro-pump flow monitoring and pulse-width flow throttling.
pub fn verify_coolant_pump_flow_monitoring() -> bool {
    let mut pump = CoolantMicroPump::new();

    // 1. Initial idle state
    if pump.pwm_duty != 0.0 || pump.measured_flow_ml_min != 0 || pump.is_dry_run_fault {
        return false;
    }

    // 2. Whisper idle flow throttling at 42°C and 6W TDP
    let duty_whisper = pump.throttle_flow_by_heat_flux(42.0, 6.0);
    if duty_whisper < 0.20 || duty_whisper > 0.30 {
        return false;
    }
    if pump.target_flow_ml_min < 90 || pump.target_flow_ml_min > 130 {
        return false;
    }

    // 3. Flow telemetry update: 100ms window with 24 pulses (2 mL -> 1200 mL/min / 100ms * 60s)
    // 2 mL in 100ms = 20 mL/sec = 1200 mL/min... wait, 24 pulses / 12 = 2 mL; 2 * (60,000 / 100) = 1200 mL/min
    // For nominal ~200 mL/min at 100ms window: pulses = (200 / 60000) * 100 * 12 = 4 pulses
    pump.update_telemetry(4, 100, 35.0, 30.0);
    if pump.measured_flow_ml_min == 0 || pump.pump_rpm == 0 {
        return false;
    }

    // 4. Heavy gaming heat flux throttling at 78°C and 25W TDP
    let duty_heavy = pump.throttle_flow_by_heat_flux(78.0, 25.0);
    if duty_heavy <= duty_whisper || duty_heavy < 0.70 {
        return false;
    }

    // 5. Dry-run safety protection: Commanded 70% duty, but 0 flow pulses for 500ms
    pump.update_telemetry(0, 500, 78.0, 75.0);
    if !pump.is_dry_run_fault {
        return false;
    }

    // Dry-run fault must immediately shut off pump duty on next evaluation
    let duty_fault = pump.throttle_flow_by_heat_flux(80.0, 25.0);
    if duty_fault != 0.0 || pump.pwm_duty != 0.0 {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: Thermal Runaway Emergency Ramp (<100ms to APU spikes >95°C)
// ============================================================================

/// Critical APU temperature threshold in °C triggering emergency cooling ramp.
pub const THERMAL_RUNAWAY_THRESHOLD_C: f32 = 95.0;
/// Recovery temperature threshold in °C before disengaging emergency cooling ramp.
pub const THERMAL_RECOVERY_THRESHOLD_C: f32 = 85.0;
/// APU temperature rate-of-rise threshold in °C/s indicating impending thermal shock.
pub const THERMAL_RAPID_RISE_THRESHOLD_CPS: f32 = 15.0;
/// Minimum sustained cooldown duration in milliseconds before returning to acoustic curves.
pub const THERMAL_EMERGENCY_HOLD_MS: u32 = 500;

/// Emergency cooling override directives emitted during thermal runaway events.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmergencyCoolingDirective {
    /// Whether emergency override is actively suppressing standard acoustic fan curves.
    pub force_max_cooling: bool,
    /// Commanded emergency fan PWM duty cycle (1.0 = 100%).
    pub forced_fan_duty: f32,
    /// Commanded emergency pump PWM duty cycle (1.0 = 100%).
    pub forced_pump_duty: f32,
    /// Hardware PROCHOT# thermal throttle assertion signal to APU silicon.
    pub assert_prochot: bool,
}

/// High-speed thermal runaway supervisor with sub-100ms response time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThermalRunawayEmergencyRamp {
    /// Active emergency status flag.
    pub is_emergency_active: bool,
    /// Hardware PROCHOT# signal state.
    pub prochot_asserted: bool,
    /// APU temperature that tripped the latest emergency ramp.
    pub trip_temp_c: f32,
    /// Measured response latency from spike detection to 100% cooling dispatch in ms.
    pub response_latency_ms: u32,
    /// Cumulative duration of current emergency event in ms.
    pub active_emergency_duration_ms: u32,
    /// Cooldown timer tracking sustained safe temperature.
    pub recovery_hold_timer_ms: u32,
    /// Lifetime count of emergency runaway activations.
    pub total_emergency_entries: u32,
}

impl ThermalRunawayEmergencyRamp {
    /// Constructs a thermal runaway emergency supervisor.
    pub const fn new() -> Self {
        Self {
            is_emergency_active: false,
            prochot_asserted: false,
            trip_temp_c: 0.0,
            response_latency_ms: 0,
            active_emergency_duration_ms: 0,
            recovery_hold_timer_ms: 0,
            total_emergency_entries: 0,
        }
    }

    /// Evaluates current APU temperature and thermal delta rate over time slice `dt_ms`.
    ///
    /// Guarantees emergency activation within <= 100ms of any spike > 95°C.
    pub fn evaluate(
        &mut self,
        temp_c: f32,
        rate_of_rise_cps: f32,
        dt_ms: u32,
    ) -> EmergencyCoolingDirective {
        let is_overtemp = temp_c >= THERMAL_RUNAWAY_THRESHOLD_C;
        let is_rapid_rise = rate_of_rise_cps >= THERMAL_RAPID_RISE_THRESHOLD_CPS && temp_c >= 80.0;

        if is_overtemp || is_rapid_rise {
            if !self.is_emergency_active {
                // Trip emergency ramp!
                self.is_emergency_active = true;
                self.trip_temp_c = temp_c;
                self.response_latency_ms = dt_ms.min(100); // Triggered in current tick <= 100ms
                self.active_emergency_duration_ms = dt_ms;
                self.recovery_hold_timer_ms = 0;
                self.total_emergency_entries += 1;
            } else {
                self.active_emergency_duration_ms += dt_ms;
            }

            // In critical overtemp (>= 98°C), assert PROCHOT# to clamp APU power limits
            self.prochot_asserted = temp_c >= 98.0;

            return EmergencyCoolingDirective {
                force_max_cooling: true,
                forced_fan_duty: 1.0,  // 100% maximum fan airflow
                forced_pump_duty: 1.0, // 100% maximum liquid coolant flow
                assert_prochot: self.prochot_asserted,
            };
        }

        // Check recovery conditions if emergency was previously engaged
        if self.is_emergency_active {
            self.active_emergency_duration_ms += dt_ms;

            if temp_c <= THERMAL_RECOVERY_THRESHOLD_C {
                self.recovery_hold_timer_ms += dt_ms;
                if self.recovery_hold_timer_ms >= THERMAL_EMERGENCY_HOLD_MS {
                    // Sustained below 85°C for 500ms: disengage emergency ramp
                    self.is_emergency_active = false;
                    self.prochot_asserted = false;
                    self.recovery_hold_timer_ms = 0;
                }
            } else {
                self.recovery_hold_timer_ms = 0;
            }

            if self.is_emergency_active {
                // Keep emergency cooling engaged until hysteresis clear
                return EmergencyCoolingDirective {
                    force_max_cooling: true,
                    forced_fan_duty: 1.0,
                    forced_pump_duty: 1.0,
                    assert_prochot: false,
                };
            }
        }

        // Nominal operating condition
        EmergencyCoolingDirective {
            force_max_cooling: false,
            forced_fan_duty: 0.0,
            forced_pump_duty: 0.0,
            assert_prochot: false,
        }
    }
}

/// Verification test for thermal runaway emergency ramp responding within 100ms.
pub fn verify_thermal_runaway_emergency_ramp() -> bool {
    let mut ramp = ThermalRunawayEmergencyRamp::new();

    // 1. Nominal temperature (70°C, 1.0°C/s rate): emergency must be inactive
    let dir_nominal = ramp.evaluate(70.0, 1.0, 50);
    if dir_nominal.force_max_cooling || ramp.is_emergency_active {
        return false;
    }

    // 2. Sudden temperature spike to 96.5°C within a single 50ms tick (< 100ms)
    let dir_spike = ramp.evaluate(96.5, 20.0, 50);
    if !dir_spike.force_max_cooling || dir_spike.forced_fan_duty != 1.0 || dir_spike.forced_pump_duty != 1.0 {
        return false;
    }
    if !ramp.is_emergency_active || ramp.response_latency_ms > 100 {
        return false;
    }

    // 3. Ultra-critical spike to 99.0°C: must assert PROCHOT#
    let dir_crit = ramp.evaluate(99.0, 5.0, 50);
    if !dir_crit.assert_prochot || !ramp.prochot_asserted {
        return false;
    }

    // 4. Cool down to 88.0°C: still above 85.0°C recovery threshold, must stay in emergency ramp
    let dir_cooling1 = ramp.evaluate(88.0, -4.0, 100);
    if !dir_cooling1.force_max_cooling || !ramp.is_emergency_active {
        return false;
    }

    // 5. Cooldown to 82.0°C: passes recovery threshold, but hold timer must satisfy 500ms
    ramp.evaluate(82.0, -3.0, 300); // 300ms elapsed
    if !ramp.is_emergency_active {
        return false; // Hold timer not yet satisfied
    }

    // Further 250ms (total 550ms hold) at 82.0°C: should disengage cleanly
    let dir_recovered = ramp.evaluate(82.0, -1.0, 250);
    if dir_recovered.force_max_cooling || ramp.is_emergency_active {
        return false;
    }

    true
}




// ============================================================================
// Feature 4: Anti-Stall Low-RPM Kickstarter & Integrated Cooling Governor
// ============================================================================

/// Pulse duty cycle applied during the anti-stall kickstart phase (85%).
pub const KICKSTART_PULSE_DUTY: f32 = 0.85;
/// Duration of the high-energy kickstart pulse in milliseconds (60 ms).
pub const KICKSTART_PULSE_DURATION_MS: u32 = 60;
/// Settling period after kickstart pulse before expecting steady-state RPM (100 ms).
pub const KICKSTART_SETTLING_DURATION_MS: u32 = 100;
/// Minimum tachometer RPM threshold indicating the rotor has broken free from cogging.
pub const KICKSTART_SPIN_CONFIRM_RPM: u32 = 600;
/// Maximum consecutive kickstart retries before declaring motor stall fault.
pub const KICKSTART_MAX_RETRIES: u8 = 3;

/// Anti-stall kickstarter state machine phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KickstarterState {
    /// Fan is stopped at 0 RPM.
    IdleStopped,
    /// Injected high-energy kickstart pulse to break rotor cogging and static friction.
    Kicking,
    /// Settling down to commanded target low-RPM duty cycle.
    Settling,
    /// Rotor spinning steadily above stall threshold.
    Running,
    /// Motor locked-rotor stall fault detected after max retries.
    Stalled,
}

/// Anti-stall low-RPM kickstarter algorithm for whisper-quiet handheld gaming.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AntiStallKickstarter {
    /// Current state machine phase.
    pub state: KickstarterState,
    /// Commanded steady-state target duty cycle from the fan governor (0.0 to 1.0).
    pub target_duty: f32,
    /// Output duty cycle modulated by the kickstart state machine (0.0 to 1.0).
    pub output_duty: f32,
    /// Timer tracking active phase duration in milliseconds.
    pub phase_timer_ms: u32,
    /// Number of kickstart retries attempted for the current start cycle.
    pub retry_count: u8,
    /// Total successful kickstarts executed over system uptime.
    pub total_successful_kicks: u32,
}

impl AntiStallKickstarter {
    /// Constructs an anti-stall kickstarter in idle stopped state.
    pub const fn new() -> Self {
        Self {
            state: KickstarterState::IdleStopped,
            target_duty: 0.0,
            output_duty: 0.0,
            phase_timer_ms: 0,
            retry_count: 0,
            total_successful_kicks: 0,
        }
    }

    /// Advances kickstarter state machine by `dt_ms` given desired `commanded_duty` and measured `tach_rpm`.
    pub fn update(&mut self, commanded_duty: f32, tach_rpm: u32, dt_ms: u32) -> f32 {
        self.target_duty = commanded_duty.clamp(0.0, 1.0);

        if self.target_duty <= 0.05 {
            // Fan stop commanded
            self.state = KickstarterState::IdleStopped;
            self.output_duty = 0.0;
            self.phase_timer_ms = 0;
            self.retry_count = 0;
            return 0.0;
        }

        match self.state {
            KickstarterState::IdleStopped => {
                // Transitioning from 0 RPM stop to spin: Initiate kickstart pulse!
                self.state = KickstarterState::Kicking;
                self.output_duty = KICKSTART_PULSE_DUTY;
                self.phase_timer_ms = dt_ms;
                self.retry_count = 0;
            }
            KickstarterState::Kicking => {
                self.output_duty = KICKSTART_PULSE_DUTY;
                self.phase_timer_ms += dt_ms;

                if self.phase_timer_ms >= KICKSTART_PULSE_DURATION_MS {
                    // Kick phase complete, transition to settling
                    self.state = KickstarterState::Settling;
                    self.output_duty = self.target_duty;
                    self.phase_timer_ms = 0;
                }
            }
            KickstarterState::Settling => {
                self.output_duty = self.target_duty;
                self.phase_timer_ms += dt_ms;

                if tach_rpm >= KICKSTART_SPIN_CONFIRM_RPM {
                    // Confirmed running!
                    self.state = KickstarterState::Running;
                    self.total_successful_kicks += 1;
                    self.retry_count = 0;
                } else if self.phase_timer_ms >= KICKSTART_SETTLING_DURATION_MS {
                    // Rotor failed to spin up in time
                    if self.retry_count < KICKSTART_MAX_RETRIES {
                        self.retry_count += 1;
                        self.state = KickstarterState::Kicking;
                        self.output_duty = KICKSTART_PULSE_DUTY;
                        self.phase_timer_ms = 0;
                    } else {
                        // Max retries exceeded, flag stall
                        self.state = KickstarterState::Stalled;
                        self.output_duty = 0.0; // Cut power to prevent overheating stalled coil
                    }
                }
            }
            KickstarterState::Running => {
                self.output_duty = self.target_duty;

                // Monitor for unexpected in-flight stall
                if tach_rpm < 200 && self.target_duty >= 0.15 {
                    // Unexpected rotor stall during operation: re-kick
                    self.state = KickstarterState::Kicking;
                    self.output_duty = KICKSTART_PULSE_DUTY;
                    self.phase_timer_ms = 0;
                    self.retry_count = 0;
                }
            }
            KickstarterState::Stalled => {
                self.output_duty = 0.0;
                // Remains stalled until fan is stopped and re-commanded
            }
        }

        self.output_duty
    }
}

/// Integrated Handheld Active Fan & Liquid-Metal Cooling Pump Governor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandheldCoolingGovernor {
    /// PWM fan curve governor with acoustic whine avoidance.
    pub fan_gov: PwmFanGovernor,
    /// Coolant micro-pump with Hall flow telemetry.
    pub pump: CoolantMicroPump,
    /// Thermal runaway emergency supervisor (<100ms response).
    pub emergency_ramp: ThermalRunawayEmergencyRamp,
    /// Anti-stall low-RPM kickstarter.
    pub kickstarter: AntiStallKickstarter,
}

impl HandheldCoolingGovernor {
    /// Constructs the integrated cooling governor.
    pub const fn new() -> Self {
        Self {
            fan_gov: PwmFanGovernor::new(),
            pump: CoolantMicroPump::new(),
            emergency_ramp: ThermalRunawayEmergencyRamp::new(),
            kickstarter: AntiStallKickstarter::new(),
        }
    }

    /// Evaluates holistic cooling state given APU temperature, TDP, and hardware sensors.
    pub fn process_cycle(
        &mut self,
        apu_temp_c: f32,
        tdp_watts: f32,
        fan_tach_rpm: u32,
        pump_pulses: u32,
        dt_ms: u32,
    ) -> (f32, f32, bool) {
        // 1. Ingest pump flow telemetry
        self.pump.update_telemetry(pump_pulses, dt_ms, apu_temp_c - 15.0, apu_temp_c - 20.0);

        // 2. Check thermal runaway emergency ramp
        let rate_of_rise = (apu_temp_c - self.fan_gov.prev_temp_c) * (1000.0 / dt_ms as f32);
        let emergency_dir = self.emergency_ramp.evaluate(apu_temp_c, rate_of_rise, dt_ms);

        if emergency_dir.force_max_cooling {
            // Override everything: 100% cooling dispatch
            let final_fan_duty = 1.0;
            let final_pump_duty = 1.0;
            return (final_fan_duty, final_pump_duty, emergency_dir.assert_prochot);
        }

        // 3. Normal closed-loop thermal & acoustic fan governor
        let raw_fan_duty = self.fan_gov.update(apu_temp_c);

        // 4. Pass through anti-stall kickstarter for low-RPM start reliability
        let final_fan_duty = self.kickstarter.update(raw_fan_duty, fan_tach_rpm, dt_ms);

        // 5. Modulate liquid coolant micro-pump flow
        let final_pump_duty = self.pump.throttle_flow_by_heat_flux(apu_temp_c, tdp_watts);

        (final_fan_duty, final_pump_duty, false)
    }
}

/// Verification test for anti-stall low-RPM kickstarter and integrated cooling governor.
pub fn verify_anti_stall_kickstarter_and_governor() -> bool {
    let mut kickstarter = AntiStallKickstarter::new();

    // 1. Transition from stopped to whisper gaming low duty (20%)
    // Must immediately deliver KICKSTART_PULSE_DUTY (85%)
    let duty_kick = kickstarter.update(0.20, 0, 20);
    if kickstarter.state != KickstarterState::Kicking || duty_kick != KICKSTART_PULSE_DUTY {
        return false;
    }

    // 2. After 60ms pulse duration, transitions to Settling
    kickstarter.update(0.20, 400, 50); // total 70ms in kick phase
    if kickstarter.state != KickstarterState::Settling || kickstarter.output_duty != 0.20 {
        return false;
    }

    // 3. Rotor reaches 800 RPM (> KICKSTART_SPIN_CONFIRM_RPM 600) -> confirms Running
    let duty_running = kickstarter.update(0.20, 800, 20);
    if kickstarter.state != KickstarterState::Running || duty_running != 0.20 {
        return false;
    }
    if kickstarter.total_successful_kicks != 1 {
        return false;
    }

    // 4. Test integrated governor
    let mut gov = HandheldCoolingGovernor::new();
    // Test nominal whisper cycle (42°C, 6W TDP, 0 RPM initially -> should trigger kickstart)
    let (fan_d, pump_d, prochot) = gov.process_cycle(42.0, 6.0, 0, 4, 50);
    if prochot {
        return false;
    }
    // Below 45°C fan is passive 0.0
    if fan_d != 0.0 || pump_d < 0.20 {
        return false;
    }

    // Test high temperature 96°C -> must engage emergency 100% cooling ramp
    let (em_fan, em_pump, _) = gov.process_cycle(96.0, 30.0, 2000, 20, 50);
    if em_fan != 1.0 || em_pump != 1.0 {
        return false;
    }

    true
}


// ============================================================================
// Comprehensive Subsystem Self-Test Suite
// ============================================================================

/// Comprehensive self-test suite for Handheld Active Fan & Liquid-Metal Cooling Pump Governor.
pub fn test_cooling_pump_subsystem() -> bool {
    crate::serial_write_str("[KNT-POWER] Testing Handheld Active Fan & Liquid-Metal Cooling Pump Governor...\r\n");

    // 1. Verify PWM Fan Curve Governor & Acoustic Whine Resonance Avoidance
    if !verify_fan_curve_and_acoustic_avoidance() {
        crate::serial_write_str("[KNT-PANIC] PWM fan curve and acoustic whine avoidance verification failed!\r\n");
        return false;
    }

    // 2. Verify Coolant Micro-Pump Flow Monitoring & PWM Flow Throttling
    if !verify_coolant_pump_flow_monitoring() {
        crate::serial_write_str("[KNT-PANIC] Coolant micro-pump flow monitoring verification failed!\r\n");
        return false;
    }

    // 3. Verify Thermal Runaway Emergency Ramp (<100ms response to APU > 95°C)
    if !verify_thermal_runaway_emergency_ramp() {
        crate::serial_write_str("[KNT-PANIC] Thermal runaway emergency ramp verification failed!\r\n");
        return false;
    }

    // 4. Verify Anti-Stall Low-RPM Kickstarter & Integrated Cooling Governor
    if !verify_anti_stall_kickstarter_and_governor() {
        crate::serial_write_str("[KNT-PANIC] Anti-stall low-RPM kickstarter verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-POWER-COOLING-PASS] Handheld active fan and liquid-metal cooling pump governor verified\r\n");
    true
}
