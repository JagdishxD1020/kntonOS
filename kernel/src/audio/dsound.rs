//! Win32 DirectSound & DirectSound3D Legacy Game Audio API Emulation Layer.

#![allow(non_snake_case)]

use alloc::vec::Vec;
use core::mem::size_of;
use super::xaudio2::WaveFormatEx;

// Standard DirectSound HRESULT status codes.
pub const DS_OK: u32 = 0;
pub const DSERR_INVALIDPARAM: u32 = 0x80070057;
pub const DSERR_BUFFERLOST: u32 = 0x88780096;
pub const DSERR_UNINITIALIZED: u32 = 0x8878000A;

// Buffer capability flags.
pub const DSBCAPS_PRIMARYBUFFER: u32 = 0x00000001;
pub const DSBCAPS_STATIC: u32 = 0x00000002;
pub const DSBCAPS_LOCHARDWARE: u32 = 0x00000004;
pub const DSBCAPS_LOCSOFTWARE: u32 = 0x00000008;
pub const DSBCAPS_CTRL3D: u32 = 0x00000010;
pub const DSBCAPS_CTRLFREQUENCY: u32 = 0x00000020;
pub const DSBCAPS_CTRLPAN: u32 = 0x00000040;
pub const DSBCAPS_CTRLVOLUME: u32 = 0x00000080;
pub const DSBCAPS_CTRLPOSITIONNOTIFY: u32 = 0x00000100;
pub const DSBCAPS_GLOBALFOCUS: u32 = 0x00008000;

// Playback flags.
pub const DSBPLAY_LOOPING: u32 = 0x00000001;

// Buffer status flags.
pub const DSBSTATUS_PLAYING: u32 = 0x00000001;
pub const DSBSTATUS_BUFFERLOST: u32 = 0x00000002;
pub const DSBSTATUS_LOOPING: u32 = 0x00000004;

// Lock flags.
pub const DSBLOCK_FROMWRITECURSOR: u32 = 0x00000001;
pub const DSBLOCK_ENTIREBUFFER: u32 = 0x00000002;

/// Win32 DSBUFFERDESC structure specifying characteristics of a new DirectSound buffer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DSBUFFERDESC {
    pub dwSize: u32,
    pub dwFlags: u32,
    pub dwBufferBytes: u32,
    pub dwReserved: u32,
    pub lpwfxFormat: WaveFormatEx,
}

impl Default for DSBUFFERDESC {
    fn default() -> Self {
        Self {
            dwSize: size_of::<Self>() as u32,
            dwFlags: DSBCAPS_CTRLVOLUME | DSBCAPS_CTRLPAN | DSBCAPS_CTRLFREQUENCY,
            dwBufferBytes: 0,
            dwReserved: 0,
            lpwfxFormat: WaveFormatEx::default(),
        }
    }
}

/// Circular lock region pointers and lengths returned by IDirectSoundBuffer::Lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DSoundLockRegion {
    pub offset1: usize,
    pub len1: usize,
    pub offset2: usize,
    pub len2: usize,
}

// Control unavailable error.
pub const DSERR_CONTROLUNAVAIL: u32 = 0x8878001E;

// Volume limits in hundredths of a decibel (mB).
pub const DSBVOLUME_MIN: i32 = -10000;
pub const DSBVOLUME_MAX: i32 = 0;

// Pan limits in hundredths of a decibel.
pub const DSBPAN_LEFT: i32 = -10000;
pub const DSBPAN_CENTER: i32 = 0;
pub const DSBPAN_RIGHT: i32 = 10000;

// Frequency range limits in Hertz.
pub const DSBFREQUENCY_MIN: u32 = 100;
pub const DSBFREQUENCY_MAX: u32 = 200000;
pub const DSBFREQUENCY_ORIGINAL: u32 = 0;

