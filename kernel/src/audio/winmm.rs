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

use alloc::collections::BTreeMap;

// WaveOut device handle identifier.
pub type HWAVEOUT = u32;

// Callback mechanism flags.
pub const CALLBACK_NULL: u32 = 0x00000000;
pub const CALLBACK_WINDOW: u32 = 0x00010000;
pub const CALLBACK_TASK: u32 = 0x00020000;
pub const CALLBACK_FUNCTION: u32 = 0x00030000;
pub const CALLBACK_TYPEMASK: u32 = 0x00070000;

// WaveOut callback notification messages.
pub const WOM_OPEN: u32 = 0x3BB;
pub const WOM_CLOSE: u32 = 0x3BC;
pub const WOM_DONE: u32 = 0x3BD;

// Fixed-point 16.16 scalar for unity pitch / playback rate.
pub const WINMM_FIXED_POINT_UNITY: u32 = 0x00010000;

/// State representation of an active WaveOut device instance.
use alloc::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct WaveOutInstance {
    pub handle: HWAVEOUT,
    pub device_id: u32,
    pub format: WaveFormatEx,
    pub callback: usize,
    pub instance_data: usize,
    pub flags: u32,
    pub volume: u32,
    pub pitch: u32,
    pub playback_rate: u32,
    pub is_paused: bool,
    pub buffer_queue: VecDeque<WAVEHDR>,
    pub playback_byte_offset: usize,
    pub current_loops_remaining: u32,
    pub completed_headers: Vec<WAVEHDR>,
}

impl WaveOutInstance {
    // Creates a new WaveOut instance initialized to full stereo volume.
    pub fn new(handle: HWAVEOUT, device_id: u32, format: WaveFormatEx, callback: usize, instance_data: usize, flags: u32) -> Self {
        Self {
            handle,
            device_id,
            format,
            callback,
            instance_data,
            flags,
            volume: 0xFFFFFFFF,
            pitch: WINMM_FIXED_POINT_UNITY,
            playback_rate: WINMM_FIXED_POINT_UNITY,
            is_paused: false,
            buffer_queue: VecDeque::new(),
            playback_byte_offset: 0,
            current_loops_remaining: 0,
            completed_headers: Vec::new(),
        }
    }

    // Extracts left and right normalized linear gains (0.0 .. 1.0) from packed 32-bit volume.
    pub fn get_linear_gains(&self) -> (f32, f32) {
        let left_u16 = (self.volume & 0xFFFF) as u16;
        let right_u16 = ((self.volume >> 16) & 0xFFFF) as u16;
        ((left_u16 as f32) / 65535.0, (right_u16 as f32) / 65535.0)
    }
}

/// Registry and lifecycle supervisor managing physical and virtual WinMM WaveOut devices.
#[derive(Debug, Clone, Default)]
pub struct WaveOutDeviceRegistry {
    pub default_caps: WAVEOUTCAPS,
    pub instances: BTreeMap<HWAVEOUT, WaveOutInstance>,
    pub next_handle: HWAVEOUT,
}

impl WaveOutDeviceRegistry {
    // Creates a new device registry with next handle starting at 1.
    pub fn new() -> Self {
        Self {
            default_caps: WAVEOUTCAPS::default(),
            instances: BTreeMap::new(),
            next_handle: 1,
        }
    }

    // Win32 waveOutGetNumDevs implementation.
    pub fn wave_out_get_num_devs(&self) -> u32 {
        1
    }

    // Win32 waveOutGetDevCaps implementation.
    pub fn wave_out_get_dev_caps(&self, device_id: u32, caps: &mut WAVEOUTCAPS) -> u32 {
        if device_id != 0 && device_id != WAVE_MAPPER {
            return MMSYSERR_BADDEVICEID;
        }
        *caps = self.default_caps;
        MMSYSERR_NOERROR
    }

    // Win32 waveOutOpen implementation.
    pub fn wave_out_open(
        &mut self,
        device_id: u32,
        format: &WaveFormatEx,
        callback: usize,
        instance_data: usize,
        flags: u32,
    ) -> Result<HWAVEOUT, u32> {
        if device_id != 0 && device_id != WAVE_MAPPER {
            return Err(MMSYSERR_BADDEVICEID);
        }
        if !format.is_valid() {
            return Err(WAVERR_BADFORMAT);
        }

        let handle = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1).max(1);

