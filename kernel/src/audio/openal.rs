//! Win32 OpenAL 1.1 3D Spatial Audio & EAX Environmental Reverb Engine.

#![allow(non_snake_case)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use crate::graphics::types::Vec3;

// OpenAL 1.1 core primitive types.
pub type ALboolean = u8;
pub type ALchar = u8;
pub type ALbyte = i8;
pub type ALubyte = u8;
pub type ALshort = i16;
pub type ALushort = u16;
pub type ALint = i32;
pub type ALuint = u32;
pub type ALsizei = i32;
pub type ALenum = i32;
pub type ALfloat = f32;
pub type ALdouble = f64;

// AL boolean values.
pub const AL_FALSE: ALboolean = 0;
pub const AL_TRUE: ALboolean = 1;
pub const AL_NONE: ALenum = 0;

// OpenAL error codes.
pub const AL_NO_ERROR: ALenum = 0;
pub const AL_INVALID_NAME: ALenum = 0xA001;
pub const AL_INVALID_ENUM: ALenum = 0xA002;
pub const AL_INVALID_VALUE: ALenum = 0xA003;
pub const AL_INVALID_OPERATION: ALenum = 0xA004;
pub const AL_OUT_OF_MEMORY: ALenum = 0xA005;

// OpenAL buffer formats.
pub const AL_FORMAT_MONO8: ALenum = 0x1100;
pub const AL_FORMAT_MONO16: ALenum = 0x1101;
pub const AL_FORMAT_STEREO8: ALenum = 0x1102;
pub const AL_FORMAT_STEREO16: ALenum = 0x1103;

// OpenAL buffer query tokens.
pub const AL_FREQUENCY: ALenum = 0x2001;
pub const AL_BITS: ALenum = 0x2002;
pub const AL_CHANNELS: ALenum = 0x2003;
pub const AL_SIZE: ALenum = 0x2004;

// OpenAL source attributes.
pub const AL_SOURCE_RELATIVE: ALenum = 0x202;
pub const AL_CONE_INNER_ANGLE: ALenum = 0x1001;
pub const AL_CONE_OUTER_ANGLE: ALenum = 0x1002;
pub const AL_PITCH: ALenum = 0x1003;
pub const AL_POSITION: ALenum = 0x1004;
pub const AL_DIRECTION: ALenum = 0x1005;
pub const AL_VELOCITY: ALenum = 0x1006;
pub const AL_LOOPING: ALenum = 0x1007;
pub const AL_BUFFER: ALenum = 0x1009;
pub const AL_GAIN: ALenum = 0x100A;
pub const AL_MIN_GAIN: ALenum = 0x100D;
pub const AL_MAX_GAIN: ALenum = 0x100E;
pub const AL_ORIENTATION: ALenum = 0x100F;
pub const AL_SOURCE_STATE: ALenum = 0x1010;

// OpenAL 3D distance and source parameters.
pub const AL_REFERENCE_DISTANCE: ALenum = 0x1020;
pub const AL_ROLLOFF_FACTOR: ALenum = 0x1021;
pub const AL_CONE_OUTER_GAIN: ALenum = 0x1022;
pub const AL_MAX_DISTANCE: ALenum = 0x1023;

// OpenAL source state flags.
pub const AL_INITIAL: ALenum = 0x1011;
pub const AL_PLAYING: ALenum = 0x1012;
pub const AL_PAUSED: ALenum = 0x1013;
pub const AL_STOPPED: ALenum = 0x1014;

// OpenAL distance attenuation models.
pub const AL_INVERSE_DISTANCE: ALenum = 0xD001;
pub const AL_INVERSE_DISTANCE_CLAMPED: ALenum = 0xD002;
pub const AL_LINEAR_DISTANCE: ALenum = 0xD003;
pub const AL_LINEAR_DISTANCE_CLAMPED: ALenum = 0xD004;
pub const AL_EXPONENT_DISTANCE: ALenum = 0xD005;
pub const AL_EXPONENT_DISTANCE_CLAMPED: ALenum = 0xD006;

// ALC boolean values.
pub const ALC_FALSE: ALboolean = 0;
pub const ALC_TRUE: ALboolean = 1;

// ALC error codes.
pub const ALC_NO_ERROR: ALenum = 0;
pub const ALC_INVALID_DEVICE: ALenum = 0xA001;
pub const ALC_INVALID_CONTEXT: ALenum = 0xA002;
pub const ALC_INVALID_ENUM: ALenum = 0xA003;
pub const ALC_INVALID_VALUE: ALenum = 0xA004;
pub const ALC_OUT_OF_MEMORY: ALenum = 0xA005;

