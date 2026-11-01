//! SteamDeck Dual Linear Resonant Actuator (LRA) Audio Haptics & DSP Rumble Driver.

// Designated haptic actuator channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HapticChannel {
    Left,
    Right,
    Both,
}

// Synthesizer excitation waveform shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HapticWaveform {
    Sine,
    Square,
    Triangle,
    Pulse,
}

// Operating phase state of an LRA motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotorMode {
    Idle,
    Vibrating,
    ActiveBraking,
    ResonantCalibration,
}

// Hardware limits and electrical characteristics of an LRA motor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LraMotorConfig {
    pub channel: HapticChannel,
    pub resonant_freq_hz: u16,
    pub min_freq_hz: u16,
    pub max_freq_hz: u16,
    pub max_voltage_mv: u16,
}

impl LraMotorConfig {
    // Construct default electrical configuration for Steam Deck left or right LRA.
    pub const fn default_for(channel: HapticChannel) -> Self {
        let resonant_freq = match channel {
            HapticChannel::Left => 175,
            HapticChannel::Right => 230,
            HapticChannel::Both => 200,
        };
        Self {
            channel,
            resonant_freq_hz: resonant_freq,
            min_freq_hz: 50,
            max_freq_hz: 350,
            max_voltage_mv: 3300,
        }
    }
}

// Waveform synthesis command parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveformParameters {
    pub waveform: HapticWaveform,
    pub frequency_hz: u16,
    pub amplitude_pct: u8,
    pub duration_ms: u16,
    pub phase_deg: u16,
}

impl WaveformParameters {
    // Construct new waveform command clamped to valid operating bounds.
    pub const fn new(
        waveform: HapticWaveform,
        frequency_hz: u16,
        amplitude_pct: u8,
        duration_ms: u16,
    ) -> Self {
        let amp = if amplitude_pct > 100 { 100 } else { amplitude_pct };
        Self {
            waveform,
            frequency_hz,
            amplitude_pct: amp,
            duration_ms,
            phase_deg: 0,
        }
    }
}

// Integer fixed-point wave synthesizer producing discrete LRA coil drive values [-127..=127].
pub struct WaveSynthesizer;

impl WaveSynthesizer {
    // Quadrant sine table for 0..90 degrees scaled 0..127.
    const SINE_QUADRANT: [i8; 16] = [
        0, 12, 25, 37, 49, 61, 72, 82, 92, 101, 109, 115, 121, 124, 126, 127,
    ];

    // Compute approximate sine sample for angle in degrees [0..360].
    pub fn sin_sample(deg: u16) -> i8 {
        let norm_deg = deg % 360;
        let quad = norm_deg / 90;
        let rem = norm_deg % 90;
        let idx = ((rem as u32 * 15) / 90) as usize;

        match quad {
            0 => Self::SINE_QUADRANT[idx],
            1 => Self::SINE_QUADRANT[15 - idx],
            2 => -Self::SINE_QUADRANT[idx],
            _ => -Self::SINE_QUADRANT[15 - idx],
        }
    }

    // Synthesize instantaneous output sample [-127..=127] based on waveform parameters and sample time.
    pub fn sample_at_time(params: &WaveformParameters, time_us: u64) -> i8 {
        if params.amplitude_pct == 0 || params.frequency_hz == 0 {
            return 0;
        }

        // Compute phase angle [0..360)
        let period_us = 1_000_000u64 / params.frequency_hz as u64;
        let cycle_time_us = time_us % period_us;
        let deg = (((cycle_time_us * 360) / period_us) as u16 + params.phase_deg) % 360;

        let raw_val: i8 = match params.waveform {
            HapticWaveform::Sine => Self::sin_sample(deg),
            HapticWaveform::Square => {
                if deg < 180 { 127 } else { -127 }
            }
            HapticWaveform::Triangle => {
                if deg < 180 {
                    ((deg as i32 * 254) / 180 - 127) as i8
                } else {
                    (127 - ((deg - 180) as i32 * 254) / 180) as i8
                }
            }
            HapticWaveform::Pulse => {
                if deg < 45 { 127 } else { 0 }
            }
        };

        // Scale by amplitude percentage
        ((raw_val as i32 * params.amplitude_pct as i32) / 100) as i8
    }
}

