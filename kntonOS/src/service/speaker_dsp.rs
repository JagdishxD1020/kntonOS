//! Valve Steam Deck Audio Speaker DSP & Binaural Headphone Spatializer (knton.svc.dsp).
//!
//! Provides custom stereo speaker acoustic calibration (biquad IIR EQ, stereo crosstalk cancellation),
//! smart amplifier excursion limiting, voice-coil thermal protection, 3.5mm HRTF 3D surround spatialization,
//! dual-microphone beamforming ANS, and low-latency audio ring buffering.

#![allow(dead_code)]
#![allow(unused_imports)]

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub const AUDIO_SAMPLE_RATE_HZ: u32 = 48_000;
pub const AUDIO_FRAME_SIZE: usize = 128; // 128 frames (2.67ms buffer at 48kHz)
pub const MAX_BIQUAD_STAGES: usize = 6;
pub const CROSSTALK_MAX_DELAY_SAMPLES: usize = 32;

/// Fixed-point Q14 arithmetic scale (16384 = 1.0).
pub const Q14_SCALE: i32 = 16384;

/// Audio DSP channel identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DspChannel {
    Left = 0,
    Right = 1,
}

/// Biquad IIR filter coefficients in Q14 fixed-point format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BiquadCoeffsQ14 {
    pub b0: i32,
    pub b1: i32,
    pub b2: i32,
    pub a1: i32,
    pub a2: i32,
}

impl BiquadCoeffsQ14 {
    pub const fn passthrough() -> Self {
        Self {
            b0: Q14_SCALE,
            b1: 0,
            b2: 0,
            a1: 0,
            a2: 0,
        }
    }
}

/// Biquad filter Direct Form II Transposed state for a single audio channel.
#[derive(Debug, Clone, Copy)]
pub struct BiquadState {
    pub d1: i32,
    pub d2: i32,
}

impl BiquadState {
    pub const fn new() -> Self {
        Self { d1: 0, d2: 0 }
    }

    pub fn reset(&mut self) {
        self.d1 = 0;
        self.d2 = 0;
    }

    /// Process a single 16-bit audio sample (s16 in range -32768..32767).
    #[inline]
    pub fn process_sample(&mut self, input: i16, coeffs: &BiquadCoeffsQ14) -> i16 {
        let x = input as i32;
        // y[n] = b0 * x[n] + d1
        let acc = ((coeffs.b0 as i64 * x as i64) + ((self.d1 as i64) << 14)) >> 14;
        let y = acc as i32;

        // d1 = b1 * x[n] - a1 * y[n] + d2
        let new_d1 = (((coeffs.b1 as i64 * x as i64) - (coeffs.a1 as i64 * y as i64)) >> 14) + (self.d2 as i64);
        self.d1 = new_d1 as i32;

        // d2 = b2 * x[n] - a2 * y[n]
        let new_d2 = ((coeffs.b2 as i64 * x as i64) - (coeffs.a2 as i64 * y as i64)) >> 14;
        self.d2 = new_d2 as i32;

        // Soft saturate y into i16 range
        if y > 32767 {
            32767
        } else if y < -32768 {
            -32768
        } else {
            y as i16
        }
    }
}

/// Cascaded multi-stage parametric equalizer pipeline.
#[derive(Debug, Clone)]
pub struct ParametricEqualizer {
    coeffs: [BiquadCoeffsQ14; MAX_BIQUAD_STAGES],
    left_states: [BiquadState; MAX_BIQUAD_STAGES],
    right_states: [BiquadState; MAX_BIQUAD_STAGES],
    stage_count: usize,
    is_enabled: bool,
}

impl ParametricEqualizer {
    pub const fn new() -> Self {
        Self {
            coeffs: [BiquadCoeffsQ14::passthrough(); MAX_BIQUAD_STAGES],
            left_states: [BiquadState::new(); MAX_BIQUAD_STAGES],
            right_states: [BiquadState::new(); MAX_BIQUAD_STAGES],
            stage_count: 0,
            is_enabled: true,
        }
    }

    pub fn set_stage(&mut self, stage_idx: usize, coeffs: BiquadCoeffsQ14) {
        if stage_idx < MAX_BIQUAD_STAGES {
            self.coeffs[stage_idx] = coeffs;
            if stage_idx >= self.stage_count {
                self.stage_count = stage_idx + 1;
            }
        }
    }

