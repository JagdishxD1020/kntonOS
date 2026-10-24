//! Handheld Audio DSP Dynamic Range Compressor & Psychoacoustic Bass Booster Engine.
//!
//! Provides bare-metal, low-latency audio processing for handheld console micro-speakers:
//! 1. Real-time lookahead peak limiter and soft-knee dynamic range compression curve with sub-millisecond attack.
//! 2. Harmonic bass synthesis (psychoacoustic bass booster) generating upper harmonics for handheld micro-speakers.
//! 3. Voice-coil thermal and diaphragm excursion protection governor clamping peak voice-coil voltage.
//! 4. Modular audio DSP dynamics pipeline with speaker calibration presets and zero-copy stereo streaming.

#![allow(dead_code)]

// ============================================================================
// Math & Numerical Helpers (Pure no_std)
// ============================================================================

use crate::audio::hrtf::AUDIO_SAMPLE_RATE;
pub const LOOKAHEAD_BUFFER_CAPACITY: usize = 64;

/// Computes natural exponential e^x using range reduction and 6th-order Taylor polynomial.
fn exp_f32(x: f32) -> f32 {
    if x <= -80.0 {
        return 0.0;
    }
    if x >= 80.0 {
        return f32::MAX;
    }

    // x = k * ln(2) + r, where ln(2) ~= 0.69314718
    let inv_ln2 = 1.44269504;
    let k = (x * inv_ln2 + (if x >= 0.0 { 0.5 } else { -0.5 })) as i32;
    let r = x - (k as f32) * 0.69314718;

    let r2 = r * r;
    let r3 = r2 * r;
    let r4 = r2 * r2;
    let r5 = r4 * r;
    let r6 = r3 * r3;

    let poly = 1.0 + r + (r2 * 0.5) + (r3 * 0.16666667) + (r4 * 0.041666668)
        + (r5 * 0.008333334) + (r6 * 0.001388889);

    let exp_bits = ((k + 127).clamp(1, 254) as u32) << 23;
    let scale = f32::from_bits(exp_bits);

    poly * scale
}

/// Natural logarithm ln(x) using mantissa-exponent decomposition and minimax polynomial.
fn ln_f32(x: f32) -> f32 {
    if x <= 0.0 {
        return -80.0;
    }
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127;
    let mantissa = f32::from_bits((bits & 0x007FFFFF) | 0x3F800000);

    // ln(mantissa) for mantissa in [1.0, 2.0]
    let y = (mantissa - 1.0) / (mantissa + 1.0);
    let y2 = y * y;
    let ln_m = 2.0 * y * (1.0 + y2 * (0.33333333 + y2 * (0.2 + y2 * 0.14285714)));

    (exponent as f32) * 0.69314718 + ln_m
}

/// Base-10 logarithm log10(x).
fn log10_f32(x: f32) -> f32 {
    ln_f32(x) * 0.43429448
}

/// Linear amplitude to decibels full-scale (dBFS).
fn linear_to_db(lin: f32) -> f32 {
    let abs_val = if lin < 0.0 { -lin } else { lin };
    if abs_val <= 1e-6 {
        -120.0
    } else {
        20.0 * log10_f32(abs_val)
    }
}

/// Decibels full-scale (dBFS) to linear amplitude.
fn db_to_linear(db: f32) -> f32 {
    exp_f32(db * 0.11512925)
}

/// Absolute value for f32.
fn abs_f32(val: f32) -> f32 {
    if val < 0.0 { -val } else { val }
}

/// Maximum of two f32 values.
fn max_f32(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// Minimum of two f32 values.
fn min_f32(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

// ============================================================================
// Feature 1: Lookahead Peak Limiter & Soft-Knee Dynamic Range Compressor
// ============================================================================

/// Parameters for soft-knee dynamic range compression curve.
#[derive(Debug, Clone, Copy)]
pub struct CompressorParams {
    /// Compression threshold in dBFS (e.g. -18.0 dBFS).
    pub threshold_db: f32,
    /// Compression slope ratio (e.g. 4.0 for 4:1 ratio).
    pub ratio: f32,
    /// Soft-knee transition width in dB (e.g. 6.0 dB).
    pub knee_width_db: f32,
    /// Sub-millisecond attack time in milliseconds (e.g. 0.25 ms = 12 samples at 48kHz).
    pub attack_ms: f32,
    /// Release time in milliseconds (e.g. 50.0 ms).
    pub release_ms: f32,
    /// Post-compression makeup gain in dB (e.g. 3.0 dB).
    pub makeup_gain_db: f32,
    /// Brickwall ceiling for peak limiter in dBFS (e.g. -0.5 dBFS).
    pub ceiling_db: f32,
}

impl CompressorParams {
    pub const fn handheld_default() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 4.0,
            knee_width_db: 6.0,
            attack_ms: 0.25,
            release_ms: 50.0,
            makeup_gain_db: 3.0,
            ceiling_db: -0.5,
        }
    }
}