// Controller managing a single physical LRA actuator with active braking damping.
pub struct LraMotorController {
    pub config: LraMotorConfig,
    pub mode: MotorMode,
    pub current_params: WaveformParameters,
    pub elapsed_ms: u16,
    pub total_duration_ms: u16,
    pub braking_remaining_ms: u8,
    pub current_pwm_sample: i8,
}

impl LraMotorController {
    // Construct single LRA motor driver instance.
    pub const fn new(config: LraMotorConfig) -> Self {
        Self {
            config,
            mode: MotorMode::Idle,
            current_params: WaveformParameters::new(HapticWaveform::Sine, 175, 0, 0),
            elapsed_ms: 0,
            total_duration_ms: 0,
            braking_remaining_ms: 0,
            current_pwm_sample: 0,
        }
    }

    // Trigger playback of a synthesized waveform packet.
    pub fn play_waveform(&mut self, mut params: WaveformParameters) {
        // Clamp frequency to actuator supported physical range
        params.frequency_hz = params
            .frequency_hz
            .clamp(self.config.min_freq_hz, self.config.max_freq_hz);

        self.current_params = params;
        self.elapsed_ms = 0;
        self.total_duration_ms = params.duration_ms;
        self.braking_remaining_ms = 0;
        self.mode = MotorMode::Vibrating;
    }

    // Stop actuator immediately with active reverse-phase electromagnetic braking pulse.
    pub fn stop_with_braking(&mut self) {
        if self.mode == MotorMode::Vibrating {
            self.mode = MotorMode::ActiveBraking;
            // 6ms reverse counter-EMF pulse to eliminate spring overshoot
            self.braking_remaining_ms = 6;
            self.current_pwm_sample = -((self.current_pwm_sample as i32) / 2) as i8;
        } else {
            self.mode = MotorMode::Idle;
            self.current_pwm_sample = 0;
        }
    }

    // Advance motor internal clock by delta_ms and update active drive sample.
    pub fn tick_ms(&mut self, delta_ms: u16) {
        match self.mode {
            MotorMode::Idle => {
                self.current_pwm_sample = 0;
            }
            MotorMode::Vibrating => {
                self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
                if self.elapsed_ms >= self.total_duration_ms {
                    self.stop_with_braking();
                } else {
                    let time_us = (self.elapsed_ms as u64) * 1000;
                    self.current_pwm_sample =
                        WaveSynthesizer::sample_at_time(&self.current_params, time_us);
                }
            }
            MotorMode::ActiveBraking => {
                if self.braking_remaining_ms <= delta_ms as u8 {
                    self.braking_remaining_ms = 0;
                    self.mode = MotorMode::Idle;
                    self.current_pwm_sample = 0;
                } else {
                    self.braking_remaining_ms -= delta_ms as u8;
                }
            }
            MotorMode::ResonantCalibration => {
                self.current_pwm_sample = 64;
            }
        }
    }

    // Check if motor is actively driven or actively braking.
    pub fn is_active(&self) -> bool {
        self.mode == MotorMode::Vibrating || self.mode == MotorMode::ActiveBraking
    }
}

// Unified dual LRA controller for Steam Deck handheld ergonomics.
pub struct DualLraController {
    pub left: LraMotorController,
    pub right: LraMotorController,
}

impl DualLraController {
    // Construct dual LRA haptic subsystem.
    pub const fn new() -> Self {
        Self {
            left: LraMotorController::new(LraMotorConfig::default_for(HapticChannel::Left)),
            right: LraMotorController::new(LraMotorConfig::default_for(HapticChannel::Right)),
        }
    }

    // Dispatch waveform pulse to designated haptic channels.
    pub fn trigger_haptic(&mut self, channel: HapticChannel, params: WaveformParameters) {
        match channel {
            HapticChannel::Left => self.left.play_waveform(params),
            HapticChannel::Right => self.right.play_waveform(params),
            HapticChannel::Both => {
                self.left.play_waveform(params);
                self.right.play_waveform(params);
            }
        }
    }