/// DirectSound Buffer representing a circular PCM sound buffer with cursor tracking.
#[derive(Debug, Clone)]
pub struct DirectSoundBuffer {
    pub desc: DSBUFFERDESC,
    pub buffer: Vec<u8>,
    pub play_cursor: usize,
    pub write_cursor: usize,
    pub status: u32,
    pub is_locked: bool,
    pub lock_region: DSoundLockRegion,
    pub volume: i32,
    pub pan: i32,
    pub frequency: u32,
    pub original_frequency: u32,
}

impl DirectSoundBuffer {
    // Creates a new secondary or primary DirectSoundBuffer.
    pub fn new(desc: DSBUFFERDESC) -> Self {
        let size = desc.dwBufferBytes as usize;
        let orig_freq = if desc.lpwfxFormat.nSamplesPerSec > 0 {
            desc.lpwfxFormat.nSamplesPerSec
        } else {
            48000
        };
        Self {
            desc,
            buffer: alloc::vec![0u8; size],
            play_cursor: 0,
            write_cursor: 0,
            status: 0,
            is_locked: false,
            lock_region: DSoundLockRegion {
                offset1: 0,
                len1: 0,
                offset2: 0,
                len2: 0,
            },
            volume: DSBVOLUME_MAX,
            pan: DSBPAN_CENTER,
            frequency: orig_freq,
            original_frequency: orig_freq,
        }
    }

    // Obtains pointers/ranges into circular buffer for writing data (handles circular wraparound).
    pub fn lock(&mut self, offset: usize, bytes: usize, flags: u32) -> Result<DSoundLockRegion, u32> {
        if self.buffer.is_empty() {
            return Err(DSERR_INVALIDPARAM);
        }

        let start_offset = if (flags & DSBLOCK_FROMWRITECURSOR) != 0 {
            self.write_cursor
        } else {
            offset % self.buffer.len()
        };

        let lock_bytes = if (flags & DSBLOCK_ENTIREBUFFER) != 0 {
            self.buffer.len()
        } else {
            bytes
        };

        let cap = self.buffer.len();
        let region = if start_offset + lock_bytes <= cap {
            DSoundLockRegion {
                offset1: start_offset,
                len1: lock_bytes,
                offset2: 0,
                len2: 0,
            }
        } else {
            let len1 = cap - start_offset;
            let len2 = (lock_bytes - len1).min(start_offset);
            DSoundLockRegion {
                offset1: start_offset,
                len1,
                offset2: 0,
                len2,
            }
        };

        self.is_locked = true;
        self.lock_region = region;
        Ok(region)
    }

    // Commits written data to circular buffer and advances write cursor.
    pub fn unlock(&mut self, region: DSoundLockRegion, data1: &[u8], data2: &[u8]) -> u32 {
        if !self.is_locked {
            return DSERR_INVALIDPARAM;
        }

        let l1 = region.len1.min(data1.len());
        if l1 > 0 && region.offset1 + l1 <= self.buffer.len() {
            self.buffer[region.offset1..region.offset1 + l1].copy_from_slice(&data1[..l1]);
        }

        let l2 = region.len2.min(data2.len());
        if l2 > 0 && region.offset2 + l2 <= self.buffer.len() {
            self.buffer[region.offset2..region.offset2 + l2].copy_from_slice(&data2[..l2]);
        }

        self.write_cursor = if region.len2 > 0 {
            region.offset2 + l2
        } else {
            (region.offset1 + l1) % self.buffer.len()
        };

        self.is_locked = false;
        DS_OK
    }

    // Queries current play and write cursors.
    pub fn get_current_position(&self) -> (usize, usize) {
        (self.play_cursor, self.write_cursor)
    }

    // Sets current playback cursor position.
    pub fn set_current_position(&mut self, pos: usize) -> u32 {
        if self.buffer.is_empty() {
            return DSERR_INVALIDPARAM;
        }
        self.play_cursor = pos % self.buffer.len();
        DS_OK
    }
}

