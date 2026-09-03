//! Win32 WinMM (Windows Multimedia) Legacy WaveOut & MIDI Synthesizer Engine.

#![allow(non_snake_case)]

use alloc::vec::Vec;
pub use super::xaudio2::WaveFormatEx;
use core::mem::size_of;

// MMSYSERR status codes.
pub const MMSYSERR_NOERROR: u32 = 0;
pub const MMSYSERR_ERROR: u32 = 1;
pub const MMSYSERR_BADDEVICEID: u32 = 2;
pub const MMSYSERR_NOTENABLED: u32 = 3;
pub const MMSYSERR_ALLOCATED: u32 = 4;
pub const MMSYSERR_INVALHANDLE: u32 = 5;
pub const MMSYSERR_NODRIVER: u32 = 6;
pub const MMSYSERR_NOMEM: u32 = 7;
pub const MMSYSERR_NOTSUPPORTED: u32 = 8;
pub const MMSYSERR_BADERRNUM: u32 = 9;
pub const MMSYSERR_INVALFLAG: u32 = 10;
pub const MMSYSERR_INVALPARAM: u32 = 11;

// WaveOut error codes.
pub const WAVERR_BADFORMAT: u32 = 32;
pub const WAVERR_STILLPLAYING: u32 = 33;
pub const WAVERR_UNPREPARED: u32 = 34;
pub const WAVERR_SYNC: u32 = 35;

// MIDI error codes.
pub const MIDIERR_UNPREPARED: u32 = 64;
pub const MIDIERR_STILLPLAYING: u32 = 65;
pub const MIDIERR_NOMAP: u32 = 66;
pub const MIDIERR_NOTREADY: u32 = 67;
pub const MIDIERR_NODEVICE: u32 = 68;

// WaveOut constants and capabilities.
pub const WAVE_MAPPER: u32 = 0xFFFFFFFF;
pub const WAVECAPS_PITCH: u32 = 0x0001;
pub const WAVECAPS_PLAYBACKRATE: u32 = 0x0002;
pub const WAVECAPS_VOLUME: u32 = 0x0004;
pub const WAVECAPS_LRVOLUME: u32 = 0x0008;
pub const WAVECAPS_SYNC: u32 = 0x0010;
pub const WAVECAPS_SAMPLEACCURATE: u32 = 0x0020;

// Supported standard waveform formats bitmask.
pub const WAVE_FORMAT_1M08: u32 = 0x00000001;
pub const WAVE_FORMAT_1S08: u32 = 0x00000002;
pub const WAVE_FORMAT_1M16: u32 = 0x00000004;
pub const WAVE_FORMAT_1S16: u32 = 0x00000008;
pub const WAVE_FORMAT_2M08: u32 = 0x00000010;
pub const WAVE_FORMAT_2S08: u32 = 0x00000020;
pub const WAVE_FORMAT_2M16: u32 = 0x00000040;
pub const WAVE_FORMAT_2S16: u32 = 0x00000080;
pub const WAVE_FORMAT_4M08: u32 = 0x00000100;
pub const WAVE_FORMAT_4S08: u32 = 0x00000200;
pub const WAVE_FORMAT_4M16: u32 = 0x00000400;
pub const WAVE_FORMAT_4S16: u32 = 0x00000800;
pub const WAVE_FORMAT_48M16: u32 = 0x00001000;
pub const WAVE_FORMAT_48S16: u32 = 0x00002000;
pub const WAVE_FORMAT_96S16: u32 = 0x00008000;

// WAVEHDR buffer header flags.
pub const WHDR_DONE: u32 = 0x00000001;
pub const WHDR_PREPARED: u32 = 0x00000002;
pub const WHDR_BEGINLOOP: u32 = 0x00000004;
pub const WHDR_ENDLOOP: u32 = 0x00000008;
pub const WHDR_INQUEUE: u32 = 0x00000010;

// MIDIHDR buffer header flags.
pub const MHDR_DONE: u32 = 0x00000001;
pub const MHDR_PREPARED: u32 = 0x00000002;
pub const MHDR_INQUEUE: u32 = 0x00000004;
pub const MHDR_ISSTRM: u32 = 0x00000008;

// MIDI technology types.
pub const MOD_MIDIPORT: u16 = 1;
pub const MOD_SYNTH: u16 = 2;
pub const MOD_SQSYNTH: u16 = 3;
pub const MOD_FMSYNTH: u16 = 4;
pub const MOD_MAPPER: u16 = 5;
pub const MOD_WAVETABLE: u16 = 6;
pub const MOD_SWSYNTH: u16 = 7;

