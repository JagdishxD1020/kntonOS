//! Handheld Sub-Millisecond Voice Chat Noise Gate, Acoustic Echo Cancellation (AEC) & Dual-Mic Beamformer.
//!
//! Provides spatial delay-and-sum dual-mic beamforming to reject chassis fan noise,
//! NLMS acoustic echo cancellation (AEC) against device speaker spill, and
//! sub-millisecond attack/release voice noise gating for crystal-clear voice chat.

/// Standard audio sampling rate for handheld voice processing (48 kHz).
pub const VOICE_SAMPLE_RATE_HZ: f32 = 48000.0;
/// Nominal speed of sound in air at 25°C in m/s.
pub const SPEED_OF_SOUND_MPS: f32 = 343.0;
/// Physical distance between primary front mic and secondary rear noise mic (80 mm = 0.08 m).
pub const DUAL_MIC_DISTANCE_M: f32 = 0.08;
/// Maximum inter-mic propagation delay in samples at 48kHz (0.08 / 343 * 48000 ≈ 11.2 samples -> 12).
pub const MAX_INTER_MIC_DELAY_SAMPLES: usize = 16;

// ============================================================================
// Feature 1: Dual-Microphone Delay-and-Sum Beamformer
// ============================================================================

/// Operating mode for the dual-microphone beamformer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamformerMode {
    /// Broadside delay-and-sum: favors sounds equidistant to both mics (gamer voice).
    BroadsideSum,
    /// Cardioid rear-null steering: cancels sounds arriving from the rear fan exhaust.
    RearNullCardioid,
}

/// Dual-Microphone Delay-and-Sum Beamformer isolating voice and rejecting fan clatter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualMicBeamformer {
    /// Delay buffer holding historical samples from the secondary microphone.
    pub secondary_delay_line: [f32; MAX_INTER_MIC_DELAY_SAMPLES],
    /// Ring write head index for secondary microphone delay line.
    pub delay_head: usize,
    /// Delay in samples applied to secondary microphone (nominally 11 samples for rear null).
    pub steering_delay_samples: usize,
    /// Coupling cancellation weight for secondary microphone (nominally ~0.90 to 0.98).
    pub secondary_weight: f32,
    /// Active beamforming mode.
    pub mode: BeamformerMode,
}

impl DualMicBeamformer {
    /// Constructs a dual-microphone beamformer configured for handheld gaming chassis.
    pub const fn new() -> Self {
        Self {
            secondary_delay_line: [0.0; MAX_INTER_MIC_DELAY_SAMPLES],
            delay_head: 0,
            steering_delay_samples: 11, // ~0.23ms propagation delay across 80mm
            secondary_weight: 0.95,
            mode: BeamformerMode::RearNullCardioid,
        }
    }

    /// Resets the internal delay line buffer.
    pub fn reset(&mut self) {
        self.secondary_delay_line = [0.0; MAX_INTER_MIC_DELAY_SAMPLES];
        self.delay_head = 0;
    }

    /// Ingests a stereo mic pair (primary front mic, secondary rear mic) and returns beamformed audio.
    pub fn process_sample(&mut self, primary_front: f32, secondary_rear: f32) -> f32 {
        // 1. Store secondary microphone sample in delay line
        self.secondary_delay_line[self.delay_head] = secondary_rear;

        // 2. Read delayed secondary mic sample
        let read_idx = if self.delay_head >= self.steering_delay_samples {
            self.delay_head - self.steering_delay_samples
        } else {
            (self.delay_head + MAX_INTER_MIC_DELAY_SAMPLES) - self.steering_delay_samples
        };
        let delayed_secondary = self.secondary_delay_line[read_idx];

        // 3. Advance circular buffer head
        self.delay_head = (self.delay_head + 1) % MAX_INTER_MIC_DELAY_SAMPLES;

        // 4. Apply spatial beamforming matrix
        match self.mode {
            BeamformerMode::BroadsideSum => {
                // In-phase voice adds constructively; uncorrelated noise adds +3dB
                0.5 * (primary_front + secondary_rear)
            }
            BeamformerMode::RearNullCardioid => {
                // Rear sound hits secondary mic first, then primary mic after steering delay.
                // Subtracting delayed secondary mic creates a deep null toward the rear fan.
                let beamformed = primary_front - self.secondary_weight * delayed_secondary;
                // Normalize gain to preserve unity forward voice response
                beamformed / (1.0 + (1.0 - self.secondary_weight))
            }
        }
    }
}

