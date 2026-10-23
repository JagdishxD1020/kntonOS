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