/// Lookahead circular delay buffer for stereo audio frames.
#[derive(Debug, Clone)]
pub struct LookaheadDelayBuffer {
    buffer_l: [f32; LOOKAHEAD_BUFFER_CAPACITY],
    buffer_r: [f32; LOOKAHEAD_BUFFER_CAPACITY],
    write_pos: usize,
    delay_samples: usize,
}

impl LookaheadDelayBuffer {
    pub const fn new(delay_samples: usize) -> Self {
        let clamped_delay = if delay_samples >= LOOKAHEAD_BUFFER_CAPACITY {
            LOOKAHEAD_BUFFER_CAPACITY - 1
        } else {
            delay_samples
        };
        Self {
            buffer_l: [0.0; LOOKAHEAD_BUFFER_CAPACITY],
            buffer_r: [0.0; LOOKAHEAD_BUFFER_CAPACITY],
            write_pos: 0,
            delay_samples: clamped_delay,
        }
    }

    /// Push an incoming stereo frame and retrieve the delayed stereo frame.
    pub fn push_and_pop(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let read_pos = (self.write_pos + LOOKAHEAD_BUFFER_CAPACITY - self.delay_samples) % LOOKAHEAD_BUFFER_CAPACITY;
        let out_l = self.buffer_l[read_pos];
        let out_r = self.buffer_r[read_pos];

        self.buffer_l[self.write_pos] = in_l;
        self.buffer_r[self.write_pos] = in_r;

        self.write_pos = (self.write_pos + 1) % LOOKAHEAD_BUFFER_CAPACITY;
        (out_l, out_r)
    }

    pub fn delay_samples(&self) -> usize {
        self.delay_samples
    }
}

/// Real-time lookahead peak limiter and soft-knee dynamic range compressor.
#[derive(Debug, Clone)]
pub struct LookaheadSoftKneeCompressor {
    params: CompressorParams,
    delay_buffer: LookaheadDelayBuffer,
    envelope_db: f32,
    attack_coeff: f32,
    release_coeff: f32,
    limiter_envelope_db: f32,
    limiter_attack_coeff: f32,
    limiter_release_coeff: f32,
    total_processed_samples: u64,
    peak_gain_reduction_db: f32,
}

impl LookaheadSoftKneeCompressor {
    pub fn new(params: CompressorParams, sample_rate: f32) -> Self {
        // Lookahead delay: default to 48 samples (~1.0 ms at 48 kHz)
        let delay_samples = ((params.attack_ms * 4.0 * sample_rate) / 1000.0) as usize;
        let clamped_delay = if delay_samples == 0 { 1 } else if delay_samples >= LOOKAHEAD_BUFFER_CAPACITY { LOOKAHEAD_BUFFER_CAPACITY - 1 } else { delay_samples };

        // Attack/release coefficients for compressor
        let attack_coeff = exp_f32(-1.0 / ((params.attack_ms * 0.001 * sample_rate).max(1.0)));
        let release_coeff = exp_f32(-1.0 / ((params.release_ms * 0.001 * sample_rate).max(1.0)));

        // Brickwall limiter attack is ultra-fast (sub-0.1 ms)
        let limiter_attack_coeff = exp_f32(-1.0 / ((0.05 * 0.001 * sample_rate).max(1.0)));
        let limiter_release_coeff = exp_f32(-1.0 / ((10.0 * 0.001 * sample_rate).max(1.0)));

        Self {
            params,
            delay_buffer: LookaheadDelayBuffer::new(clamped_delay),
            envelope_db: -120.0,
            attack_coeff,
            release_coeff,
            limiter_envelope_db: -120.0,
            limiter_attack_coeff,
            limiter_release_coeff,
            total_processed_samples: 0,
            peak_gain_reduction_db: 0.0,
        }
    }

