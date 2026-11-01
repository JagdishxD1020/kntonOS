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


/// 3.5mm Headphone Binaural HRTF 3D Surround Spatializer.
/// Performs virtual 7.1 surround sound positioning and downmix to binaural stereo headphones.

pub const SURROUND_7_1_CHANNELS: usize = 8;
pub const HRTF_MAX_DELAY_SAMPLES: usize = 48;

/// Virtual 7.1 surround channel indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurroundChannel {
    FrontLeft = 0,
    FrontRight = 1,
    Center = 2,
    Lfe = 3,
    BackLeft = 4,
    BackRight = 5,
    SideLeft = 6,
    SideRight = 7,
}

/// Binaural acoustic parameters per virtual loudspeaker position.
#[derive(Debug, Clone, Copy)]
pub struct ChannelHrtfParams {
    pub azimuth_deg: i16,
    pub itd_left_samples: usize,
    pub itd_right_samples: usize,
    pub ild_left_gain_q8: u16,
    pub ild_right_gain_q8: u16,
    pub head_shadow_coeff_q14: i32, // One-pole low-pass coefficient for contralateral ear
}

/// Precalculated HRTF parameters for 7.1 virtual speaker layout (ITU-R BS.775):
/// Angles: FL(-30°), FR(+30°), FC(0°), LFE(0°), BL(-135°), BR(+135°), SL(-90°), SR(+90°).
pub const VIRTUAL_7_1_HRTF_TABLE: [ChannelHrtfParams; SURROUND_7_1_CHANNELS] = [
    // 0: Front Left (-30°) - Ipsi: Left (0 delay, gain 256), Contra: Right (14 samples, gain 170)
    ChannelHrtfParams {
        azimuth_deg: -30,
        itd_left_samples: 0,
        itd_right_samples: 14,
        ild_left_gain_q8: 256,
        ild_right_gain_q8: 170,
        head_shadow_coeff_q14: 9800,
    },
    // 1: Front Right (+30°) - Ipsi: Right (0 delay, gain 256), Contra: Left (14 samples, gain 170)
    ChannelHrtfParams {
        azimuth_deg: 30,
        itd_left_samples: 14,
        itd_right_samples: 0,
        ild_left_gain_q8: 170,
        ild_right_gain_q8: 256,
        head_shadow_coeff_q14: 9800,
    },
    // 2: Center (0°) - Symmetric arrival to both ears, slightly reduced to fit stereo power sum
    ChannelHrtfParams {
        azimuth_deg: 0,
        itd_left_samples: 2,
        itd_right_samples: 2,
        ild_left_gain_q8: 181, // 1/sqrt(2) approx = 0.707 * 256 = 181
        ild_right_gain_q8: 181,
        head_shadow_coeff_q14: 16384, // No shadow
    },
    // 3: LFE (Subwoofer) - Omnidirectional non-spatialized bass split equally
    ChannelHrtfParams {
        azimuth_deg: 0,
        itd_left_samples: 0,
        itd_right_samples: 0,
        ild_left_gain_q8: 181,
        ild_right_gain_q8: 181,
        head_shadow_coeff_q14: 16384,
    },
    // 4: Back Left (-135°) - Left ipsi (2 samples, gain 220), Right contra (22 samples, gain 120)
    ChannelHrtfParams {
        azimuth_deg: -135,
        itd_left_samples: 2,
        itd_right_samples: 22,
        ild_left_gain_q8: 220,
        ild_right_gain_q8: 120,
        head_shadow_coeff_q14: 7500, // Stronger rear pinna shadow
    },
    // 5: Back Right (+135°) - Right ipsi (2 samples, gain 220), Left contra (22 samples, gain 120)
    ChannelHrtfParams {
        azimuth_deg: 135,
        itd_left_samples: 22,
        itd_right_samples: 2,
        ild_left_gain_q8: 120,
        ild_right_gain_q8: 220,
        head_shadow_coeff_q14: 7500,
    },
    // 6: Side Left (-90°) - Maximum ITD/ILD separation (0 vs 30 samples delay, 256 vs 90 gain)
    ChannelHrtfParams {
        azimuth_deg: -90,
        itd_left_samples: 0,
        itd_right_samples: 30,
        ild_left_gain_q8: 256,
        ild_right_gain_q8: 90,
        head_shadow_coeff_q14: 6200, // Maximum head shadow
    },
    // 7: Side Right (+90°) - Maximum ITD/ILD separation (30 vs 0 samples delay, 90 vs 256 gain)
    ChannelHrtfParams {
        azimuth_deg: 90,
        itd_left_samples: 30,
        itd_right_samples: 0,
        ild_left_gain_q8: 90,
        ild_right_gain_q8: 256,
        head_shadow_coeff_q14: 6200,
    },
];

