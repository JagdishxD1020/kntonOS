//! Handheld Userspace Audio & Acoustic Governor Service (TASK-KNT-323).
//!
//! Provides dynamic audio endpoint routing, logarithmic volume scaling, master acoustic
//! loudness limiting, low-latency PCM frame mixing, and headphone hotplug coordination.

#![allow(dead_code)]

// ============================================================================
// Feature 1: Audio Sink Routing & Endpoint Management
// ============================================================================

/// Physical audio output sink endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioEndpoint {
    /// Built-in stereo micro-speakers with acoustic enclosure.
    InternalSpeakers,
    /// 3.5mm analog stereo headphone/headset TRRS jack.
    HeadphoneJack35mm,
    /// External USB Audio Class 2.0 DAC / docking station.
    UsbAudioDac,
    /// Bluetooth A2DP wireless audio sink (aptX / LDAC).
    BluetoothA2dp,
}

/// Endpoint state descriptor.
#[derive(Debug, Clone, Copy)]
pub struct EndpointDescriptor {
    pub endpoint: AudioEndpoint,
    pub is_connected: bool,
    pub sample_rate_hz: u32,
    pub channels: u8,
}

impl EndpointDescriptor {
    pub const fn new(endpoint: AudioEndpoint, is_connected: bool) -> Self {
        Self {
            endpoint,
            is_connected,
            sample_rate_hz: 48000,
            channels: 2, // Stereo
        }
    }
}

/// Dynamic audio routing arbiter.
#[derive(Debug, Clone, Copy)]
pub struct AudioRouter {
    pub endpoints: [EndpointDescriptor; 4],
    pub active_sink: AudioEndpoint,
}

impl AudioRouter {
    /// Initializes router with internal speakers active by default.
    pub const fn new() -> Self {
        Self {
            endpoints: [
                EndpointDescriptor::new(AudioEndpoint::InternalSpeakers, true),
                EndpointDescriptor::new(AudioEndpoint::HeadphoneJack35mm, false),
                EndpointDescriptor::new(AudioEndpoint::UsbAudioDac, false),
                EndpointDescriptor::new(AudioEndpoint::BluetoothA2dp, false),
            ],
            active_sink: AudioEndpoint::InternalSpeakers,
        }
    }

    /// Updates connection status of an endpoint and recalculates preferred routing.
    pub fn set_endpoint_connected(&mut self, endpoint: AudioEndpoint, connected: bool) {
        let idx = endpoint as usize;
        self.endpoints[idx].is_connected = connected;
        self.auto_route_preferred_sink();
    }

    /// Selects active sink with priority: Headphone 3.5mm > USB DAC > Bluetooth > Speakers.
    pub fn auto_route_preferred_sink(&mut self) {
        if self.endpoints[AudioEndpoint::HeadphoneJack35mm as usize].is_connected {
            self.active_sink = AudioEndpoint::HeadphoneJack35mm;
        } else if self.endpoints[AudioEndpoint::UsbAudioDac as usize].is_connected {
            self.active_sink = AudioEndpoint::UsbAudioDac;
        } else if self.endpoints[AudioEndpoint::BluetoothA2dp as usize].is_connected {
            self.active_sink = AudioEndpoint::BluetoothA2dp;
        } else {
            self.active_sink = AudioEndpoint::InternalSpeakers;
        }
    }

    /// Explicitly forces a specific active sink if connected.
    pub fn force_select_sink(&mut self, endpoint: AudioEndpoint) -> bool {
        let idx = endpoint as usize;
        if self.endpoints[idx].is_connected {
            self.active_sink = endpoint;
            true
        } else {
            false
        }
    }
}

/// Verification test for audio sink routing and endpoint management.
pub fn verify_audio_sink_routing() -> bool {
    let mut router = AudioRouter::new();

    // 1. Initial state: only internal speakers connected and active
    if router.active_sink != AudioEndpoint::InternalSpeakers {
        return false;
    }
    if !router.endpoints[AudioEndpoint::InternalSpeakers as usize].is_connected {
        return false;
    }

    // 2. Connect USB Audio DAC: should auto-route to USB DAC
    router.set_endpoint_connected(AudioEndpoint::UsbAudioDac, true);
    if router.active_sink != AudioEndpoint::UsbAudioDac {
        return false;
    }

    // 3. Connect 3.5mm headphones: should take higher priority over USB DAC
    router.set_endpoint_connected(AudioEndpoint::HeadphoneJack35mm, true);
    if router.active_sink != AudioEndpoint::HeadphoneJack35mm {
        return false;
    }

    // 4. Unplug 3.5mm headphones: should fall back to USB DAC
    router.set_endpoint_connected(AudioEndpoint::HeadphoneJack35mm, false);
    if router.active_sink != AudioEndpoint::UsbAudioDac {
        return false;
    }

    // 5. Unplug USB DAC: should fall back to internal speakers
    router.set_endpoint_connected(AudioEndpoint::UsbAudioDac, false);
    if router.active_sink != AudioEndpoint::InternalSpeakers {
        return false;
    }

    true
}