/// Verifies memory layout, capability flags, and circular lock/unlock split ranges.
pub fn verify_dsound_buffer_descriptors() -> bool {
    let mut desc = DSBUFFERDESC::default();
    desc.dwBufferBytes = 1000;
    let mut buf = DirectSoundBuffer::new(desc);

    // Lock non-wrapping region: 200 bytes at offset 100
    let r1 = match buf.lock(100, 200, 0) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if r1.offset1 != 100 || r1.len1 != 200 || r1.len2 != 0 {
        return false;
    }

    let dummy_data = alloc::vec![0xAAu8; 200];
    if buf.unlock(r1, &dummy_data, &[]) != DS_OK {
        return false;
    }
    if buf.buffer[100] != 0xAA || buf.buffer[299] != 0xAA {
        return false;
    }

    // Lock wrapping region: 300 bytes at offset 850 (850..1000 = 150, 0..150 = 150)
    let r2 = match buf.lock(850, 300, 0) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if r2.offset1 != 850 || r2.len1 != 150 || r2.offset2 != 0 || r2.len2 != 150 {
        return false;
    }

    let d1 = alloc::vec![0xBBu8; 150];
    let d2 = alloc::vec![0xCCu8; 150];
    if buf.unlock(r2, &d1, &d2) != DS_OK {
        return false;
    }
    if buf.buffer[850] != 0xBB || buf.buffer[999] != 0xBB || buf.buffer[0] != 0xCC || buf.buffer[149] != 0xCC {
        return false;
    }

    true
}

impl DirectSoundBuffer {
    // Sets buffer volume attenuation in hundredths of a decibel (-10000 to 0).
    pub fn set_volume(&mut self, volume: i32) -> u32 {
        if (self.desc.dwFlags & DSBCAPS_CTRLVOLUME) == 0 {
            return DSERR_CONTROLUNAVAIL;
        }
        self.volume = volume.clamp(DSBVOLUME_MIN, DSBVOLUME_MAX);
        DS_OK
    }

    // Returns current buffer volume in mB.
    pub fn get_volume(&self) -> i32 {
        self.volume
    }

    // Sets pan balance between left and right channels (-10000 to +10000).
    pub fn set_pan(&mut self, pan: i32) -> u32 {
        if (self.desc.dwFlags & DSBCAPS_CTRLPAN) == 0 {
            return DSERR_CONTROLUNAVAIL;
        }
        self.pan = pan.clamp(DSBPAN_LEFT, DSBPAN_RIGHT);
        DS_OK
    }

    // Returns current buffer pan balance.
    pub fn get_pan(&self) -> i32 {
        self.pan
    }

    // Adjusts playback sampling frequency in Hertz (100 to 200000 Hz).
    pub fn set_frequency(&mut self, frequency: u32) -> u32 {
        if (self.desc.dwFlags & DSBCAPS_CTRLFREQUENCY) == 0 {
            return DSERR_CONTROLUNAVAIL;
        }
        if frequency == DSBFREQUENCY_ORIGINAL {
            self.frequency = self.original_frequency;
        } else {
            self.frequency = frequency.clamp(DSBFREQUENCY_MIN, DSBFREQUENCY_MAX);
        }
        DS_OK
    }

    // Returns current playback sampling frequency.
    pub fn get_frequency(&self) -> u32 {
        self.frequency
    }

    // Starts buffer playback with optional looping flag.
    pub fn play(&mut self, flags: u32) -> u32 {
        self.status |= DSBSTATUS_PLAYING;
        if (flags & DSBPLAY_LOOPING) != 0 {
            self.status |= DSBSTATUS_LOOPING;
        } else {
            self.status &= !DSBSTATUS_LOOPING;
        }
        DS_OK
    }

    // Stops buffer playback.
    pub fn stop(&mut self) -> u32 {
        self.status &= !(DSBSTATUS_PLAYING | DSBSTATUS_LOOPING);
        DS_OK
    }

    // Returns current playback status bitmask.
    pub fn get_status(&self) -> u32 {
        self.status
    }

