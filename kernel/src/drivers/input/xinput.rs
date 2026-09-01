//! Userland Game Controller and Win32 XInput Runtime Service.

#![allow(non_snake_case)]

use core::mem::size_of;

// Standard Win32 XInput button bitmasks.
pub const XINPUT_GAMEPAD_DPAD_UP: u16 = 0x0001;
pub const XINPUT_GAMEPAD_DPAD_DOWN: u16 = 0x0002;
pub const XINPUT_GAMEPAD_DPAD_LEFT: u16 = 0x0004;
pub const XINPUT_GAMEPAD_DPAD_RIGHT: u16 = 0x0008;
pub const XINPUT_GAMEPAD_START: u16 = 0x0010;
pub const XINPUT_GAMEPAD_BACK: u16 = 0x0020;
pub const XINPUT_GAMEPAD_LEFT_THUMB: u16 = 0x0040;
pub const XINPUT_GAMEPAD_RIGHT_THUMB: u16 = 0x0080;
pub const XINPUT_GAMEPAD_LEFT_SHOULDER: u16 = 0x0100;
pub const XINPUT_GAMEPAD_RIGHT_SHOULDER: u16 = 0x0200;
pub const XINPUT_GAMEPAD_A: u16 = 0x1000;
pub const XINPUT_GAMEPAD_B: u16 = 0x2000;
pub const XINPUT_GAMEPAD_X: u16 = 0x4000;
pub const XINPUT_GAMEPAD_Y: u16 = 0x8000;

// Standard Win32 XInput deadzone and trigger thresholds.
pub const XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE: i16 = 7849;
pub const XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE: i16 = 8689;
pub const XINPUT_GAMEPAD_TRIGGER_THRESHOLD: u8 = 30;

// Standard Win32 error codes for XInput APIs.
pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_DEVICE_NOT_CONNECTED: u32 = 1167;
pub const ERROR_EMPTY: u32 = 4306;

// Device types and subtypes.
pub const XINPUT_DEVTYPE_GAMEPAD: u8 = 0x01;
pub const XINPUT_DEVSUBTYPE_GAMEPAD: u8 = 0x01;
pub const XINPUT_FLAG_GAMEPAD: u16 = 0x0001;

// Battery types and levels.
pub const BATTERY_TYPE_DISCONNECTED: u8 = 0x00;
pub const BATTERY_TYPE_WIRED: u8 = 0x01;
pub const BATTERY_TYPE_ALKALINE: u8 = 0x02;
pub const BATTERY_TYPE_NIMH: u8 = 0x03;
pub const BATTERY_LEVEL_EMPTY: u8 = 0x00;
pub const BATTERY_LEVEL_LOW: u8 = 0x01;
pub const BATTERY_LEVEL_MEDIUM: u8 = 0x02;
pub const BATTERY_LEVEL_FULL: u8 = 0x03;

/// Describes current state of standard Win32 XInput controller buttons, triggers, and thumbsticks.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputGamepad {
    pub wButtons: u16,
    pub bLeftTrigger: u8,
    pub bRightTrigger: u8,
    pub sThumbLX: i16,
    pub sThumbLY: i16,
    pub sThumbRX: i16,
    pub sThumbRY: i16,
}

impl XInputGamepad {
    // Tests whether a specific button bitflag is set.
    #[inline]
    pub const fn has_button(&self, mask: u16) -> bool {
        (self.wButtons & mask) != 0
    }

    // Sets or clears a specific button bitflag.
    #[inline]
    pub fn set_button(&mut self, mask: u16, pressed: bool) {
        if pressed {
            self.wButtons |= mask;
        } else {
            self.wButtons &= !mask;
        }
    }
}

/// Represents state packet of XInput controller with monotonic sequence counter.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputState {
    pub dwPacketNumber: u32,
    pub Gamepad: XInputGamepad,
}

/// Vibration speeds for controller dual rumble motors.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputVibration {
    pub wLeftMotorSpeed: u16,
    pub wRightMotorSpeed: u16,
}

/// Capabilities descriptor for connected XInput device.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputCapabilities {
    pub Type: u8,
    pub SubType: u8,
    pub Flags: u16,
    pub Gamepad: XInputGamepad,
    pub Vibration: XInputVibration,
}

/// Battery status report for wireless or wired controller.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XInputBatteryInformation {
    pub BatteryType: u8,
    pub BatteryLevel: u8,
}

/// Verifies memory layout, field alignments, and structure sizes for standard Win32 XInput compatibility.
pub fn verify_xinput_structures() -> bool {
    if size_of::<XInputGamepad>() != 12 {
        return false;
    }
    if size_of::<XInputState>() != 16 {
        return false;
    }
    if size_of::<XInputVibration>() != 4 {
        return false;
    }
    if size_of::<XInputBatteryInformation>() != 2 {
        return false;
    }

    let mut pad = XInputGamepad::default();
    pad.set_button(XINPUT_GAMEPAD_A | XINPUT_GAMEPAD_DPAD_UP, true);
    if !pad.has_button(XINPUT_GAMEPAD_A) || !pad.has_button(XINPUT_GAMEPAD_DPAD_UP) {
        return false;
    }
    if pad.has_button(XINPUT_GAMEPAD_B) {
        return false;
    }
    pad.set_button(XINPUT_GAMEPAD_A, false);
    if pad.has_button(XINPUT_GAMEPAD_A) || !pad.has_button(XINPUT_GAMEPAD_DPAD_UP) {
        return false;
    }

    true
}

use super::imu::sqrt_f32;

// Stick curvature response types for analog sticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeadzoneCurve {
    #[default]
    Linear,
    Quadratic,
    Cubic,
}

