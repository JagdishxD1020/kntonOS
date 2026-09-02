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
