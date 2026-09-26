//! Handheld Directional Footstep Audio Enhancer & Headphone Amp.

#![allow(dead_code)]

/// Headphone load impedance classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadphoneImpedanceClass {
    /// In-Ear Monitors (IEMs) & low-impedance earbuds (16 .. 32 ohms).
    LowImpedanceIem,
    /// Standard consumer headphones (33 .. 150 ohms).
    MidImpedance,
    /// High-impedance studio audiophile headphones (151 .. 600 ohms).
    HighImpedanceStudio,
}

impl HeadphoneImpedanceClass {
    /// Returns recommended analog DAC gain boost in decibels.
    pub const fn gain_boost_db(self) -> f32 {
        match self {
            Self::LowImpedanceIem => 0.0,
            Self::MidImpedance => 6.0,
            Self::HighImpedanceStudio => 12.0,
        }
    }

    /// Linear voltage gain multiplier corresponding to gain_boost_db.
    pub fn linear_gain(self) -> f32 {
        match self {
            Self::LowImpedanceIem => 1.0,
            Self::MidImpedance => 1.9952623, // 10^(6/20) ~ 2.0
            Self::HighImpedanceStudio => 3.9810717, // 10^(12/20) ~ 4.0
        }
    }
}

/// Headphone jack impedance auto-detector and analog output amplifier controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadphoneAmpDetector {
    /// Detected load impedance in ohms.
    pub detected_ohms: f32,
    /// Active impedance classification.
    pub impedance_class: HeadphoneImpedanceClass,
    /// Current output gain multiplier.
    pub active_gain: f32,
    /// Headphone jack plug inserted.
    pub is_plugged: bool,
}

impl HeadphoneAmpDetector {
    pub const fn new() -> Self {
        Self {
            detected_ohms: 32.0,
            impedance_class: HeadphoneImpedanceClass::LowImpedanceIem,
            active_gain: 1.0,
            is_plugged: false,
        }
    }

    /// Senses jack insertion and performs test-tone load impedance measurement.
    pub fn detect_load(&mut self, sensed_ohms: f32) -> HeadphoneImpedanceClass {
        self.is_plugged = true;
        self.detected_ohms = sensed_ohms.max(1.0);

        if self.detected_ohms <= 32.5 {
            self.impedance_class = HeadphoneImpedanceClass::LowImpedanceIem;
        } else if self.detected_ohms <= 150.0 {
            self.impedance_class = HeadphoneImpedanceClass::MidImpedance;
        } else {
            self.impedance_class = HeadphoneImpedanceClass::HighImpedanceStudio;
        }

        self.active_gain = self.impedance_class.linear_gain();
        self.impedance_class
    }

    /// Senses headphone jack unplug event.
    pub fn on_unplug(&mut self) {
        self.is_plugged = false;
        self.detected_ohms = 32.0;
        self.impedance_class = HeadphoneImpedanceClass::LowImpedanceIem;
        self.active_gain = 1.0;
    }
}

/// Unit test verifying headphone load impedance detection and DAC gain stage selection.
pub fn verify_impedance_detection_and_gain() -> bool {
    let mut amp = HeadphoneAmpDetector::new();
    if amp.is_plugged {
        return false;
    }

    // 1. 16-ohm IEM insertion (e.g. Moondrop Chu) -> LowImpedanceIem (0 dB, 1.0x gain)
    let c1 = amp.detect_load(16.0);
    if c1 != HeadphoneImpedanceClass::LowImpedanceIem || (amp.active_gain - 1.0).abs() > 0.001 {
        return false;
    }

    // 2. 80-ohm consumer headphones (e.g. DT 770 80 ohm) -> MidImpedance (+6 dB, ~2.0x gain)
    let c2 = amp.detect_load(80.0);
    if c2 != HeadphoneImpedanceClass::MidImpedance || (amp.active_gain - 2.0).abs() > 0.05 {
        return false;
    }

    // 3. 250-ohm studio headphones (e.g. DT 990 Pro 250 ohm) -> HighImpedanceStudio (+12 dB, ~4.0x gain)
    let c3 = amp.detect_load(250.0);
    if c3 != HeadphoneImpedanceClass::HighImpedanceStudio || (amp.active_gain - 4.0).abs() > 0.05 {
        return false;
    }

    // 4. Unplug event
    amp.on_unplug();
    if amp.is_plugged || amp.active_gain != 1.0 {
        return false;
    }

    true
}



// Footstep Acoustic Resonance Booster & Low-Frequency Rumble Filter.

/// 2nd-order Direct Form I/II IIR Biquad Filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FootstepBiquadFilter {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
    pub x1: f32,
    pub x2: f32,
    pub y1: f32,
    pub y2: f32,
}