        let instance = WaveOutInstance::new(handle, device_id, *format, callback, instance_data, flags);
        self.instances.insert(handle, instance);
        Ok(handle)
    }

    // Win32 waveOutClose implementation.
    pub fn wave_out_close(&mut self, handle: HWAVEOUT) -> u32 {
        if self.instances.remove(&handle).is_some() {
            MMSYSERR_NOERROR
        } else {
            MMSYSERR_INVALHANDLE
        }
    }

    // Win32 waveOutGetVolume implementation.
    pub fn wave_out_get_volume(&self, handle: HWAVEOUT, out_vol: &mut u32) -> u32 {
        if let Some(inst) = self.instances.get(&handle) {
            *out_vol = inst.volume;
            MMSYSERR_NOERROR
        } else {
            MMSYSERR_INVALHANDLE
        }
    }

    // Win32 waveOutSetVolume implementation.
    pub fn wave_out_set_volume(&mut self, handle: HWAVEOUT, volume: u32) -> u32 {
        if let Some(inst) = self.instances.get_mut(&handle) {
            inst.volume = volume;
            MMSYSERR_NOERROR
        } else {
            MMSYSERR_INVALHANDLE
        }
    }

    // Win32 waveOutPause implementation.
    pub fn wave_out_pause(&mut self, handle: HWAVEOUT) -> u32 {
        if let Some(inst) = self.instances.get_mut(&handle) {
            inst.is_paused = true;
            MMSYSERR_NOERROR
        } else {
            MMSYSERR_INVALHANDLE
        }
    }

    // Win32 waveOutRestart implementation.
    pub fn wave_out_restart(&mut self, handle: HWAVEOUT) -> u32 {
        if let Some(inst) = self.instances.get_mut(&handle) {
            inst.is_paused = false;
            MMSYSERR_NOERROR
        } else {
            MMSYSERR_INVALHANDLE
        }
    }
}

/// Verifies WaveOut device enumeration, capabilities query, instance opening, and volume manipulation.
pub fn verify_waveout_device_lifecycle() -> bool {
    let mut reg = WaveOutDeviceRegistry::new();
    if reg.wave_out_get_num_devs() != 1 {
        return false;
    }

    let mut caps = WAVEOUTCAPS::default();
    if reg.wave_out_get_dev_caps(0, &mut caps) != MMSYSERR_NOERROR {
        return false;
    }

    // Test open with invalid format
    let mut bad_fmt = WaveFormatEx::default();
    bad_fmt.nChannels = 0;
    if reg.wave_out_open(0, &bad_fmt, 0, 0, CALLBACK_NULL).is_ok() {
        return false;
    }

    // Test open with supported format
    let valid_fmt = WaveFormatEx::pcm_stereo(48000, 16);
    let handle = match reg.wave_out_open(WAVE_MAPPER, &valid_fmt, 0, 0, CALLBACK_NULL) {
        Ok(h) => h,
        Err(_) => return false,
    };

    // Test volume controls (0x80008000 = ~50% each)
    if reg.wave_out_set_volume(handle, 0x80008000) != MMSYSERR_NOERROR {
        return false;
    }
    let mut vol = 0;
    if reg.wave_out_get_volume(handle, &mut vol) != MMSYSERR_NOERROR || vol != 0x80008000 {
        return false;
    }

    let inst = reg.instances.get(&handle).unwrap();
    let (gl, gr) = inst.get_linear_gains();
    if (gl - 0.5).abs() > 0.01 || (gr - 0.5).abs() > 0.01 {
        return false;
    }

    // Test pause and restart
    if reg.wave_out_pause(handle) != MMSYSERR_NOERROR {
        return false;
    }
    if !reg.instances.get(&handle).unwrap().is_paused {
        return false;
    }
    if reg.wave_out_restart(handle) != MMSYSERR_NOERROR {
        return false;
    }
    if reg.instances.get(&handle).unwrap().is_paused {
        return false;
    }

    // Test close and duplicate close
    if reg.wave_out_close(handle) != MMSYSERR_NOERROR {
        return false;
    }
    if reg.wave_out_close(handle) != MMSYSERR_INVALHANDLE {
        return false;
    }

    true
}