    // Advance simulation tick on both motors.
    pub fn tick_ms(&mut self, delta_ms: u16) {
        self.left.tick_ms(delta_ms);
        self.right.tick_ms(delta_ms);
    }

    // Stop all active actuators.
    pub fn stop_all(&mut self) {
        self.left.stop_with_braking();
        self.right.stop_with_braking();
    }
}

// Configuration for real-time audio-to-haptics DSP bass extraction filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioDspFilterConfig {
    pub sample_rate_hz: u32,
    pub cutoff_freq_hz: u16,
    pub threshold_amplitude: i16,
    pub bass_boost_gain: u8,
    pub enabled: bool,
}

impl AudioDspFilterConfig {
    // Construct default 150 Hz sub-bass transcode filter for 48kHz audio streams.
    pub const fn default_48k() -> Self {
        Self {
            sample_rate_hz: 48000,
            cutoff_freq_hz: 150,
            threshold_amplitude: 3200,
            bass_boost_gain: 2,
            enabled: true,
        }
    }
}

// Result of audio PCM chunk analysis for tactile transducer excitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioHapticAnalysis {
    pub punch_detected: bool,
    pub peak_bass_energy: u32,
    pub recommended_amplitude_pct: u8,
    pub pulse_duration_ms: u16,
}

// Low-latency audio-to-haptics DSP transcode filter extracting punch < 150 Hz.
pub struct AudioToHapticsDspFilter {
    pub config: AudioDspFilterConfig,
    lowpass_accumulator: i32,
    envelope_energy: u32,
    total_samples_processed: u64,
    total_punches_detected: u32,
}

impl AudioToHapticsDspFilter {
    // Construct DSP filter instance.
    pub const fn new(config: AudioDspFilterConfig) -> Self {
        Self {
            config,
            lowpass_accumulator: 0,
            envelope_energy: 0,
            total_samples_processed: 0,
            total_punches_detected: 0,
        }
    }

    // Process a single signed 16-bit PCM audio sample through 150 Hz low-pass filter.
    pub fn filter_sample(&mut self, sample: i16) -> i16 {
        if !self.config.enabled {
            return 0;
        }

        // Fixed-point single-pole IIR filter: alpha = (2 * pi * 150) / 48000 ~= 20 / 1024
        let input_val = sample as i32;
        let diff = input_val - (self.lowpass_accumulator >> 10);
        self.lowpass_accumulator += diff * 20;

        let filtered_sample = (self.lowpass_accumulator >> 10).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let abs_val = filtered_sample.unsigned_abs() as u32;

        // Peak envelope follower with instant attack and exponential decay
        if abs_val > self.envelope_energy {
            self.envelope_energy = abs_val;
        } else {
            // Decay by ~1/128 per sample (approx 10ms decay at 48kHz)
            self.envelope_energy = self.envelope_energy.saturating_sub(self.envelope_energy >> 7);
        }

        self.total_samples_processed += 1;
        filtered_sample
    }

    // Analyze a chunk of audio PCM samples and produce haptic rumble parameters.
    pub fn process_chunk(&mut self, samples: &[i16]) -> AudioHapticAnalysis {
        let mut max_filtered_energy = 0u32;

        for &sample in samples {
            let filtered = self.filter_sample(sample);
            let energy = filtered.unsigned_abs() as u32;
            if energy > max_filtered_energy {
                max_filtered_energy = energy;
            }
        }

        let thresh = self.config.threshold_amplitude as u32;
        let punch = max_filtered_energy > thresh;

        let amp_pct = if punch {
            self.total_punches_detected += 1;
            let excess = max_filtered_energy.saturating_sub(thresh);
            // Dynamic range compression and bass boost scaling
            let raw_pct = 20 + (((excess * self.config.bass_boost_gain as u32) * 80) / (32767 - thresh));
            (raw_pct.min(100)) as u8
        } else {
            0
        };

        // Standard tactile transient duration for low-end punch
        let duration = if punch {
            if amp_pct > 60 { 24 } else { 16 }
        } else {
            0
        };

        AudioHapticAnalysis {
            punch_detected: punch,
            peak_bass_energy: max_filtered_energy,
            recommended_amplitude_pct: amp_pct,
            pulse_duration_ms: duration,
        }
    }