// ============================================================================
// Feature 2: Per-Stream Volume Curve & Logarithmic dB Attenuation Scaler
// ============================================================================

/// Audio stream classification channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    Master = 0,
    GameAudio = 1,
    VoiceChat = 2,
    SystemUi = 3,
    MusicPlayback = 4,
}

pub const TOTAL_STREAMS: usize = 5;

/// High-precision volume governor implementing perceptual logarithmic attenuation.
#[derive(Debug, Clone, Copy)]
pub struct VolumeScaler {
    /// Slider volume percentages (0 to 100%).
    pub volumes: [u8; TOTAL_STREAMS],
    /// Explicit mute flags.
    pub muted: [bool; TOTAL_STREAMS],
}

impl VolumeScaler {
    /// Initializes default volumes (Master 80%, Game 100%, Voice 90%, UI 60%, Music 75%).
    pub const fn new() -> Self {
        Self {
            volumes: [80, 100, 90, 60, 75],
            muted: [false; TOTAL_STREAMS],
        }
    }

    /// Sets slider volume level (0 to 100%).
    pub fn set_volume(&mut self, stream: StreamType, pct: u8) {
        self.volumes[stream as usize] = pct.min(100);
    }

    /// Sets mute flag.
    pub fn set_mute(&mut self, stream: StreamType, muted: bool) {
        self.muted[stream as usize] = muted;
    }

    /// Maps linear 0-100 percentage to perceptual quadratic/logarithmic Q8 amplitude scalar (0 to 256).
    pub fn calculate_q8_scalar(pct: u8) -> u16 {
        let p = pct as u32;
        // Perceptual audio curve: Amp = (Pct / 100)^2 * 256
        ((p * p * 256) / 10000) as u16
    }

    /// Calculates composite effective Q8 amplitude scalar for a stream including Master.
    pub fn get_effective_scalar(&self, stream: StreamType) -> u16 {
        // Master mute cuts all audio
        if self.muted[StreamType::Master as usize] {
            return 0;
        }

        let stream_idx = stream as usize;
        if self.muted[stream_idx] {
            return 0;
        }

        let master_scalar = Self::calculate_q8_scalar(self.volumes[StreamType::Master as usize]) as u32;
        if stream == StreamType::Master {
            return master_scalar as u16;
        }

        let stream_scalar = Self::calculate_q8_scalar(self.volumes[stream_idx]) as u32;
        ((master_scalar * stream_scalar) / 256) as u16
    }

    /// Scales a 16-bit PCM audio sample according to stream and master attenuation.
    pub fn scale_sample(&self, stream: StreamType, sample: i16) -> i16 {
        let scalar = self.get_effective_scalar(stream) as i32;
        if scalar == 0 {
            return 0;
        }
        let scaled = (sample as i32 * scalar) / 256;
        scaled.clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }
}

/// Verification test for per-stream volume curve and logarithmic attenuation scaler.
pub fn verify_volume_curve_scaler() -> bool {
    let mut scaler = VolumeScaler::new();

    // 1. Initial defaults test: 100% volume should map to full scale 256
    let full = VolumeScaler::calculate_q8_scalar(100);
    if full != 256 {
        return false;
    }

    // 50% volume (-12 dB) should map to 64
    let half = VolumeScaler::calculate_q8_scalar(50);
    if half != 64 {
        return false;
    }

    // 0% volume should map to 0
    let zero = VolumeScaler::calculate_q8_scalar(0);
    if zero != 0 {
        return false;
    }

    // 2. Test master unity (100%) with game audio at 50%
    scaler.set_volume(StreamType::Master, 100);
    scaler.set_volume(StreamType::GameAudio, 50);
    if scaler.get_effective_scalar(StreamType::GameAudio) != 64 {
        return false;
    }

    // Scale a max PCM peak (10000) by 50% amplitude: 10000 * 64 / 256 = 2500
    let scaled_pcm = scaler.scale_sample(StreamType::GameAudio, 10000);
    if scaled_pcm != 2500 {
        return false;
    }

    // 3. Test Master Mute: should silence GameAudio regardless of game slider
    scaler.set_mute(StreamType::Master, true);
    if scaler.get_effective_scalar(StreamType::GameAudio) != 0 || scaler.scale_sample(StreamType::GameAudio, 10000) != 0 {
        return false;
    }

    // Unmute master, test individual stream mute
    scaler.set_mute(StreamType::Master, false);
    scaler.set_mute(StreamType::VoiceChat, true);
    if scaler.get_effective_scalar(StreamType::VoiceChat) != 0 {
        return false;
    }

    true
}