// ALC device and context query tokens.
pub const ALC_FREQUENCY: ALenum = 0x1007;
pub const ALC_REFRESH: ALenum = 0x1008;
pub const ALC_SYNC: ALenum = 0x1009;
pub const ALC_MONO_SOURCES: ALenum = 0x1010;
pub const ALC_STEREO_SOURCES: ALenum = 0x1011;
pub const ALC_DEFAULT_DEVICE_SPECIFIER: ALenum = 0x1004;
pub const ALC_DEVICE_SPECIFIER: ALenum = 0x1005;
pub const ALC_EXTENSIONS: ALenum = 0x1006;

/// OpenAL audio buffer holding raw PCM data and decoded floating-point frames.
#[derive(Debug, Clone, Default)]
pub struct ALBuffer {
    pub id: ALuint,
    pub format: ALenum,
    pub frequency: ALsizei,
    pub bits: ALsizei,
    pub channels: ALsizei,
    pub raw_bytes: Vec<u8>,
    pub pcm_left: Vec<f32>,
    pub pcm_right: Vec<f32>,
}

impl ALBuffer {
    // Creates an empty uninitialized OpenAL buffer.
    pub fn new(id: ALuint) -> Self {
        Self {
            id,
            format: AL_NONE,
            frequency: 0,
            bits: 0,
            channels: 0,
            raw_bytes: Vec::new(),
            pcm_left: Vec::new(),
            pcm_right: Vec::new(),
        }
    }

    // Number of decoded audio frames in buffer.
    pub fn num_frames(&self) -> usize {
        self.pcm_left.len()
    }

    // Loads and decodes raw PCM audio data into normalized floating-point samples.
    pub fn load_pcm_data(&mut self, format: ALenum, data: &[u8], size: usize, freq: ALsizei) -> bool {
        self.format = format;
        self.frequency = freq;
        self.raw_bytes = data[..size].to_vec();
        self.pcm_left.clear();
        self.pcm_right.clear();

        match format {
            AL_FORMAT_MONO8 => {
                self.bits = 8;
                self.channels = 1;
                let frames = size;
                self.pcm_left.reserve(frames);
                self.pcm_right.reserve(frames);
                for &b in &data[..frames] {
                    let s = (b as f32 - 128.0) / 128.0;
                    self.pcm_left.push(s);
                    self.pcm_right.push(s);
                }
            }
            AL_FORMAT_MONO16 => {
                self.bits = 16;
                self.channels = 1;
                let frames = size / 2;
                self.pcm_left.reserve(frames);
                self.pcm_right.reserve(frames);
                for i in 0..frames {
                    let b0 = data[i * 2];
                    let b1 = data[i * 2 + 1];
                    let raw = i16::from_le_bytes([b0, b1]);
                    let s = (raw as f32) / 32768.0;
                    self.pcm_left.push(s);
                    self.pcm_right.push(s);
                }
            }
            AL_FORMAT_STEREO8 => {
                self.bits = 8;
                self.channels = 2;
                let frames = size / 2;
                self.pcm_left.reserve(frames);
                self.pcm_right.reserve(frames);
                for i in 0..frames {
                    let l = (data[i * 2] as f32 - 128.0) / 128.0;
                    let r = (data[i * 2 + 1] as f32 - 128.0) / 128.0;
                    self.pcm_left.push(l);
                    self.pcm_right.push(r);
                }
            }
            AL_FORMAT_STEREO16 => {
                self.bits = 16;
                self.channels = 2;
                let frames = size / 4;
                self.pcm_left.reserve(frames);
                self.pcm_right.reserve(frames);
                for i in 0..frames {
                    let l_raw = i16::from_le_bytes([data[i * 4], data[i * 4 + 1]]);
                    let r_raw = i16::from_le_bytes([data[i * 4 + 2], data[i * 4 + 3]]);
                    self.pcm_left.push((l_raw as f32) / 32768.0);
                    self.pcm_right.push((r_raw as f32) / 32768.0);
                }
            }
            _ => return false,
        }

        true
    }
}

/// OpenAL 3D listener state representing the virtual listener in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ALListener {
    pub position: Vec3,
    pub velocity: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    pub gain: f32,
}

impl Default for ALListener {
    fn default() -> Self {
        Self {
            position: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            forward: Vec3 { x: 0.0, y: 0.0, z: -1.0 },
            up: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
            gain: 1.0,
        }
    }
}

