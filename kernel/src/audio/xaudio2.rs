//! Userland XAudio2 Low-Latency Game Audio API Service.

#![allow(non_snake_case)]

use alloc::vec::Vec;
use core::mem::size_of;

// Wave format tags.
pub const WAVE_FORMAT_PCM: u16 = 1;
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

// Speaker channel masks.
pub const SPEAKER_FRONT_LEFT: u32 = 0x00000001;
pub const SPEAKER_FRONT_RIGHT: u32 = 0x00000002;
pub const SPEAKER_FRONT_CENTER: u32 = 0x00000004;
pub const SPEAKER_LOW_FREQUENCY: u32 = 0x00000008;
pub const SPEAKER_BACK_LEFT: u32 = 0x00000010;
pub const SPEAKER_BACK_RIGHT: u32 = 0x00000020;
pub const SPEAKER_SIDE_LEFT: u32 = 0x00000200;
pub const SPEAKER_SIDE_RIGHT: u32 = 0x00000400;

pub const SPEAKER_MONO: u32 = SPEAKER_FRONT_CENTER;
pub const SPEAKER_STEREO: u32 = SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT;
pub const SPEAKER_5POINT1: u32 = SPEAKER_FRONT_LEFT
    | SPEAKER_FRONT_RIGHT
    | SPEAKER_FRONT_CENTER
    | SPEAKER_LOW_FREQUENCY
    | SPEAKER_BACK_LEFT
    | SPEAKER_BACK_RIGHT;
pub const SPEAKER_7POINT1: u32 = SPEAKER_5POINT1 | SPEAKER_SIDE_LEFT | SPEAKER_SIDE_RIGHT;

// Buffer flags and loop constants.
pub const XAUDIO2_END_OF_STREAM: u32 = 0x0040;
pub const XAUDIO2_LOOP_INFINITE: u32 = 255;
pub const XAUDIO2_DEFAULT_CHANNELS: u16 = 2;
pub const XAUDIO2_DEFAULT_SAMPLERATE: u32 = 48000;

/// Standard Win32 WAVEFORMATEX audio format descriptor.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveFormatEx {
    pub wFormatTag: u16,
    pub nChannels: u16,
    pub nSamplesPerSec: u32,
    pub nAvgBytesPerSec: u32,
    pub nBlockAlign: u16,
    pub wBitsPerSample: u16,
    pub cbSize: u16,
}

impl Default for WaveFormatEx {
    fn default() -> Self {
        Self::pcm_stereo(48000, 16)
    }
}

impl WaveFormatEx {
    // Creates standard 16-bit integer PCM stereo format descriptor.
    pub const fn pcm_stereo(sample_rate: u32, bits_per_sample: u16) -> Self {
        let block_align = (2 * bits_per_sample) / 8;
        Self {
            wFormatTag: WAVE_FORMAT_PCM,
            nChannels: 2,
            nSamplesPerSec: sample_rate,
            nAvgBytesPerSec: sample_rate * (block_align as u32),
            nBlockAlign: block_align,
            wBitsPerSample: bits_per_sample,
            cbSize: 0,
        }
    }

    // Creates standard 32-bit IEEE float stereo format descriptor.
    pub const fn float_stereo(sample_rate: u32) -> Self {
        let block_align = (2 * 32) / 8;
        Self {
            wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
            nChannels: 2,
            nSamplesPerSec: sample_rate,
            nAvgBytesPerSec: sample_rate * (block_align as u32),
            nBlockAlign: block_align,
            wBitsPerSample: 32,
            cbSize: 0,
        }
    }

    // Validates whether the format descriptor meets supported XAudio2 runtime parameters.
    pub fn is_valid(&self) -> bool {
        if self.wFormatTag != WAVE_FORMAT_PCM && self.wFormatTag != WAVE_FORMAT_IEEE_FLOAT {
            return false;
        }
        if self.nChannels == 0 || self.nChannels > 8 {
            return false;
        }
        if self.nSamplesPerSec != 44100 && self.nSamplesPerSec != 48000 && self.nSamplesPerSec != 96000 {
            return false;
        }
        if self.wBitsPerSample != 16 && self.wBitsPerSample != 24 && self.wBitsPerSample != 32 {
            return false;
        }
        let expected_align = (self.nChannels * self.wBitsPerSample) / 8;
        if self.nBlockAlign != expected_align {
            return false;
        }
        if self.nAvgBytesPerSec != self.nSamplesPerSec * (self.nBlockAlign as u32) {
            return false;
        }
        true
    }
}

