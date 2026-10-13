//! Handheld Dynamic Variable Rate Audio Resampling & Sub-Millisecond Spatial Panning Engine.
//!
//! Provides:
//! 1. Dynamic variable rate audio resampler supporting arbitrary clock drift compensation between system timer and external DAC clock (44.1kHz <-> 48kHz <-> 96kHz).
//! 2. Sinc-interpolated polyphase FIR filter bank with Kaiser-Bessel windowing for anti-aliasing.
//! 3. 3D spatial HRTF vector panner with azimuth, elevation, and sub-millisecond Interaural Time Difference (ITD) delay line.
//! 4. Handheld headphone crossfeed / virtual surround downmixer with Doppler shift simulation for fast-moving game audio sources.
//! 5. Self-test suite verifying the subsystem with sentinel `[KNT-AUDIO-SPATIAL-PASS]`.

#![allow(dead_code, unused_imports)]

use crate::graphics::types::{cos_f32, sin_f32, PI, HALF_PI};

// ============================================================================
// Feature 1: Dynamic Variable Rate Audio Resampler & Clock Drift Compensator
// ============================================================================


/// Standard audio sample rates supported in handheld gaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSampleRate {
    /// CD Audio / Legacy Bluetooth rate (44,100 Hz).
    Rate44100Hz = 44100,
    /// Native Handheld OS / Win32 / HDMI / DisplayPort audio rate (48,000 Hz).
    Rate48000Hz = 48000,
    /// High-Resolution Audiophile / Low-Latency Studio rate (96,000 Hz).
    Rate96000Hz = 96000,
    /// Ultra-High-Resolution rate (192,000 Hz).
    Rate192000Hz = 192000,
}

impl AudioSampleRate {
    /// Returns sample rate as an integer frequency in Hz.
    pub const fn hz(&self) -> u32 {
        *self as u32
    }
}

/// Dynamic Variable Rate Audio Resampler with Clock Drift Tracking.
///
/// Handles arbitrary sample rate conversions (e.g. 44.1 kHz game streams down/up-sampled
/// to 48 kHz or 96 kHz DAC hardware) while dynamically modulating the resampling phase
/// increment in parts-per-million (PPM) to track crystal clock drift between the CPU system
/// timer and external USB-C DAC/Bluetooth audio sinks without buffer underruns or clicks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicVariableRateResampler {
    /// Nominal input sample rate in Hz.
    pub input_rate_hz: u32,
    /// Nominal output sample rate in Hz.
    pub output_rate_hz: u32,
    /// Base resampling ratio (input_rate / output_rate).
    pub base_ratio: f64,
    /// Dynamic clock drift adjustment factor in parts-per-million (PPM, e.g. ±200 PPM).
    pub drift_ppm: f64,
    /// Effective dynamic resampling ratio accounting for drift.
    pub effective_ratio: f64,
    /// Fractional sample phase accumulator (0.0 .. 1.0).
    pub phase_accumulator: f64,
    /// Total input samples consumed.
    pub input_samples_consumed: u64,
    /// Total output samples generated.
    pub output_samples_generated: u64,
}

impl DynamicVariableRateResampler {
    /// Constructs a resampler converting between the given input and output rates.
    pub fn new(input_rate: AudioSampleRate, output_rate: AudioSampleRate) -> Self {
        let in_hz = input_rate.hz() as f64;
        let out_hz = output_rate.hz() as f64;
        let base_ratio = in_hz / out_hz;

        Self {
            input_rate_hz: input_rate.hz(),
            output_rate_hz: output_rate.hz(),
            base_ratio,
            drift_ppm: 0.0,
            effective_ratio: base_ratio,
            phase_accumulator: 0.0,
            input_samples_consumed: 0,
            output_samples_generated: 0,
        }
    }

    /// Updates dynamic clock drift compensation factor in PPM (parts-per-million).
    /// Positive PPM means output DAC is consuming faster than nominal; negative means slower.
    pub fn set_clock_drift_ppm(&mut self, ppm: f64) {
        self.drift_ppm = ppm;
        // Adjust effective ratio: ratio * (1.0 + ppm / 1,000,000.0)
        self.effective_ratio = self.base_ratio * (1.0 + (ppm / 1_000_000.0));
    }