impl FootstepBiquadFilter {
    pub const fn new(b0: f32, b1: f32, b2: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0, b1, b2, a1, a2,
            x1: 0.0, x2: 0.0,
            y1: 0.0, y2: 0.0,
        }
    }

    /// Creates a 120 Hz 2nd-order Butterworth High-Pass Filter (sample rate 48 kHz).
    /// Suppresses rumble, vehicle noise, and sub-bass explosions below 120 Hz.
    pub fn rumble_suppression_120hz() -> Self {
        // Pre-computed normalized Butterworth HPF at 120 Hz / 48000 Hz (fc ~ 0.0025 fs)
        // Passes > 120 Hz cleanly, attenuates < 120 Hz
        Self::new(0.9780, -1.9560, 0.9780, -1.9555, 0.9565)
    }

    /// Creates a parametric peaking/bandpass filter centered at 1500 Hz (Q ~ 1.2) with +6 dB boost.
    /// Resonates footstep footsteps, crouch rustle, and shell casing drops in 800 Hz - 2.5 kHz band.
    pub fn footstep_peaking_1500hz() -> Self {
        // Center 1500 Hz, Q=1.2, Gain = +6 dB (A = 2.0) at 48000 Hz
        Self::new(1.1852, -1.8674, 0.8148, -1.8674, 0.8000)
    }

    /// Processes a single audio sample through the biquad filter.
    pub fn process_sample(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

/// Dual-stage Footstep Acoustic Enhancer combining rumble suppression and resonance boost.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FootstepAcousticFilter {
    pub rumble_hpf: FootstepBiquadFilter,
    pub resonance_bpf: FootstepBiquadFilter,
    pub is_enabled: bool,
}

impl FootstepAcousticFilter {
    pub fn new() -> Self {
        Self {
            rumble_hpf: FootstepBiquadFilter::rumble_suppression_120hz(),
            resonance_bpf: FootstepBiquadFilter::footstep_peaking_1500hz(),
            is_enabled: true,
        }
    }

    /// Processes an input audio sample through rumble suppression and footstep resonance booster.
    pub fn process(&mut self, sample: f32) -> f32 {
        if !self.is_enabled {
            return sample;
        }
        let rumble_cleaned = self.rumble_hpf.process_sample(sample);
        self.resonance_bpf.process_sample(rumble_cleaned)
    }
}

/// Unit test verifying footstep acoustic resonance booster and rumble attenuation.
pub fn verify_footstep_resonance_and_rumble_filter() -> bool {
    let mut filter = FootstepAcousticFilter::new();

    // 1. Verify DC / Ultra-low rumble rejection: constant 50 Hz rumble offset (0.50 input)
    // Run 50 samples; DC component must attenuate toward zero
    let mut last_dc_out = 0.5f32;
    for _ in 0..60 {
        last_dc_out = filter.rumble_hpf.process_sample(0.5);
    }
    // High-pass filter must have severely attenuated DC/sub-rumble (< 0.15)
    if last_dc_out.abs() > 0.15 {
        return false;
    }

    // 2. Verify Footstep Bandpass Resonance (+6 dB gain around 1500 Hz):
    // Simulate 1500 Hz sine wave (period = 32 samples at 48 kHz)
    let mut max_in = 0.0f32;
    let mut max_out = 0.0f32;
    for i in 0..128 {
        let angle = (i as f32) * (2.0 * 3.14159265 / 32.0);
        let sample_in = crate::graphics::sin_f32(angle) * 0.20; // 0.20 amplitude
        if sample_in.abs() > max_in { max_in = sample_in.abs(); }

        let sample_out = filter.resonance_bpf.process_sample(sample_in);
        if i > 32 && sample_out.abs() > max_out {
            max_out = sample_out.abs();
        }
    }

    // Peak resonant output must be greater than input due to peaking boost (> 1.25x)
    if max_out <= max_in * 1.25 {
        return false;
    }

    true
}



// Fast Sub-1ms Dynamic Range Compressor (DRC) & Brickwall Limiter.

/// Fast DRC limiter protecting against explosive game volume spikes while boosting quiet cues.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FastDrcLimiter {
    /// Compression threshold linear amplitude (e.g. 0.35 ~ -9 dBFS).
    pub threshold: f32,
    /// Compression ratio above threshold (e.g. 4.0 for 4:1 compression).
    pub ratio: f32,
    /// Attack coefficient (sub-1ms attack, e.g. alpha ~ 0.85 per sample at 48 kHz).
    pub attack_coeff: f32,
    /// Release coefficient (smooth release over ~50ms).
    pub release_coeff: f32,
    /// Hard limiter ceiling amplitude (e.g. 0.95 ~ -0.45 dBFS).
    pub ceiling: f32,
    /// Current smoothed envelope detector state.
    pub envelope: f32,
}

