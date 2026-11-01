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