/// Binaural 3D Surround Spatializer Engine for 3.5mm Headphone Jack.
#[derive(Debug, Clone)]
pub struct BinauralHrtfSpatializer {
    delay_rings: [[i16; HRTF_MAX_DELAY_SAMPLES]; SURROUND_7_1_CHANNELS],
    write_indices: [usize; SURROUND_7_1_CHANNELS],
    shadow_filters_left: [i32; SURROUND_7_1_CHANNELS],
    shadow_filters_right: [i32; SURROUND_7_1_CHANNELS],
    is_enabled: bool,
}

impl BinauralHrtfSpatializer {
    pub const fn new() -> Self {
        Self {
            delay_rings: [[0; HRTF_MAX_DELAY_SAMPLES]; SURROUND_7_1_CHANNELS],
            write_indices: [0; SURROUND_7_1_CHANNELS],
            shadow_filters_left: [0; SURROUND_7_1_CHANNELS],
            shadow_filters_right: [0; SURROUND_7_1_CHANNELS],
            is_enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.is_enabled = enabled;
    }

    pub fn reset(&mut self) {
        self.delay_rings = [[0; HRTF_MAX_DELAY_SAMPLES]; SURROUND_7_1_CHANNELS];
        self.write_indices = [0; SURROUND_7_1_CHANNELS];
        self.shadow_filters_left = [0; SURROUND_7_1_CHANNELS];
        self.shadow_filters_right = [0; SURROUND_7_1_CHANNELS];
    }

    /// Downmix 8-channel 7.1 surround buffers to binaural stereo headphones.
    pub fn spatial_downmix_7_1(
        &mut self,
        inputs: &[&[i16]; SURROUND_7_1_CHANNELS],
        out_left: &mut [i16],
        out_right: &mut [i16],
    ) {
        let len = out_left.len().min(out_right.len());
        if len == 0 {
            return;
        }

        // Initialize accumulators
        let mut acc_l = [0i32; AUDIO_FRAME_SIZE];
        let mut acc_r = [0i32; AUDIO_FRAME_SIZE];
        let frame_len = len.min(AUDIO_FRAME_SIZE);

        for ch in 0..SURROUND_7_1_CHANNELS {
            let hrtf = &VIRTUAL_7_1_HRTF_TABLE[ch];
            let in_buf = inputs[ch];
            let in_len = in_buf.len().min(frame_len);
            let ring = &mut self.delay_rings[ch];
            let mut w_idx = self.write_indices[ch];

            for i in 0..in_len {
                let sample = in_buf[i];

                // Write current sample to delay ring
                ring[w_idx] = sample;

                // Read left ear delayed sample
                let r_left_idx = if w_idx >= hrtf.itd_left_samples {
                    w_idx - hrtf.itd_left_samples
                } else {
                    w_idx + HRTF_MAX_DELAY_SAMPLES - hrtf.itd_left_samples
                };
                let mut s_left = ring[r_left_idx] as i32;

                // Read right ear delayed sample
                let r_right_idx = if w_idx >= hrtf.itd_right_samples {
                    w_idx - hrtf.itd_right_samples
                } else {
                    w_idx + HRTF_MAX_DELAY_SAMPLES - hrtf.itd_right_samples
                };
                let mut s_right = ring[r_right_idx] as i32;

                // Apply one-pole head shadow filter if contralateral
                if hrtf.itd_left_samples > 0 {
                    // Contralateral left ear shadow
                    let alpha = hrtf.head_shadow_coeff_q14;
                    let prev = self.shadow_filters_left[ch];
                    let filtered = ((alpha as i64 * s_left as i64) + ((16384 - alpha) as i64 * prev as i64)) >> 14;
                    self.shadow_filters_left[ch] = filtered as i32;
                    s_left = filtered as i32;
                }
                if hrtf.itd_right_samples > 0 {
                    // Contralateral right ear shadow
                    let alpha = hrtf.head_shadow_coeff_q14;
                    let prev = self.shadow_filters_right[ch];
                    let filtered = ((alpha as i64 * s_right as i64) + ((16384 - alpha) as i64 * prev as i64)) >> 14;
                    self.shadow_filters_right[ch] = filtered as i32;
                    s_right = filtered as i32;
                }

                // Accumulate with ILD gains
                acc_l[i] += (s_left * (hrtf.ild_left_gain_q8 as i32)) >> 8;
                acc_r[i] += (s_right * (hrtf.ild_right_gain_q8 as i32)) >> 8;

                w_idx = (w_idx + 1) % HRTF_MAX_DELAY_SAMPLES;
            }

            self.write_indices[ch] = w_idx;
        }

        // Soft clip / master normalize into output buffers
        for i in 0..frame_len {
            // Master gain scaling: 0.65 to maintain headroom
            let l_scaled = (acc_l[i] * 166) >> 8;
            let r_scaled = (acc_r[i] * 166) >> 8;

            out_left[i] = if l_scaled > 32767 {
                32767
            } else if l_scaled < -32768 {
                -32768
            } else {
                l_scaled as i16
            };

            out_right[i] = if r_scaled > 32767 {
                32767
            } else if r_scaled < -32768 {
                -32768
            } else {
                r_scaled as i16
            };
        }
    }
}


/// Steam Deck Dual-Microphone Beamforming & Active Noise Suppression (ANS) Pipeline.
/// Focuses voice capture on the player's mouth while suppressing top fan exhaust and button clicks.

pub const DUAL_MIC_SEPARATION_MM: u16 = 65; // Physical separation distance on Steam Deck bezel
pub const MIC_DELAY_BUFFER_SIZE: usize = 16;

/// Voice Activity Detector (VAD) analyzing short-term energy and zero crossings.
#[derive(Debug, Clone)]
pub struct VoiceActivityDetector {
    energy_threshold: u32,
    hangover_frames: u8,
    hangover_counter: u8,
    is_speech_active: bool,
}

impl VoiceActivityDetector {
    pub const fn new() -> Self {
        Self {
            energy_threshold: 400, // Energy baseline for speech detection
            hangover_frames: 8,    // 8 frames (~21ms) hangover to keep trailing syllables
            hangover_counter: 0,
            is_speech_active: false,
        }
    }