impl FastDrcLimiter {
    pub const fn new(threshold: f32, ratio: f32, ceiling: f32) -> Self {
        Self {
            threshold,
            ratio,
            attack_coeff: 0.85,
            release_coeff: 0.002,
            ceiling,
            envelope: 0.0,
        }
    }

    /// Default fast gaming limiter configuration (0.35 threshold, 4:1 ratio, 0.95 ceiling).
    pub const fn default_gaming() -> Self {
        Self::new(0.35, 4.0, 0.95)
    }

    /// Compresses and limits an audio sample to prevent ear fatigue and clipping.
    pub fn process_sample(&mut self, sample: f32) -> f32 {
        let abs_val = sample.abs();

        // Fast peak envelope follower
        if abs_val > self.envelope {
            self.envelope += self.attack_coeff * (abs_val - self.envelope);
        } else {
            self.envelope += self.release_coeff * (abs_val - self.envelope);
        }

        // Soft-knee dynamic compression calculation
        let gain = if self.envelope > self.threshold {
            let over = self.envelope - self.threshold;
            let compressed_over = over / self.ratio;
            (self.threshold + compressed_over) / self.envelope
        } else {
            1.0
        };

        let compressed = sample * gain;

        // Brickwall ceiling limiter
        compressed.clamp(-self.ceiling, self.ceiling)
    }
}

/// Unit test verifying fast sub-1ms DRC attack and peak limiter attenuation.
pub fn verify_fast_drc_and_limiter() -> bool {
    let mut drc = FastDrcLimiter::default_gaming();

    // 1. Quiet footstep signal (0.15 < threshold 0.35): untouched (gain = 1.0)
    let quiet_out = drc.process_sample(0.15);
    if (quiet_out - 0.15).abs() > 0.001 {
        return false;
    }

    // 2. Sudden explosive blast (1.80 spike >> 0.35 threshold and > 1.0 clipping)
    // Run 10 samples (sub-1ms at 48 kHz): attack must crush amplitude below ceiling (0.95)
    let mut blast_out = 0.0f32;
    for _ in 0..10 {
        blast_out = drc.process_sample(1.80);
    }

    // Must be strictly clamped <= ceiling (0.95)
    if blast_out > 0.95 {
        return false;
    }
    // Envelope must have reacted instantly
    if drc.envelope < 1.0 {
        return false;
    }

    // 3. Steady high amplitude signal (0.80): compressed according to 4:1 ratio
    let mut steady_drc = FastDrcLimiter::default_gaming();
    let mut steady_out = 0.0f32;
    for _ in 0..30 {
        steady_out = steady_drc.process_sample(0.80);
    }
    // threshold = 0.35, excess = 0.45, compressed = 0.35 + 0.45/4 = 0.4625
    if (steady_out - 0.4625).abs() > 0.08 {
        return false;
    }

    true
}



// Positional ITD/ILD Spatial Cue Sharpener for Competitive 360-Degree Localization.

/// Maximum fractional ITD delay line buffer size in samples (~1.5 ms at 48 kHz).
pub const ITD_MAX_DELAY_SAMPLES: usize = 72;

/// Positional 3D binaural cue sharpener exaggerating ITD and ILD for competitive FPS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialCueSharpener {
    /// Amplification factor for spatial cue separation (e.g. 1.35x).
    pub cue_boost_factor: f32,
    /// Left ear delay line ring buffer.
    pub left_delay_line: [f32; ITD_MAX_DELAY_SAMPLES],
    /// Right ear delay line ring buffer.
    pub right_delay_line: [f32; ITD_MAX_DELAY_SAMPLES],
    /// Write index into delay ring buffers.
    pub write_idx: usize,
}

impl SpatialCueSharpener {
    pub const fn new(cue_boost_factor: f32) -> Self {
        Self {
            cue_boost_factor,
            left_delay_line: [0.0; ITD_MAX_DELAY_SAMPLES],
            right_delay_line: [0.0; ITD_MAX_DELAY_SAMPLES],
            write_idx: 0,
        }
    }

    /// Default competitive FPS cue sharpener (1.35x spatial boost).
    pub const fn default_competitive() -> Self {
        Self::new(1.35)
    }