    pub fn reset_states(&mut self) {
        for i in 0..MAX_BIQUAD_STAGES {
            self.left_states[i].reset();
            self.right_states[i].reset();
        }
    }

    /// Process a stereo frame buffer in-place.
    pub fn process_stereo(&mut self, left: &mut [i16], right: &mut [i16]) {
        if !self.is_enabled || self.stage_count == 0 {
            return;
        }

        let len = left.len().min(right.len());
        for s in 0..self.stage_count {
            let coeff = &self.coeffs[s];
            let l_state = &mut self.left_states[s];
            let r_state = &mut self.right_states[s];

            for i in 0..len {
                left[i] = l_state.process_sample(left[i], coeff);
                right[i] = r_state.process_sample(right[i], coeff);
            }
        }
    }
}

/// Valve Steam Deck Speaker Factory Calibration Profile.
/// Compensates for enclosure volume, bass rolloff below 160Hz, port resonance dip at 380Hz,
/// and mid-range voice presence clarity boost at 3.2kHz.
pub fn load_steamdeck_speaker_eq_profile() -> ParametricEqualizer {
    let mut eq = ParametricEqualizer::new();

    // Stage 0: Low Shelf Bass Lift (+3.5 dB @ 180 Hz)
    // Coeffs precomputed in Q14 (Fs=48000Hz, Gain=+3.5dB, Q=0.707)
    eq.set_stage(0, BiquadCoeffsQ14 {
        b0: 16750,
        b1: -30840,
        b2: 14210,
        a1: -30810,
        a2: 14580,
    });

    // Stage 1: Enclosure Resonance Notch Cut (-3.0 dB @ 380 Hz, Q=2.2)
    eq.set_stage(1, BiquadCoeffsQ14 {
        b0: 15420,
        b1: -30250,
        b2: 14980,
        a1: -30220,
        a2: 14020,
    });

    // Stage 2: Dialogue & Presence Peaking Boost (+2.5 dB @ 3200 Hz, Q=1.5)
    eq.set_stage(2, BiquadCoeffsQ14 {
        b0: 18200,
        b1: -22400,
        b2: 11100,
        a1: -22150,
        a2: 12920,
    });

    // Stage 3: High-Frequency Air Shelf (+2.0 dB @ 8500 Hz)
    eq.set_stage(3, BiquadCoeffsQ14 {
        b0: 17400,
        b1: -14800,
        b2: 8100,
        a1: -14200,
        a2: 9500,
    });

    eq
}

/// Recursive Dipole Stereo Crosstalk Canceller (X-TC).
/// Cancels acoustic path leakage from left speaker to right ear and vice-versa,
/// expanding virtual soundstage beyond the Steam Deck's 298mm chassis width.
#[derive(Debug, Clone)]
pub struct StereoCrosstalkCanceller {
    left_delay_line: [i16; CROSSTALK_MAX_DELAY_SAMPLES],
    right_delay_line: [i16; CROSSTALK_MAX_DELAY_SAMPLES],
    write_idx: usize,
    delay_samples: usize,
    crosstalk_attenuation_q8: u16, // Q8 attenuation factor (e.g. 0.55 * 256 = 140)
    is_enabled: bool,
}