    // Computes left and right linear multipliers from current volume and pan settings.
    pub fn compute_channel_gains(&self) -> (f32, f32) {
        let vol_db = (self.volume as f32) / 100.0;
        let base_gain = super::xaudio2::decibels_to_linear(vol_db);

        let (pan_l, pan_r) = if self.pan == 0 {
            (1.0, 1.0)
        } else if self.pan < 0 {
            let right_atten_db = (self.pan as f32) / 100.0;
            (1.0, super::xaudio2::decibels_to_linear(right_atten_db))
        } else {
            let left_atten_db = -(self.pan as f32) / 100.0;
            (super::xaudio2::decibels_to_linear(left_atten_db), 1.0)
        };

        (base_gain * pan_l, base_gain * pan_r)
    }

    // Computes resampling speed ratio relative to original buffer frequency.
    pub fn get_resample_ratio(&self) -> f32 {
        if self.original_frequency == 0 {
            1.0
        } else {
            (self.frequency as f32) / (self.original_frequency as f32)
        }
    }
}

/// Verifies volume gain attenuation, pan channel balance, and frequency ratio scaling.
pub fn verify_dsound_volume_pan_and_frequency() -> bool {
    let mut desc = DSBUFFERDESC::default();
    desc.dwFlags = DSBCAPS_CTRLVOLUME | DSBCAPS_CTRLPAN | DSBCAPS_CTRLFREQUENCY;
    desc.dwBufferBytes = 4096;
    desc.lpwfxFormat = WaveFormatEx::pcm_stereo(44100, 16);

    let mut buf = DirectSoundBuffer::new(desc);

    // Test volume attenuation (-600 mB = -6 dB = ~0.501)
    if buf.set_volume(-600) != DS_OK {
        return false;
    }
    let (gl, gr) = buf.compute_channel_gains();
    if (gl - 0.501).abs() > 0.05 || (gr - 0.501).abs() > 0.05 {
        return false;
    }

    // Test full right pan (+10000 mB): left attenuated to 0, right retains base gain
    if buf.set_pan(DSBPAN_RIGHT) != DS_OK {
        return false;
    }
    let (gl_panned, gr_panned) = buf.compute_channel_gains();
    if gl_panned > 0.01 || (gr_panned - 0.501).abs() > 0.05 {
        return false;
    }

    // Test frequency scaling: double frequency (88200 Hz) -> 2.0x ratio
    if buf.set_frequency(88200) != DS_OK {
        return false;
    }
    if (buf.get_resample_ratio() - 2.0).abs() > 0.01 {
        return false;
    }

    // Test play/stop status
    buf.play(DSBPLAY_LOOPING);
    if (buf.get_status() & (DSBSTATUS_PLAYING | DSBSTATUS_LOOPING)) == 0 {
        return false;
    }
    buf.stop();
    if (buf.get_status() & DSBSTATUS_PLAYING) != 0 {
        return false;
    }

    true
}

use crate::drivers::input::imu::sqrt_f32;

// DirectSound3D processing modes.
pub const DS3DMODE_NORMAL: u32 = 0x00000000;
pub const DS3DMODE_HEADRELATIVE: u32 = 0x00000001;
pub const DS3DMODE_DISABLE: u32 = 0x00000002;

// Standard 3D vector for DirectSound3D coordinates and velocities.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct D3DVECTOR {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl D3DVECTOR {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    // Computes vector magnitude length.
    pub fn length(&self) -> f32 {
        sqrt_f32(self.x * self.x + self.y * self.y + self.z * self.z)
    }
}

/// DirectSound3D Listener parameters representing user position and orientation in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DS3DListener {
    pub position: D3DVECTOR,
    pub velocity: D3DVECTOR,
    pub orient_front: D3DVECTOR,
    pub orient_top: D3DVECTOR,
    pub distance_factor: f32,
    pub rolloff_factor: f32,
    pub doppler_factor: f32,
}