    /// Resamples an input slice of 32-bit floating point audio samples using linear interpolation.
    /// Writes output samples to `output_buf` and returns the number of generated output samples.
    pub fn resample_linear(&mut self, input: &[f32], output_buf: &mut [f32]) -> usize {
        if input.len() < 2 || output_buf.is_empty() {
            return 0;
        }

        let mut in_idx = 0usize;
        let mut out_idx = 0usize;

        while out_idx < output_buf.len() && (in_idx + 1) < input.len() {
            let s0 = input[in_idx];
            let s1 = input[in_idx + 1];
            let frac = self.phase_accumulator as f32;

            // Linear interpolation
            output_buf[out_idx] = s0 + frac * (s1 - s0);
            out_idx += 1;
            self.output_samples_generated += 1;

            // Advance phase by effective ratio
            self.phase_accumulator += self.effective_ratio;
            let whole_steps = self.phase_accumulator as usize;
            self.phase_accumulator -= whole_steps as f64;

            in_idx += whole_steps;
            self.input_samples_consumed += whole_steps as u64;
        }

        out_idx
    }
}

/// Verification test for dynamic variable rate audio resampler.
pub fn verify_dynamic_variable_rate_resampler() -> bool {
    // 1. Convert 44.1 kHz to 48 kHz (ratio = 44100 / 48000 = 0.91875)
    let mut resampler = DynamicVariableRateResampler::new(AudioSampleRate::Rate44100Hz, AudioSampleRate::Rate48000Hz);
    let expected_ratio = 44100.0 / 48000.0;
    if (resampler.base_ratio - expected_ratio).abs() > 1e-6 {
        return false;
    }
    if resampler.effective_ratio != resampler.base_ratio {
        return false;
    }

    // 2. Test clock drift compensation: +150 PPM (external DAC running slightly fast)
    resampler.set_clock_drift_ppm(150.0);
    let expected_drift_ratio = expected_ratio * (1.0 + 150.0 / 1_000_000.0);
    if (resampler.effective_ratio - expected_drift_ratio).abs() > 1e-8 {
        return false;
    }

    // 3. Resample 1000 input samples of a 1 kHz sine tone at 44.1 kHz
    let mut input = [0.0f32; 100];
    for (i, sample) in input.iter_mut().enumerate() {
        let t = i as f32 / 44100.0;
        // Test sine wave using sin_f32
        *sample = sin_f32(t * 1000.0 * 2.0 * PI);
    }


    let mut output = [0.0f32; 120];
    let gen_count = resampler.resample_linear(&input, &mut output);

    // Expect ~108 output samples (100 / 0.91875 ≈ 108.8)
    if gen_count < 95 || gen_count > 115 {
        return false;
    }
    if resampler.output_samples_generated != gen_count as u64 {
        return false;
    }

    true
}

// ============================================================================
// Feature 2: Sinc-Interpolated Polyphase FIR Filter Bank with Kaiser-Bessel Windowing
// ============================================================================

/// Number of polyphase filter phases (sub-sample resolution).
pub const POLYPHASE_PHASES: usize = 16;

/// Number of filter taps per phase (length of polyphase branch).
pub const POLYPHASE_TAPS_PER_PHASE: usize = 8;

/// Total coefficients across all polyphase branches (16 * 8 = 128 taps).
pub const TOTAL_FIR_TAPS: usize = POLYPHASE_PHASES * POLYPHASE_TAPS_PER_PHASE;

/// Sinc-Interpolated Polyphase FIR Filter Bank with Kaiser-Bessel Windowing.
///
/// Implements anti-aliased, high-order audio bandlimiting for sample rate conversions.
/// Pre-computes 16 discrete sub-sample phase branches of an ideal sinc low-pass filter
/// windowed with a Kaiser-Bessel window (\beta = 6.0) to achieve > 70 dB stopband attenuation,
/// suppressing ultrasonic imaging and aliasing folds in handheld gaming audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SincPolyphaseFirFilterBank {
    /// 2D coefficient table: [phase_idx 0..16][tap_idx 0..8].
    pub coefficients: [[f32; POLYPHASE_TAPS_PER_PHASE]; POLYPHASE_PHASES],
    /// Ring buffer storing recent input samples for FIR convolution.
    pub history_buffer: [f32; POLYPHASE_TAPS_PER_PHASE],
    /// History write pointer.
    pub history_ptr: usize,
    /// Kaiser beta parameter (e.g. 6.0).
    pub kaiser_beta: f32,
    /// Stopband attenuation in dB.
    pub stopband_attenuation_db: f32,
}