    /// Evaluates the soft-knee compression curve in dBFS.
    ///
    /// - Below (Threshold - Knee/2): 1:1 linear pass-through (0 dB reduction)
    /// - Inside Knee [Threshold - Knee/2, Threshold + Knee/2]: 2nd-order polynomial transition
    /// - Above (Threshold + Knee/2): Linear ratio compression
    pub fn compute_static_characteristic_db(&self, input_db: f32) -> f32 {
        let t = self.params.threshold_db;
        let w = self.params.knee_width_db;
        let r = self.params.ratio;

        let lower = t - w * 0.5;
        let upper = t + w * 0.5;

        if input_db <= lower {
            input_db
        } else if input_db <= upper {
            let delta = input_db - lower;
            input_db + ((1.0 / r) - 1.0) * (delta * delta) / (2.0 * w)
        } else {
            t + (input_db - t) / r
        }
    }

    /// Process a stereo sample frame through the lookahead compressor & peak limiter.
    pub fn process_stereo_frame(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        // 1. Stereo-linked peak detection on un-delayed incoming signal
        let peak_abs = max_f32(abs_f32(in_l), abs_f32(in_r));
        let input_db = linear_to_db(peak_abs);

        // 2. Smooth envelope detector (sub-millisecond attack, smooth release)
        if input_db > self.envelope_db {
            self.envelope_db = self.attack_coeff * self.envelope_db + (1.0 - self.attack_coeff) * input_db;
        } else {
            self.envelope_db = self.release_coeff * self.envelope_db + (1.0 - self.release_coeff) * input_db;
        }

        // 3. Compute static gain reduction from smoothed envelope
        let target_out_db = self.compute_static_characteristic_db(self.envelope_db);
        let comp_gain_db = target_out_db - self.envelope_db;

        // Total stage gain including makeup gain
        let total_stage_gain_db = comp_gain_db + self.params.makeup_gain_db;

        // 4. Lookahead brickwall peak limiter calculation
        let predicted_peak_db = input_db + total_stage_gain_db;
        let limiter_overshoot_db = predicted_peak_db - self.params.ceiling_db;
        let limiter_target_reduction_db = if limiter_overshoot_db > 0.0 { -limiter_overshoot_db } else { 0.0 };

        if limiter_target_reduction_db < self.limiter_envelope_db {
            self.limiter_envelope_db = self.limiter_attack_coeff * self.limiter_envelope_db + (1.0 - self.limiter_attack_coeff) * limiter_target_reduction_db;
        } else {
            self.limiter_envelope_db = self.limiter_release_coeff * self.limiter_envelope_db + (1.0 - self.limiter_release_coeff) * limiter_target_reduction_db;
        }

        let combined_gain_db = total_stage_gain_db + self.limiter_envelope_db;
        if combined_gain_db < self.peak_gain_reduction_db {
            self.peak_gain_reduction_db = combined_gain_db;
        }

        let final_gain_linear = db_to_linear(combined_gain_db);

        // 5. Retrieve delayed audio samples from lookahead delay line
        let (delayed_l, delayed_r) = self.delay_buffer.push_and_pop(in_l, in_r);

        self.total_processed_samples += 1;

        // 6. Apply pre-calculated gain to delayed samples
        (delayed_l * final_gain_linear, delayed_r * final_gain_linear)
    }

    pub fn peak_gain_reduction_db(&self) -> f32 {
        self.peak_gain_reduction_db
    }

    pub fn total_processed_samples(&self) -> u64 {
        self.total_processed_samples
    }
}