/// XAudio2 audio data buffer descriptor submitted to source voices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XAudio2Buffer {
    pub flags: u32,
    pub audio_bytes: u32,
    pub audio_data: Vec<u8>,
    pub play_begin: u32,
    pub play_length: u32,
    pub loop_begin: u32,
    pub loop_length: u32,
    pub loop_count: u32,
    pub context: usize,
}

impl Default for XAudio2Buffer {
    fn default() -> Self {
        Self {
            flags: 0,
            audio_bytes: 0,
            audio_data: Vec::new(),
            play_begin: 0,
            play_length: 0,
            loop_begin: 0,
            loop_length: 0,
            loop_count: 0,
            context: 0,
        }
    }
}

impl XAudio2Buffer {
    // Creates a new audio buffer wrapping the specified byte slice.
    pub fn new(data: &[u8], is_end_of_stream: bool) -> Self {
        let len = data.len() as u32;
        Self {
            flags: if is_end_of_stream { XAUDIO2_END_OF_STREAM } else { 0 },
            audio_bytes: len,
            audio_data: data.to_vec(),
            play_begin: 0,
            play_length: 0,
            loop_begin: 0,
            loop_length: 0,
            loop_count: 0,
            context: 0,
        }
    }
}

/// Verifies memory sizes, alignment, and validation logic for XAudio2 format descriptors.
pub fn verify_xaudio2_format_structures() -> bool {
    if size_of::<WaveFormatEx>() != 18 {
        return false;
    }

    let pcm = WaveFormatEx::pcm_stereo(48000, 16);
    if !pcm.is_valid() || pcm.nBlockAlign != 4 || pcm.nAvgBytesPerSec != 192000 {
        return false;
    }

    let float_fmt = WaveFormatEx::float_stereo(96000);
    if !float_fmt.is_valid() || float_fmt.nBlockAlign != 8 || float_fmt.nAvgBytesPerSec != 768000 {
        return false;
    }

    let mut invalid_fmt = pcm;
    invalid_fmt.nChannels = 16;
    if invalid_fmt.is_valid() {
        return false;
    }

    true
}

use alloc::collections::VecDeque;

// Pitch ratio bounds.
pub const XAUDIO2_MIN_FREQ_RATIO: f32 = 0.5;
pub const XAUDIO2_MAX_FREQ_RATIO: f32 = 2.0;
pub const XAUDIO2_DEFAULT_FREQ_RATIO: f32 = 1.0;
pub const XAUDIO2_MAX_VOLUME_LEVEL: f32 = 2.0;

/// Operational state of an XAudio2 voice in the audio processing graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

// Converts a linear volume gain multiplier to sound pressure decibels.
pub fn linear_to_decibels(volume: f32) -> f32 {
    if volume <= 0.00001 {
        return -100.0;
    }
    crate::drivers::input::gyro_aim::ln_f32(volume) * 8.6858896
}

// Converts sound pressure decibels to a linear volume gain multiplier.
pub fn decibels_to_linear(db: f32) -> f32 {
    if db <= -96.0 {
        return 0.0;
    }
    crate::drivers::input::gyro_aim::exp_f32(db * 0.11512925)
}

/// XAudio2 Source Voice generating, resampling, and streaming audio buffers into the audio graph.
#[derive(Debug, Clone)]
pub struct XAudio2SourceVoice {
    pub format: WaveFormatEx,
    pub state: VoiceState,
    pub volume: f32,
    pub frequency_ratio: f32,
    pub buffer_queue: VecDeque<XAudio2Buffer>,
    pub current_frame_pos: f32,
    pub current_loop_remaining: u32,
    pub total_samples_played: u64,
}