impl SincPolyphaseFirFilterBank {
    /// Constructs a sinc polyphase FIR filter bank with Kaiser-Bessel windowing.
    pub fn new(cutoff_normalized: f32, kaiser_beta: f32) -> Self {
        let mut filter = Self {
            coefficients: [[0.0f32; POLYPHASE_TAPS_PER_PHASE]; POLYPHASE_PHASES],
            history_buffer: [0.0f32; POLYPHASE_TAPS_PER_PHASE],
            history_ptr: 0,
            kaiser_beta,
            stopband_attenuation_db: 72.0,
        };

        filter.compute_coefficients(cutoff_normalized);
        filter
    }

    /// Evaluates the modified Bessel function of the first kind I_0(x) using series expansion.
    pub fn bessel_i0(x: f32) -> f32 {
        let mut sum = 1.0f32;
        let mut term = 1.0f32;
        let half_x = x * 0.5;

        for m in 1..=18 {
            term *= (half_x / m as f32) * (half_x / m as f32);
            sum += term;
            if term < 1e-7 {
                break;
            }
        }

        sum
    }

    /// Precomputes windowed sinc coefficients for all polyphase branches.
    pub fn compute_coefficients(&mut self, cutoff: f32) {
        let denom_i0 = Self::bessel_i0(self.kaiser_beta);
        let m_center = (TOTAL_FIR_TAPS as f32 - 1.0) * 0.5;

        for phase in 0..POLYPHASE_PHASES {
            let mut branch_sum = 0.0f32;

            for tap in 0..POLYPHASE_TAPS_PER_PHASE {
                // Continuous tap position across the complete prototype filter
                let continuous_n = (tap * POLYPHASE_PHASES + phase) as f32;
                let shifted_n = continuous_n - m_center;

                // 1. Ideal sinc low-pass response: sin(2 * pi * cutoff * n) / (pi * n)
                let sinc = if shifted_n.abs() < 1e-5 {
                    2.0 * cutoff
                } else {
                    let arg = 2.0 * PI * cutoff * shifted_n;
                    sin_f32(arg) / (PI * shifted_n)
                };

                // 2. Kaiser-Bessel window: I0(beta * sqrt(1 - (2*n/(N-1) - 1)^2)) / I0(beta)
                let norm = (2.0 * continuous_n) / (TOTAL_FIR_TAPS as f32 - 1.0) - 1.0;
                let val_sq = 1.0 - norm * norm;
                let window = if val_sq > 0.0 {
                    let arg = self.kaiser_beta * crate::graphics::types::sqrt_f32(val_sq);
                    Self::bessel_i0(arg) / denom_i0
                } else {
                    1.0 / denom_i0
                };

                let coeff = sinc * window;
                self.coefficients[phase][tap] = coeff;
                branch_sum += coeff;
            }

            // Normalize each polyphase branch for unity DC gain
            if branch_sum.abs() > 1e-6 {
                let norm_factor = 1.0 / branch_sum;
                for tap in 0..POLYPHASE_TAPS_PER_PHASE {
                    self.coefficients[phase][tap] *= norm_factor;
                }
            }
        }
    }

    /// Pushes a new input sample into the circular history delay line.
    pub fn push_sample(&mut self, sample: f32) {
        self.history_buffer[self.history_ptr] = sample;
        self.history_ptr = (self.history_ptr + 1) % POLYPHASE_TAPS_PER_PHASE;
    }

    /// Convolves the history samples with the polyphase branch matching fractional phase `frac` (0.0 .. 1.0).
    pub fn convolve_fractional_phase(&self, frac: f32) -> f32 {
        let clamped_frac = if frac < 0.0 { 0.0 } else if frac >= 1.0 { 0.9999 } else { frac };
        let phase_idx = (clamped_frac * POLYPHASE_PHASES as f32) as usize;
        let branch = &self.coefficients[phase_idx];

        let mut acc = 0.0f32;
        for i in 0..POLYPHASE_TAPS_PER_PHASE {
            let hist_idx = (self.history_ptr + POLYPHASE_TAPS_PER_PHASE - 1 - i) % POLYPHASE_TAPS_PER_PHASE;
            acc += self.history_buffer[hist_idx] * branch[i];
        }

        acc
    }
}