    // Transcode audio chunk directly into tactile waveform command for resonant actuator.
    pub fn transcode_to_haptic(&mut self, samples: &[i16], resonant_freq_hz: u16) -> Option<WaveformParameters> {
        let analysis = self.process_chunk(samples);
        if analysis.punch_detected && analysis.recommended_amplitude_pct > 0 {
            Some(WaveformParameters::new(
                HapticWaveform::Sine,
                resonant_freq_hz,
                analysis.recommended_amplitude_pct,
                analysis.pulse_duration_ms,
            ))
        } else {
            None
        }
    }
}

// Haptic tactile feel profile for handheld trackpad interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackpadClickProfile {
    SharpClick,
    SoftTap,
    ReleaseClick,
    Buzz,
    RollingRatchetDetent,
}

// Configuration parameters for trackpad haptic synthesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackpadHapticConfig {
    pub click_intensity_pct: u8,
    pub ratchet_step_units: i32,
    pub detent_damping_pct: u8,
}

impl TrackpadHapticConfig {
    // Construct default ergonomic trackpad tactile feel configuration.
    pub const fn default_config() -> Self {
        Self {
            click_intensity_pct: 85,
            ratchet_step_units: 32,
            detent_damping_pct: 10,
        }
    }
}

// Trackpad haptic click feeling generator simulating mechanical dome clicks and scroll wheel ratchets.
pub struct TrackpadHapticGenerator {
    pub config: TrackpadHapticConfig,
    accumulated_motion: i32,
    total_clicks_emitted: u32,
    total_ratchets_emitted: u32,
}

impl TrackpadHapticGenerator {
    // Construct trackpad haptic tactile generator.
    pub const fn new(config: TrackpadHapticConfig) -> Self {
        Self {
            config,
            accumulated_motion: 0,
            total_clicks_emitted: 0,
            total_ratchets_emitted: 0,
        }
    }

    // Synthesize physical haptic pulse packet for requested click profile.
    pub fn synthesize_profile(&mut self, profile: TrackpadClickProfile, resonant_freq_hz: u16) -> WaveformParameters {
        self.total_clicks_emitted += 1;
        match profile {
            TrackpadClickProfile::SharpClick => {
                // 6ms high-amplitude square/pulse pulse to mimic crisp mechanical dome collapse
                WaveformParameters::new(
                    HapticWaveform::Pulse,
                    resonant_freq_hz,
                    self.config.click_intensity_pct,
                    6,
                )
            }
            TrackpadClickProfile::SoftTap => {
                // Gentle 8ms sine wave for light capacitive touch confirmation
                WaveformParameters::new(
                    HapticWaveform::Sine,
                    resonant_freq_hz,
                    ((self.config.click_intensity_pct as u16 * 40) / 100) as u8,
                    8,
                )
            }
            TrackpadClickProfile::ReleaseClick => {
                // Light 4ms release snap
                WaveformParameters::new(
                    HapticWaveform::Pulse,
                    resonant_freq_hz,
                    ((self.config.click_intensity_pct as u16 * 30) / 100) as u8,
                    4,
                )
            }
            TrackpadClickProfile::Buzz => {
                // 60ms continuous sine buzz for barrier or boundary alert
                WaveformParameters::new(
                    HapticWaveform::Sine,
                    120,
                    ((self.config.click_intensity_pct as u16 * 60) / 100) as u8,
                    60,
                )
            }
            TrackpadClickProfile::RollingRatchetDetent => {
                // Crisp 3ms micro-click for individual scroll notch
                WaveformParameters::new(
                    HapticWaveform::Pulse,
                    resonant_freq_hz + 30,
                    ((self.config.click_intensity_pct as u16 * 55) / 100) as u8,
                    3,
                )
            }
        }
    }

    // Process finger delta movement on trackpad surface and synthesize rolling ratchet detents.
    pub fn on_trackpad_motion(&mut self, delta_units: i32, resonant_freq_hz: u16) -> Option<WaveformParameters> {
        self.accumulated_motion += delta_units;
        let step = self.config.ratchet_step_units;

        if self.accumulated_motion.abs() >= step {
            self.accumulated_motion %= step;
            self.total_ratchets_emitted += 1;
            Some(self.synthesize_profile(TrackpadClickProfile::RollingRatchetDetent, resonant_freq_hz))
        } else {
            None
        }
    }