/// OpenAL 3D audio source emitting sound within virtual 3D space.
#[derive(Debug, Clone, PartialEq)]
pub struct ALSource {
    pub id: ALuint,
    pub state: ALenum,
    pub buffer_id: ALuint,
    pub source_relative: bool,
    pub looping: bool,
    pub pitch: f32,
    pub gain: f32,
    pub min_gain: f32,
    pub max_gain: f32,
    pub reference_distance: f32,
    pub max_distance: f32,
    pub rolloff_factor: f32,
    pub position: Vec3,
    pub velocity: Vec3,
    pub direction: Vec3,
    pub playback_sample_offset: f32,
}

impl ALSource {
    // Creates a new OpenAL sound source initialized to default attributes.
    pub fn new(id: ALuint) -> Self {
        Self {
            id,
            state: AL_INITIAL,
            buffer_id: 0,
            source_relative: false,
            looping: false,
            pitch: 1.0,
            gain: 1.0,
            min_gain: 0.0,
            max_gain: 1.0,
            reference_distance: 1.0,
            max_distance: 10000.0,
            rolloff_factor: 1.0,
            position: Vec3::zero(),
            velocity: Vec3::zero(),
            direction: Vec3::zero(),
            playback_sample_offset: 0.0,
        }
    }

    // Calculates left/right spatial gain attenuation and panning based on distance and listener position.
    pub fn calculate_spatial_gains(&self, listener: &ALListener, distance_model: ALenum) -> (f32, f32) {
        let (distance, pan) = if self.source_relative {
            let dist = self.position.length();
            let p = if dist > 1e-4 {
                (self.position.x / dist).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            (dist, p)
        } else {
            let delta = self.position.sub(listener.position);
            let dist = delta.length();

            let forward = listener.forward.normalize();
            let up = listener.up.normalize();
            let right = forward.cross(up).normalize();

            let x_rel = delta.dot(right);
            let p = if dist > 1e-4 {
                (x_rel / dist).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            (dist, p)
        };

        // Distance attenuation
        let atten = match distance_model {
            AL_INVERSE_DISTANCE_CLAMPED => {
                let d = distance.clamp(self.reference_distance, self.max_distance);
                self.reference_distance / (self.reference_distance + self.rolloff_factor * (d - self.reference_distance))
            }
            AL_LINEAR_DISTANCE_CLAMPED => {
                let d = distance.clamp(self.reference_distance, self.max_distance);
                1.0 - self.rolloff_factor * (d - self.reference_distance) / (self.max_distance - self.reference_distance).max(1e-4)
            }
            _ => {
                if distance > 1e-4 {
                    self.reference_distance / (self.reference_distance + self.rolloff_factor * (distance - self.reference_distance).max(0.0))
                } else {
                    1.0
                }
            }
        };

        let effective_gain = (self.gain * listener.gain * atten.clamp(0.0, 1.0)).clamp(self.min_gain, self.max_gain);
        let pan_l = ((1.0 - pan) * 0.5).clamp(0.0, 1.0);
        let pan_r = ((1.0 + pan) * 0.5).clamp(0.0, 1.0);

        (effective_gain * pan_l, effective_gain * pan_r)
    }

    // Renders audio frames from bound buffer into destination left and right mixing buffers.
    pub fn render_frames(
        &mut self,
        frames: usize,
        buffer: Option<&ALBuffer>,
        listener: &ALListener,
        distance_model: ALenum,
        out_l: &mut [f32],
        out_r: &mut [f32],
    ) -> usize {
        if self.state != AL_PLAYING {
            return 0;
        }

        let buf = match buffer {
            Some(b) if b.num_frames() > 0 => b,
            _ => {
                self.state = AL_STOPPED;
                return 0;
            }
        };

        let (gain_l, gain_r) = self.calculate_spatial_gains(listener, distance_model);
        let total_frames = buf.num_frames();
        let step = self.pitch.clamp(0.1, 4.0);
        let mut frames_rendered = 0;

        for f in 0..frames {
            let idx_floor = self.playback_sample_offset as usize;
            if idx_floor >= total_frames {
                if self.looping {
                    self.playback_sample_offset %= total_frames as f32;
                } else {
                    self.state = AL_STOPPED;
                    self.playback_sample_offset = 0.0;
                    break;
                }
            }

            let curr = self.playback_sample_offset as usize;
            let next = if curr + 1 < total_frames {
                curr + 1
            } else if self.looping {
                0
            } else {
                curr
            };

            let frac = self.playback_sample_offset - (curr as f32);
            let s_l = buf.pcm_left[curr] + frac * (buf.pcm_left[next] - buf.pcm_left[curr]);
            let s_r = buf.pcm_right[curr] + frac * (buf.pcm_right[next] - buf.pcm_right[curr]);

            out_l[f] += s_l * gain_l;
            out_r[f] += s_r * gain_r;

            self.playback_sample_offset += step;
            frames_rendered += 1;
        }

        frames_rendered
    }
}

/// ALC physical or virtual audio output device descriptor.
#[derive(Debug, Clone)]
pub struct ALCDevice {
    pub handle: ALuint,
    pub name: String,
    pub sample_rate: u32,
    pub is_open: bool,
    pub last_error: ALenum,
}

impl ALCDevice {
    // Creates a new OpenAL audio playback device.
    pub fn new(handle: ALuint, name: &str, sample_rate: u32) -> Self {
        Self {
            handle,
            name: String::from(name),
            sample_rate,
            is_open: true,
            last_error: ALC_NO_ERROR,
        }
    }
}

/// ALC execution context encapsulating listener, sources, and buffers.
#[derive(Debug, Clone)]
pub struct ALCContext {
    pub handle: ALuint,
    pub device_handle: ALuint,
    pub listener: ALListener,
    pub distance_model: ALenum,
    pub last_error: ALenum,
}

impl ALCContext {
    // Creates a new context bound to specified device.
    pub fn new(handle: ALuint, device_handle: ALuint) -> Self {
        Self {
            handle,
            device_handle,
            listener: ALListener::default(),
            distance_model: AL_INVERSE_DISTANCE_CLAMPED,
            last_error: AL_NO_ERROR,
        }
    }
}

/// Global OpenAL and ALC engine manager tracking devices, contexts, and active state.
#[derive(Debug, Clone, Default)]
pub struct OpenALEngine {
    pub devices: BTreeMap<ALuint, ALCDevice>,
    pub contexts: BTreeMap<ALuint, ALCContext>,
    pub buffers: BTreeMap<ALuint, ALBuffer>,
    pub sources: BTreeMap<ALuint, ALSource>,
    pub current_context: ALuint,
    pub next_device_handle: ALuint,
    pub next_context_handle: ALuint,
    pub next_buffer_id: ALuint,
    pub next_source_id: ALuint,
}

impl OpenALEngine {
    // Creates an empty OpenAL engine manager.
    pub fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
            contexts: BTreeMap::new(),
            buffers: BTreeMap::new(),
            sources: BTreeMap::new(),
            current_context: 0,
            next_device_handle: 1,
            next_context_handle: 1,
            next_buffer_id: 1,
            next_source_id: 1,
        }
    }