impl WaveOutInstance {
    // Processes audio frames from active queue into destination buffers, applying volume attenuation.
    pub fn render_frames(&mut self, frames: usize, out_l: &mut [f32], out_r: &mut [f32]) -> usize {
        if self.is_paused {
            return 0;
        }

        let (gl, gr) = self.get_linear_gains();
        let block_align = self.format.nBlockAlign as usize;
        if block_align == 0 {
            return 0;
        }

        let channels = self.format.nChannels as usize;
        let mut frames_written = 0;

        while frames_written < frames {
            if self.buffer_queue.is_empty() {
                break;
            }

            let (sample_l, sample_r, has_sample, buffer_finished) = {
                let hdr = self.buffer_queue.front().unwrap();
                let data_len = hdr.data.len();
                if self.playback_byte_offset + block_align > data_len {
                    (0.0f32, 0.0f32, false, true)
                } else {
                    let off = self.playback_byte_offset;
                    let s_l = if self.format.wBitsPerSample == 16 {
                        let s_i16 = i16::from_le_bytes([hdr.data[off], hdr.data[off + 1]]);
                        (s_i16 as f32) / 32768.0
                    } else {
                        let s_u8 = hdr.data[off] as f32;
                        (s_u8 - 128.0) / 128.0
                    };

                    let s_r = if channels > 1 {
                        if self.format.wBitsPerSample == 16 {
                            let s_i16 = i16::from_le_bytes([hdr.data[off + 2], hdr.data[off + 3]]);
                            (s_i16 as f32) / 32768.0
                        } else {
                            let s_u8 = hdr.data[off + 1] as f32;
                            (s_u8 - 128.0) / 128.0
                        }
                    } else {
                        s_l
                    };

                    let finished = off + block_align >= data_len;
                    (s_l, s_r, true, finished)
                }
            };

            if has_sample {
                out_l[frames_written] += sample_l * gl;
                out_r[frames_written] += sample_r * gr;
                frames_written += 1;
                self.playback_byte_offset += block_align;
            }

            if buffer_finished {
                let loops = self.buffer_queue.front().unwrap().loops;
                if loops > 1 && self.current_loops_remaining + 1 < loops {
                    self.current_loops_remaining += 1;
                    self.playback_byte_offset = 0;
                } else {
                    let mut done_hdr = self.buffer_queue.pop_front().unwrap();
                    done_hdr.flags &= !WHDR_INQUEUE;
                    done_hdr.flags |= WHDR_DONE;
                    self.completed_headers.push(done_hdr);
                    self.playback_byte_offset = 0;
                    self.current_loops_remaining = 0;
                }
            }
        }

        frames_written
    }
}

impl WaveOutDeviceRegistry {
    // Win32 waveOutPrepareHeader implementation.
    pub fn wave_out_prepare_header(&mut self, handle: HWAVEOUT, hdr: &mut WAVEHDR) -> u32 {
        if !self.instances.contains_key(&handle) {
            return MMSYSERR_INVALHANDLE;
        }
        hdr.flags |= WHDR_PREPARED;
        hdr.flags &= !WHDR_DONE;
        MMSYSERR_NOERROR
    }

    // Win32 waveOutUnprepareHeader implementation.
    pub fn wave_out_unprepare_header(&mut self, handle: HWAVEOUT, hdr: &mut WAVEHDR) -> u32 {
        if !self.instances.contains_key(&handle) {
            return MMSYSERR_INVALHANDLE;
        }
        if (hdr.flags & WHDR_INQUEUE) != 0 {
            return WAVERR_STILLPLAYING;
        }
        hdr.flags &= !WHDR_PREPARED;
        MMSYSERR_NOERROR
    }

    // Win32 waveOutWrite implementation.
    pub fn wave_out_write(&mut self, handle: HWAVEOUT, mut hdr: WAVEHDR) -> u32 {
        if (hdr.flags & WHDR_PREPARED) == 0 {
            return WAVERR_UNPREPARED;
        }
        let inst = match self.instances.get_mut(&handle) {
            Some(i) => i,
            None => return MMSYSERR_INVALHANDLE,
        };

        hdr.flags |= WHDR_INQUEUE;
        hdr.flags &= !WHDR_DONE;
        inst.buffer_queue.push_back(hdr);
        MMSYSERR_NOERROR
    }

    // Win32 waveOutReset implementation.
    pub fn wave_out_reset(&mut self, handle: HWAVEOUT) -> u32 {
        let inst = match self.instances.get_mut(&handle) {
            Some(i) => i,
            None => return MMSYSERR_INVALHANDLE,
        };

        while let Some(mut hdr) = inst.buffer_queue.pop_front() {
            hdr.flags &= !WHDR_INQUEUE;
            hdr.flags |= WHDR_DONE;
            inst.completed_headers.push(hdr);
        }
        inst.playback_byte_offset = 0;
        MMSYSERR_NOERROR
    }
}