/// Verification test for lookahead peak limiter and soft-knee dynamic range compression curve.
pub fn verify_lookahead_soft_knee_compressor() -> bool {
    let params = CompressorParams {
        threshold_db: -18.0,
        ratio: 4.0,
        knee_width_db: 6.0,
        attack_ms: 0.25,
        release_ms: 50.0,
        makeup_gain_db: 0.0,
        ceiling_db: -0.5,
    };

    let mut comp = LookaheadSoftKneeCompressor::new(params, AUDIO_SAMPLE_RATE);

    // 1. Verify soft-knee curve characteristics
    // Well below knee: -30 dBFS -> unchanged (-30 dBFS)
    let out_low = comp.compute_static_characteristic_db(-30.0);
    if abs_f32(out_low - (-30.0)) > 0.01 {
        return false;
    }

    // Mid knee: at threshold (-18 dBFS)
    // lower = -21, upper = -15, delta = 3.
    // y = -18 + (0.25 - 1.0) * 9 / 12 = -18 - 0.5625 = -18.5625 dBFS
    let out_thresh = comp.compute_static_characteristic_db(-18.0);
    let expected_mid = -18.0 + (0.25 - 1.0) * 9.0 / 12.0;
    if abs_f32(out_thresh - expected_mid) > 0.05 {
        return false;
    }

    // Well above knee: 0 dBFS input
    // y = -18 + (0 - (-18)) / 4 = -13.5 dBFS
    let out_high = comp.compute_static_characteristic_db(0.0);
    if abs_f32(out_high - (-13.5)) > 0.05 {
        return false;
    }

    // 2. Continuous derivative: within knee, response must be monotonic
    let mut prev = -40.0;
    let mut d = -40.0;
    while d <= 10.0 {
        let val = comp.compute_static_characteristic_db(d);
        if val < prev {
            return false;
        }
        prev = val;
        d += 0.5;
    }

    // 3. Transient peak limiting and lookahead zero overshoot
    // Feed 100 samples of loud transient (+3.0 dBFS = linear ~1.412)
    let loud_linear = db_to_linear(3.0);
    let ceiling_linear = db_to_linear(params.ceiling_db);

    let mut peak_out: f32 = 0.0;
    for _ in 0..120 {
        let (out_l, out_r) = comp.process_stereo_frame(loud_linear, loud_linear);
        if out_l > peak_out {
            peak_out = out_l;
        }
        if out_r > peak_out {
            peak_out = out_r;
        }
    }

    // After lookahead fill, no output sample may overshoot ceiling (ceiling + 0.05 safety margin)
    if peak_out > ceiling_linear * 1.02 {
        return false;
    }

    // Must have registered gain reduction (< -3.0 dB)
    if comp.peak_gain_reduction_db() > -3.0 {
        return false;
    }

    true
}

// ============================================================================
// Feature 2: Psychoacoustic Bass Booster & Harmonic Bass Synthesizer
// ============================================================================

use crate::graphics::types::{cos_f32, sin_f32, PI};

/// 2nd-order Direct Form II Transposed IIR Biquad Filter.
#[derive(Debug, Clone, Copy)]
pub struct DspBiquadFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1_l: f32,
    z2_l: f32,
    z1_r: f32,
    z2_r: f32,
}