    // Opens an audio output device.
    pub fn alc_open_device(&mut self, dev_name: Option<&str>) -> Option<ALuint> {
        let handle = self.next_device_handle;
        self.next_device_handle = self.next_device_handle.wrapping_add(1).max(1);

        let name = dev_name.unwrap_or("KntonOS High Definition Audio");
        let dev = ALCDevice::new(handle, name, 48000);
        self.devices.insert(handle, dev);
        Some(handle)
    }

    // Closes an active audio output device.
    pub fn alc_close_device(&mut self, dev_handle: ALuint) -> ALboolean {
        if let Some(mut dev) = self.devices.remove(&dev_handle) {
            dev.is_open = false;
            ALC_TRUE
        } else {
            ALC_FALSE
        }
    }

    // Creates an audio execution context on specified device.
    pub fn alc_create_context(&mut self, dev_handle: ALuint) -> Option<ALuint> {
        if !self.devices.contains_key(&dev_handle) {
            return None;
        }

        let ctx_handle = self.next_context_handle;
        self.next_context_handle = self.next_context_handle.wrapping_add(1).max(1);

        let ctx = ALCContext::new(ctx_handle, dev_handle);
        self.contexts.insert(ctx_handle, ctx);
        Some(ctx_handle)
    }

    // Sets the current active execution context.
    pub fn alc_make_context_current(&mut self, ctx_handle: ALuint) -> ALboolean {
        if ctx_handle == 0 {
            self.current_context = 0;
            return ALC_TRUE;
        }

        if self.contexts.contains_key(&ctx_handle) {
            self.current_context = ctx_handle;
            ALC_TRUE
        } else {
            ALC_FALSE
        }
    }

    // Returns the current execution context handle.
    pub fn alc_get_current_context(&self) -> ALuint {
        self.current_context
    }