/// Verifies WaveOut header preparation, queueing semantics, and stream consumption.
pub fn verify_waveout_buffer_queue_and_prepare() -> bool {
    let mut reg = WaveOutDeviceRegistry::new();
    let fmt = WaveFormatEx::pcm_stereo(48000, 16);
    let handle = reg.wave_out_open(WAVE_MAPPER, &fmt, 0, 0, CALLBACK_NULL).unwrap();

    let mut raw_pcm = Vec::new();
    for _ in 0..64 {
        let sample: i16 = 16384; // 0.5
        raw_pcm.extend_from_slice(&sample.to_le_bytes());
        raw_pcm.extend_from_slice(&sample.to_le_bytes());
    }
    let mut hdr = WAVEHDR::new(&raw_pcm, 1);

    // Write before prepare must fail
    if reg.wave_out_write(handle, hdr.clone()) != WAVERR_UNPREPARED {
        return false;
    }

    // Prepare header
    if reg.wave_out_prepare_header(handle, &mut hdr) != MMSYSERR_NOERROR {
        return false;
    }
    if (hdr.flags & WHDR_PREPARED) == 0 {
        return false;
    }

    // Unprepare while not in queue succeeds
    if reg.wave_out_unprepare_header(handle, &mut hdr) != MMSYSERR_NOERROR {
        return false;
    }
    if (hdr.flags & WHDR_PREPARED) != 0 {
        return false;
    }

    // Re-prepare and write
    reg.wave_out_prepare_header(handle, &mut hdr);
    if reg.wave_out_write(handle, hdr.clone()) != MMSYSERR_NOERROR {
        return false;
    }

    // Render 64 frames
    let mut out_l = alloc::vec![0.0f32; 64];
    let mut out_r = alloc::vec![0.0f32; 64];
    let written = reg.instances.get_mut(&handle).unwrap().render_frames(64, &mut out_l, &mut out_r);
    if written != 64 || (out_l[0] - 0.5).abs() > 0.01 {
        return false;
    }

    // Header should now be in completed_headers marked DONE
    let inst = reg.instances.get(&handle).unwrap();
    if inst.completed_headers.is_empty() || (inst.completed_headers[0].flags & WHDR_DONE) == 0 {
        return false;
    }

    true
}

// Standard MIDI Channel Status opcodes.
pub const MIDI_STATUS_NOTE_OFF: u8 = 0x80;
pub const MIDI_STATUS_NOTE_ON: u8 = 0x90;
pub const MIDI_STATUS_POLY_KEY_PRESSURE: u8 = 0xA0;
pub const MIDI_STATUS_CONTROL_CHANGE: u8 = 0xB0;
pub const MIDI_STATUS_PROGRAM_CHANGE: u8 = 0xC0;
pub const MIDI_STATUS_CHANNEL_PRESSURE: u8 = 0xD0;
pub const MIDI_STATUS_PITCH_BEND: u8 = 0xE0;
pub const MIDI_STATUS_SYSTEM_RESET: u8 = 0xFF;

// Common MIDI Control Change controllers.
pub const MIDI_CC_BANK_SELECT: u8 = 0;
pub const MIDI_CC_MODULATION: u8 = 1;
pub const MIDI_CC_VOLUME: u8 = 7;
pub const MIDI_CC_PAN: u8 = 10;
pub const MIDI_CC_EXPRESSION: u8 = 11;
pub const MIDI_CC_SUSTAIN: u8 = 64;
pub const MIDI_CC_ALL_SOUND_OFF: u8 = 120;
pub const MIDI_CC_ALL_NOTES_OFF: u8 = 123;

/// Decoded standard MIDI 1.0 channel voice event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiEvent {
    NoteOff { channel: u8, note: u8, velocity: u8 },
    NoteOn { channel: u8, note: u8, velocity: u8 },
    PolyKeyPressure { channel: u8, note: u8, pressure: u8 },
    ControlChange { channel: u8, controller: u8, value: u8 },
    ProgramChange { channel: u8, program: u8 },
    ChannelPressure { channel: u8, pressure: u8 },
    PitchBend { channel: u8, bend: i16 },
    SystemReset,
    Unknown(u8),
}

/// Helper parsing packed 32-bit short messages and byte stream buffers.
pub struct MidiMessageParser;

impl MidiMessageParser {
    // Decodes standard Win32 32-bit packed short message (status in byte 0, data1 in byte 1, data2 in byte 2).
    pub fn parse_short_msg(msg: u32) -> MidiEvent {
        let status = (msg & 0xFF) as u8;
        let data1 = ((msg >> 8) & 0x7F) as u8;
        let data2 = ((msg >> 16) & 0x7F) as u8;

        let channel = status & 0x0F;
        let opcode = status & 0xF0;

        match opcode {
            MIDI_STATUS_NOTE_OFF => MidiEvent::NoteOff {
                channel,
                note: data1,
                velocity: data2,
            },
            MIDI_STATUS_NOTE_ON => {
                if data2 == 0 {
                    // Velocity 0 is treated as Note Off in standard MIDI spec
                    MidiEvent::NoteOff {
                        channel,
                        note: data1,
                        velocity: 0,
                    }
                } else {
                    MidiEvent::NoteOn {
                        channel,
                        note: data1,
                        velocity: data2,
                    }
                }
            }
            MIDI_STATUS_POLY_KEY_PRESSURE => MidiEvent::PolyKeyPressure {
                channel,
                note: data1,
                pressure: data2,
            },
            MIDI_STATUS_CONTROL_CHANGE => MidiEvent::ControlChange {
                channel,
                controller: data1,
                value: data2,
            },
            MIDI_STATUS_PROGRAM_CHANGE => MidiEvent::ProgramChange {
                channel,
                program: data1,
            },
            MIDI_STATUS_CHANNEL_PRESSURE => MidiEvent::ChannelPressure {
                channel,
                pressure: data1,
            },
            MIDI_STATUS_PITCH_BEND => {
                // 14-bit unsigned value centered at 0x2000 (8192)
                let raw14 = (data1 as u16) | ((data2 as u16) << 7);
                let bend = (raw14 as i32 - 8192) as i16;
                MidiEvent::PitchBend { channel, bend }
            }
            _ => {
                if status == MIDI_STATUS_SYSTEM_RESET {
                    MidiEvent::SystemReset
                } else {
                    MidiEvent::Unknown(status)
                }
            }
        }
    }