    // Reset accumulated scroll distance.
    pub fn reset_motion(&mut self) {
        self.accumulated_motion = 0;
    }

    // Retrieve lifetime total click and ratchet count.
    pub fn stats(&self) -> (u32, u32) {
        (self.total_clicks_emitted, self.total_ratchets_emitted)
    }
}

// Operating thermal protection safety classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalSafetyState {
    Normal,
    WarningThrottled,
    CriticalTrip,
}

// Thermal protection limiter and coil overload monitor preventing LRA burnout.
pub struct MotorThermalLimiter {
    pub ambient_temp_c: u8,
    pub estimated_temp_c: u8,
    heat_accumulator: u32,
    pub state: ThermalSafetyState,
    pub trip_count: u32,
    pub throttle_count: u32,
}

impl MotorThermalLimiter {
    // Construct thermal safety monitor with specified chassis ambient baseline.
    pub const fn new(ambient_temp_c: u8) -> Self {
        Self {
            ambient_temp_c,
            estimated_temp_c: ambient_temp_c,
            heat_accumulator: (ambient_temp_c as u32) * 1000,
            state: ThermalSafetyState::Normal,
            trip_count: 0,
            throttle_count: 0,
        }
    }

    // Step thermal simulation by delta_ms given active driving amplitude percentage [0..100].
    pub fn tick_ms(&mut self, amplitude_pct: u8, delta_ms: u16) {
        // Heating power proportional to square of coil current/amplitude: P = I^2 * R
        let p_heat = (amplitude_pct as u32 * amplitude_pct as u32 * 6) / 100;
        let delta = delta_ms as u32;
        let heat_added = p_heat * delta;

        // Passive convective/conductive heat dissipation towards ambient
        let cur_temp = self.heat_accumulator / 1000;
        let diff_to_ambient = cur_temp.saturating_sub(self.ambient_temp_c as u32);
        let heat_dissipated = diff_to_ambient * 6 * delta;

        self.heat_accumulator = self.heat_accumulator
            .saturating_add(heat_added)
            .saturating_sub(heat_dissipated);

        // Keep accumulator at or above ambient
        let min_accum = (self.ambient_temp_c as u32) * 1000;
        if self.heat_accumulator < min_accum {
            self.heat_accumulator = min_accum;
        }

        self.estimated_temp_c = ((self.heat_accumulator / 1000).min(125)) as u8;

        // Evaluate safety state with hysteresis recovery
        match self.state {
            ThermalSafetyState::Normal => {
                if self.estimated_temp_c >= 75 {
                    self.state = ThermalSafetyState::CriticalTrip;
                    self.trip_count += 1;
                } else if self.estimated_temp_c >= 60 {
                    self.state = ThermalSafetyState::WarningThrottled;
                    self.throttle_count += 1;
                }
            }
            ThermalSafetyState::WarningThrottled => {
                if self.estimated_temp_c >= 75 {
                    self.state = ThermalSafetyState::CriticalTrip;
                    self.trip_count += 1;
                } else if self.estimated_temp_c <= 50 {
                    self.state = ThermalSafetyState::Normal;
                }
            }
            ThermalSafetyState::CriticalTrip => {
                if self.estimated_temp_c <= 52 {
                    self.state = ThermalSafetyState::Normal;
                }
            }
        }
    }

    // Filter requested excitation amplitude according to active thermal safety state.
    pub fn regulate_amplitude(&self, requested_amplitude: u8) -> u8 {
        match self.state {
            ThermalSafetyState::Normal => requested_amplitude,
            ThermalSafetyState::WarningThrottled => (requested_amplitude / 2).min(50),
            ThermalSafetyState::CriticalTrip => 0,
        }
    }

    // Check if motor drive is completely inhibited due to thermal emergency.
    pub fn is_tripped(&self) -> bool {
        self.state == ThermalSafetyState::CriticalTrip
    }
}