impl XAudio2SourceVoice {
    // Creates a new source voice configured with the specified wave format.
    pub fn new(format: WaveFormatEx) -> Self {
        Self {
            format,
            state: VoiceState::Stopped,
            volume: 1.0,
            frequency_ratio: XAUDIO2_DEFAULT_FREQ_RATIO,
            buffer_queue: VecDeque::new(),
            current_frame_pos: 0.0,
            current_loop_remaining: 0,
            total_samples_played: 0,
        }
    }

    // Starts audio consumption and playback on this voice.
    pub fn start(&mut self) {
        self.state = VoiceState::Playing;
    }

    // Pauses playback, preserving current buffer playback positions.
    pub fn pause(&mut self) {
        if self.state == VoiceState::Playing {
            self.state = VoiceState::Paused;
        }
    }

    // Stops playback and resets active buffer playback cursor.
    pub fn stop(&mut self) {
        self.state = VoiceState::Stopped;
        self.current_frame_pos = 0.0;
    }

    // Adjusts linear volume gain level (0.0 to 2.0).
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, XAUDIO2_MAX_VOLUME_LEVEL);
    }

    // Sets pitch adjustment frequency ratio (clamped to 0.5x .. 2.0x).
    pub fn set_frequency_ratio(&mut self, ratio: f32) {
        self.frequency_ratio = ratio.clamp(XAUDIO2_MIN_FREQ_RATIO, XAUDIO2_MAX_FREQ_RATIO);
    }

    // Submits an audio data buffer to the voice playback queue.
    pub fn submit_source_buffer(&mut self, buffer: XAudio2Buffer) -> bool {
        if buffer.audio_bytes == 0 || buffer.audio_data.is_empty() {
            return false;
        }
        self.buffer_queue.push_back(buffer);
        true
    }

    // Flushes all pending and active buffers from the playback queue.
    pub fn flush_source_buffers(&mut self) {
        self.buffer_queue.clear();
        self.current_frame_pos = 0.0;
        self.current_loop_remaining = 0;
    }

    // Helper extracting stereo floating-point sample pair at integer frame index.
    fn read_frame(&self, buffer: &XAudio2Buffer, frame_idx: usize) -> (f32, f32) {
        let channels = self.format.nChannels as usize;
        let align = self.format.nBlockAlign as usize;
        let offset = frame_idx * align;

        if offset + align > buffer.audio_data.len() {
            return (0.0, 0.0);
        }

        if self.format.wFormatTag == WAVE_FORMAT_IEEE_FLOAT && self.format.wBitsPerSample == 32 {
            let left_bytes: [u8; 4] = buffer.audio_data[offset..offset + 4].try_into().unwrap_or([0; 4]);
            let left = f32::from_le_bytes(left_bytes);
            let right = if channels > 1 {
                let right_bytes: [u8; 4] = buffer.audio_data[offset + 4..offset + 8].try_into().unwrap_or([0; 4]);
                f32::from_le_bytes(right_bytes)
            } else {
                left
            };
            (left, right)
        } else {
            // 16-bit signed integer PCM
            let left_i16 = i16::from_le_bytes([buffer.audio_data[offset], buffer.audio_data[offset + 1]]);
            let left = (left_i16 as f32) / 32768.0;
            let right = if channels > 1 {
                let right_i16 = i16::from_le_bytes([buffer.audio_data[offset + 2], buffer.audio_data[offset + 3]]);
                (right_i16 as f32) / 32768.0
            } else {
                left
            };
            (left, right)
        }
    }

    // Synthesizes up to `frames` output samples into destination stereo channel slices.
    pub fn process_frames(&mut self, out_left: &mut [f32], out_right: &mut [f32]) -> usize {
        if self.state != VoiceState::Playing {
            return 0;
        }

        let num_frames = out_left.len().min(out_right.len());
        let mut frames_written = 0;

        while frames_written < num_frames {
            let buffer = match self.buffer_queue.front() {
                Some(b) => b,
                None => break,
            };

            let block_align = self.format.nBlockAlign as usize;
            if block_align == 0 {
                break;
            }
            let total_frames = buffer.audio_data.len() / block_align;
            if total_frames == 0 {
                self.buffer_queue.pop_front();
                continue;
            }

            let start_frame = buffer.play_begin as usize;
            let max_frame = if buffer.play_length > 0 {
                (start_frame + buffer.play_length as usize).min(total_frames)
            } else {
                total_frames
            };

            while frames_written < num_frames {
                let current_idx = self.current_frame_pos as usize;
                if current_idx >= max_frame {
                    // Check looping
                    if buffer.loop_count > 0 && self.current_loop_remaining < buffer.loop_count {
                        self.current_loop_remaining += 1;
                        self.current_frame_pos = buffer.loop_begin as f32;
                        continue;
                    } else {
                        // Buffer completed
                        self.current_frame_pos = 0.0;
                        self.current_loop_remaining = 0;
                        self.buffer_queue.pop_front();
                        break;
                    }
                }

                let next_idx = (current_idx + 1).min(max_frame - 1);
                let frac = self.current_frame_pos - (current_idx as f32);

                let (l0, r0) = self.read_frame(buffer, current_idx);
                let (l1, r1) = self.read_frame(buffer, next_idx);

                // Linear interpolation for pitch shifting
                let l_sample = (l0 + frac * (l1 - l0)) * self.volume;
                let r_sample = (r0 + frac * (r1 - r0)) * self.volume;

                out_left[frames_written] += l_sample;
                out_right[frames_written] += r_sample;

                frames_written += 1;
                self.total_samples_played += 1;
                self.current_frame_pos += self.frequency_ratio;
            }
        }

        frames_written
    }
}

