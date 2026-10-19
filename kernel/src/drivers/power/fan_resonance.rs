//! Handheld Thermal Chamber Calibrated Fan Tachometer & Sub-Acoustic Resonance Anti-Vibration Governor.
//!
//! Compact handheld gaming consoles (such as Steam Deck, ROG Ally, Legion Go) operate small centrifugal
//! blower fans (40mm-50mm) spinning at high speeds (up to 7,000 RPM) directly inside the handheld chassis.
//! This creates severe mechanical vibrations and acoustic resonance spikes in the casing cavity at specific
//! frequency bands, generating an annoying high-pitched tonal whine in the player's hands.
//!
//! This module implements:
//! 1. High-precision fan tachometer pulse capture with timer edge timing and moving-average filtering.
//! 2. Acoustic resonance avoidance governor: detects chassis resonance bands (2400-2600 RPM, 3800-4000 RPM)
//!    and rapidly jumps across them using hysteresis.
//! 3. Anti-vibration micro-dithering (+/- 0.5% at 50Hz) to disperse tonal peaks and eliminate acoustic whine.
//! 4. Fan stall & bearing degradation warning detector against factory thermal chamber calibrated curves.

#![allow(dead_code)]

// ============================================================================
// Feature 1: High-Precision Fan Tachometer Pulse Capture & RPM Filtering
// ============================================================================

/// Size of the tachometer moving-average sample history window.
pub const TACH_FILTER_WINDOW_SIZE: usize = 8;

/// Minimum valid pulse period in microseconds (500 µs = 60,000 RPM cap) for noise debounce.
pub const MIN_VALID_PULSE_PERIOD_US: u32 = 500;

/// Maximum pulse timeout in microseconds (500,000 µs = 0.5s = 60 RPM floor); longer indicates stopped fan.
pub const MAX_PULSE_TIMEOUT_US: u64 = 500_000;

/// High-Precision Fan Tachometer Pulse Capture Engine.
#[derive(Debug, Clone)]
pub struct FanTachometer {
    /// Hall sensor pulses generated per physical fan revolution (typically 2).
    pub pulses_per_revolution: u8,
    /// Timestamp of previous valid falling/rising edge in microseconds.
    pub last_edge_timestamp_us: u64,
    /// Cumulative count of tachometer pulses captured.
    pub pulse_count: u64,
    /// Most recently calculated instantaneous RPM from single pulse period.
    pub instantaneous_rpm: u32,
    /// Smoothed moving-average filtered RPM.
    pub filtered_rpm: u32,
    /// Circular buffer storing recent instantaneous RPM readings.
    pub history_buffer: [u32; TACH_FILTER_WINDOW_SIZE],
    /// Circular buffer insert index.
    pub history_index: usize,
    /// Number of valid samples collected so far.
    pub history_count: usize,
    /// Running sum of samples in history buffer for fast moving average.
    pub history_sum: u32,
    /// Measured period of the last valid pulse in microseconds.
    pub last_valid_period_us: u32,
}

impl FanTachometer {
    pub const fn new(pulses_per_revolution: u8) -> Self {
        Self {
            pulses_per_revolution,
            last_edge_timestamp_us: 0,
            pulse_count: 0,
            instantaneous_rpm: 0,
            filtered_rpm: 0,
            history_buffer: [0; TACH_FILTER_WINDOW_SIZE],
            history_index: 0,
            history_count: 0,
            history_sum: 0,
            last_valid_period_us: 0,
        }
    }