// Integrated SteamDeck audio-haptic feedback and DSP rumble daemon supervisor.
pub struct SteamDeckHapticsDaemon {
    pub dual_lra: DualLraController,
    pub audio_filter: AudioToHapticsDspFilter,
    pub trackpad_left: TrackpadHapticGenerator,
    pub trackpad_right: TrackpadHapticGenerator,
    pub thermal_left: MotorThermalLimiter,
    pub thermal_right: MotorThermalLimiter,
}

impl SteamDeckHapticsDaemon {
    // Construct initialized haptics daemon supervisor.
    pub const fn new() -> Self {
        Self {
            dual_lra: DualLraController::new(),
            audio_filter: AudioToHapticsDspFilter::new(AudioDspFilterConfig::default_48k()),
            trackpad_left: TrackpadHapticGenerator::new(TrackpadHapticConfig::default_config()),
            trackpad_right: TrackpadHapticGenerator::new(TrackpadHapticConfig::default_config()),
            thermal_left: MotorThermalLimiter::new(35),
            thermal_right: MotorThermalLimiter::new(35),
        }
    }

    // Trigger haptic waveform pulse on specified channel subject to thermal limits.
    pub fn trigger_haptic(&mut self, channel: HapticChannel, mut params: WaveformParameters) -> bool {
        let left_amp = self.thermal_left.regulate_amplitude(params.amplitude_pct);
        let right_amp = self.thermal_right.regulate_amplitude(params.amplitude_pct);

        match channel {
            HapticChannel::Left => {
                if left_amp == 0 {
                    return false;
                }
                params.amplitude_pct = left_amp;
                self.dual_lra.left.play_waveform(params);
                true
            }
            HapticChannel::Right => {
                if right_amp == 0 {
                    return false;
                }
                params.amplitude_pct = right_amp;
                self.dual_lra.right.play_waveform(params);
                true
            }
            HapticChannel::Both => {
                let both_active = left_amp > 0 || right_amp > 0;
                if left_amp > 0 {
                    let mut p = params;
                    p.amplitude_pct = left_amp;
                    self.dual_lra.left.play_waveform(p);
                }
                if right_amp > 0 {
                    let mut p = params;
                    p.amplitude_pct = right_amp;
                    self.dual_lra.right.play_waveform(p);
                }
                both_active
            }
        }
    }

    // Process incoming audio PCM block and dynamically synthesize bass rumble.
    pub fn process_audio_block(&mut self, samples: &[i16]) -> bool {
        let res_left = self.dual_lra.left.config.resonant_freq_hz;
        if let Some(params) = self.audio_filter.transcode_to_haptic(samples, res_left) {
            self.trigger_haptic(HapticChannel::Both, params)
        } else {
            false
        }
    }

    // Trigger capacitive trackpad click feel.
    pub fn trigger_trackpad_click(&mut self, channel: HapticChannel, profile: TrackpadClickProfile) -> bool {
        let resonant = match channel {
            HapticChannel::Left => self.dual_lra.left.config.resonant_freq_hz,
            HapticChannel::Right => self.dual_lra.right.config.resonant_freq_hz,
            HapticChannel::Both => 200,
        };

        let params = match channel {
            HapticChannel::Left => self.trackpad_left.synthesize_profile(profile, resonant),
            HapticChannel::Right => self.trackpad_right.synthesize_profile(profile, resonant),
            HapticChannel::Both => self.trackpad_left.synthesize_profile(profile, resonant),
        };

        self.trigger_haptic(channel, params)
    }

    // Advance daemon clock tick for thermal decay, active playback, and braking.
    pub fn tick_ms(&mut self, delta_ms: u16) {
        let left_amp = self.dual_lra.left.current_params.amplitude_pct;
        let right_amp = self.dual_lra.right.current_params.amplitude_pct;

        self.thermal_left.tick_ms(
            if self.dual_lra.left.is_active() { left_amp } else { 0 },
            delta_ms,
        );
        self.thermal_right.tick_ms(
            if self.dual_lra.right.is_active() { right_amp } else { 0 },
            delta_ms,
        );

        self.dual_lra.tick_ms(delta_ms);
    }
}