    // Converts MIDI note number (0..127) and pitch bend semitones to fundamental frequency in Hertz.
    pub fn note_to_frequency(note: u8, pitch_bend_semitones: f32) -> f32 {
        let semitones = (note as f32 - 69.0) + pitch_bend_semitones;
        let exponent = semitones / 12.0;
        440.0 * crate::drivers::input::gyro_aim::powf_f32(2.0, exponent)
    }
}

/// Verifies parsing of Note On, Note Off, Control Change, Pitch Bend, and note frequency conversion.
pub fn verify_midi_message_parser() -> bool {
    // 1. Note On on channel 2, note 60 (Middle C), velocity 90
    // Packed: status = 0x92, data1 = 60 (0x3C), data2 = 90 (0x5A) -> 0x005A3C92
    let msg_on = 0x92 | (60 << 8) | (90 << 16);
    match MidiMessageParser::parse_short_msg(msg_on) {
        MidiEvent::NoteOn { channel, note, velocity } => {
            if channel != 2 || note != 60 || velocity != 90 {
                return false;
            }
        }
        _ => return false,
    }

    // 2. Note On with velocity 0 must translate to NoteOff
    let msg_zero_vel = 0x92 | (60 << 8) | (0 << 16);
    match MidiMessageParser::parse_short_msg(msg_zero_vel) {
        MidiEvent::NoteOff { channel, note, .. } => {
            if channel != 2 || note != 60 {
                return false;
            }
        }
        _ => return false,
    }

    // 3. Pitch bend: center 8192 (0x2000) -> data1 = 0x00, data2 = 0x40
    let msg_bend_center = 0xE0 | (0x00 << 8) | (0x40 << 16);
    match MidiMessageParser::parse_short_msg(msg_bend_center) {
        MidiEvent::PitchBend { channel, bend } => {
            if channel != 0 || bend != 0 {
                return false;
            }
        }
        _ => return false,
    }

    // 4. Frequency conversion for A4 (note 69) = exactly 440 Hz
    let f_a4 = MidiMessageParser::note_to_frequency(69, 0.0);
    if (f_a4 - 440.0).abs() > 0.01 {
        return false;
    }

    // Frequency conversion for A5 (note 81) = 880 Hz
    let f_a5 = MidiMessageParser::note_to_frequency(81, 0.0);
    if (f_a5 - 880.0).abs() > 0.1 {
        return false;
    }

    true
}

use crate::graphics::types::sin_f32;

// Standard number of polyphonic hardware voices.
pub const MIDI_SYNTH_VOICES: usize = 16;
pub const TWO_PI: f32 = 6.2831853;

/// Individual polyphonic synthesizer voice utilizing dual-oscillator wavetable synthesis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynthVoice {
    pub is_active: bool,
    pub channel: u8,
    pub note: u8,
    pub velocity: u8,
    pub frequency: f32,
    pub phase_osc1: f32,
    pub phase_osc2: f32,
    pub detune_factor: f32,
    pub envelope_level: f32,
    pub decay_rate: f32,
    pub sample_rate: f32,
}

impl Default for SynthVoice {
    fn default() -> Self {
        Self {
            is_active: false,
            channel: 0,
            note: 0,
            velocity: 0,
            frequency: 440.0,
            phase_osc1: 0.0,
            phase_osc2: 0.0,
            detune_factor: 1.006,
            envelope_level: 0.0,
            decay_rate: 0.99985,
            sample_rate: 48000.0,
        }
    }
}

impl SynthVoice {
    // Triggers key press note event, resetting phase and envelope.
    pub fn trigger_note_on(&mut self, channel: u8, note: u8, velocity: u8, bend_semitones: f32, sample_rate: f32) {
        self.is_active = true;
        self.channel = channel;
        self.note = note;
        self.velocity = velocity;
        self.sample_rate = sample_rate;
        self.frequency = MidiMessageParser::note_to_frequency(note, bend_semitones);
        self.phase_osc1 = 0.0;
        self.phase_osc2 = 0.0;
        self.envelope_level = (velocity as f32) / 127.0;
    }