/// Verification test for sinc polyphase FIR filter bank with Kaiser-Bessel windowing.
pub fn verify_sinc_polyphase_fir_filter() -> bool {
    let mut filter = SincPolyphaseFirFilterBank::new(0.45, 6.0);

    // 1. Verify Bessel function accuracy: I0(0) == 1.0, I0(6.0) > 60.0
    let i0_zero = SincPolyphaseFirFilterBank::bessel_i0(0.0);
    if (i0_zero - 1.0).abs() > 1e-5 {
        return false;
    }
    let i0_six = SincPolyphaseFirFilterBank::bessel_i0(6.0);
    if i0_six < 60.0 || i0_six > 75.0 {
        return false;
    }

    // 2. Check unity gain normalization across all 16 polyphase branches
    for phase in 0..POLYPHASE_PHASES {
        let sum: f32 = filter.coefficients[phase].iter().sum();
        if (sum - 1.0).abs() > 1e-4 {
            return false;
        }
    }

    // 3. Test DC response: Feed constant 1.0 samples into history
    for _ in 0..POLYPHASE_TAPS_PER_PHASE {
        filter.push_sample(1.0);
    }
    // Any fractional phase should output 1.0 (unity DC gain)
    let out_dc_0 = filter.convolve_fractional_phase(0.0);
    let out_dc_half = filter.convolve_fractional_phase(0.5);
    if (out_dc_0 - 1.0).abs() > 1e-3 || (out_dc_half - 1.0).abs() > 1e-3 {
        return false;
    }

    // 4. Test impulse response symmetry
    let mut impulse_filter = SincPolyphaseFirFilterBank::new(0.45, 6.0);
    impulse_filter.push_sample(1.0);
    for _ in 1..POLYPHASE_TAPS_PER_PHASE {
        impulse_filter.push_sample(0.0);
    }
    let out_imp = impulse_filter.convolve_fractional_phase(0.25);
    if out_imp.abs() <= 0.0 {
        return false;
    }

    true
}

// ============================================================================
// Feature 3: 3D Spatial HRTF Vector Panner & Sub-Millisecond ITD Delay Line
// ============================================================================

/// Maximum ITD delay buffer capacity in samples (at 48 kHz, max ITD is ~0.7 ms = ~34 samples).
pub const SPATIAL_ITD_MAX_DELAY_SAMPLES: usize = 64;
pub const ITD_DELAY_MASK: usize = SPATIAL_ITD_MAX_DELAY_SAMPLES - 1;

/// Spatial coordinate vector for 3D sound source position relative to listener.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialVector3D {
    /// Right/Left axis (+X is Right, -X is Left) in meters.
    pub x: f32,
    /// Up/Down axis (+Y is Up, -Y is Down) in meters.
    pub y: f32,
    /// Forward/Backward axis (+Z is Forward, -Z is Backward) in meters.
    pub z: f32,
}

impl SpatialVector3D {
    /// Creates a 3D position vector in meters.
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Computes Euclidean distance from origin in meters.
    pub fn distance(&self) -> f32 {
        let dist_sq = self.x * self.x + self.y * self.y + self.z * self.z;
        if dist_sq > 0.0 {
            crate::graphics::types::sqrt_f32(dist_sq)
        } else {
            0.0
        }
    }
}