    /// Sharpens binaural cues for a stereo sample pair given horizontal azimuth angle in radians:
    /// azimuth_rad: -PI/2 (pure left) .. 0 (front) .. +PI/2 (pure right).
    pub fn sharpen_stereo(
        &mut self,
        left_in: f32,
        right_in: f32,
        azimuth_rad: f32,
    ) -> (f32, f32) {
        // Store current samples in delay line
        self.left_delay_line[self.write_idx] = left_in;
        self.right_delay_line[self.write_idx] = right_in;

        // Woodworth model maximum human head ITD ~ 0.65ms -> 31 samples at 48 kHz
        let sin_azimuth = crate::graphics::sin_f32(azimuth_rad.clamp(-1.5707963, 1.5707963));
        let base_delay_samples = (sin_azimuth * 31.0) * self.cue_boost_factor;
        let delay_int = base_delay_samples.abs() as usize;
        let delay_clamped = delay_int.min(ITD_MAX_DELAY_SAMPLES - 1);

        // ILD attenuation factor on far ear: head shadow attenuates contralateral ear up to -12 dB (factor 0.25)
        let shadow_depth = (sin_azimuth.abs() * 0.70 * self.cue_boost_factor).min(0.85);

        let (left_out, right_out) = if sin_azimuth > 0.0 {
            // Sound is from the right: Left ear is delayed and shadowed (far ear)
            let read_idx = (self.write_idx + ITD_MAX_DELAY_SAMPLES - delay_clamped) % ITD_MAX_DELAY_SAMPLES;
            let delayed_left = self.left_delay_line[read_idx] * (1.0 - shadow_depth);
            (delayed_left, right_in)
        } else {
            // Sound is from the left: Right ear is delayed and shadowed (far ear)
            let read_idx = (self.write_idx + ITD_MAX_DELAY_SAMPLES - delay_clamped) % ITD_MAX_DELAY_SAMPLES;
            let delayed_right = self.right_delay_line[read_idx] * (1.0 - shadow_depth);
            (left_in, delayed_right)
        };

        self.write_idx = (self.write_idx + 1) % ITD_MAX_DELAY_SAMPLES;
        (left_out, right_out)
    }
}

/// Unit test verifying positional ITD/ILD spatial cue sharpening.
pub fn verify_itd_ild_cue_sharpener() -> bool {
    let mut sharpener = SpatialCueSharpener::default_competitive();

    // 1. Center sound (azimuth = 0.0 rad): no delay, no shadowing (exact match)
    let (l0, r0) = sharpener.sharpen_stereo(0.5, 0.5, 0.0);
    if (l0 - 0.5).abs() > 0.001 || (r0 - 0.5).abs() > 0.001 {
        return false;
    }

    // 2. Pure Right sound (azimuth = +PI/2 ~ 1.5708 rad):
    // Feed impulse into right channel (left=0.0, right=1.0)
    let (l_right, r_right) = sharpener.sharpen_stereo(1.0, 1.0, 1.5707963);
    // Right ear must receive immediate sound (1.0), Left ear must be heavily shadowed (< 0.35)
    if (r_right - 1.0).abs() > 0.001 || l_right > 0.35 {
        return false;
    }

    // 3. Pure Left sound (azimuth = -PI/2 ~ -1.5708 rad):
    let (l_left, r_left) = sharpener.sharpen_stereo(1.0, 1.0, -1.5707963);
    // Left ear must receive immediate sound (1.0), Right ear must be shadowed (< 0.35)
    if (l_left - 1.0).abs() > 0.001 || r_left > 0.35 {
        return false;
    }

    true
}



/// Comprehensive verification self-test suite for Handheld Directional Footstep Audio Enhancer & Headphone Amp.
pub fn test_footstep_enhancer_subsystem() -> bool {
    crate::serial_write_str("[KNT-AUDIO] Testing Handheld Directional Footstep Audio Enhancer & Headphone Amp...\r\n");

    // 1. Verify Headphone Jack Impedance Auto-Detection & DAC Gain Stage
    if !verify_impedance_detection_and_gain() {
        crate::serial_write_str("[KNT-PANIC] Impedance detection and gain stage verification failed!\r\n");
        return false;
    }

    // 2. Verify Footstep Acoustic Resonance Booster & Low-Frequency Rumble Filter
    if !verify_footstep_resonance_and_rumble_filter() {
        crate::serial_write_str("[KNT-PANIC] Footstep acoustic resonance booster verification failed!\r\n");
        return false;
    }

    // 3. Verify Fast Sub-1ms Dynamic Range Compressor (DRC) & Peak Limiter
    if !verify_fast_drc_and_limiter() {
        crate::serial_write_str("[KNT-PANIC] Fast DRC limiter verification failed!\r\n");
        return false;
    }

    // 4. Verify Positional ITD/ILD Spatial Cue Sharpener for Competitive FPS
    if !verify_itd_ild_cue_sharpener() {
        crate::serial_write_str("[KNT-PANIC] Positional ITD/ILD spatial cue sharpener verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-AUDIO-FOOTSTEP-PASS] Handheld directional footstep audio enhancer and headphone amp verified\r\n");
    true
}