    // Destroys an existing execution context.
    pub fn alc_destroy_context(&mut self, ctx_handle: ALuint) -> ALboolean {
        if self.current_context == ctx_handle {
            self.current_context = 0;
        }
        if self.contexts.remove(&ctx_handle).is_some() {
            ALC_TRUE
        } else {
            ALC_FALSE
        }
    }

    // Retrieves and clears the last ALC error for device.
    pub fn alc_get_error(&mut self, dev_handle: ALuint) -> ALenum {
        if let Some(dev) = self.devices.get_mut(&dev_handle) {
            let err = dev.last_error;
            dev.last_error = ALC_NO_ERROR;
            err
        } else {
            ALC_INVALID_DEVICE
        }
    }

    // Retrieves and clears the last AL error for current context.
    pub fn al_get_error(&mut self) -> ALenum {
        if let Some(ctx) = self.contexts.get_mut(&self.current_context) {
            let err = ctx.last_error;
            ctx.last_error = AL_NO_ERROR;
            err
        } else {
            AL_NO_ERROR
        }
    }

    // Allocates n buffer object names.
    pub fn al_gen_buffers(&mut self, n: ALsizei, out_buffers: &mut [ALuint]) -> ALenum {
        if n < 0 || out_buffers.len() < n as usize {
            return AL_INVALID_VALUE;
        }
        for i in 0..n as usize {
            let id = self.next_buffer_id;
            self.next_buffer_id = self.next_buffer_id.wrapping_add(1).max(1);
            self.buffers.insert(id, ALBuffer::new(id));
            out_buffers[i] = id;
        }
        AL_NO_ERROR
    }

    // Deletes n buffer objects.
    pub fn al_delete_buffers(&mut self, n: ALsizei, buffers: &[ALuint]) -> ALenum {
        if n < 0 || buffers.len() < n as usize {
            return AL_INVALID_VALUE;
        }
        for &id in &buffers[..n as usize] {
            if id != 0 {
                self.buffers.remove(&id);
            }
        }
        AL_NO_ERROR
    }

    // Checks if identifier is a valid OpenAL buffer.
    pub fn al_is_buffer(&self, buffer: ALuint) -> ALboolean {
        if self.buffers.contains_key(&buffer) {
            AL_TRUE
        } else {
            AL_FALSE
        }
    }

    // Uploads PCM audio data into specified OpenAL buffer with bounds and alignment verification.
    pub fn al_buffer_data(&mut self, buffer: ALuint, format: ALenum, data: &[u8], size: ALsizei, freq: ALsizei) -> ALenum {
        if size < 0 || freq <= 0 || (size as usize) > data.len() {
            return AL_INVALID_VALUE;
        }

        let sz = size as usize;
        let align = match format {
            AL_FORMAT_MONO8 => 1,
            AL_FORMAT_MONO16 => 2,
            AL_FORMAT_STEREO8 => 2,
            AL_FORMAT_STEREO16 => 4,
            _ => return AL_INVALID_ENUM,
        };

        if sz % align != 0 {
            return AL_INVALID_VALUE;
        }

        let buf = match self.buffers.get_mut(&buffer) {
            Some(b) => b,
            None => return AL_INVALID_NAME,
        };

        if buf.load_pcm_data(format, data, sz, freq) {
            AL_NO_ERROR
        } else {
            AL_INVALID_VALUE
        }
    }

    // Queries integer parameters from an OpenAL buffer.
    pub fn al_get_bufferi(&self, buffer: ALuint, param: ALenum, out_val: &mut ALint) -> ALenum {
        let buf = match self.buffers.get(&buffer) {
            Some(b) => b,
            None => return AL_INVALID_NAME,
        };

        match param {
            AL_FREQUENCY => {
                *out_val = buf.frequency;
                AL_NO_ERROR
            }
            AL_BITS => {
                *out_val = buf.bits;
                AL_NO_ERROR
            }
            AL_CHANNELS => {
                *out_val = buf.channels;
                AL_NO_ERROR
            }
            AL_SIZE => {
                *out_val = buf.raw_bytes.len() as ALint;
                AL_NO_ERROR
            }
            _ => AL_INVALID_ENUM,
        }
    }

    // Allocates n source object names.
    pub fn al_gen_sources(&mut self, n: ALsizei, out_sources: &mut [ALuint]) -> ALenum {
        if n < 0 || out_sources.len() < n as usize {
            return AL_INVALID_VALUE;
        }
        for i in 0..n as usize {
            let id = self.next_source_id;
            self.next_source_id = self.next_source_id.wrapping_add(1).max(1);
            self.sources.insert(id, ALSource::new(id));
            out_sources[i] = id;
        }
        AL_NO_ERROR
    }