    // Releases key press note event.
    pub fn trigger_note_off(&mut self) {
        self.is_active = false;
        self.envelope_level = 0.0;
    }

    // Recalculates fundamental frequency when channel pitch bend changes.
    pub fn update_pitch_bend(&mut self, bend_semitones: f32) {
        if self.is_active {
            self.frequency = MidiMessageParser::note_to_frequency(self.note, bend_semitones);
        }
    }

    // Synthesizes next mono sample using dual-oscillator wavetable synthesis.
    pub fn render_sample(&mut self) -> f32 {
        if !self.is_active || self.envelope_level <= 0.001 {
            self.is_active = false;
            return 0.0;
        }

        // Primary fundamental oscillator (sine wave)
        let s1 = sin_f32(self.phase_osc1 * TWO_PI);
        // Secondary harmonic detuned oscillator (octave/chorus overtone)
        let s2 = sin_f32(self.phase_osc2 * TWO_PI) * 0.4;

        let sample = (s1 + s2) * 0.5 * self.envelope_level;

        // Exponential decay envelope
        self.envelope_level *= self.decay_rate;

        // Advance oscillator phases
        let step1 = self.frequency / self.sample_rate;
        let step2 = (self.frequency * self.detune_factor) / self.sample_rate;

        self.phase_osc1 = (self.phase_osc1 + step1) % 1.0;
        self.phase_osc2 = (self.phase_osc2 + step2) % 1.0;

        sample
    }
}

/// Polyphonic 16-voice wavetable synthesizer generating dual-oscillator PCM output from MIDI commands.
#[derive(Debug, Clone)]
pub struct MidiWavetableSynth {
    pub voices: [SynthVoice; MIDI_SYNTH_VOICES],
    pub channel_pitch_bend: [i16; 16],
    pub channel_volume: [u8; 16],
    pub channel_pan: [u8; 16],
    pub master_volume: f32,
    pub sample_rate: f32,
}

impl Default for MidiWavetableSynth {
    fn default() -> Self {
        Self::new(48000.0)
    }
}

impl MidiWavetableSynth {
    // Creates a new 16-voice synthesizer instance initialized for standard 48kHz audio.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            voices: [SynthVoice::default(); MIDI_SYNTH_VOICES],
            channel_pitch_bend: [0; 16],
            channel_volume: [100; 16],
            channel_pan: [64; 16],
            master_volume: 1.0,
            sample_rate,
        }
    }

    // Converts 14-bit pitch bend offset (-8192..8191) to semitones with +/- 2 semitone range.
    fn bend_to_semitones(bend: i16) -> f32 {
        ((bend as f32) / 8192.0) * 2.0
    }

    // Handles incoming decoded MIDI event.
    pub fn handle_event(&mut self, event: MidiEvent) {
        match event {
            MidiEvent::NoteOn { channel, note, velocity } => {
                let ch_idx = (channel as usize) % 16;
                let semitones = Self::bend_to_semitones(self.channel_pitch_bend[ch_idx]);

                // Find free voice or allocate voice
                let mut target_idx = None;
                for i in 0..MIDI_SYNTH_VOICES {
                    if !self.voices[i].is_active {
                        target_idx = Some(i);
                        break;
                    }
                }
                let idx = target_idx.unwrap_or(0); // Fallback: voice stealing
                self.voices[idx].trigger_note_on(channel, note, velocity, semitones, self.sample_rate);
            }
            MidiEvent::NoteOff { channel, note, .. } => {
                for voice in &mut self.voices {
                    if voice.is_active && voice.channel == channel && voice.note == note {
                        voice.trigger_note_off();
                    }
                }
            }
            MidiEvent::PitchBend { channel, bend } => {
                let ch_idx = (channel as usize) % 16;
                self.channel_pitch_bend[ch_idx] = bend;
                let semitones = Self::bend_to_semitones(bend);
                for voice in &mut self.voices {
                    if voice.is_active && voice.channel == channel {
                        voice.update_pitch_bend(semitones);
                    }
                }
            }
            MidiEvent::ControlChange { channel, controller, value } => {
                let ch_idx = (channel as usize) % 16;
                match controller {
                    MIDI_CC_VOLUME => self.channel_volume[ch_idx] = value,
                    MIDI_CC_PAN => self.channel_pan[ch_idx] = value,
                    MIDI_CC_ALL_NOTES_OFF | MIDI_CC_ALL_SOUND_OFF => {
                        for voice in &mut self.voices {
                            if voice.channel == channel {
                                voice.trigger_note_off();
                            }
                        }
                    }
                    _ => {}
                }
            }
            MidiEvent::SystemReset => {
                for voice in &mut self.voices {
                    voice.trigger_note_off();
                }
                self.channel_pitch_bend.fill(0);
                self.channel_volume.fill(100);
                self.channel_pan.fill(64);
            }
            _ => {}
        }
    }

    // Synthesizes polyphonic multi-channel PCM audio frames into destination stereo buffers.
    pub fn render_frames(&mut self, frames: usize, out_l: &mut [f32], out_r: &mut [f32]) -> usize {
        let count = frames.min(out_l.len()).min(out_r.len());

        for f in 0..count {
            for voice in &mut self.voices {
                if !voice.is_active {
                    continue;
                }
                let sample = voice.render_sample();
                let ch = (voice.channel as usize) % 16;
                let ch_vol = (self.channel_volume[ch] as f32) / 127.0;
                let pan = ((self.channel_pan[ch] as f32) - 64.0) / 64.0; // -1.0 to 1.0

                let pan_l = ((1.0 - pan) * 0.5).clamp(0.0, 1.0);
                let pan_r = ((1.0 + pan) * 0.5).clamp(0.0, 1.0);

                let mixed = sample * ch_vol * self.master_volume;
                out_l[f] += mixed * pan_l;
                out_r[f] += mixed * pan_r;
            }
        }

        count
    }
}