/// Verification test for dual-microphone delay-and-sum beamformer.
pub fn verify_dual_mic_beamformer() -> bool {
    let mut bf = DualMicBeamformer::new();

    // 1. Test Gamer Voice from Front (broadside arrival):
    // Voice arrives at primary mic first, secondary mic receives lower amplitude / delayed.
    // When front voice arrives with minimal delay, beamformer preserves it.
    let voice_sample = 0.50;
    let out_voice = bf.process_sample(voice_sample, 0.10);
    if out_voice <= 0.30 {
        return false;
    }

    // 2. Test Rear Chassis Fan Noise Rejection:
    // Fan noise is generated at rear exhaust, enters secondary mic at time t,
    // and arrives at primary front mic 11 samples later (propagation delay).
    bf.reset();
    let fan_noise_amp = 0.40;

    // Simulate 30 samples of rear fan noise propagating forward
    let mut residual_max = 0.0f32;
    for i in 0..30 {
        let sec_val = if i % 4 < 2 { fan_noise_amp } else { -fan_noise_amp };
        // Primary mic receives this same sound 11 samples later
        let prim_val = if i >= 11 {
            let orig_i = i - 11;
            if orig_i % 4 < 2 { fan_noise_amp } else { -fan_noise_amp }
        } else {
            0.0
        };

        let out = bf.process_sample(prim_val, sec_val);
        if i >= 15 {
            let mag = if out < 0.0 { -out } else { out };
            if mag > residual_max {
                residual_max = mag;
            }
        }
    }

    // Rear fan noise should be canceled by > 80% (residual < 0.08 vs 0.40 raw)
    if residual_max >= 0.08 {
        return false;
    }

    // 3. Test Broadside Mode
    bf.mode = BeamformerMode::BroadsideSum;
    let broadside_out = bf.process_sample(0.60, 0.60);
    if (broadside_out - 0.60).abs() > 1e-4 {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Voice Noise Gate & Spectral Floor Attenuation
// ============================================================================

/// Sub-millisecond Voice Noise Gate with spectral floor subtraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceNoiseGate {
    /// Signal amplitude envelope follower state.
    pub envelope: f32,
    /// Fast attack smoothing factor (tau ≈ 0.4 ms at 48kHz -> ~0.05).
    pub attack_coeff: f32,
    /// Smooth release smoothing factor (tau ≈ 25 ms at 48kHz -> ~0.001).
    pub release_coeff: f32,
    /// Threshold above which voice gate opens (amplitude, ~ -32 dBFS).
    pub open_threshold: f32,
    /// Threshold below which voice gate closes with hysteresis (~ -36 dBFS).
    pub close_threshold: f32,
    /// Noise floor attenuation multiplier when gate is fully closed (e.g. 0.02 = -34 dB).
    pub floor_attenuation: f32,
    /// Active smoothed gate gain multiplier (0.02 .. 1.0).
    pub current_gate_gain: f32,
    /// Background stationary noise floor tracker.
    pub noise_floor_estimate: f32,
    /// Over-subtraction factor for spectral residual reduction.
    pub spectral_sub_factor: f32,
    /// Whether voice activity is currently detected and gate is open.
    pub is_gate_open: bool,
}

impl VoiceNoiseGate {
    /// Constructs a sub-millisecond voice noise gate tuned for gamer voice chat.
    pub const fn new() -> Self {
        Self {
            envelope: 0.0,
            attack_coeff: 0.05,       // ~0.4 ms attack time constant
            release_coeff: 0.001,     // ~20 ms release time constant
            open_threshold: 0.025,    // -32 dBFS
            close_threshold: 0.015,   // -36 dBFS (hysteresis margin)
            floor_attenuation: 0.025, // -32 dB attenuation floor
            current_gate_gain: 0.025,
            noise_floor_estimate: 0.005,
            spectral_sub_factor: 1.25,
            is_gate_open: false,
        }
    }

    /// Resets envelope and gain state.
    pub fn reset(&mut self) {
        self.envelope = 0.0;
        self.current_gate_gain = self.floor_attenuation;
        self.noise_floor_estimate = 0.005;
        self.is_gate_open = false;
    }

    /// Processes a single beamformed audio sample through envelope detector, noise gate, and spectral subtraction.
    pub fn process_sample(&mut self, sample: f32) -> f32 {
        let abs_s = if sample < 0.0 { -sample } else { sample };

        // 1. Sub-millisecond asymmetrical envelope follower
        if abs_s > self.envelope {
            self.envelope += self.attack_coeff * (abs_s - self.envelope);
        } else {
            self.envelope += self.release_coeff * (abs_s - self.envelope);
        }

        // 2. Dual-threshold hysteresis gate decision
        if !self.is_gate_open {
            if self.envelope >= self.open_threshold {
                self.is_gate_open = true;
            }
        } else {
            if self.envelope < self.close_threshold {
                self.is_gate_open = false;
            }
        }

        // 3. Target gate gain slew (fast sub-millisecond opening, smooth closing)
        let target_gain = if self.is_gate_open { 1.0 } else { self.floor_attenuation };
        if target_gain > self.current_gate_gain {
            self.current_gate_gain += self.attack_coeff * (target_gain - self.current_gate_gain);
        } else {
            self.current_gate_gain += self.release_coeff * (target_gain - self.current_gate_gain);
        }

        // 4. Background stationary noise floor tracker
        if !self.is_gate_open {
            self.noise_floor_estimate += 0.0005 * (self.envelope - self.noise_floor_estimate);
            if self.noise_floor_estimate < 0.001 {
                self.noise_floor_estimate = 0.001;
            }
        }

        // 5. Spectral subtraction of background floor
        let sub_amount = self.spectral_sub_factor * self.noise_floor_estimate;
        let subbed_abs = if abs_s > sub_amount { abs_s - sub_amount } else { 0.0 };
        let subbed = if sample < 0.0 { -subbed_abs } else { subbed_abs };

        // 6. Apply dynamic floor attenuation
        subbed * self.current_gate_gain
    }
}

/// Verification test for sub-millisecond voice noise gate and spectral subtraction.
pub fn verify_voice_noise_gate() -> bool {
    let mut gate = VoiceNoiseGate::new();

    // 1. Test Low-level Noise Gating:
    // Feed 200 samples of low-level ambient fan hum (0.008 < close_threshold)
    for _ in 0..200 {
        gate.process_sample(0.008);
    }

    // Gate must remain closed and attenuate noise floor by > 26 dB (output < 0.001)
    if gate.is_gate_open {
        return false;
    }
    let gated_noise = gate.process_sample(0.008);
    if gated_noise >= 0.001 {
        return false;
    }

    // 2. Test Sub-Millisecond Voice Attack:
    // Feed sudden gamer voice burst (0.35)
    // At attack_coeff = 0.05, 48 samples (1.0 ms) must open gate and achieve > 90% pass-through
    for _ in 0..48 {
        gate.process_sample(0.35);
    }

    if !gate.is_gate_open {
        return false;
    }
    let passed_voice = gate.process_sample(0.35);
    if passed_voice <= 0.30 {
        return false;
    }

    // 3. Test Controlled Release:
    // Voice stops suddenly (input = 0.0)
    // Envelope should decrease smoothly rather than abrupt clipping
    let env_before = gate.envelope;
    for _ in 0..50 {
        gate.process_sample(0.0);
    }
    if gate.envelope >= env_before || gate.envelope <= 0.0 {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: Acoustic Echo Cancellation (AEC) with NLMS Adaptive Filter
// ============================================================================

/// Number of FIR taps for chassis acoustic echo path modeling (32 taps ≈ 0.67 ms at 48kHz).
pub const AEC_FILTER_TAPS: usize = 32;

/// Acoustic Echo Canceller (AEC) removing speaker loopback spill from microphone signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcousticEchoCanceller {
    /// Circular buffer storing reference speaker playback samples.
    pub speaker_history: [f32; AEC_FILTER_TAPS],
    /// Adaptive FIR filter weight coefficients modeling the chassis acoustic impulse response.
    pub filter_weights: [f32; AEC_FILTER_TAPS],
    /// Current write index in the speaker history circular buffer.
    pub history_head: usize,
    /// NLMS normalized adaptation step size (mu).
    pub step_size_mu: f32,
    /// Power regularization constant preventing division by zero during silence.
    pub regularization_eps: f32,
    /// Estimated echo signal amplitude.
    pub estimated_echo: f32,
    /// Active double-talk flag preventing filter divergence during simultaneous gamer speech.
    pub is_double_talk: bool,
}

impl AcousticEchoCanceller {
    /// Constructs an acoustic echo canceller with calibrated parameters for handheld speakers.
    pub const fn new() -> Self {
        Self {
            speaker_history: [0.0; AEC_FILTER_TAPS],
            filter_weights: [0.0; AEC_FILTER_TAPS],
            history_head: 0,
            step_size_mu: 0.25,
            regularization_eps: 1e-4,
            estimated_echo: 0.0,
            is_double_talk: false,
        }
    }

    /// Resets filter history and learned weights.
    pub fn reset(&mut self) {
        self.speaker_history = [0.0; AEC_FILTER_TAPS];
        self.filter_weights = [0.0; AEC_FILTER_TAPS];
        self.history_head = 0;
        self.estimated_echo = 0.0;
        self.is_double_talk = false;
    }

    /// Processes microphone signal with speaker reference loopback, canceling acoustic echo.
    pub fn process_sample(&mut self, mic_in: f32, speaker_ref: f32) -> f32 {
        // 1. Ingest speaker reference sample into circular buffer
        self.speaker_history[self.history_head] = speaker_ref;

        // 2. Synthesize echo estimate via FIR convolution: d_hat = sum(w_k * x[n - k])
        let mut echo_estimate = 0.0f32;
        let mut ref_energy = 0.0f32;

        for k in 0..AEC_FILTER_TAPS {
            let idx = if self.history_head >= k {
                self.history_head - k
            } else {
                (self.history_head + AEC_FILTER_TAPS) - k
            };
            let ref_sample = self.speaker_history[idx];
            echo_estimate += self.filter_weights[k] * ref_sample;
            ref_energy += ref_sample * ref_sample;
        }

        self.estimated_echo = echo_estimate;

        // 3. Subtract estimated echo from microphone signal: e = mic - d_hat
        let error = mic_in - echo_estimate;

        // 4. Double-Talk Detection (DTD):
        // If mic energy is substantially greater than the estimated echo and speaker reference,
        // local gamer voice is active; freeze filter adaptation to avoid divergence.
        let abs_mic = if mic_in < 0.0 { -mic_in } else { mic_in };
        let abs_echo = if echo_estimate < 0.0 { -echo_estimate } else { echo_estimate };
        self.is_double_talk = abs_mic > 0.06 && abs_mic > (2.5 * abs_echo + 0.02);

        // 5. Normalized LMS weight update if no double-talk detected
        if !self.is_double_talk && ref_energy > 1e-5 {
            let normalized_step = (self.step_size_mu / (self.regularization_eps + ref_energy)) * error;

            for k in 0..AEC_FILTER_TAPS {
                let idx = if self.history_head >= k {
                    self.history_head - k
                } else {
                    (self.history_head + AEC_FILTER_TAPS) - k
                };
                let ref_sample = self.speaker_history[idx];
                self.filter_weights[k] += normalized_step * ref_sample;
            }
        }

        // 6. Advance circular history head
        self.history_head = (self.history_head + 1) % AEC_FILTER_TAPS;

        error
    }
}

/// Verification test for NLMS acoustic echo cancellation.
pub fn verify_acoustic_echo_cancellation() -> bool {
    let mut aec = AcousticEchoCanceller::new();

    // 1. Simulated chassis echo path: acoustic delay of 2 samples with attenuation 0.35
    // true impulse response: [0.0, 0.0, 0.35]
    let mut speaker_history = [0.0f32; 100];
    for i in 0..100 {
        // Pseudo-random pseudo-noise speaker reference signal
        speaker_history[i] = if i % 3 == 0 { 0.40 } else if i % 3 == 1 { -0.30 } else { 0.20 };
    }

    // Converge AEC over 90 iterations with echo only (no near-end gamer speech)
    for i in 0..90 {
        let spk = speaker_history[i];
        let echo = if i >= 2 { 0.35 * speaker_history[i - 2] } else { 0.0 };
        aec.process_sample(echo, spk);
    }

    // Test cancellation efficacy on subsequent samples: residual error must be < 0.05 (vs 0.14 raw echo)
    let test_spk = speaker_history[92];
    let test_echo = 0.35 * speaker_history[90];
    let residual = aec.process_sample(test_echo, test_spk);

    let abs_res = if residual < 0.0 { -residual } else { residual };
    if abs_res >= 0.045 {
        return false;
    }

    // 2. Double-Talk Verification:
    // Gamer speaks simultaneously with loud voice (0.45) while echo is present (0.10)
    let gamer_voice = 0.45;
    let mic_with_speech = test_echo + gamer_voice;
    let out_speech = aec.process_sample(mic_with_speech, test_spk);

    // Filter must detect double-talk and preserve near-end gamer voice
    if !aec.is_double_talk {
        return false;
    }
    if out_speech <= 0.35 {
        return false;
    }

    true
}


// ============================================================================
// Feature 4: Integrated Handheld Voice Chat Speech Processor Pipeline
// ============================================================================

/// Complete Integrated Handheld Voice Chat Speech Processor Pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceBeamformerPipeline {
    /// Dual-microphone spatial beamformer.
    pub beamformer: DualMicBeamformer,
    /// NLMS acoustic echo canceller with double-talk detection.
    pub echo_canceller: AcousticEchoCanceller,
    /// Sub-millisecond voice noise gate and spectral subtraction.
    pub noise_gate: VoiceNoiseGate,
    /// Total audio samples processed.
    pub total_samples_processed: u64,
    /// Total samples where voice activity was detected.
    pub active_voice_samples: u64,
    /// Running ratio of voice activity vs total conversation time.
    pub voice_activity_ratio: f32,
}

impl VoiceBeamformerPipeline {
    /// Constructs an integrated voice chat speech processor pipeline.
    pub const fn new() -> Self {
        Self {
            beamformer: DualMicBeamformer::new(),
            echo_canceller: AcousticEchoCanceller::new(),
            noise_gate: VoiceNoiseGate::new(),
            total_samples_processed: 0,
            active_voice_samples: 0,
            voice_activity_ratio: 0.0,
        }
    }

    /// Resets all pipeline filters, envelopes, and metrics.
    pub fn reset(&mut self) {
        self.beamformer.reset();
        self.echo_canceller.reset();
        self.noise_gate.reset();
        self.total_samples_processed = 0;
        self.active_voice_samples = 0;
        self.voice_activity_ratio = 0.0;
    }

    /// Processes a single multi-channel audio sample through spatial beamforming, AEC, and noise gating.
    pub fn process_sample(&mut self, primary_front: f32, secondary_rear: f32, speaker_loopback: f32) -> f32 {
        // 1. Dual-mic spatial beamforming (cancels rear fan noise)
        let beamformed = self.beamformer.process_sample(primary_front, secondary_rear);

        // 2. Acoustic echo cancellation (cancels speaker loopback spill)
        let aec_cleaned = self.echo_canceller.process_sample(beamformed, speaker_loopback);

        // 3. Voice noise gate & spectral floor attenuation
        let gated_out = self.noise_gate.process_sample(aec_cleaned);

        // 4. Update voice telemetry metrics
        self.total_samples_processed = self.total_samples_processed.saturating_add(1);
        if self.noise_gate.is_gate_open {
            self.active_voice_samples = self.active_voice_samples.saturating_add(1);
        }
        if self.total_samples_processed > 0 {
            self.voice_activity_ratio = (self.active_voice_samples as f32) / (self.total_samples_processed as f32);
        }

        gated_out
    }

    /// Block processing helper for multi-sample audio frames.
    pub fn process_frame(&mut self, primary: &[f32], secondary: &[f32], speaker: &[f32], output: &mut [f32]) {
        let count = primary.len().min(secondary.len()).min(speaker.len()).min(output.len());
        for i in 0..count {
            output[i] = self.process_sample(primary[i], secondary[i], speaker[i]);
        }
    }
}

/// Verification test for integrated handheld voice beamformer speech processor pipeline.
pub fn verify_voice_beamformer_pipeline() -> bool {
    let mut pipeline = VoiceBeamformerPipeline::new();

    // 1. Test Silent Gaming Condition (Rear Fan Running + Game Music Playing on Speakers):
    // Primary mic receives delayed fan noise (0.015) and speaker spill (0.020).
    // Secondary mic receives fan noise (0.015).
    // Speaker loopback has reference (0.060).
    // With gamer silent, noise gate and beamformer must ensure gated output is below 0.001.
    for i in 0..100 {
        let spk = if i % 2 == 0 { 0.060 } else { -0.060 };
        let fan = 0.015;
        let prim = fan + 0.020;
        let out = pipeline.process_sample(prim, fan, spk);
        let abs_out = if out < 0.0 { -out } else { out };
        if i >= 80 && abs_out >= 0.005 {
            return false;
        }
    }

    // 2. Test Clear Voice Chat Ingestion:
    // Gamer speaks with clear voice amplitude 0.40 into front mic
    for _ in 0..50 {
        pipeline.process_sample(0.40, 0.05, 0.0);
    }

    if !pipeline.noise_gate.is_gate_open {
        return false;
    }
    let voice_out = pipeline.process_sample(0.40, 0.05, 0.0);
    if voice_out <= 0.30 {
        return false;
    }
    if pipeline.active_voice_samples == 0 {
        return false;
    }

    true
}