    // Deletes n source objects.
    pub fn al_delete_sources(&mut self, n: ALsizei, sources: &[ALuint]) -> ALenum {
        if n < 0 || sources.len() < n as usize {
            return AL_INVALID_VALUE;
        }
        for &id in &sources[..n as usize] {
            if id != 0 {
                self.sources.remove(&id);
            }
        }
        AL_NO_ERROR
    }

    // Checks if identifier is a valid OpenAL source.
    pub fn al_is_source(&self, source: ALuint) -> ALboolean {
        if self.sources.contains_key(&source) {
            AL_TRUE
        } else {
            AL_FALSE
        }
    }

    // Sets integer source parameters.
    pub fn al_sourcei(&mut self, source: ALuint, param: ALenum, value: ALint) -> ALenum {
        let src = match self.sources.get_mut(&source) {
            Some(s) => s,
            None => return AL_INVALID_NAME,
        };

        match param {
            AL_BUFFER => {
                src.buffer_id = value as ALuint;
                src.playback_sample_offset = 0.0;
                AL_NO_ERROR
            }
            AL_LOOPING => {
                src.looping = value == (AL_TRUE as ALint);
                AL_NO_ERROR
            }
            AL_SOURCE_RELATIVE => {
                src.source_relative = value == (AL_TRUE as ALint);
                AL_NO_ERROR
            }
            _ => AL_INVALID_ENUM,
        }
    }

    // Sets floating-point source parameters.
    pub fn al_sourcef(&mut self, source: ALuint, param: ALenum, value: ALfloat) -> ALenum {
        let src = match self.sources.get_mut(&source) {
            Some(s) => s,
            None => return AL_INVALID_NAME,
        };

        match param {
            AL_PITCH => {
                src.pitch = value.clamp(0.1, 4.0);
                AL_NO_ERROR
            }
            AL_GAIN => {
                src.gain = value.max(0.0);
                AL_NO_ERROR
            }
            AL_MIN_GAIN => {
                src.min_gain = value.clamp(0.0, 1.0);
                AL_NO_ERROR
            }
            AL_MAX_GAIN => {
                src.max_gain = value.clamp(0.0, 1.0);
                AL_NO_ERROR
            }
            AL_REFERENCE_DISTANCE => {
                src.reference_distance = value.max(0.0);
                AL_NO_ERROR
            }
            AL_MAX_DISTANCE => {
                src.max_distance = value.max(0.0);
                AL_NO_ERROR
            }
            AL_ROLLOFF_FACTOR => {
                src.rolloff_factor = value.max(0.0);
                AL_NO_ERROR
            }
            _ => AL_INVALID_ENUM,
        }
    }

    // Sets 3-component floating-point vector source parameters.
    pub fn al_source3f(&mut self, source: ALuint, param: ALenum, v1: ALfloat, v2: ALfloat, v3: ALfloat) -> ALenum {
        let src = match self.sources.get_mut(&source) {
            Some(s) => s,
            None => return AL_INVALID_NAME,
        };

        match param {
            AL_POSITION => {
                src.position = Vec3::new(v1, v2, v3);
                AL_NO_ERROR
            }
            AL_VELOCITY => {
                src.velocity = Vec3::new(v1, v2, v3);
                AL_NO_ERROR
            }
            AL_DIRECTION => {
                src.direction = Vec3::new(v1, v2, v3);
                AL_NO_ERROR
            }
            _ => AL_INVALID_ENUM,
        }
    }

    // Starts playback on specified source.
    pub fn al_source_play(&mut self, source: ALuint) -> ALenum {
        let src = match self.sources.get_mut(&source) {
            Some(s) => s,
            None => return AL_INVALID_NAME,
        };
        src.state = AL_PLAYING;
        AL_NO_ERROR
    }

    // Pauses playback on specified source.
    pub fn al_source_pause(&mut self, source: ALuint) -> ALenum {
        let src = match self.sources.get_mut(&source) {
            Some(s) => s,
            None => return AL_INVALID_NAME,
        };
        if src.state == AL_PLAYING {
            src.state = AL_PAUSED;
        }
        AL_NO_ERROR
    }

    // Stops playback on specified source and resets offset.
    pub fn al_source_stop(&mut self, source: ALuint) -> ALenum {
        let src = match self.sources.get_mut(&source) {
            Some(s) => s,
            None => return AL_INVALID_NAME,
        };
        src.state = AL_STOPPED;
        src.playback_sample_offset = 0.0;
        AL_NO_ERROR
    }