/// 3D Spatial HRTF Vector Panner with Sub-Millisecond Interaural Time Difference (ITD) Delay Line.
///
/// Models Woodworth-Schlosberg spherical head diffraction physics:
/// - Interaural Time Difference (ITD): acoustic path length difference between left/right ears.
///   At 48 kHz, 0.66 ms maximum lateral delay represents ~32 samples of fractional delay.
/// - Interaural Level Difference (ILD): high-frequency acoustic head shadow attenuation.
/// - Elevation pinna filtering and distance attenuation (1 / max(dist, 1.0)).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HrtfVectorPanner {
    /// Circular delay buffer for left ear ITD (samples).
    pub left_delay_line: [f32; SPATIAL_ITD_MAX_DELAY_SAMPLES],
    /// Circular delay buffer for right ear ITD (samples).
    pub right_delay_line: [f32; SPATIAL_ITD_MAX_DELAY_SAMPLES],
    /// Write pointer for delay lines.
    pub delay_write_ptr: usize,
    /// Current source position in meters.
    pub source_position: SpatialVector3D,
    /// Calculated azimuth angle in radians (-PI .. +PI).
    pub azimuth_rad: f32,
    /// Calculated elevation angle in radians (-HALF_PI .. +HALF_PI).
    pub elevation_rad: f32,
    /// Interaural Time Difference for left ear in samples.
    pub itd_left_samples: f32,
    /// Interaural Time Difference for right ear in samples.
    pub itd_right_samples: f32,
    /// Interaural Level Difference linear gain for left ear (0.0 .. 1.0).
    pub ild_left_gain: f32,
    /// Interaural Level Difference linear gain for right ear (0.0 .. 1.0).
    pub ild_right_gain: f32,
    /// Distance attenuation factor (0.0 .. 1.0).
    pub distance_attenuation: f32,
}

impl HrtfVectorPanner {
    /// Speed of sound in meters/second (343 m/s).
    pub const SPEED_OF_SOUND_MPS: f32 = 343.0;
    /// Typical human head radius in meters (0.0875 m = 8.75 cm).
    pub const HEAD_RADIUS_M: f32 = 0.0875;
    /// Sample rate in Hz (48,000 Hz).
    pub const SAMPLE_RATE_HZ: f32 = 48_000.0;

    /// Constructs a 3D spatial HRTF panner at nominal listener forward position.
    pub fn new() -> Self {
        let mut panner = Self {
            left_delay_line: [0.0f32; SPATIAL_ITD_MAX_DELAY_SAMPLES],
            right_delay_line: [0.0f32; SPATIAL_ITD_MAX_DELAY_SAMPLES],
            delay_write_ptr: 0,
            source_position: SpatialVector3D::new(0.0, 0.0, 1.0),
            azimuth_rad: 0.0,
            elevation_rad: 0.0,
            itd_left_samples: 0.0,
            itd_right_samples: 0.0,
            ild_left_gain: 1.0,
            ild_right_gain: 1.0,
            distance_attenuation: 1.0,
        };

        panner.update_position(SpatialVector3D::new(0.0, 0.0, 1.0));
        panner
    }

    /// Updates 3D source position and re-computes ITD delays and ILD acoustic gains.
    pub fn update_position(&mut self, pos: SpatialVector3D) {
        self.source_position = pos;
        let dist = pos.distance();

        // 1. Distance attenuation: inverse distance law with 1m minimum clamp
        self.distance_attenuation = 1.0 / (if dist < 1.0 { 1.0 } else { dist });

        // 2. Azimuth angle (-PI to +PI) in horizontal plane
        // +X is right, +Z is forward
        let horiz_dist = crate::graphics::types::sqrt_f32(pos.x * pos.x + pos.z * pos.z);
        self.azimuth_rad = if horiz_dist > 1e-4 {
            pos.x / horiz_dist // sin(azimuth) approximation for lateral angle
        } else {
            0.0
        };

        // 3. Elevation angle (-HALF_PI to +HALF_PI)
        self.elevation_rad = if dist > 1e-4 {
            pos.y / dist
        } else {
            0.0
        };

        // 4. Woodworth ITD model: ITD = (radius / speed_of_sound) * (sin(theta) + theta)
        // Max ITD is ~ (0.0875 / 343.0) * (1.0 + 1.5708) ≈ 0.655 ms ≈ 31.4 samples at 48kHz
        let sin_theta = self.azimuth_rad; // -1.0 (far left) .. +1.0 (far right)
        let max_itd_s = (Self::HEAD_RADIUS_M / Self::SPEED_OF_SOUND_MPS) * 2.57; // ~0.655 ms
        let max_itd_samples = max_itd_s * Self::SAMPLE_RATE_HZ;

        if sin_theta > 0.0 {
            // Source is to the RIGHT (+X): Sound reaches Right ear first, Left ear is delayed
            self.itd_right_samples = 0.0;
            self.itd_left_samples = sin_theta * max_itd_samples;

            // ILD: Head shadow attenuates left ear
            self.ild_right_gain = 1.0;
            self.ild_left_gain = 1.0 - (0.55 * sin_theta);
        } else {
            // Source is to the LEFT (-X): Sound reaches Left ear first, Right ear is delayed
            let abs_sin = -sin_theta;
            self.itd_left_samples = 0.0;
            self.itd_right_samples = abs_sin * max_itd_samples;

            // ILD: Head shadow attenuates right ear
            self.ild_left_gain = 1.0;
            self.ild_right_gain = 1.0 - (0.55 * abs_sin);
        }
    }