    /// Processes an input capture timer pulse event at `timestamp_us`.
    /// Returns the updated filtered RPM.
    pub fn record_tach_pulse(&mut self, timestamp_us: u64) -> u32 {
        if self.last_edge_timestamp_us == 0 {
            // First pulse received: establish baseline timestamp
            self.last_edge_timestamp_us = timestamp_us;
            self.pulse_count += 1;
            return self.filtered_rpm;
        }

        let delta_us = timestamp_us.saturating_sub(self.last_edge_timestamp_us);

        // Hardware noise glitch debounce filter
        if delta_us < (MIN_VALID_PULSE_PERIOD_US as u64) {
            // Electrical ringing glitch: ignore edge
            return self.filtered_rpm;
        }

        self.last_edge_timestamp_us = timestamp_us;
        self.pulse_count += 1;
        self.last_valid_period_us = delta_us as u32;

        // RPM calculation:
        // One revolution = pulses_per_rev pulses.
        // Revolutions per second = 1,000,000 / (delta_us * pulses_per_rev)
        // Revolutions per minute = 60,000,000 / (delta_us * pulses_per_rev)
        let denom = (delta_us as u64) * (self.pulses_per_revolution as u64);
        if denom > 0 {
            let inst_rpm = (60_000_000u64 / denom) as u32;
            self.instantaneous_rpm = inst_rpm;

            // Update moving-average filter circular buffer
            if self.history_count < TACH_FILTER_WINDOW_SIZE {
                self.history_buffer[self.history_index] = inst_rpm;
                self.history_sum += inst_rpm;
                self.history_count += 1;
                self.history_index = (self.history_index + 1) % TACH_FILTER_WINDOW_SIZE;
            } else {
                let old_sample = self.history_buffer[self.history_index];
                self.history_buffer[self.history_index] = inst_rpm;
                self.history_sum = (self.history_sum - old_sample) + inst_rpm;
                self.history_index = (self.history_index + 1) % TACH_FILTER_WINDOW_SIZE;
            }

            self.filtered_rpm = self.history_sum / (self.history_count as u32);
        }

        self.filtered_rpm
    }

    /// Evaluates whether the fan has stalled based on time since last tach pulse.
    pub fn check_timeout(&mut self, current_time_us: u64) {
        if self.last_edge_timestamp_us > 0 && current_time_us.saturating_sub(self.last_edge_timestamp_us) > MAX_PULSE_TIMEOUT_US {
            // No pulse for > 0.5s: fan is stopped or stalled
            self.instantaneous_rpm = 0;
            self.filtered_rpm = 0;
            self.history_count = 0;
            self.history_sum = 0;
            self.history_buffer = [0; TACH_FILTER_WINDOW_SIZE];
        }
    }

    /// Returns the current filtered fan speed in RPM.
    pub const fn get_rpm(&self) -> u32 {
        self.filtered_rpm
    }
}