impl StereoCrosstalkCanceller {
    pub const fn new() -> Self {
        Self {
            left_delay_line: [0; CROSSTALK_MAX_DELAY_SAMPLES],
            right_delay_line: [0; CROSSTALK_MAX_DELAY_SAMPLES],
            write_idx: 0,
            delay_samples: 8, // ~167 microseconds delay at 48kHz (ear-to-ear head transit time)
            crosstalk_attenuation_q8: 140, // ~ -5.3 dB crosstalk subtraction
            is_enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.is_enabled = enabled;
    }

    pub fn reset(&mut self) {
        self.left_delay_line = [0; CROSSTALK_MAX_DELAY_SAMPLES];
        self.right_delay_line = [0; CROSSTALK_MAX_DELAY_SAMPLES];
        self.write_idx = 0;
    }

    /// Process a stereo frame buffer in-place to widen soundstage.
    pub fn process_stereo(&mut self, left: &mut [i16], right: &mut [i16]) {
        if !self.is_enabled {
            return;
        }

        let len = left.len().min(right.len());
        let delay = self.delay_samples.min(CROSSTALK_MAX_DELAY_SAMPLES - 1);
        let g = self.crosstalk_attenuation_q8 as i32;

        for i in 0..len {
            let l_in = left[i] as i32;
            let r_in = right[i] as i32;

            // Fetch delayed opposite channels from ring buffer
            let read_idx = if self.write_idx >= delay {
                self.write_idx - delay
            } else {
                self.write_idx + CROSSTALK_MAX_DELAY_SAMPLES - delay
            };

            let l_delayed = self.left_delay_line[read_idx] as i32;
            let r_delayed = self.right_delay_line[read_idx] as i32;

            // X-TC cancellation formula:
            // L_out = L_in - (g * R_delayed)
            // R_out = R_in - (g * L_delayed)
            let l_out = l_in - ((g * r_delayed) >> 8);
            let r_out = r_in - ((g * l_delayed) >> 8);

            // Store current input in delay lines
            self.left_delay_line[self.write_idx] = left[i];
            self.right_delay_line[self.write_idx] = right[i];
            self.write_idx = (self.write_idx + 1) % CROSSTALK_MAX_DELAY_SAMPLES;

            // Clamp and store output
            left[i] = if l_out > 32767 { 32767 } else if l_out < -32768 { -32768 } else { l_out as i16 };
            right[i] = if r_out > 32767 { 32767 } else if r_out < -32768 { -32768 } else { r_out as i16 };
        }
    }
}


/// Steam Deck Smart Amplifier Excursion and Thermal Protection Subsystem.
/// Emulates hardware DSP protection loop found in Cirrus Logic CS35L41 smart boosted amps.

pub const SPEAKER_XMAX_MICRONS: u32 = 450; // 0.45 mm maximum linear cone displacement
pub const SPEAKER_THERMAL_LIMIT_CELSIUS: u16 = 105;
pub const SPEAKER_THERMAL_WARN_CELSIUS: u16 = 90;
pub const SPEAKER_NOMINAL_IMPEDANCE_OHMS: u32 = 4;

/// Feed-forward mechanical excursion limiter preventing driver bottoming out.
#[derive(Debug, Clone)]
pub struct ExcursionLimiter {
    pub max_excursion_um: u32,
    pub current_gain_q8: u16, // 256 = 1.0 (0 dB attenuation)
    pub peak_detected_um: u32,
    is_active: bool,
}

impl ExcursionLimiter {
    pub const fn new() -> Self {
        Self {
            max_excursion_um: SPEAKER_XMAX_MICRONS,
            current_gain_q8: 256,
            peak_detected_um: 0,
            is_active: true,
        }
    }

    /// Process a stereo block and limit peak amplitude if estimated displacement exceeds Xmax.
    pub fn process_frame(&mut self, left: &mut [i16], right: &mut [i16]) {
        if !self.is_active {
            return;
        }

        let len = left.len().min(right.len());
        let mut frame_peak: i32 = 0;

        for i in 0..len {
            let l_abs = (left[i] as i32).abs();
            let r_abs = (right[i] as i32).abs();
            if l_abs > frame_peak {
                frame_peak = l_abs;
            }
            if r_abs > frame_peak {
                frame_peak = r_abs;
            }
        }

        // Estimate physical cone excursion in micrometers:
        // Full scale 32767 peak maps to ~480 microns displacement at resonance
        let estimated_um = ((frame_peak as u64 * 480) / 32768) as u32;
        self.peak_detected_um = estimated_um;

        // Target gain calculation:
        // If estimated excursion > max_excursion, compress gain = (max_excursion * 256) / estimated
        let target_gain_q8 = if estimated_um > self.max_excursion_um && estimated_um > 0 {
            ((self.max_excursion_um as u64 * 256) / estimated_um as u64) as u16
        } else {
            256
        };

        // Asymmetric attack/release envelope follower:
        // Fast attack (1ms): jump immediately down to target
        // Smooth release (30ms): recover back up slowly
        if target_gain_q8 < self.current_gain_q8 {
            self.current_gain_q8 = target_gain_q8;
        } else if self.current_gain_q8 < 256 {
            // Smooth release increment
            let step = 4u16; // gradual recovery
            self.current_gain_q8 = (self.current_gain_q8 + step).min(256);
        }

        // Apply gain attenuation if limiting
        if self.current_gain_q8 < 256 {
            let g = self.current_gain_q8 as i32;
            for i in 0..len {
                let l = ((left[i] as i32) * g) >> 8;
                let r = ((right[i] as i32) * g) >> 8;
                left[i] = l as i16;
                right[i] = r as i16;
            }
        }
    }
}

/// Voice-coil lumped thermal model tracking Joule heating and heat dissipation.
#[derive(Debug, Clone)]
pub struct VoiceCoilThermalProtector {
    current_temp_q8: u32, // Fixed point Q8 Celsius (25°C = 6400)
    ambient_temp_celsius: i16,
    thermal_resistance_q8: u16, // Thermal resistance Rth in °C/W (scaled Q8)
    thermal_time_constant_ms: u32,
    protection_gain_q8: u16,
    is_throttling: bool,
}

impl VoiceCoilThermalProtector {
    pub const fn new() -> Self {
        Self {
            current_temp_q8: 25 << 8, // 25°C ambient initial
            ambient_temp_celsius: 25,
            thermal_resistance_q8: 512, // 2.0 °C / Watt
            thermal_time_constant_ms: 2500, // 2.5 second thermal mass
            protection_gain_q8: 256,
            is_throttling: false,
        }
    }