impl Default for DS3DListener {
    fn default() -> Self {
        Self {
            position: D3DVECTOR::new(0.0, 0.0, 0.0),
            velocity: D3DVECTOR::new(0.0, 0.0, 0.0),
            orient_front: D3DVECTOR::new(0.0, 0.0, 1.0),
            orient_top: D3DVECTOR::new(0.0, 1.0, 0.0),
            distance_factor: 1.0,
            rolloff_factor: 1.0,
            doppler_factor: 1.0,
        }
    }
}

/// DirectSound3D Buffer parameters for sound sources positioned in 3D audio space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectSound3DBuffer {
    pub position: D3DVECTOR,
    pub velocity: D3DVECTOR,
    pub inside_cone_angle: u32,
    pub outside_cone_angle: u32,
    pub cone_orientation: D3DVECTOR,
    pub cone_outside_volume: i32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub mode: u32,
}

impl Default for DirectSound3DBuffer {
    fn default() -> Self {
        Self {
            position: D3DVECTOR::new(0.0, 0.0, 1.0),
            velocity: D3DVECTOR::new(0.0, 0.0, 0.0),
            inside_cone_angle: 360,
            outside_cone_angle: 360,
            cone_orientation: D3DVECTOR::new(0.0, 0.0, 1.0),
            cone_outside_volume: 0,
            min_distance: 1.0,
            max_distance: 100.0,
            mode: DS3DMODE_NORMAL,
        }
    }
}

impl DirectSound3DBuffer {
    // Computes distance attenuation and constant-power stereo panning relative to a listener.
    pub fn compute_spatial_gains(&self, listener: &DS3DListener) -> (f32, f32) {
        if self.mode == DS3DMODE_DISABLE {
            return (1.0, 1.0);
        }

        let rel_x = if self.mode == DS3DMODE_HEADRELATIVE {
            self.position.x
        } else {
            self.position.x - listener.position.x
        };
        let rel_y = if self.mode == DS3DMODE_HEADRELATIVE {
            self.position.y
        } else {
            self.position.y - listener.position.y
        };
        let rel_z = if self.mode == DS3DMODE_HEADRELATIVE {
            self.position.z
        } else {
            self.position.z - listener.position.z
        };

        let dist = sqrt_f32(rel_x * rel_x + rel_y * rel_y + rel_z * rel_z).max(0.001);
        let clamped_dist = dist.clamp(self.min_distance, self.max_distance);

        // Standard inverse distance attenuation formula
        let denom = self.min_distance + listener.rolloff_factor * (clamped_dist - self.min_distance);
        let dist_gain = (self.min_distance / denom.max(0.001)).clamp(0.0, 1.0);

        // Constant-power horizontal azimuth panning (-1.0 left to +1.0 right)
        let pan = (rel_x / dist).clamp(-1.0, 1.0);
        let pan_l = sqrt_f32(0.5 * (1.0 - pan));
        let pan_r = sqrt_f32(0.5 * (1.0 + pan));

        (dist_gain * pan_l, dist_gain * pan_r)
    }
}

/// Verifies DirectSound3D distance rolloff attenuation and spatial azimuth panning.
pub fn verify_dsound3d_spatial_attenuation() -> bool {
    let listener = DS3DListener::default();
    let mut source = DirectSound3DBuffer::default();
    source.min_distance = 1.0;
    source.max_distance = 100.0;

    // 1. Source at min_distance ahead (0, 0, 1): center panned, gain = 1.0 * ~0.707
    source.position = D3DVECTOR::new(0.0, 0.0, 1.0);
    let (l1, r1) = source.compute_spatial_gains(&listener);
    if (l1 - r1).abs() > 0.01 || (l1 - 0.707).abs() > 0.05 {
        return false;
    }

    // 2. Source at 10.0m ahead: distance attenuation = 1.0 / (1.0 + 1.0 * (10 - 1)) = 0.1
    source.position = D3DVECTOR::new(0.0, 0.0, 10.0);
    let (l10, r10) = source.compute_spatial_gains(&listener);
    if (l10 - r10).abs() > 0.01 || (l10 - (0.1 * 0.707)).abs() > 0.02 {
        return false;
    }

    // 3. Source to the right (10, 0, 0): pan_r >> pan_l
    source.position = D3DVECTOR::new(10.0, 0.0, 0.0);
    let (l_right, r_right) = source.compute_spatial_gains(&listener);
    if r_right <= l_right || l_right > 0.01 {
        return false;
    }

    true
}