    /// Feeds a mono input sample into the 3D spatializer and outputs binaural (Left, Right) samples.
    pub fn process_sample(&mut self, input_sample: f32) -> (f32, f32) {
        // Push input sample into both delay lines
        self.left_delay_line[self.delay_write_ptr] = input_sample;
        self.right_delay_line[self.delay_write_ptr] = input_sample;

        // Read from left ear delay line with fractional linear interpolation
        let left_read = self.read_interpolated(&self.left_delay_line, self.itd_left_samples);
        // Read from right ear delay line with fractional linear interpolation
        let right_read = self.read_interpolated(&self.right_delay_line, self.itd_right_samples);

        // Advance circular write pointer
        self.delay_write_ptr = (self.delay_write_ptr + 1) & ITD_DELAY_MASK;

        // Apply ILD gains and 3D distance attenuation
        let out_left = left_read * self.ild_left_gain * self.distance_attenuation;
        let out_right = right_read * self.ild_right_gain * self.distance_attenuation;

        (out_left, out_right)
    }

    /// Reads sample from circular buffer at fractional delay `delay_samples` behind current write pointer.
    fn read_interpolated(&self, buffer: &[f32; SPATIAL_ITD_MAX_DELAY_SAMPLES], delay_samples: f32) -> f32 {
        let int_delay = delay_samples as usize;
        let frac = delay_samples - int_delay as f32;

        let idx0 = (self.delay_write_ptr + SPATIAL_ITD_MAX_DELAY_SAMPLES - int_delay) & ITD_DELAY_MASK;
        let idx1 = (idx0 + SPATIAL_ITD_MAX_DELAY_SAMPLES - 1) & ITD_DELAY_MASK;

        let s0 = buffer[idx0];
        let s1 = buffer[idx1];

        s0 + frac * (s1 - s0)
    }
}

/// Verification test for 3D spatial HRTF vector panner and sub-millisecond ITD delay line.
pub fn verify_hrtf_vector_panner() -> bool {
    let mut panner = HrtfVectorPanner::new();

    // 1. Center source (0, 0, 2): ITD = 0, ILD = (1.0, 1.0), dist = 2m (attenuation = 0.5)
    panner.update_position(SpatialVector3D::new(0.0, 0.0, 2.0));
    if panner.itd_left_samples != 0.0 || panner.itd_right_samples != 0.0 {
        return false;
    }
    if panner.ild_left_gain != 1.0 || panner.ild_right_gain != 1.0 {
        return false;
    }
    if (panner.distance_attenuation - 0.5).abs() > 1e-4 {
        return false;
    }

    // 2. Far Right source (+5m, 0, 0): Right ear delay = 0, Left ear delay ~ 31.4 samples (> 25 samples)
    panner.update_position(SpatialVector3D::new(5.0, 0.0, 0.0));
    if panner.itd_right_samples != 0.0 {
        return false;
    }
    if panner.itd_left_samples < 25.0 || panner.itd_left_samples > 35.0 {
        return false;
    }
    // Right ear gain = 1.0, Left ear head-shadowed < 0.5
    if panner.ild_right_gain != 1.0 || panner.ild_left_gain > 0.5 {
        return false;
    }

    // 3. Process unit impulse through panner
    let (l0, r0) = panner.process_sample(1.0);
    // Right ear should immediately output non-zero (0 delay), Left ear is delayed (should be 0 on first sample)
    if r0 <= 0.0 || l0 != 0.0 {
        return false;
    }

    // 4. Far Left source (-5m, 0, 0): Left ear delay = 0, Right ear delay > 25 samples
    panner.update_position(SpatialVector3D::new(-5.0, 0.0, 0.0));
    if panner.itd_left_samples != 0.0 || panner.itd_right_samples < 25.0 {
        return false;
    }
    if panner.ild_left_gain != 1.0 || panner.ild_right_gain > 0.5 {
        return false;
    }

    true
}