/// Verifies polyphonic 16-voice synthesizer allocation, dual-oscillator rendering, and note release.
pub fn verify_polyphonic_midi_synth() -> bool {
    let mut synth = MidiWavetableSynth::new(48000.0);

    // 1. Play Note C4 (60) and E4 (64) simultaneously on channel 0
    synth.handle_event(MidiEvent::NoteOn { channel: 0, note: 60, velocity: 100 });
    synth.handle_event(MidiEvent::NoteOn { channel: 0, note: 64, velocity: 100 });

    let mut active_count = 0;
    for v in &synth.voices {
        if v.is_active {
            active_count += 1;
        }
    }
    if active_count != 2 {
        return false;
    }

    // Render 64 frames and check non-zero output
    let mut out_l = alloc::vec![0.0f32; 64];
    let mut out_r = alloc::vec![0.0f32; 64];
    let rendered = synth.render_frames(64, &mut out_l, &mut out_r);
    if rendered != 64 {
        return false;
    }
    let mut has_sound = false;
    for &sample in &out_l {
        if sample.abs() > 0.05 {
            has_sound = true;
            break;
        }
    }
    if !has_sound {
        return false;
    }

    // 2. NoteOff on C4 (60): only 1 voice should remain active
    synth.handle_event(MidiEvent::NoteOff { channel: 0, note: 60, velocity: 0 });
    let mut remaining_count = 0;
    for v in &synth.voices {
        if v.is_active {
            remaining_count += 1;
        }
    }
    if remaining_count != 1 {
        return false;
    }

    true
}

use super::client_ring::AudioFrame;

/// Central WinMM Multimedia Mixer combining WaveOut PCM streams and MIDI synthesizer into unified audio stream.
#[derive(Debug, Clone, Default)]
pub struct WinMMServiceMixer {
    pub wave_registry: WaveOutDeviceRegistry,
    pub midi_synth: MidiWavetableSynth,
    pub master_volume: f32,
}

impl WinMMServiceMixer {
    // Creates a new multimedia mixer instance with default registry and 48kHz MIDI synth.
    pub fn new() -> Self {
        Self {
            wave_registry: WaveOutDeviceRegistry::new(),
            midi_synth: MidiWavetableSynth::new(48000.0),
            master_volume: 1.0,
        }
    }

    // Sets master output volume level.
    pub fn set_master_volume(&mut self, volume: f32) {
        self.master_volume = volume.clamp(0.0, 2.0);
    }

    // Obtains a mutable reference to the WaveOut device registry.
    pub fn wave_registry_mut(&mut self) -> &mut WaveOutDeviceRegistry {
        &mut self.wave_registry
    }

    // Obtains a mutable reference to the MIDI synthesizer.
    pub fn midi_synth_mut(&mut self) -> &mut MidiWavetableSynth {
        &mut self.midi_synth
    }

