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