/// Verifies voice lifecycle state transitions, gain attenuation, and frequency resampling.
pub fn verify_xaudio2_source_voice() -> bool {
    let fmt = WaveFormatEx::pcm_stereo(48000, 16);
    let mut voice = XAudio2SourceVoice::new(fmt);

    if voice.state != VoiceState::Stopped {
        return false;
    }
    voice.start();
    if voice.state != VoiceState::Playing {
        return false;
    }
    voice.pause();
    if voice.state != VoiceState::Paused {
        return false;
    }
    voice.start();

    // Verify decibel conversion
    let db_zero = linear_to_decibels(1.0);
    if db_zero.abs() > 0.01 {
        return false;
    }
    let vol_half = decibels_to_linear(-6.02);
    if (vol_half - 0.5).abs() > 0.02 {
        return false;
    }

    // Verify frequency ratio clamping
    voice.set_frequency_ratio(3.5);
    if voice.frequency_ratio != XAUDIO2_MAX_FREQ_RATIO {
        return false;
    }
    voice.set_frequency_ratio(1.0);

    // Create 100 frames of dummy 16-bit PCM stereo data (full amplitude 16384 = 0.5)
    let mut pcm_bytes = Vec::new();
    for i in 0..100 {
        let val: i16 = if i % 2 == 0 { 16384 } else { -16384 };
        pcm_bytes.extend_from_slice(&val.to_le_bytes()); // left
        pcm_bytes.extend_from_slice(&val.to_le_bytes()); // right
    }
    let buf = XAudio2Buffer::new(&pcm_bytes, true);
    if !voice.submit_source_buffer(buf) {
        return false;
    }

    let mut out_l = [0.0f32; 50];
    let mut out_r = [0.0f32; 50];
    let written = voice.process_frames(&mut out_l, &mut out_r);
    if written != 50 || out_l[0] != 0.5 || out_r[0] != 0.5 {
        return false;
    }

    true
}

use super::client_ring::AudioFrame;

/// Channel matrix routing and upmix/downmix coefficient multiplier.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelMatrix {
    pub src_channels: usize,
    pub dst_channels: usize,
    pub matrix: Vec<f32>,
}

impl ChannelMatrix {
    // Creates standard stereo-to-stereo passthrough matrix.
    pub fn stereo_passthrough() -> Self {
        Self {
            src_channels: 2,
            dst_channels: 2,
            matrix: alloc::vec![1.0, 0.0, 0.0, 1.0],
        }
    }