    // Blends all active WaveOut PCM streams and polyphonic MIDI voices into stereo AudioFrames.
    pub fn render_mixdown(&mut self, frames: usize) -> Vec<AudioFrame> {
        let mut mix_left = alloc::vec![0.0f32; frames];
        let mut mix_right = alloc::vec![0.0f32; frames];

        // 1. Accumulate all active WaveOut instances
        let mut voice_l = alloc::vec![0.0f32; frames];
        let mut voice_r = alloc::vec![0.0f32; frames];

        for inst in self.wave_registry.instances.values_mut() {
            voice_l.fill(0.0);
            voice_r.fill(0.0);
            inst.render_frames(frames, &mut voice_l, &mut voice_r);

            for i in 0..frames {
                mix_left[i] += voice_l[i];
                mix_right[i] += voice_r[i];
            }
        }

        // 2. Accumulate polyphonic MIDI synthesizer output
        let mut midi_l = alloc::vec![0.0f32; frames];
        let mut midi_r = alloc::vec![0.0f32; frames];
        self.midi_synth.render_frames(frames, &mut midi_l, &mut midi_r);

        for i in 0..frames {
            mix_left[i] += midi_l[i];
            mix_right[i] += midi_r[i];
        }

        // 3. Final master volume scaling and saturation clamping
        let mut output = Vec::with_capacity(frames);
        for i in 0..frames {
            output.push(AudioFrame {
                left: (mix_left[i] * self.master_volume).clamp(-1.0, 1.0),
                right: (mix_right[i] * self.master_volume).clamp(-1.0, 1.0),
            });
        }

        output
    }
}

/// Verifies simultaneous rendering and mixdown of WaveOut PCM audio and MIDI synthesis.
pub fn verify_winmm_service_mixer() -> bool {
    let mut mixer = WinMMServiceMixer::new();
    let fmt = WaveFormatEx::pcm_stereo(48000, 16);
    let handle = mixer.wave_registry_mut().wave_out_open(WAVE_MAPPER, &fmt, 0, 0, CALLBACK_NULL).unwrap();

    // Prepare and write a 64-frame WaveOut buffer with amplitude 0.4
    let mut pcm_bytes = Vec::new();
    for _ in 0..64 {
        let s: i16 = 13107; // ~0.4
        pcm_bytes.extend_from_slice(&s.to_le_bytes());
        pcm_bytes.extend_from_slice(&s.to_le_bytes());
    }
    let mut hdr = WAVEHDR::new(&pcm_bytes, 1);
    mixer.wave_registry_mut().wave_out_prepare_header(handle, &mut hdr);
    mixer.wave_registry_mut().wave_out_write(handle, hdr);

    // Trigger a MIDI note on channel 0, note 69 (A4, 440Hz), velocity 100
    mixer.midi_synth_mut().handle_event(MidiEvent::NoteOn { channel: 0, note: 69, velocity: 100 });

    let rendered = mixer.render_mixdown(64);
    if rendered.len() != 64 {
        return false;
    }

    // Output must combine WaveOut (0.4) and MIDI (>0)
    let mut has_combined_energy = false;
    for frame in &rendered {
        if frame.left > 0.4 {
            has_combined_energy = true;
            break;
        }
    }
    if !has_combined_energy {
        return false;
    }

    true
}

use crate::serial_write_str;

/// Comprehensive verification self-test suite for Win32 WinMM WaveOut & MIDI Synthesizer Engine.
pub fn test_winmm_service_subsystem() -> bool {
    serial_write_str("[KNT-AUDIO] Testing WinMM legacy WaveOut & MIDI synthesizer service...\r\n");

    // 1. Verify capability descriptors, status codes, and audio buffer headers
    if !verify_winmm_types_and_descriptors() {
        serial_write_str("[KNT-PANIC] WinMM capability structures and header verification failed!\r\n");
        return false;
    }

    // 2. Verify waveOut device registry, capabilities queries, and lifecycle
    if !verify_waveout_device_lifecycle() {
        serial_write_str("[KNT-PANIC] WaveOut device registry and lifecycle verification failed!\r\n");
        return false;
    }

    // 3. Verify waveOut buffer queueing, header preparation, and PCM consumption
    if !verify_waveout_buffer_queue_and_prepare() {
        serial_write_str("[KNT-PANIC] WaveOut buffer queueing and header preparation failed!\r\n");
        return false;
    }

    // 4. Verify MIDI message parser, channel event decoding, and frequency calculations
    if !verify_midi_message_parser() {
        serial_write_str("[KNT-PANIC] MIDI message parser and frequency conversion failed!\r\n");
        return false;
    }

    // 5. Verify polyphonic 16-voice wavetable synthesizer and dual-oscillator voice engine
    if !verify_polyphonic_midi_synth() {
        serial_write_str("[KNT-PANIC] Polyphonic MIDI synthesizer verification failed!\r\n");
        return false;
    }

    // 6. Verify WinMM multimedia mixer blending WaveOut streams and MIDI synth output
    if !verify_winmm_service_mixer() {
        serial_write_str("[KNT-PANIC] WinMM multimedia mixer stream rendering verification failed!\r\n");
        return false;
    }

    serial_write_str("[KNT-WINMM-PASS] WinMM legacy WaveOut audio and MIDI synthesizer service verified\r\n");
    true
}