// Comprehensive self-test suite verifying Dual LRA, DSP audio transcode, trackpad clicks, and thermal safety.
pub fn verify_haptics_subsystem() -> bool {
    let mut daemon = SteamDeckHapticsDaemon::new();

    // 1. Verify Dual LRA Configuration and Waveform Synthesizer
    if daemon.dual_lra.left.config.resonant_freq_hz != 175 {
        return false;
    }
    if daemon.dual_lra.right.config.resonant_freq_hz != 230 {
        return false;
    }

    if WaveSynthesizer::sin_sample(0) != 0 || WaveSynthesizer::sin_sample(90) != 127 {
        return false;
    }

    let wave_cmd = WaveformParameters::new(HapticWaveform::Sine, 175, 80, 20);
    let launched = daemon.trigger_haptic(HapticChannel::Left, wave_cmd);
    if !launched || !daemon.dual_lra.left.is_active() {
        return false;
    }

    daemon.tick_ms(10);
    if daemon.dual_lra.left.elapsed_ms != 10 {
        return false;
    }

    daemon.dual_lra.left.stop_with_braking();
    if daemon.dual_lra.left.mode != MotorMode::ActiveBraking {
        return false;
    }
    daemon.tick_ms(8);
    if daemon.dual_lra.left.is_active() {
        return false;
    }

    // 2. Verify Audio-to-Haptics DSP Transcode Filter (< 150 Hz)
    let silence = [0i16; 64];
    let silence_res = daemon.audio_filter.process_chunk(&silence);
    if silence_res.punch_detected {
        return false;
    }

    // High amplitude sub-100Hz bass punch pulse
    let mut bass_samples = [0i16; 64];
    for sample in bass_samples.iter_mut() {
        *sample = 20_000;
    }

    let bass_res = daemon.audio_filter.process_chunk(&bass_samples);
    if !bass_res.punch_detected || bass_res.recommended_amplitude_pct == 0 {
        return false;
    }

    let transcode = daemon.audio_filter.transcode_to_haptic(&bass_samples, 175);
    if transcode.is_none() {
        return false;
    }
    let p = transcode.unwrap();
    if p.frequency_hz != 175 || p.amplitude_pct == 0 {
        return false;
    }

    // 3. Verify Trackpad Tactile Profiles and Rolling Ratchet
    let sharp = daemon.trackpad_left.synthesize_profile(TrackpadClickProfile::SharpClick, 175);
    if sharp.duration_ms != 6 || sharp.amplitude_pct != 85 {
        return false;
    }

    let soft = daemon.trackpad_left.synthesize_profile(TrackpadClickProfile::SoftTap, 175);
    if soft.duration_ms != 8 {
        return false;
    }

    // Step delta below ratchet step (threshold 32)
    let detent1 = daemon.trackpad_left.on_trackpad_motion(20, 175);
    if detent1.is_some() {
        return false;
    }
    // Cumulative motion now 20 + 15 = 35 >= 32 -> fires detent
    let detent2 = daemon.trackpad_left.on_trackpad_motion(15, 175);
    if detent2.is_none() {
        return false;
    }
    let (clicks, ratchets) = daemon.trackpad_left.stats();
    if clicks < 3 || ratchets != 1 {
        return false;
    }

    // 4. Verify Motor Thermal Limiter and Overload Protection
    let mut thermal = MotorThermalLimiter::new(35);
    if thermal.state != ThermalSafetyState::Normal {
        return false;
    }

    // Heat up coil with sustained 100% duty-cycle load (P = 600 per ms)
    thermal.tick_ms(100, 50);
    if thermal.estimated_temp_c < 60 || thermal.state != ThermalSafetyState::WarningThrottled {
        return false;
    }
    if thermal.regulate_amplitude(100) != 50 {
        return false;
    }

    // Push into thermal overload trip (>= 75 C)
    thermal.tick_ms(100, 30);
    if !thermal.is_tripped() || thermal.regulate_amplitude(100) != 0 {
        return false;
    }

    // Cool down to safe ambient levels
    thermal.tick_ms(0, 500);
    if thermal.is_tripped() {
        return false;
    }

    true
}