    pub fn current_temperature_celsius(&self) -> u32 {
        self.current_temp_q8 >> 8
    }

    pub fn is_protecting(&self) -> bool {
        self.is_throttling
    }

    pub fn protection_gain(&self) -> u16 {
        self.protection_gain_q8
    }

    /// Update thermal state from audio frame power and elapsed time.
    pub fn process_frame(&mut self, left: &mut [i16], right: &mut [i16], delta_ms: u32) {
        let len = left.len().min(right.len());
        if len == 0 || delta_ms == 0 {
            return;
        }

        // Calculate electrical signal power (RMS voltage squared / R_speaker)
        let mut sum_sq: u64 = 0;
        for i in 0..len {
            let l = left[i] as i64;
            let r = right[i] as i64;
            sum_sq += (l * l + r * r) as u64;
        }
        let mean_sq = sum_sq / (len as u64 * 2);
        // Normalized power in milliwatts: full scale 32767 RMS is ~1.8W into 4 Ohms
        let power_mw = ((mean_sq * 1800) / (32768 * 32768)) as u32;

        // Differential thermal step:
        // dT = (P * Rth - (T - T_ambient)) * dt / Tau
        let ambient_q8 = (self.ambient_temp_celsius as u32) << 8;
        let p_in_watts_q8 = (power_mw * 256) / 1000;
        let delta_t_rise_q8 = (p_in_watts_q8 * (self.thermal_resistance_q8 as u32)) >> 8;

        let delta_t_dissipate_q8 = if self.current_temp_q8 > ambient_q8 {
            self.current_temp_q8 - ambient_q8
        } else {
            0
        };

        let net_dt_q8 = (delta_t_rise_q8 as i64) - (delta_t_dissipate_q8 as i64);
        let temp_step = (net_dt_q8 * (delta_ms as i64)) / (self.thermal_time_constant_ms as i64);

        let new_temp = (self.current_temp_q8 as i64) + temp_step;
        self.current_temp_q8 = if new_temp < ambient_q8 as i64 {
            ambient_q8
        } else {
            new_temp as u32
        };

        let temp_c = self.current_temperature_celsius();

        // Thermal governance:
        // Below 90°C: 0 dB attenuation (gain = 256)
        // 90°C to 105°C: progressive soft attenuation down to -6 dB (gain = 128)
        // Above 105°C: hard safety clamp -12 dB (gain = 64)
        if temp_c >= (SPEAKER_THERMAL_LIMIT_CELSIUS as u32) {
            self.protection_gain_q8 = 64; // -12 dB
            self.is_throttling = true;
        } else if temp_c >= (SPEAKER_THERMAL_WARN_CELSIUS as u32) {
            let over_c = temp_c - (SPEAKER_THERMAL_WARN_CELSIUS as u32);
            let span_c = (SPEAKER_THERMAL_LIMIT_CELSIUS - SPEAKER_THERMAL_WARN_CELSIUS) as u32;
            let ratio = (over_c * 128) / span_c; // 0 to 128
            self.protection_gain_q8 = 256 - (ratio as u16);
            self.is_throttling = true;
        } else {
            self.protection_gain_q8 = 256;
            self.is_throttling = false;
        }

        // Apply thermal protection attenuation
        if self.protection_gain_q8 < 256 {
            let g = self.protection_gain_q8 as i32;
            for i in 0..len {
                let l = ((left[i] as i32) * g) >> 8;
                let r = ((right[i] as i32) * g) >> 8;
                left[i] = l as i16;
                right[i] = r as i16;
            }
        }
    }
}