// ============================================================================
// Feature 3: Master Acoustic Loudness Limiter & Anti-Clipping Dynamics Processor
// ============================================================================

/// Dynamic Range Compressor (DRC) and anti-clipping peak limiter.
#[derive(Debug, Clone, Copy)]
pub struct AcousticLimiter {
    /// Normal operating ceiling threshold (e.g. 28000 / 32767 = -1.3 dBFS).
    pub standard_ceiling: i16,
    /// Power-saving ceiling threshold under low battery (e.g. 20000 = -4.3 dBFS).
    pub battery_safe_ceiling: i16,
    /// Battery safe mode active.
    pub battery_safe_mode: bool,
    /// Current instantaneous gain reduction scalar in Q8 (256 = unity, < 256 = gain reduced).
    pub current_gain_q8: u16,
    /// Counter tracking peak samples clamped by limiter to prevent digital wrap.
    pub clips_prevented: u32,
    /// Maximum absolute peak observed during current monitoring window.
    pub peak_hold: i16,
}

impl AcousticLimiter {
    /// Initializes acoustic limiter with handheld speaker protection profile.
    pub const fn new() -> Self {
        Self {
            standard_ceiling: 28000,
            battery_safe_ceiling: 20000,
            battery_safe_mode: false,
            current_gain_q8: 256,
            clips_prevented: 0,
            peak_hold: 0,
        }
    }

    /// Toggles battery safe loudness mode to protect speaker amplifier rails.
    pub fn set_battery_safe_mode(&mut self, enabled: bool) {
        self.battery_safe_mode = enabled;
    }

    /// Returns the active ceiling threshold.
    pub const fn active_ceiling(&self) -> i16 {
        if self.battery_safe_mode {
            self.battery_safe_ceiling
        } else {
            self.standard_ceiling
        }
    }

    /// Processes an individual 16-bit PCM sample through the anti-clipping limiter.
    pub fn process_sample(&mut self, sample: i16) -> i16 {
        let abs_sample = if sample == i16::MIN {
            i16::MAX as u32
        } else {
            sample.abs() as u32
        };

        if abs_sample as i16 > self.peak_hold {
            self.peak_hold = abs_sample as i16;
        }

        let ceiling = self.active_ceiling() as u32;

        if abs_sample <= ceiling {
            // Below threshold: smooth release towards unity gain (256)
            if self.current_gain_q8 < 256 {
                self.current_gain_q8 = (self.current_gain_q8 + 1).min(256);
            }
            sample
        } else {
            // Above threshold: fast attack peak limiting
            self.clips_prevented = self.clips_prevented.saturating_add(1);
            let target_gain = ((ceiling * 256) / abs_sample) as u16;
            self.current_gain_q8 = target_gain.min(self.current_gain_q8);

            // Apply limiter attenuation with sign preservation
            let limited = (sample as i32 * self.current_gain_q8 as i32) / 256;
            limited.clamp(-(ceiling as i32), ceiling as i32) as i16
        }
    }

    /// Resets maximum peak hold register.
    pub fn reset_peak_hold(&mut self) {
        self.peak_hold = 0;
    }
}

/// Verification test for master acoustic loudness limiter and anti-clipping dynamics processor.
pub fn verify_acoustic_limiter() -> bool {
    let mut limiter = AcousticLimiter::new();

    // 1. Initial state checks
    if limiter.active_ceiling() != 28000 || limiter.current_gain_q8 != 256 || limiter.clips_prevented != 0 {
        return false;
    }

    // 2. Normal sample within headroom (15000) passes through unattenuated
    let out_normal = limiter.process_sample(15000);
    if out_normal != 15000 || limiter.current_gain_q8 != 256 || limiter.clips_prevented != 0 {
        return false;
    }

    // 3. Excessive burst sample exceeding ceiling (32000): should be clamped to <= 28000
    let out_clipped = limiter.process_sample(32000);
    if out_clipped > 28000 || limiter.clips_prevented != 1 || limiter.current_gain_q8 >= 256 {
        return false;
    }

    // 4. Test battery safe mode (ceiling drops to 20000)
    limiter.set_battery_safe_mode(true);
    if limiter.active_ceiling() != 20000 {
        return false;
    }

    // 25000 sample now exceeds 20000 ceiling, must be limited to <= 20000
    let out_bat = limiter.process_sample(25000);
    if out_bat > 20000 || limiter.clips_prevented < 2 {
        return false;
    }

    // 5. Check negative peak clipping symmetry
    let out_neg = limiter.process_sample(-30000);
    if out_neg < -20000 {
        return false;
    }

    true
}

// ============================================================================
// Feature 4: Low-Latency Ring-Buffer Audio Frame Mixer & PCM Sample Pipeline
// ============================================================================