/// Capabilities descriptor for a WinMM WaveOut audio playback device.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WAVEOUTCAPS {
    pub wMid: u16,
    pub wPid: u16,
    pub vDriverVersion: u32,
    pub szPname: [u8; 32],
    pub dwFormats: u32,
    pub wChannels: u16,
    pub wReserved1: u16,
    pub dwSupport: u32,
}

impl Default for WAVEOUTCAPS {
    fn default() -> Self {
        let mut name = [0u8; 32];
        let src = b"KntonOS WaveOut Audio Engine\0";
        let copy_len = src.len().min(32);
        name[..copy_len].copy_from_slice(&src[..copy_len]);

        Self {
            wMid: 1, // MM_MICROSOFT
            wPid: 100,
            vDriverVersion: 0x00010000,
            szPname: name,
            dwFormats: WAVE_FORMAT_1M16
                | WAVE_FORMAT_1S16
                | WAVE_FORMAT_2M16
                | WAVE_FORMAT_2S16
                | WAVE_FORMAT_4M16
                | WAVE_FORMAT_4S16
                | WAVE_FORMAT_48S16
                | WAVE_FORMAT_96S16,
            wChannels: 2,
            wReserved1: 0,
            dwSupport: WAVECAPS_VOLUME | WAVECAPS_LRVOLUME | WAVECAPS_PITCH | WAVECAPS_PLAYBACKRATE,
        }
    }
}

/// WinMM WaveOut audio buffer header submitted via waveOutPrepareHeader and waveOutWrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WAVEHDR {
    pub data: Vec<u8>,
    pub buffer_length: u32,
    pub bytes_recorded: u32,
    pub user: usize,
    pub flags: u32,
    pub loops: u32,
}

impl Default for WAVEHDR {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            buffer_length: 0,
            bytes_recorded: 0,
            user: 0,
            flags: 0,
            loops: 0,
        }
    }
}

impl WAVEHDR {
    // Creates a new WaveOut header containing the specified audio PCM byte payload.
    pub fn new(pcm_bytes: &[u8], loops: u32) -> Self {
        let len = pcm_bytes.len() as u32;
        Self {
            data: pcm_bytes.to_vec(),
            buffer_length: len,
            bytes_recorded: len,
            user: 0,
            flags: 0,
            loops,
        }
    }
}

/// Capabilities descriptor for a WinMM MIDI output synthesizer device.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MIDIOUTCAPS {
    pub wMid: u16,
    pub wPid: u16,
    pub vDriverVersion: u32,
    pub szPname: [u8; 32],
    pub wTechnology: u16,
    pub wVoices: u16,
    pub wNotes: u16,
    pub wChannelMask: u16,
    pub dwSupport: u32,
}

impl Default for MIDIOUTCAPS {
    fn default() -> Self {
        let mut name = [0u8; 32];
        let src = b"KntonOS MIDI Wavetable Synth\0";
        let copy_len = src.len().min(32);
        name[..copy_len].copy_from_slice(&src[..copy_len]);

        Self {
            wMid: 1,
            wPid: 200,
            vDriverVersion: 0x00010000,
            szPname: name,
            wTechnology: MOD_WAVETABLE,
            wVoices: 16,
            wNotes: 16,
            wChannelMask: 0xFFFF,
            dwSupport: 0x0001, // MIDICAPS_VOLUME
        }
    }
}

/// WinMM MIDI buffer header for sysex and stream messages.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MIDIHDR {
    pub data: Vec<u8>,
    pub buffer_length: u32,
    pub bytes_recorded: u32,
    pub user: usize,
    pub flags: u32,
    pub offset: u32,
}

/// Verifies memory sizes, capability flags, and status definitions for WinMM types.
pub fn verify_winmm_types_and_descriptors() -> bool {
    if size_of::<WAVEOUTCAPS>() != 52 || size_of::<MIDIOUTCAPS>() != 52 {
        return false;
    }
    let wave_caps = WAVEOUTCAPS::default();
    if wave_caps.wChannels != 2 || (wave_caps.dwSupport & WAVECAPS_VOLUME) == 0 {
        return false;
    }
    if (wave_caps.dwFormats & WAVE_FORMAT_48S16) == 0 {
        return false;
    }

    let midi_caps = MIDIOUTCAPS::default();
    if midi_caps.wTechnology != MOD_WAVETABLE || midi_caps.wVoices != 16 {
        return false;
    }

    let hdr = WAVEHDR::new(&[0x12, 0x34, 0x56, 0x78], 1);
    if hdr.buffer_length != 4 || hdr.flags != 0 || hdr.loops != 1 {
        return false;
    }

    true
}