    // Rewinds source playback to initial state.
    pub fn al_source_rewind(&mut self, source: ALuint) -> ALenum {
        let src = match self.sources.get_mut(&source) {
            Some(s) => s,
            None => return AL_INVALID_NAME,
        };
        src.state = AL_INITIAL;
        src.playback_sample_offset = 0.0;
        AL_NO_ERROR
    }

    // Queries integer parameters from an OpenAL source.
    pub fn al_get_sourcei(&self, source: ALuint, param: ALenum, out_val: &mut ALint) -> ALenum {
        let src = match self.sources.get(&source) {
            Some(s) => s,
            None => return AL_INVALID_NAME,
        };

        match param {
            AL_SOURCE_STATE => {
                *out_val = src.state;
                AL_NO_ERROR
            }
            AL_BUFFER => {
                *out_val = src.buffer_id as ALint;
                AL_NO_ERROR
            }
            AL_LOOPING => {
                *out_val = if src.looping { AL_TRUE as ALint } else { AL_FALSE as ALint };
                AL_NO_ERROR
            }
            _ => AL_INVALID_ENUM,
        }
    }

    // Blends all active playing sources into output audio frames.
    pub fn render_sources(&mut self, frames: usize, out_l: &mut [f32], out_r: &mut [f32]) -> usize {
        let listener = if let Some(ctx) = self.contexts.get(&self.current_context) {
            ctx.listener
        } else {
            ALListener::default()
        };
        let distance_model = if let Some(ctx) = self.contexts.get(&self.current_context) {
            ctx.distance_model
        } else {
            AL_INVERSE_DISTANCE_CLAMPED
        };

        let mut total_rendered = 0;
        for src in self.sources.values_mut() {
            if src.state == AL_PLAYING {
                let buf_opt = self.buffers.get(&src.buffer_id);
                let count = src.render_frames(frames, buf_opt, &listener, distance_model, out_l, out_r);
                total_rendered = total_rendered.max(count);
            }
        }

        total_rendered
    }
}

/// Verifies ALC device opening, context creation, binding, and error tracking.
pub fn verify_alc_device_and_context_lifecycle() -> bool {
    let mut engine = OpenALEngine::new();

    // 1. Open default audio device
    let dev_opt = engine.alc_open_device(None);
    if dev_opt.is_none() {
        return false;
    }
    let dev = dev_opt.unwrap();

    // 2. Create context on opened device
    let ctx_opt = engine.alc_create_context(dev);
    if ctx_opt.is_none() {
        return false;
    }
    let ctx = ctx_opt.unwrap();

    // 3. Make context current and verify
    if engine.alc_make_context_current(ctx) != ALC_TRUE {
        return false;
    }
    if engine.alc_get_current_context() != ctx {
        return false;
    }

    // 4. Verify context listener defaults
    let cur_ctx = engine.contexts.get(&ctx).unwrap();
    if cur_ctx.listener.gain != 1.0 || cur_ctx.listener.forward.z != -1.0 {
        return false;
    }

    // 5. Test context unbinding and destruction
    if engine.alc_make_context_current(0) != ALC_TRUE {
        return false;
    }
    if engine.alc_get_current_context() != 0 {
        return false;
    }
    if engine.alc_destroy_context(ctx) != ALC_TRUE {
        return false;
    }
    if engine.alc_close_device(dev) != ALC_TRUE {
        return false;
    }

    true
}