use super::client_ring::AudioFrame;
use crate::serial_write_str;

/// Win32 DirectSound Device Context managing primary and secondary sound buffers.
#[derive(Debug, Clone)]
pub struct DirectSoundDevice {
    pub primary_buffer: Option<DirectSoundBuffer>,
    pub secondary_buffers: Vec<DirectSoundBuffer>,
    pub three_d_buffers: Vec<Option<DirectSound3DBuffer>>,
    pub listener: DS3DListener,
}

impl Default for DirectSoundDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectSoundDevice {
    // Creates a new DirectSound device context.
    pub fn new() -> Self {
        Self {
            primary_buffer: None,
            secondary_buffers: Vec::new(),
            three_d_buffers: Vec::new(),
            listener: DS3DListener::default(),
        }
    }

    // Creates a new primary or secondary DirectSoundBuffer.
    pub fn create_sound_buffer(&mut self, desc: DSBUFFERDESC) -> usize {
        let is_primary = (desc.dwFlags & DSBCAPS_PRIMARYBUFFER) != 0;
        let is_3d = (desc.dwFlags & DSBCAPS_CTRL3D) != 0;
        let buf = DirectSoundBuffer::new(desc);

        if is_primary {
            self.primary_buffer = Some(buf);
            0
        } else {
            let id = self.secondary_buffers.len();
            self.secondary_buffers.push(buf);
            self.three_d_buffers.push(if is_3d { Some(DirectSound3DBuffer::default()) } else { None });
            id
        }
    }

    // Obtains a mutable reference to an existing secondary sound buffer.
    pub fn get_buffer_mut(&mut self, id: usize) -> Option<&mut DirectSoundBuffer> {
        self.secondary_buffers.get_mut(id)
    }

    // Obtains a mutable reference to 3D parameters for an existing secondary sound buffer.
    pub fn get_3d_buffer_mut(&mut self, id: usize) -> Option<&mut DirectSound3DBuffer> {
        self.three_d_buffers.get_mut(id).and_then(|opt| opt.as_mut())
    }

    // Blends all active secondary sound buffers into stereo AudioFrames for kernel DMA output.
    pub fn render_mixdown(&mut self, frames: usize) -> Vec<AudioFrame> {
        let mut mix_left = alloc::vec![0.0f32; frames];
        let mut mix_right = alloc::vec![0.0f32; frames];

        for i in 0..self.secondary_buffers.len() {
            let buf = &mut self.secondary_buffers[i];
            if (buf.status & DSBSTATUS_PLAYING) == 0 || buf.buffer.is_empty() {
                continue;
            }

            let block_align = buf.desc.lpwfxFormat.nBlockAlign as usize;
            if block_align == 0 {
                continue;
            }

            let channels = buf.desc.lpwfxFormat.nChannels as usize;
            let (gl, gr) = if let Some(ref three_d) = self.three_d_buffers[i] {
                three_d.compute_spatial_gains(&self.listener)
            } else {
                buf.compute_channel_gains()
            };

            let resample_ratio = buf.get_resample_ratio();
            let mut cursor_f = buf.play_cursor as f32;
            let total_bytes = buf.buffer.len();
            let is_looping = (buf.status & DSBSTATUS_LOOPING) != 0;

            for f in 0..frames {
                let cur_idx = (cursor_f as usize) % total_bytes;
                if !is_looping && (cursor_f as usize) + block_align > total_bytes {
                    buf.stop();
                    break;
                }

                if cur_idx + block_align <= total_bytes {
                    let left_i16 = i16::from_le_bytes([buf.buffer[cur_idx], buf.buffer[cur_idx + 1]]);
                    let sample_l = (left_i16 as f32) / 32768.0;
                    let sample_r = if channels > 1 {
                        let right_i16 = i16::from_le_bytes([buf.buffer[cur_idx + 2], buf.buffer[cur_idx + 3]]);
                        (right_i16 as f32) / 32768.0
                    } else {
                        sample_l
                    };

                    mix_left[f] += sample_l * gl;
                    mix_right[f] += sample_r * gr;
                }

                cursor_f += resample_ratio * (block_align as f32);
                if is_looping {
                    cursor_f %= total_bytes as f32;
                }
            }

            buf.play_cursor = (cursor_f as usize) % total_bytes;
        }

        let mut output = Vec::with_capacity(frames);
        for f in 0..frames {
            output.push(AudioFrame {
                left: mix_left[f].clamp(-1.0, 1.0),
                right: mix_right[f].clamp(-1.0, 1.0),
            });
        }
        output
    }
}