impl DspBiquadFilter {
    pub const fn empty() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1_l: 0.0,
            z2_l: 0.0,
            z1_r: 0.0,
            z2_r: 0.0,
        }
    }

    /// 2nd-order Butterworth low-pass filter.
    pub fn lowpass(cutoff_hz: f32, q: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate;
        let alpha = sin_f32(w0) / (2.0 * q);
        let cos_w0 = cos_f32(w0);

        let a0 = 1.0 + alpha;
        let b0 = ((1.0 - cos_w0) * 0.5) / a0;
        let b1 = (1.0 - cos_w0) / a0;
        let b2 = ((1.0 - cos_w0) * 0.5) / a0;
        let a1 = (-2.0 * cos_w0) / a0;
        let a2 = (1.0 - alpha) / a0;

        Self {
            b0, b1, b2, a1, a2,
            z1_l: 0.0, z2_l: 0.0,
            z1_r: 0.0, z2_r: 0.0,
        }
    }

    /// 2nd-order Butterworth high-pass filter.
    pub fn highpass(cutoff_hz: f32, q: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate;
        let alpha = sin_f32(w0) / (2.0 * q);
        let cos_w0 = cos_f32(w0);

        let a0 = 1.0 + alpha;
        let b0 = ((1.0 + cos_w0) * 0.5) / a0;
        let b1 = (-(1.0 + cos_w0)) / a0;
        let b2 = ((1.0 + cos_w0) * 0.5) / a0;
        let a1 = (-2.0 * cos_w0) / a0;
        let a2 = (1.0 - alpha) / a0;

        Self {
            b0, b1, b2, a1, a2,
            z1_l: 0.0, z2_l: 0.0,
            z1_r: 0.0, z2_r: 0.0,
        }
    }

    /// 2nd-order constant peak gain bandpass filter.
    pub fn bandpass(center_hz: f32, q: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * center_hz / sample_rate;
        let alpha = sin_f32(w0) / (2.0 * q);
        let cos_w0 = cos_f32(w0);

        let a0 = 1.0 + alpha;
        let b0 = alpha / a0;
        let b1 = 0.0;
        let b2 = (-alpha) / a0;
        let a1 = (-2.0 * cos_w0) / a0;
        let a2 = (1.0 - alpha) / a0;

        Self {
            b0, b1, b2, a1, a2,
            z1_l: 0.0, z2_l: 0.0,
            z1_r: 0.0, z2_r: 0.0,
        }
    }

    /// Process stereo sample frame using Direct Form II Transposed structure.
    #[inline]
    pub fn process_stereo(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let out_l = self.b0 * in_l + self.z1_l;
        self.z1_l = self.b1 * in_l - self.a1 * out_l + self.z2_l;
        self.z2_l = self.b2 * in_l - self.a2 * out_l;

        let out_r = self.b0 * in_r + self.z1_r;
        self.z1_r = self.b1 * in_r - self.a1 * out_r + self.z2_r;
        self.z2_r = self.b2 * in_r - self.a2 * out_r;

        (out_l, out_r)
    }

    /// Reset internal filter state delays.
    pub fn reset(&mut self) {
        self.z1_l = 0.0;
        self.z2_l = 0.0;
        self.z1_r = 0.0;
        self.z2_r = 0.0;
    }
}

/// Psychoacoustic Bass Booster with missing fundamental harmonic synthesis.
///
/// Handheld micro-speakers physically cannot reproduce acoustic frequencies below ~80-100Hz.
/// This module synthesizes upper harmonics (2nd, 3rd, 4th) in the 140-300Hz passband.
/// The human brain reconstructs the missing fundamental pitch, creating the perception of
/// deep sub-bass without driving the micro-speaker diaphragms into destructive excursion.
#[derive(Debug, Clone)]
pub struct HarmonicBassBooster {
    /// Crossover low-pass filter to extract sub-bass (< 100 Hz).
    sub_lowpass: DspBiquadFilter,
    /// High-pass filter to relieve micro-speaker cones from inaudible sub-bass (< 75 Hz).
    speaker_highpass: DspBiquadFilter,
    /// Bandpass resonator shaping synthesized harmonics (centered around 180 Hz).
    harmonic_bandpass: DspBiquadFilter,
    /// 2nd harmonic blend factor (octave above).
    pub h2_weight: f32,
    /// 3rd harmonic blend factor (octave + fifth).
    pub h3_weight: f32,
    /// 4th harmonic blend factor (two octaves).
    pub h4_weight: f32,
    /// Master harmonic injection gain.
    pub harmonic_gain: f32,
    /// DC offset blocker state for left and right channels.
    dc_prev_in_l: f32,
    dc_prev_out_l: f32,
    dc_prev_in_r: f32,
    dc_prev_out_r: f32,
    /// Total processed harmonic frames.
    total_processed_frames: u64,
}

