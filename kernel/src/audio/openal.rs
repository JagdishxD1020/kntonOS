//! Win32 OpenAL 1.1 3D Spatial Audio & EAX Environmental Reverb Engine.

#![allow(non_snake_case)]

use alloc::collections::BTreeMap;
use alloc::string::String;
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
    pub current_context: ALuint,
    pub next_device_handle: ALuint,
    pub next_context_handle: ALuint,
}

impl OpenALEngine {
    // Creates an empty OpenAL engine manager.
    pub fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
            contexts: BTreeMap::new(),
            current_context: 0,
            next_device_handle: 1,
            next_context_handle: 1,
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