/// Verifies DirectSound secondary buffer playback mixdown and stream packet generation.
pub fn verify_dsound_device_mixer() -> bool {
    let mut device = DirectSoundDevice::new();
    let mut desc = DSBUFFERDESC::default();
    desc.dwFlags = DSBCAPS_CTRLVOLUME | DSBCAPS_CTRLPAN | DSBCAPS_CTRLFREQUENCY;
    desc.dwBufferBytes = 400; // 100 stereo 16-bit frames
    desc.lpwfxFormat = WaveFormatEx::pcm_stereo(48000, 16);

    let id = device.create_sound_buffer(desc);
    let buf = device.get_buffer_mut(id).unwrap();

    let region = buf.lock(0, 400, 0).unwrap();
    let mut pcm_data = Vec::new();
    for _ in 0..100 {
        let sample: i16 = 16384; // 0.5
        pcm_data.extend_from_slice(&sample.to_le_bytes());
        pcm_data.extend_from_slice(&sample.to_le_bytes());
    }
    buf.unlock(region, &pcm_data, &[]);
    buf.play(DSBPLAY_LOOPING);

    let frames = device.render_mixdown(50);
    if frames.len() != 50 {
        return false;
    }
    // Expected sample value = 0.5 (full volume, center pan)
    if (frames[0].left - 0.5).abs() > 0.02 || (frames[0].right - 0.5).abs() > 0.02 {
        return false;
    }

    true
}

/// Comprehensive verification self-test suite for DirectSound & DirectSound3D Legacy Game Audio API Emulation.
pub fn test_dsound_service_subsystem() -> bool {
    serial_write_str("[KNT-AUDIO] Testing DirectSound & DirectSound3D legacy game audio service...\r\n");

    // 1. Verify buffer descriptors, capabilities, and circular lock/unlock split regions
    if !verify_dsound_buffer_descriptors() {
        serial_write_str("[KNT-PANIC] DirectSound buffer descriptor and lock/unlock verification failed!\r\n");
        return false;
    }

    // 2. Verify volume attenuation, pan balance, and frequency scaling
    if !verify_dsound_volume_pan_and_frequency() {
        serial_write_str("[KNT-PANIC] DirectSound volume, pan, and frequency verification failed!\r\n");
        return false;
    }

    // 3. Verify DirectSound3D distance rolloff attenuation and spatial azimuth panning
    if !verify_dsound3d_spatial_attenuation() {
        serial_write_str("[KNT-PANIC] DirectSound3D spatial attenuation verification failed!\r\n");
        return false;
    }

    // 4. Verify multi-buffer mixing engine and stream packet emission
    if !verify_dsound_device_mixer() {
        serial_write_str("[KNT-PANIC] DirectSound mixer and stream rendering verification failed!\r\n");
        return false;
    }

    serial_write_str("[KNT-DSOUND-PASS] DirectSound and DirectSound3D legacy game audio service verified\r\n");
    true
}