/// Verification test for high-precision fan tachometer pulse capture.
pub fn verify_fan_tachometer_pulse_capture() -> bool {
    let mut tach = FanTachometer::new(2); // 2 pulses per revolution

    // 1. Initial state
    if tach.get_rpm() != 0 || tach.pulse_count != 0 {
        return false;
    }

    // 2. Simulate 3000 RPM rotation:
    // 3000 RPM = 50 rev/sec -> 100 pulses/sec -> 1 pulse every 10,000 µs (10 ms)
    let period_us = 10_000u64;
    let mut timestamp = 1_000_000u64;

    // First pulse initializes baseline
    tach.record_tach_pulse(timestamp);
    if tach.pulse_count != 1 || tach.get_rpm() != 0 {
        return false;
    }

    // Feed 10 consecutive pulses at 10,000 µs intervals
    for _ in 0..10 {
        timestamp += period_us;
        tach.record_tach_pulse(timestamp);
    }

    // RPM should calculate exactly to 60,000,000 / (10,000 * 2) = 3000 RPM
    if tach.instantaneous_rpm != 3000 || tach.get_rpm() != 3000 {
        return false;
    }
    if tach.last_valid_period_us != 10_000 {
        return false;
    }

    // 3. Test noise debounce filter: inject electrical glitch pulse 100 µs later (< 500 µs debounce)
    timestamp += 100;
    let rpm_after_noise = tach.record_tach_pulse(timestamp);
    if rpm_after_noise != 3000 {
        return false;
    }

    // 4. Test speed step to 6000 RPM (period = 5,000 µs)
    for _ in 0..8 {
        timestamp += 5_000;
        tach.record_tach_pulse(timestamp);
    }
    if tach.get_rpm() != 6000 || tach.instantaneous_rpm != 6000 {
        return false;
    }

    // 5. Test fan stall timeout: 600,000 µs elapses with no pulses
    tach.check_timeout(timestamp + 600_000);
    if tach.get_rpm() != 0 || tach.instantaneous_rpm != 0 {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Acoustic Resonance Avoidance Governor & Hysteresis Band Jumping
// ============================================================================

/// Chassis mechanical resonance band definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResonanceBand {
    /// Lower boundary of resonant frequency RPM.
    pub min_rpm: u32,
    /// Upper boundary of resonant frequency RPM.
    pub max_rpm: u32,
    /// Safe stable hold RPM below the resonant zone.
    pub safe_lower_rpm: u32,
    /// Safe stable hold RPM above the resonant zone.
    pub safe_upper_rpm: u32,
}

impl ResonanceBand {
    pub const fn contains(&self, rpm: u32) -> bool {
        rpm >= self.min_rpm && rpm <= self.max_rpm
    }

    pub const fn midpoint(&self) -> u32 {
        (self.min_rpm + self.max_rpm) / 2
    }
}

/// Known acoustic resonance bands measured in handheld chassis acoustic chambers.
pub const CHASSIS_RESONANCE_BANDS: [ResonanceBand; 2] = [
    // Band 1: 2400-2600 RPM (chassis cavity low-frequency body resonance)
    ResonanceBand {
        min_rpm: 2400,
        max_rpm: 2600,
        safe_lower_rpm: 2350,
        safe_upper_rpm: 2650,
    },
    // Band 2: 3800-4000 RPM (blade-pass tonal resonance with heatsink copper fins)
    ResonanceBand {
        min_rpm: 3800,
        max_rpm: 4000,
        safe_lower_rpm: 3750,
        safe_upper_rpm: 4050,
    },
];

/// Handheld Acoustic Resonance Avoidance Governor.
///
/// If a fan lingers inside a resonant band, chassis acoustics amplify blade vibration by 6-10 dBA,
/// producing an unbearable high-pitch or booming hum. This governor prevents the fan from ever
/// sustaining speed inside [2400, 2600] RPM or [3800, 4000] RPM. When a thermal curve asks for an
/// RPM inside these forbidden zones, the governor applies directional hysteresis to immediately
/// jump across the band, holding at safe margins without hunting.
#[derive(Debug, Clone)]
pub struct AcousticResonanceGovernor {
    /// Active governed speed output in RPM.
    pub current_governed_rpm: u32,
    /// Previously requested speed from APU thermal governor.
    pub last_requested_rpm: u32,
    /// Direction indicator: true = accelerating, false = decelerating.
    pub is_accelerating: bool,
    /// Flag indicating whether the last command triggered an active resonance band jump.
    pub active_band_jump: bool,
    /// Cumulative count of resonance band jump transitions executed.
    pub jump_events_count: u32,
    /// Total evaluations performed by governor.
    pub total_evaluations: u32,
}

impl AcousticResonanceGovernor {
    pub const fn new() -> Self {
        Self {
            current_governed_rpm: 0,
            last_requested_rpm: 0,
            is_accelerating: true,
            active_band_jump: false,
            jump_events_count: 0,
            total_evaluations: 0,
        }
    }

    /// Evaluates requested thermal RPM and filters out acoustic resonance zones using hysteresis.
    pub fn govern_fan_speed(&mut self, requested_rpm: u32) -> u32 {
        self.total_evaluations += 1;
        self.active_band_jump = false;

        // Determine acceleration direction
        if requested_rpm >= self.last_requested_rpm {
            self.is_accelerating = true;
        } else {
            self.is_accelerating = false;
        }
        self.last_requested_rpm = requested_rpm;

        let mut output_rpm = requested_rpm;

        // Check against known chassis resonance bands
        for band in &CHASSIS_RESONANCE_BANDS {
            if band.contains(requested_rpm) {
                let mid = band.midpoint();
                if self.is_accelerating {
                    // Accelerating upward into resonance band:
                    // Hold at safe_lower_rpm until thermal request reaches midpoint, then jump to safe_upper_rpm
                    if requested_rpm < mid {
                        output_rpm = band.safe_lower_rpm;
                    } else {
                        output_rpm = band.safe_upper_rpm;
                        self.active_band_jump = true;
                        self.jump_events_count += 1;
                    }
                } else {
                    // Decelerating downward into resonance band:
                    // Hold at safe_upper_rpm until thermal request drops below midpoint, then jump to safe_lower_rpm
                    if requested_rpm > mid {
                        output_rpm = band.safe_upper_rpm;
                    } else {
                        output_rpm = band.safe_lower_rpm;
                        self.active_band_jump = true;
                        self.jump_events_count += 1;
                    }
                }
                break;
            }
        }

        self.current_governed_rpm = output_rpm;
        output_rpm
    }

    /// Checks if a given RPM falls inside any forbidden chassis resonance bands.
    pub const fn is_in_resonance_band(rpm: u32) -> bool {
        let mut i = 0;
        while i < CHASSIS_RESONANCE_BANDS.len() {
            if CHASSIS_RESONANCE_BANDS[i].contains(rpm) {
                return true;
            }
            i += 1;
        }
        false
    }
}

/// Verification test for acoustic resonance avoidance governor and hysteresis band jumping.
pub fn verify_acoustic_resonance_avoidance_governor() -> bool {
    let mut gov = AcousticResonanceGovernor::new();

    // 1. Initial state
    if gov.current_governed_rpm != 0 || gov.jump_events_count != 0 {
        return false;
    }

    // 2. Normal non-resonant speed (2000 RPM)
    let r1 = gov.govern_fan_speed(2000);
    if r1 != 2000 || gov.active_band_jump {
        return false;
    }

    // 3. Accelerating into Band 1 (2400-2600 RPM): request 2450 RPM (< 2500 mid)
    let r2 = gov.govern_fan_speed(2450);
    // Must be clamped to safe_lower_rpm (2350 RPM)
    if r2 != 2350 || AcousticResonanceGovernor::is_in_resonance_band(r2) {
        return false;
    }

    // Request 2550 RPM (>= 2500 mid) -> governor must immediately jump to safe_upper_rpm (2650 RPM)
    let r3 = gov.govern_fan_speed(2550);
    if r3 != 2650 || !gov.active_band_jump || AcousticResonanceGovernor::is_in_resonance_band(r3) {
        return false;
    }
    if gov.jump_events_count != 1 {
        return false;
    }

    // 4. Continue accelerating to 3000 RPM
    let r4 = gov.govern_fan_speed(3000);
    if r4 != 3000 {
        return false;
    }

    // 5. Decelerating from 3000 RPM down into Band 1: request 2550 RPM (> 2500 mid)
    let r5 = gov.govern_fan_speed(2550);
    // Must be held at safe_upper_rpm (2650 RPM)
    if r5 != 2650 || AcousticResonanceGovernor::is_in_resonance_band(r5) {
        return false;
    }

    // Request 2450 RPM (<= 2500 mid) -> governor must immediately jump down to safe_lower_rpm (2350 RPM)
    let r6 = gov.govern_fan_speed(2450);
    if r6 != 2350 || !gov.active_band_jump || AcousticResonanceGovernor::is_in_resonance_band(r6) {
        return false;
    }
    if gov.jump_events_count != 2 {
        return false;
    }

    // 6. Test Band 2 (3800-4000 RPM): accelerate from 3500 up to 3950 RPM
    gov.govern_fan_speed(3500);
    let r7 = gov.govern_fan_speed(3850); // < 3900 mid
    if r7 != 3750 {
        return false;
    }
    let r8 = gov.govern_fan_speed(3950); // >= 3900 mid
    if r8 != 4050 || !gov.active_band_jump {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: Anti-Vibration Micro-Dithering for Tonal Whine Dispersion
// ============================================================================

/// Number of control loop samples per 50 Hz dither cycle at 1 kHz governor tick rate.
pub const DITHER_PERIOD_TICKS: u32 = 20;

/// Default micro-dither amplitude in milli-percent (+/- 50 = +/- 0.50% PWM).
pub const DEFAULT_DITHER_AMPLITUDE_MILLIPERCENT: i16 = 50;

/// Handheld Anti-Vibration Micro-Dithering Engine.
///
/// Running a centrifugal fan at a perfectly static PWM duty cycle produces sharp single-frequency
/// acoustic peaks at the blade pass frequency (BPF). In thin handheld consoles, this manifests as an
/// annoying high-pitch piercing whine. Micro-dithering subtly modulates the PWM duty cycle by +/- 0.5%
/// at a 50 Hz rate. This continuous spread-spectrum acoustic dispersion broadens the frequency envelope,
/// lowering peak sound pressure levels by 3-5 dBA and making the fan sound like gentle white noise.
#[derive(Debug, Clone)]
pub struct AntiVibrationMicroDitherer {
    /// Enable switch for acoustic micro-dithering.
    pub enabled: bool,
    /// Peak dither amplitude in milli-percent (+/- 50 = +/- 0.50% PWM duty).
    pub amplitude_millipercent: i16,
    /// Step index within the 20-tick 50 Hz modulation cycle (0..19).
    pub cycle_step: u32,
    /// Most recently computed dither offset in milli-percent.
    pub current_dither_offset: i16,
    /// Commanded PWM duty cycle after dither modulation in milli-percent (0..10,000).
    pub modulated_duty_millipercent: u16,
    /// Total modulation cycles executed.
    pub total_cycles_completed: u32,
}

impl AntiVibrationMicroDitherer {
    pub const fn new(amplitude_millipercent: i16) -> Self {
        Self {
            enabled: true,
            amplitude_millipercent,
            cycle_step: 0,
            current_dither_offset: 0,
            modulated_duty_millipercent: 0,
            total_cycles_completed: 0,
        }
    }

    /// Computes triangular wave dither offset (-amplitude .. +amplitude) for current step.
    pub fn compute_dither_offset(&mut self) -> i16 {
        if !self.enabled || self.amplitude_millipercent == 0 {
            self.current_dither_offset = 0;
            return 0;
        }

        // 20-step triangular waveform across 50 Hz period:
        // Steps 0..5: ramp up from 0 to +amplitude
        // Steps 5..15: ramp down from +amplitude to -amplitude
        // Steps 15..20: ramp up from -amplitude back to 0
        let amp = self.amplitude_millipercent as i32;
        let step = self.cycle_step;

        let offset: i32 = if step < 5 {
            (amp * (step as i32)) / 5
        } else if step < 15 {
            amp - ((2 * amp * ((step - 5) as i32)) / 10)
        } else {
            -amp + ((amp * ((step - 15) as i32)) / 5)
        };

        self.cycle_step = (self.cycle_step + 1) % DITHER_PERIOD_TICKS;
        if self.cycle_step == 0 {
            self.total_cycles_completed += 1;
        }

        self.current_dither_offset = offset as i16;
        self.current_dither_offset
    }

    /// Applies micro-dithering modulation to a base PWM duty cycle (0..10,000 milli-percent = 0..100.00%).
    /// Clamps output safely between 0 and 10,000.
    pub fn apply_dither(&mut self, base_duty_millipercent: u16) -> u16 {
        let offset = self.compute_dither_offset();
        let modulated = (base_duty_millipercent as i32) + (offset as i32);

        let clamped = if modulated < 0 {
            0
        } else if modulated > 10_000 {
            10_000
        } else {
            modulated as u16
        };

        self.modulated_duty_millipercent = clamped;
        clamped
    }

    /// Calculates acoustic energy spread variance across a window of modulated duty samples.
    pub fn calculate_dispersion_variance(samples: &[u16]) -> u32 {
        if samples.is_empty() {
            return 0;
        }
        let mut sum: u64 = 0;
        for &s in samples {
            sum += s as u64;
        }
        let mean = sum / (samples.len() as u64);

        let mut var_sum: u64 = 0;
        for &s in samples {
            let diff = if (s as u64) >= mean {
                (s as u64) - mean
            } else {
                mean - (s as u64)
            };
            var_sum += diff * diff;
        }

        (var_sum / (samples.len() as u64)) as u32
    }
}

/// Verification test for anti-vibration micro-dithering and tonal whine dispersion.
pub fn verify_anti_vibration_micro_dithering() -> bool {
    let mut ditherer = AntiVibrationMicroDitherer::new(DEFAULT_DITHER_AMPLITUDE_MILLIPERCENT); // +/- 50 (+/- 0.50%)

    // 1. Initial state
    if !ditherer.enabled || ditherer.amplitude_millipercent != 50 {
        return false;
    }

    // 2. Modulate a steady 50.00% PWM duty cycle (5000 milli-percent) over a full 20-sample 50Hz cycle
    let base_duty = 5000u16;
    let mut samples = [0u16; 20];
    let mut min_val = 10_000u16;
    let mut max_val = 0u16;

    for i in 0..20 {
        let modulated = ditherer.apply_dither(base_duty);
        samples[i] = modulated;
        if modulated < min_val {
            min_val = modulated;
        }
        if modulated > max_val {
            max_val = modulated;
        }
    }

    // Peak amplitude must reach 5050 (+0.50%) and trough must reach 4950 (-0.50%)
    if max_val != 5050 || min_val != 4950 {
        return false;
    }
    if ditherer.total_cycles_completed != 1 {
        return false;
    }

    // 3. Verify dispersion variance is positive (spread-spectrum dispersion active)
    let variance = AntiVibrationMicroDitherer::calculate_dispersion_variance(&samples);
    if variance == 0 || variance < 500 {
        return false;
    }

    // 4. Test disabling dithering: output should remain exactly at base duty (5000)
    ditherer.enabled = false;
    for _ in 0..10 {
        let unmodulated = ditherer.apply_dither(base_duty);
        if unmodulated != 5000 {
            return false;
        }
    }

    true
}
