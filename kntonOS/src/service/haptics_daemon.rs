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