impl HarmonicBassBooster {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sub_lowpass: DspBiquadFilter::lowpass(105.0, 0.7071, sample_rate),
            speaker_highpass: DspBiquadFilter::highpass(75.0, 0.7071, sample_rate),
            harmonic_bandpass: DspBiquadFilter::bandpass(180.0, 1.2, sample_rate),
            h2_weight: 0.60,
            h3_weight: 0.30,
            h4_weight: 0.10,
            harmonic_gain: 1.25,
            dc_prev_in_l: 0.0,
            dc_prev_out_l: 0.0,
            dc_prev_in_r: 0.0,
            dc_prev_out_r: 0.0,
            total_processed_frames: 0,
        }
    }

    /// Synthesize upper harmonics from isolated sub-bass signal using Chebyshev polynomials.
    fn synthesize_harmonics_channel(&mut self, sub: f32, is_right: bool) -> f32 {
        // Normalize and soft-saturate to prevent runaway
        let x = if sub > 1.0 { 1.0 } else if sub < -1.0 { -1.0 } else { sub };
        let x2 = x * x;

        // 2nd harmonic: T2(x) = 2x^2 - 1
        let h2 = 2.0 * x2 - 1.0;
        // 3rd harmonic: T3(x) = 4x^3 - 3x
        let h3 = 4.0 * (x2 * x) - 3.0 * x;
        // 4th harmonic: T4(x) = 8x^4 - 8x^2 + 1
        let h4 = 8.0 * (x2 * x2) - 8.0 * x2 + 1.0;

        let raw_harm = self.h2_weight * h2 + self.h3_weight * h3 + self.h4_weight * h4;

        // DC Blocker filter: y[n] = x[n] - x[n-1] + R * y[n-1], with R = 0.995
        let r_dc = 0.995f32;
        if is_right {
            let out_r = raw_harm - self.dc_prev_in_r + r_dc * self.dc_prev_out_r;
            self.dc_prev_in_r = raw_harm;
            self.dc_prev_out_r = out_r;
            out_r
        } else {
            let out_l = raw_harm - self.dc_prev_in_l + r_dc * self.dc_prev_out_l;
            self.dc_prev_in_l = raw_harm;
            self.dc_prev_out_l = out_l;
            out_l
        }
    }

    /// Process a stereo frame through the psychoacoustic harmonic bass pipeline.
    pub fn process_stereo_frame(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        // 1. Extract sub-bass below 105 Hz
        let (sub_l, sub_r) = self.sub_lowpass.process_stereo(in_l, in_r);

        // 2. Synthesize upper harmonics from sub-bass
        let harm_raw_l = self.synthesize_harmonics_channel(sub_l, false);
        let harm_raw_r = self.synthesize_harmonics_channel(sub_r, true);

        // 3. Filter synthesized harmonics through resonant acoustic bandpass
        let (harm_filtered_l, harm_filtered_r) = self.harmonic_bandpass.process_stereo(harm_raw_l, harm_raw_r);

        // 4. Clean program material with highpass filter to protect micro-speaker diaphragm
        let (clean_prog_l, clean_prog_r) = self.speaker_highpass.process_stereo(in_l, in_r);

        self.total_processed_frames += 1;

        // 5. Combine cleaned program audio with synthesized acoustic harmonics
        let out_l = clean_prog_l + harm_filtered_l * self.harmonic_gain;
        let out_r = clean_prog_r + harm_filtered_r * self.harmonic_gain;

        (out_l, out_r)
    }

    pub fn total_processed_frames(&self) -> u64 {
        self.total_processed_frames
    }
}

/// Verification test for psychoacoustic bass booster and harmonic synthesizer.
pub fn verify_psychoacoustic_harmonic_bass_booster() -> bool {
    let mut booster = HarmonicBassBooster::new(AUDIO_SAMPLE_RATE);

    // 1. Feed pure 50 Hz sub-bass sine wave
    // Frequency = 50 Hz, at 48kHz sample rate -> omega = 2*pi*50 / 48000
    let f0 = 50.0;
    let w = 2.0 * PI * f0 / AUDIO_SAMPLE_RATE;

    let mut peak_harm_output: f32 = 0.0;
    let mut sum_out_dc: f32 = 0.0;
    let test_samples = 1920; // 2 full cycles of 50 Hz (40 ms)

    for n in 0..test_samples {
        let t = n as f32;
        let input_sample = sin_f32(w * t) * 0.7; // -3 dBFS 50Hz tone

        let (out_l, out_r) = booster.process_stereo_frame(input_sample, input_sample);
        let abs_val = abs_f32(out_l);
        if abs_val > peak_harm_output {
            peak_harm_output = abs_val;
        }
        if n >= 960 {
            sum_out_dc += out_l;
        }

        // Stereo symmetry check
        if abs_f32(out_l - out_r) > 0.001 {
            return false;
        }
    }

    // Must generate substantial harmonic energy in the audible band (> 0.2 amplitude)
    if peak_harm_output < 0.20 {
        return false;
    }

    // DC blocker verification: DC mean over settled full cycle must be nearly zero (< 0.01)
    let mean_dc = abs_f32(sum_out_dc / 960.0);
    if mean_dc > 0.01 {
        return false;
    }

    if booster.total_processed_frames() != test_samples as u64 {
        return false;
    }

    true
}