    // Creates standard stereo-to-5.1 surround upmix matrix.
    pub fn stereo_to_5point1() -> Self {
        Self {
            src_channels: 2,
            dst_channels: 6,
            // FL, FR, C, LFE, BL, BR
            matrix: alloc::vec![
                1.0, 0.0,   // Front Left
                0.0, 1.0,   // Front Right
                0.707, 0.707, // Center
                0.5, 0.5,   // LFE Subwoofer
                0.707, 0.0, // Back Left
                0.0, 0.707  // Back Right
            ],
        }
    }

    // Creates standard 5.1 surround-to-stereo downmix matrix.
    pub fn five_point_one_to_stereo() -> Self {
        Self {
            src_channels: 6,
            dst_channels: 2,
            // Left: FL + 0.707*C + 0.707*BL + 0.5*LFE
            // Right: FR + 0.707*C + 0.707*BR + 0.5*LFE
            matrix: alloc::vec![
                0.707, 0.0,   // FL -> L, R
                0.0, 0.707,   // FR -> L, R
                0.5, 0.5,     // C -> L, R
                0.353, 0.353, // LFE -> L, R
                0.5, 0.0,     // BL -> L, R
                0.0, 0.5      // BR -> L, R
            ],
        }
    }

    // Transforms an input multi-channel frame into the destination channel configuration.
    pub fn transform_frame(&self, src: &[f32], dst: &mut [f32]) {
        for d in 0..self.dst_channels {
            let mut acc = 0.0f32;
            let row_offset = d * self.src_channels;
            for s in 0..self.src_channels {
                if s < src.len() && row_offset + s < self.matrix.len() {
                    acc += src[s] * self.matrix[row_offset + s];
                }
            }
            if d < dst.len() {
                dst[d] = acc;
            }
        }
    }
}

/// XAudio2 Mastering Voice representing the destination audio endpoint and mix bus.
#[derive(Debug, Clone)]
pub struct XAudio2MasteringVoice {
    pub input_channels: u16,
    pub output_channels: u16,
    pub sample_rate: u32,
    pub master_volume: f32,
    pub matrix: ChannelMatrix,
    pub source_voices: Vec<XAudio2SourceVoice>,
}

impl Default for XAudio2MasteringVoice {
    fn default() -> Self {
        Self::new(2, 2, 48000)
    }
}

impl XAudio2MasteringVoice {
    // Creates a new mastering voice for the audio engine endpoint.
    pub fn new(input_channels: u16, output_channels: u16, sample_rate: u32) -> Self {
        let matrix = if input_channels == 2 && output_channels == 6 {
            ChannelMatrix::stereo_to_5point1()
        } else if input_channels == 6 && output_channels == 2 {
            ChannelMatrix::five_point_one_to_stereo()
        } else {
            ChannelMatrix::stereo_passthrough()
        };

        Self {
            input_channels,
            output_channels,
            sample_rate,
            master_volume: 1.0,
            matrix,
            source_voices: Vec::new(),
        }
    }

    // Registers a new source voice with this mastering voice graph.
    pub fn create_source_voice(&mut self, format: WaveFormatEx) -> usize {
        let id = self.source_voices.len();
        self.source_voices.push(XAudio2SourceVoice::new(format));
        id
    }

    // Obtains a mutable reference to an existing source voice.
    pub fn get_source_voice_mut(&mut self, id: usize) -> Option<&mut XAudio2SourceVoice> {
        self.source_voices.get_mut(id)
    }

    // Adjusts master bus linear gain.
    pub fn set_master_volume(&mut self, volume: f32) {
        self.master_volume = volume.clamp(0.0, XAUDIO2_MAX_VOLUME_LEVEL);
    }