/// Number of stereo sample pairs per low-latency audio processing frame (64 samples = 1.33ms @ 48kHz).
pub const AUDIO_FRAME_SAMPLES: usize = 64;

/// Stereo 16-bit PCM audio frame buffer.
#[derive(Debug, Clone, Copy)]
pub struct AudioPcmFrame {
    pub samples_l: [i16; AUDIO_FRAME_SAMPLES],
    pub samples_r: [i16; AUDIO_FRAME_SAMPLES],
}

impl AudioPcmFrame {
    /// Initializes zero-filled silent frame.
    pub const fn new() -> Self {
        Self {
            samples_l: [0; AUDIO_FRAME_SAMPLES],
            samples_r: [0; AUDIO_FRAME_SAMPLES],
        }
    }

    /// Clears both stereo channels to silence.
    pub fn clear(&mut self) {
        self.samples_l.fill(0);
        self.samples_r.fill(0);
    }
}

/// Low-latency multi-stream audio frame mixer and PCM pipeline.
#[derive(Debug, Clone, Copy)]
pub struct AudioFrameMixer {
    /// Total audio frames processed.
    pub frames_mixed: u64,
    /// Total peak samples processed.
    pub peak_sample_seen: i16,
}

impl AudioFrameMixer {
    /// Initializes audio frame mixer.
    pub const fn new() -> Self {
        Self {
            frames_mixed: 0,
            peak_sample_seen: 0,
        }
    }

    /// Mixes two input stream frames (e.g. game and voice) into output frame
    /// with per-stream volume scaling and acoustic limiting.
    pub fn mix_streams(
        &mut self,
        game_frame: &AudioPcmFrame,
        voice_frame: &AudioPcmFrame,
        scaler: &VolumeScaler,
        limiter: &mut AcousticLimiter,
        out_frame: &mut AudioPcmFrame,
    ) {
        for i in 0..AUDIO_FRAME_SAMPLES {
            // 1. Scale left channel
            let game_l = scaler.scale_sample(StreamType::GameAudio, game_frame.samples_l[i]);
            let voice_l = scaler.scale_sample(StreamType::VoiceChat, voice_frame.samples_l[i]);
            let mixed_l = game_l.saturating_add(voice_l);
            let limited_l = limiter.process_sample(mixed_l);
            out_frame.samples_l[i] = limited_l;

            // 2. Scale right channel
            let game_r = scaler.scale_sample(StreamType::GameAudio, game_frame.samples_r[i]);
            let voice_r = scaler.scale_sample(StreamType::VoiceChat, voice_frame.samples_r[i]);
            let mixed_r = game_r.saturating_add(voice_r);
            let limited_r = limiter.process_sample(mixed_r);
            out_frame.samples_r[i] = limited_r;

            // Track max peak
            let abs_l = limited_l.abs();
            if abs_l > self.peak_sample_seen {
                self.peak_sample_seen = abs_l;
            }
        }

        self.frames_mixed = self.frames_mixed.saturating_add(1);
    }
}

/// Verification test for low-latency audio frame mixer and PCM sample pipeline.
pub fn verify_audio_frame_mixer() -> bool {
    let mut mixer = AudioFrameMixer::new();
    let mut scaler = VolumeScaler::new();
    let mut limiter = AcousticLimiter::new();

    scaler.set_volume(StreamType::Master, 100);
    scaler.set_volume(StreamType::GameAudio, 100);
    scaler.set_volume(StreamType::VoiceChat, 100);

    let mut game = AudioPcmFrame::new();
    let mut voice = AudioPcmFrame::new();
    let mut out = AudioPcmFrame::new();

    // Populate synthetic test tones: game has 8000 peak, voice has 4000 peak
    for i in 0..AUDIO_FRAME_SAMPLES {
        game.samples_l[i] = 8000;
        game.samples_r[i] = 8000;
        voice.samples_l[i] = 4000;
        voice.samples_r[i] = 4000;
    }

    mixer.mix_streams(&game, &voice, &scaler, &mut limiter, &mut out);

    // Sum should be 8000 + 4000 = 12000 on both channels
    if out.samples_l[0] != 12000 || out.samples_r[0] != 12000 {
        return false;
    }
    if mixer.frames_mixed != 1 || mixer.peak_sample_seen != 12000 {
        return false;
    }

    // Test extreme saturation / limiting: two 20000 signals sum to 40000 -> saturated 32767 -> limited to <= 28000
    for i in 0..AUDIO_FRAME_SAMPLES {
        game.samples_l[i] = 20000;
        voice.samples_l[i] = 20000;
    }
    mixer.mix_streams(&game, &voice, &scaler, &mut limiter, &mut out);
    if out.samples_l[0] > 28000 {
        return false; // Limiter failed to cap output!
    }
    if mixer.frames_mixed != 2 {
        return false;
    }

    true
}