/// Verifies OpenAL buffer generation, format bounds checking, PCM uploading, and querying.
pub fn verify_openal_buffer_management() -> bool {
    let mut engine = OpenALEngine::new();

    // 1. Generate buffers
    let mut buf_ids = [0u32; 2];
    if engine.al_gen_buffers(2, &mut buf_ids) != AL_NO_ERROR {
        return false;
    }
    if engine.al_is_buffer(buf_ids[0]) != AL_TRUE || engine.al_is_buffer(buf_ids[1]) != AL_TRUE {
        return false;
    }

    // 2. Test invalid buffer format
    let dummy_data = [0u8; 16];
    if engine.al_buffer_data(buf_ids[0], 0x9999, &dummy_data, 16, 44100) != AL_INVALID_ENUM {
        return false;
    }

    // 3. Test misaligned size (e.g. 16-bit mono requires size % 2 == 0)
    if engine.al_buffer_data(buf_ids[0], AL_FORMAT_MONO16, &dummy_data, 15, 44100) != AL_INVALID_VALUE {
        return false;
    }

    // 4. Upload valid stereo 16-bit PCM (4 frames = 16 bytes)
    let mut pcm_bytes = Vec::new();
    for _ in 0..4 {
        let l: i16 = 16384;
        let r: i16 = -16384;
        pcm_bytes.extend_from_slice(&l.to_le_bytes());
        pcm_bytes.extend_from_slice(&r.to_le_bytes());
    }

    if engine.al_buffer_data(buf_ids[0], AL_FORMAT_STEREO16, &pcm_bytes, pcm_bytes.len() as ALsizei, 48000) != AL_NO_ERROR {
        return false;
    }

    // 5. Query buffer attributes
    let mut freq = 0;
    let mut bits = 0;
    let mut channels = 0;
    let mut size = 0;
    if engine.al_get_bufferi(buf_ids[0], AL_FREQUENCY, &mut freq) != AL_NO_ERROR || freq != 48000 {
        return false;
    }
    if engine.al_get_bufferi(buf_ids[0], AL_BITS, &mut bits) != AL_NO_ERROR || bits != 16 {
        return false;
    }
    if engine.al_get_bufferi(buf_ids[0], AL_CHANNELS, &mut channels) != AL_NO_ERROR || channels != 2 {
        return false;
    }
    if engine.al_get_bufferi(buf_ids[0], AL_SIZE, &mut size) != AL_NO_ERROR || size != 16 {
        return false;
    }

    let buf = engine.buffers.get(&buf_ids[0]).unwrap();
    if buf.num_frames() != 4 || (buf.pcm_left[0] - 0.5).abs() > 0.01 || (buf.pcm_right[0] + 0.5).abs() > 0.01 {
        return false;
    }

    // 6. Delete buffers
    if engine.al_delete_buffers(2, &buf_ids) != AL_NO_ERROR {
        return false;
    }
    if engine.al_is_buffer(buf_ids[0]) == AL_TRUE {
        return false;
    }

    true
}

/// Verifies OpenAL source generation, 3D spatial panning, pitch/gain controls, and playback states.
pub fn verify_openal_source_management() -> bool {
    let mut engine = OpenALEngine::new();

    let dev = engine.alc_open_device(None).unwrap();
    let ctx = engine.alc_create_context(dev).unwrap();
    engine.alc_make_context_current(ctx);

    // 1. Generate buffer with mono 16-bit constant PCM
    let mut buf = [0u32; 1];
    engine.al_gen_buffers(1, &mut buf);
    let mut pcm = Vec::new();
    for _ in 0..16 {
        let sample: i16 = 16384; // 0.5
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    engine.al_buffer_data(buf[0], AL_FORMAT_MONO16, &pcm, pcm.len() as ALsizei, 48000);

    // 2. Generate source and bind buffer
    let mut src = [0u32; 1];
    if engine.al_gen_sources(1, &mut src) != AL_NO_ERROR {
        return false;
    }
    engine.al_sourcei(src[0], AL_BUFFER, buf[0] as ALint);
    engine.al_sourcei(src[0], AL_LOOPING, AL_TRUE as ALint);

    // Place source to the far right (+10.0, 0, 0)
    engine.al_source3f(src[0], AL_POSITION, 10.0, 0.0, 0.0);
    engine.al_sourcef(src[0], AL_GAIN, 1.0);
    engine.al_sourcef(src[0], AL_PITCH, 1.0);

    // 3. Play source
    if engine.al_source_play(src[0]) != AL_NO_ERROR {
        return false;
    }
    let mut state = 0;
    engine.al_get_sourcei(src[0], AL_SOURCE_STATE, &mut state);
    if state != AL_PLAYING {
        return false;
    }

    // 4. Render frames and check right-panning dominance
    let mut out_l = alloc::vec![0.0f32; 16];
    let mut out_r = alloc::vec![0.0f32; 16];
    let count = engine.render_sources(16, &mut out_l, &mut out_r);
    if count != 16 {
        return false;
    }
    if out_r[0] <= out_l[0] {
        return false;
    }

    // 5. Test pause and stop
    engine.al_source_pause(src[0]);
    engine.al_get_sourcei(src[0], AL_SOURCE_STATE, &mut state);
    if state != AL_PAUSED {
        return false;
    }

    engine.al_source_stop(src[0]);
    engine.al_get_sourcei(src[0], AL_SOURCE_STATE, &mut state);
    if state != AL_STOPPED {
        return false;
    }

    // 6. Delete source
    engine.al_delete_sources(1, &src);
    if engine.al_is_source(src[0]) == AL_TRUE {
        return false;
    }

    true
}