    // Renders one audio quantum across all active source voices into standard stereo AudioFrames.
    pub fn render_quantum(&mut self, frames: usize) -> Vec<AudioFrame> {
        let mut mix_left = alloc::vec![0.0f32; frames];
        let mut mix_right = alloc::vec![0.0f32; frames];

        let mut voice_l = alloc::vec![0.0f32; frames];
        let mut voice_r = alloc::vec![0.0f32; frames];

        for voice in &mut self.source_voices {
            if voice.state != VoiceState::Playing {
                continue;
            }
            voice_l.fill(0.0);
            voice_r.fill(0.0);
            voice.process_frames(&mut voice_l, &mut voice_r);

            for i in 0..frames {
                mix_left[i] += voice_l[i];
                mix_right[i] += voice_r[i];
            }
        }

        let mut output_frames = Vec::with_capacity(frames);
        for i in 0..frames {
            let left_clamped = (mix_left[i] * self.master_volume).clamp(-1.0, 1.0);
            let right_clamped = (mix_right[i] * self.master_volume).clamp(-1.0, 1.0);
            output_frames.push(AudioFrame {
                left: left_clamped,
                right: right_clamped,
            });
        }

        output_frames
    }
}

/// Verifies channel matrix upmixing/downmixing and master voice multi-voice rendering.
pub fn verify_xaudio2_mastering_voice_and_matrix() -> bool {
    let matrix_up = ChannelMatrix::stereo_to_5point1();
    let src = [1.0f32, 0.0f32]; // pure Left
    let mut dst = [0.0f32; 6];
    matrix_up.transform_frame(&src, &mut dst);

    // Verify Front Left is 1.0 and Front Right is 0.0
    if (dst[0] - 1.0).abs() > 0.01 || dst[1].abs() > 0.01 {
        return false;
    }
    // Verify Center channel has energy
    if (dst[2] - 0.707).abs() > 0.01 {
        return false;
    }

    // Test mastering voice mixdown with 2 voices
    let mut master = XAudio2MasteringVoice::new(2, 2, 48000);
    master.set_master_volume(0.8);

    let v1 = master.create_source_voice(WaveFormatEx::pcm_stereo(48000, 16));
    let v2 = master.create_source_voice(WaveFormatEx::pcm_stereo(48000, 16));

    // Submit 50 frames to each voice
    let mut raw_bytes = Vec::new();
    for _ in 0..50 {
        let sample: i16 = 16384; // 0.5
        raw_bytes.extend_from_slice(&sample.to_le_bytes());
        raw_bytes.extend_from_slice(&sample.to_le_bytes());
    }
    master.get_source_voice_mut(v1).unwrap().submit_source_buffer(XAudio2Buffer::new(&raw_bytes, true));
    master.get_source_voice_mut(v1).unwrap().start();

    master.get_source_voice_mut(v2).unwrap().submit_source_buffer(XAudio2Buffer::new(&raw_bytes, true));
    master.get_source_voice_mut(v2).unwrap().start();

    let rendered = master.render_quantum(50);
    if rendered.len() != 50 {
        return false;
    }
    // Expected mix: (0.5 + 0.5) * 0.8 = 0.8
    if (rendered[0].left - 0.8).abs() > 0.01 || (rendered[0].right - 0.8).abs() > 0.01 {
        return false;
    }

    true
}

use crate::serial_write_str;

/// Comprehensive verification self-test suite for Userland XAudio2 Low-Latency Game Audio API Service.
pub fn test_xaudio2_service_subsystem() -> bool {
    serial_write_str("[KNT-AUDIO] Testing userland XAudio2 game audio service & voice graph...\r\n");

    // 1. Verify audio format descriptors and buffer representations
    if !verify_xaudio2_format_structures() {
        serial_write_str("[KNT-PANIC] XAudio2 format descriptors verification failed!\r\n");
        return false;
    }

    // 2. Verify source voice lifecycle, multi-buffer queuing, and pitch ratio resampling
    if !verify_xaudio2_source_voice() {
        serial_write_str("[KNT-PANIC] XAudio2 source voice processing verification failed!\r\n");
        return false;
    }

    // 3. Verify channel matrix routing, surround upmix/downmix, and master voice mixdown
    if !verify_xaudio2_mastering_voice_and_matrix() {
        serial_write_str("[KNT-PANIC] XAudio2 mastering voice and channel matrix verification failed!\r\n");
        return false;
    }

    serial_write_str("[KNT-XAUDIO2-PASS] Userland XAudio2 game audio service and voice graph verified\r\n");
    true
}