/// Configuration parameters for radial inner and outer thumbstick deadzones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialDeadzoneConfig {
    pub inner_deadzone: f32,
    pub outer_saturation: f32,
    pub curve: DeadzoneCurve,
}

impl Default for RadialDeadzoneConfig {
    fn default() -> Self {
        Self {
            inner_deadzone: XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE as f32,
            outer_saturation: 32767.0,
            curve: DeadzoneCurve::Linear,
        }
    }
}

impl RadialDeadzoneConfig {
    // Creates a new configuration for left thumbstick with standard defaults.
    pub const fn left_thumb() -> Self {
        Self {
            inner_deadzone: XINPUT_GAMEPAD_LEFT_THUMB_DEADZONE as f32,
            outer_saturation: 32767.0,
            curve: DeadzoneCurve::Linear,
        }
    }

    // Creates a new configuration for right thumbstick with standard defaults.
    pub const fn right_thumb() -> Self {
        Self {
            inner_deadzone: XINPUT_GAMEPAD_RIGHT_THUMB_DEADZONE as f32,
            outer_saturation: 32767.0,
            curve: DeadzoneCurve::Linear,
        }
    }

    // Filters raw 2D analog stick inputs using radial deadzone and saturation scaling.
    pub fn apply(&self, raw_x: i16, raw_y: i16) -> (i16, i16) {
        let fx = raw_x as f32;
        let fy = raw_y as f32;
        let mag = sqrt_f32(fx * fx + fy * fy);

        if mag <= self.inner_deadzone || mag < 1.0 {
            return (0, 0);
        }

        let clamped_mag = if mag > self.outer_saturation {
            self.outer_saturation
        } else {
            mag
        };

        let span = self.outer_saturation - self.inner_deadzone;
        if span <= 1.0 {
            return (0, 0);
        }

        let normalized = (clamped_mag - self.inner_deadzone) / span;
        let scaled_factor = match self.curve {
            DeadzoneCurve::Linear => normalized,
            DeadzoneCurve::Quadratic => normalized * normalized,
            DeadzoneCurve::Cubic => normalized * normalized * normalized,
        };

        let target_mag = scaled_factor * 32767.0;
        let scale = target_mag / mag;

        let out_x = (fx * scale).clamp(-32768.0, 32767.0) as i16;
        let out_y = (fy * scale).clamp(-32768.0, 32767.0) as i16;
        (out_x, out_y)
    }
}

/// XInput thumbstick and trigger deadzone shaping and saturation filter engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XInputFilterEngine {
    pub left_stick: RadialDeadzoneConfig,
    pub right_stick: RadialDeadzoneConfig,
    pub left_trigger_threshold: u8,
    pub right_trigger_threshold: u8,
}

impl Default for XInputFilterEngine {
    fn default() -> Self {
        Self {
            left_stick: RadialDeadzoneConfig::left_thumb(),
            right_stick: RadialDeadzoneConfig::right_thumb(),
            left_trigger_threshold: XINPUT_GAMEPAD_TRIGGER_THRESHOLD,
            right_trigger_threshold: XINPUT_GAMEPAD_TRIGGER_THRESHOLD,
        }
    }
}

impl XInputFilterEngine {
    // Normalizes analog trigger input against deadzone threshold.
    pub fn filter_trigger(&self, raw_val: u8, threshold: u8) -> u8 {
        if raw_val <= threshold {
            return 0;
        }
        let span = (255 - threshold) as f32;
        let norm = ((raw_val - threshold) as f32) / span;
        (norm * 255.0).clamp(0.0, 255.0) as u8
    }

    // Applies full radial deadzone filtering and trigger thresholding to gamepad state.
    pub fn process_gamepad(&self, raw: &XInputGamepad) -> XInputGamepad {
        let (lx, ly) = self.left_stick.apply(raw.sThumbLX, raw.sThumbLY);
        let (rx, ry) = self.right_stick.apply(raw.sThumbRX, raw.sThumbRY);
        let lt = self.filter_trigger(raw.bLeftTrigger, self.left_trigger_threshold);
        let rt = self.filter_trigger(raw.bRightTrigger, self.right_trigger_threshold);

        XInputGamepad {
            wButtons: raw.wButtons,
            bLeftTrigger: lt,
            bRightTrigger: rt,
            sThumbLX: lx,
            sThumbLY: ly,
            sThumbRX: rx,
            sThumbRY: ry,
        }
    }
}

/// Verifies radial deadzone attenuation, saturation scaling, and trigger thresholding.
pub fn verify_xinput_deadzone_filter() -> bool {
    let engine = XInputFilterEngine::default();

    // 1. Stick noise within deadzone must yield exactly zero.
    let (lx0, ly0) = engine.left_stick.apply(3000, -4000);
    if lx0 != 0 || ly0 != 0 {
        return false;
    }

    // 2. Stick value exceeding outer saturation must clamp to 32767.
    let (lx_max, ly_max) = engine.left_stick.apply(0, 32767);
    if lx_max != 0 || ly_max < 32760 {
        return false;
    }

    // 3. Right stick with deadzone 8689 should attenuate noise below 8689.
    let (rx0, ry0) = engine.right_stick.apply(8000, 0);
    if rx0 != 0 || ry0 != 0 {
        return false;
    }

    // 4. Trigger values <= threshold must be 0, while full pull must reach 255.
    if engine.filter_trigger(25, XINPUT_GAMEPAD_TRIGGER_THRESHOLD) != 0 {
        return false;
    }
    if engine.filter_trigger(255, XINPUT_GAMEPAD_TRIGGER_THRESHOLD) != 255 {
        return false;
    }

    true
}