    pub fn is_speech(&self) -> bool {
        self.is_speech_active
    }

    /// Analyze a PCM block and classify as speech or non-speech.
    pub fn process_frame(&mut self, frame: &[i16]) -> bool {
        if frame.is_empty() {
            return false;
        }

        let mut energy_sum: u64 = 0;
        let mut zero_crossings: usize = 0;
        let mut prev_sign = frame[0] >= 0;

        for &sample in frame {
            let s = sample as i64;
            energy_sum += (s * s) as u64;

            let cur_sign = sample >= 0;
            if cur_sign != prev_sign {
                zero_crossings += 1;
                prev_sign = cur_sign;
            }
        }

        let mean_energy = (energy_sum / frame.len() as u64) as u32;

        // Speech criterion: significant energy above background noise floor or voiced ZCR
        let raw_speech = mean_energy > self.energy_threshold || (mean_energy > self.energy_threshold / 2 && zero_crossings > 10);

        if raw_speech {
            self.hangover_counter = self.hangover_frames;
            self.is_speech_active = true;
        } else if self.hangover_counter > 0 {
            self.hangover_counter -= 1;
            self.is_speech_active = true;
        } else {
            self.is_speech_active = false;
        }

        self.is_speech_active
    }
}

/// Dual-Microphone Broadside Delay-and-Sum Beamformer with Spatial Nulling.
#[derive(Debug, Clone)]
pub struct DualMicBeamformer {
    delay_line_left: [i16; MIC_DELAY_BUFFER_SIZE],
    delay_line_right: [i16; MIC_DELAY_BUFFER_SIZE],
    w_idx: usize,
    null_delay_samples: usize,
    is_enabled: bool,
}

impl DualMicBeamformer {
    pub const fn new() -> Self {
        Self {
            delay_line_left: [0; MIC_DELAY_BUFFER_SIZE],
            delay_line_right: [0; MIC_DELAY_BUFFER_SIZE],
            w_idx: 0,
            null_delay_samples: 4, // Steers spatial cardioid null toward rear/top exhaust vents
            is_enabled: true,
        }
    }

    /// Process raw dual-microphone inputs and produce beamformed voice signal.
    pub fn process_beam(&mut self, mic_left: &[i16], mic_right: &[i16], out_beam: &mut [i16]) {
        let len = mic_left.len().min(mic_right.len()).min(out_beam.len());
        if !self.is_enabled {
            for i in 0..len {
                out_beam[i] = ((mic_left[i] as i32 + mic_right[i] as i32) / 2) as i16;
            }
            return;
        }

        for i in 0..len {
            let l = mic_left[i] as i32;
            let r = mic_right[i] as i32;

            self.delay_line_left[self.w_idx] = mic_left[i];
            self.delay_line_right[self.w_idx] = mic_right[i];

            // In-phase broadside sum: broadside signals (user mouth in front) add constructively (+6dB)
            // Off-axis reverberations add with phase cancellation
            let sum = (l + r) / 2;

            // Delayed right channel for rear nulling
            let r_idx = if self.w_idx >= self.null_delay_samples {
                self.w_idx - self.null_delay_samples
            } else {
                self.w_idx + MIC_DELAY_BUFFER_SIZE - self.null_delay_samples
            };
            let r_delayed = self.delay_line_right[r_idx] as i32;

            // Spatial cardioid combination: enhances front sensitivity while attenuating rear exhaust
            let cardioid = (sum * 3 - r_delayed) / 2;

            self.w_idx = (self.w_idx + 1) % MIC_DELAY_BUFFER_SIZE;

            out_beam[i] = if cardioid > 32767 {
                32767
            } else if cardioid < -32768 {
                -32768
            } else {
                cardioid as i16
            };
        }
    }
}

/// Active Noise Suppression (ANS) filter attenuating stationary fan drone and trackpad clicks.
#[derive(Debug, Clone)]
pub struct ActiveNoiseSuppressor {
    noise_energy_floor: u32,
    suppression_gain_q8: u16, // Gain applied during speech vs non-speech
    comfort_noise_level: u16,
    vad: VoiceActivityDetector,
    is_enabled: bool,
}

impl ActiveNoiseSuppressor {
    pub const fn new() -> Self {
        Self {
            noise_energy_floor: 150,
            suppression_gain_q8: 256,
            comfort_noise_level: 20,
            vad: VoiceActivityDetector::new(),
            is_enabled: true,
        }
    }

    pub fn is_speech_active(&self) -> bool {
        self.vad.is_speech()
    }

    /// Process audio frame and attenuate background fan noise.
    pub fn process_frame(&mut self, input: &[i16], output: &mut [i16]) {
        let len = input.len().min(output.len());
        if len == 0 {
            return;
        }

        let is_speech = self.vad.process_frame(&input[..len]);

        // Frame RMS calculation
        let mut sum_sq: u64 = 0;
        for i in 0..len {
            let s = input[i] as i64;
            sum_sq += (s * s) as u64;
        }
        let frame_energy = (sum_sq / len as u64) as u32;

        if !is_speech {
            // Update stationary background noise floor estimate during silence
            // Leaky integrator: noise_floor = 0.9 * noise_floor + 0.1 * frame_energy
            self.noise_energy_floor = ((self.noise_energy_floor as u64 * 230) + (frame_energy as u64 * 26)) as u32 >> 8;

            // During non-speech, attenuate by 15 dB (gain = 45 / 256)
            self.suppression_gain_q8 = 45;
        } else {
            // During speech, calculate SNR and apply spectral subtraction gain
            if frame_energy > self.noise_energy_floor * 4 {
                self.suppression_gain_q8 = 256; // Full voice pass
            } else {
                // Low SNR speech: subtle attenuation to reduce background hiss
                self.suppression_gain_q8 = 192; // -2.5 dB
            }
        }

        let g = self.suppression_gain_q8 as i32;
        for i in 0..len {
            let s = ((input[i] as i32) * g) >> 8;
            output[i] = if s > 32767 { 32767 } else if s < -32768 { -32768 } else { s as i16 };
        }
    }
}

/// Integrated Voice Chat Capture Processor combining dual-mic beamforming and ANS.
pub struct VoiceChatCaptureProcessor {
    pub beamformer: DualMicBeamformer,
    pub noise_suppressor: ActiveNoiseSuppressor,
    intermediate_beam: [i16; AUDIO_FRAME_SIZE],
}

impl VoiceChatCaptureProcessor {
    pub const fn new() -> Self {
        Self {
            beamformer: DualMicBeamformer::new(),
            noise_suppressor: ActiveNoiseSuppressor::new(),
            intermediate_beam: [0; AUDIO_FRAME_SIZE],
        }
    }

    /// Process stereo microphone inputs and return cleaned voice stream.
    pub fn process_capture(&mut self, mic_left: &[i16], mic_right: &[i16], out_clean: &mut [i16]) {
        let len = mic_left.len().min(mic_right.len()).min(out_clean.len()).min(AUDIO_FRAME_SIZE);
        self.beamformer.process_beam(
            &mic_left[..len],
            &mic_right[..len],
            &mut self.intermediate_beam[..len],
        );
        self.noise_suppressor.process_frame(
            &self.intermediate_beam[..len],
            &mut out_clean[..len],
        );
    }
}
