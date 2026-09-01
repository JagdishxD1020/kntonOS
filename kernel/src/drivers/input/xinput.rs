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

use super::gamepad::{GamepadButtons, RawGamepadReport};

/// Maps raw IMU angular velocity to right-stick deflection deltas for motion aiming.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XInputGyroAimMapper {
    pub enabled: bool,
    pub yaw_sensitivity: f32,
    pub pitch_sensitivity: f32,
    pub min_velocity_threshold: f32,
    pub invert_pitch: bool,
    pub activation_mask: u16,
}

impl Default for XInputGyroAimMapper {
    fn default() -> Self {
        Self {
            enabled: true,
            yaw_sensitivity: 180.0,
            pitch_sensitivity: 180.0,
            min_velocity_threshold: 0.02,
            invert_pitch: false,
            activation_mask: 0,
        }
    }
}

impl XInputGyroAimMapper {
    // Calculates aim deltas from gyro angular velocities in radians per second.
    pub fn compute_aim_delta(&self, gyro_yaw_rad_s: f32, gyro_pitch_rad_s: f32, dt_s: f32) -> (f32, f32) {
        if !self.enabled {
            return (0.0, 0.0);
        }

        let yaw_v = if gyro_yaw_rad_s.abs() < self.min_velocity_threshold {
            0.0
        } else {
            gyro_yaw_rad_s
        };

        let pitch_v = if gyro_pitch_rad_s.abs() < self.min_velocity_threshold {
            0.0
        } else {
            gyro_pitch_rad_s
        };

        let dx = yaw_v * self.yaw_sensitivity * dt_s * 32767.0;
        let mut dy = pitch_v * self.pitch_sensitivity * dt_s * 32767.0;
        if self.invert_pitch {
            dy = -dy;
        }

        (dx, dy)
    }

    // Injects gyro angular velocity aim offsets directly into the right thumbstick.
    pub fn inject_into_stick(&self, gamepad: &mut XInputGamepad, gyro_yaw_rad_s: f32, gyro_pitch_rad_s: f32, dt_s: f32) {
        if self.activation_mask != 0 && (gamepad.wButtons & self.activation_mask) == 0 {
            return;
        }

        let (dx, dy) = self.compute_aim_delta(gyro_yaw_rad_s, gyro_pitch_rad_s, dt_s);
        let rx = (gamepad.sThumbRX as f32 + dx).clamp(-32768.0, 32767.0) as i16;
        let ry = (gamepad.sThumbRY as f32 + dy).clamp(-32768.0, 32767.0) as i16;

        gamepad.sThumbRX = rx;
        gamepad.sThumbRY = ry;
    }
}

// Haptic trigger profile behaviors for adaptive feedback motors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HapticTriggerMode {
    #[default]
    Off,
    RigidStop,
    TwoStageClick,
    ProgressiveResistance,
    VibratingRecoil,
}

/// Dual-zone force resistance profile for adaptive controller triggers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XInputTriggerHapticProfile {
    pub mode: HapticTriggerMode,
    pub start_pos: u8,
    pub end_pos: u8,
    pub resistance_force: u8,
    pub vibration_hz: u8,
}

impl Default for XInputTriggerHapticProfile {
    fn default() -> Self {
        Self {
            mode: HapticTriggerMode::Off,
            start_pos: 0,
            end_pos: 255,
            resistance_force: 0,
            vibration_hz: 0,
        }
    }
}

impl XInputTriggerHapticProfile {
    // Computes motor force resistance and vibration intensity for a trigger position.
    pub fn evaluate(&self, trigger_pos: u8) -> (u8, u8) {
        match self.mode {
            HapticTriggerMode::Off => (0, 0),
            HapticTriggerMode::RigidStop => {
                if trigger_pos >= self.start_pos {
                    (self.resistance_force, 0)
                } else {
                    (0, 0)
                }
            }
            HapticTriggerMode::TwoStageClick => {
                if trigger_pos >= self.start_pos && trigger_pos < self.end_pos {
                    (self.resistance_force, 0)
                } else if trigger_pos >= self.end_pos {
                    // Release resistance past click point with a sharp vibration tick.
                    (self.resistance_force / 4, 255)
                } else {
                    (0, 0)
                }
            }
            HapticTriggerMode::ProgressiveResistance => {
                if trigger_pos < self.start_pos {
                    (0, 0)
                } else {
                    let span = (255 - self.start_pos).max(1) as f32;
                    let factor = ((trigger_pos - self.start_pos) as f32) / span;
                    let force = (factor * (self.resistance_force as f32)).clamp(0.0, 255.0) as u8;
                    (force, 0)
                }
            }
            HapticTriggerMode::VibratingRecoil => {
                if trigger_pos >= self.start_pos {
                    (self.resistance_force / 2, self.resistance_force)
                } else {
                    (0, 0)
                }
            }
        }
    }
}

/// Controller slot state maintained by userland XInput runtime service.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XInputControllerSlot {
    pub connected: bool,
    pub state: XInputState,
    pub vibration: XInputVibration,
    pub battery: XInputBatteryInformation,
    pub filter: XInputFilterEngine,
    pub gyro: XInputGyroAimMapper,
    pub left_trigger_profile: XInputTriggerHapticProfile,
    pub right_trigger_profile: XInputTriggerHapticProfile,
}

impl Default for XInputControllerSlot {
    fn default() -> Self {
        Self {
            connected: false,
            state: XInputState::default(),
            vibration: XInputVibration::default(),
            battery: XInputBatteryInformation {
                BatteryType: BATTERY_TYPE_WIRED,
                BatteryLevel: BATTERY_LEVEL_FULL,
            },
            filter: XInputFilterEngine::default(),
            gyro: XInputGyroAimMapper::default(),
            left_trigger_profile: XInputTriggerHapticProfile::default(),
            right_trigger_profile: XInputTriggerHapticProfile::default(),
        }
    }
}

/// Central Win32 XInput controller manager supporting up to 4 virtual/physical slots.
pub struct XInputRuntimeService {
    pub slots: [XInputControllerSlot; 4],
}

impl Default for XInputRuntimeService {
    fn default() -> Self {
        Self::new()
    }
}

impl XInputRuntimeService {
    // Creates a new XInput runtime service instance with slot 0 pre-connected.
    pub const fn new() -> Self {
        let default_slot = XInputControllerSlot {
            connected: false,
            state: XInputState {
                dwPacketNumber: 0,
                Gamepad: XInputGamepad {
                    wButtons: 0,
                    bLeftTrigger: 0,
                    bRightTrigger: 0,
                    sThumbLX: 0,
                    sThumbLY: 0,
                    sThumbRX: 0,
                    sThumbRY: 0,
                },
            },
            vibration: XInputVibration {
                wLeftMotorSpeed: 0,
                wRightMotorSpeed: 0,
            },
            battery: XInputBatteryInformation {
                BatteryType: BATTERY_TYPE_WIRED,
                BatteryLevel: BATTERY_LEVEL_FULL,
            },
            filter: XInputFilterEngine {
                left_stick: RadialDeadzoneConfig::left_thumb(),
                right_stick: RadialDeadzoneConfig::right_thumb(),
                left_trigger_threshold: XINPUT_GAMEPAD_TRIGGER_THRESHOLD,
                right_trigger_threshold: XINPUT_GAMEPAD_TRIGGER_THRESHOLD,
            },
            gyro: XInputGyroAimMapper {
                enabled: true,
                yaw_sensitivity: 180.0,
                pitch_sensitivity: 180.0,
                min_velocity_threshold: 0.02,
                invert_pitch: false,
                activation_mask: 0,
            },
            left_trigger_profile: XInputTriggerHapticProfile {
                mode: HapticTriggerMode::Off,
                start_pos: 0,
                end_pos: 255,
                resistance_force: 0,
                vibration_hz: 0,
            },
            right_trigger_profile: XInputTriggerHapticProfile {
                mode: HapticTriggerMode::Off,
                start_pos: 0,
                end_pos: 255,
                resistance_force: 0,
                vibration_hz: 0,
            },
        };

        Self {
            slots: [default_slot; 4],
        }
    }

    // Translates standard kernel gamepad buttons into standard Win32 XInput bitmask.
    pub fn map_buttons_to_xinput(buttons: u32) -> u16 {
        let mut w = 0u16;
        if (buttons & GamepadButtons::DPAD_UP.0) != 0 {
            w |= XINPUT_GAMEPAD_DPAD_UP;
        }
        if (buttons & GamepadButtons::DPAD_DOWN.0) != 0 {
            w |= XINPUT_GAMEPAD_DPAD_DOWN;
        }
        if (buttons & GamepadButtons::DPAD_LEFT.0) != 0 {
            w |= XINPUT_GAMEPAD_DPAD_LEFT;
        }
        if (buttons & GamepadButtons::DPAD_RIGHT.0) != 0 {
            w |= XINPUT_GAMEPAD_DPAD_RIGHT;
        }
        if (buttons & GamepadButtons::START.0) != 0 {
            w |= XINPUT_GAMEPAD_START;
        }
        if (buttons & GamepadButtons::SELECT.0) != 0 {
            w |= XINPUT_GAMEPAD_BACK;
        }
        if (buttons & GamepadButtons::LS.0) != 0 {
            w |= XINPUT_GAMEPAD_LEFT_THUMB;
        }
        if (buttons & GamepadButtons::RS.0) != 0 {
            w |= XINPUT_GAMEPAD_RIGHT_THUMB;
        }
        if (buttons & GamepadButtons::LB.0) != 0 {
            w |= XINPUT_GAMEPAD_LEFT_SHOULDER;
        }
        if (buttons & GamepadButtons::RB.0) != 0 {
            w |= XINPUT_GAMEPAD_RIGHT_SHOULDER;
        }
        if (buttons & GamepadButtons::A.0) != 0 {
            w |= XINPUT_GAMEPAD_A;
        }
        if (buttons & GamepadButtons::B.0) != 0 {
            w |= XINPUT_GAMEPAD_B;
        }
        if (buttons & GamepadButtons::X.0) != 0 {
            w |= XINPUT_GAMEPAD_X;
        }
        if (buttons & GamepadButtons::Y.0) != 0 {
            w |= XINPUT_GAMEPAD_Y;
        }
        w
    }

    // Ingests raw hardware gamepad and gyro inputs, performing filtering and updating state.
    pub fn update_slot(
        &mut self,
        user_index: usize,
        report: &RawGamepadReport,
        gyro_yaw_rad_s: f32,
        gyro_pitch_rad_s: f32,
        dt_s: f32,
    ) -> u32 {
        if user_index >= self.slots.len() {
            return ERROR_DEVICE_NOT_CONNECTED;
        }

        let slot = &mut self.slots[user_index];
        slot.connected = true;

        let raw_pad = XInputGamepad {
            wButtons: Self::map_buttons_to_xinput(report.buttons),
            bLeftTrigger: (report.left_trigger >> 8) as u8,
            bRightTrigger: (report.right_trigger >> 8) as u8,
            sThumbLX: report.left_stick_x,
            sThumbLY: report.left_stick_y,
            sThumbRX: report.right_stick_x,
            sThumbRY: report.right_stick_y,
        };

        let mut filtered_pad = slot.filter.process_gamepad(&raw_pad);
        slot.gyro.inject_into_stick(&mut filtered_pad, gyro_yaw_rad_s, gyro_pitch_rad_s, dt_s);

        slot.state.dwPacketNumber = slot.state.dwPacketNumber.wrapping_add(1);
        slot.state.Gamepad = filtered_pad;

        ERROR_SUCCESS
    }

    // Win32 XInputGetState API implementation.
    pub fn get_state(&self, user_index: usize, out_state: &mut XInputState) -> u32 {
        if user_index >= self.slots.len() || !self.slots[user_index].connected {
            return ERROR_DEVICE_NOT_CONNECTED;
        }
        *out_state = self.slots[user_index].state;
        ERROR_SUCCESS
    }

    // Win32 XInputSetState API implementation for dual-motor vibration.
    pub fn set_state(&mut self, user_index: usize, vibration: &XInputVibration) -> u32 {
        if user_index >= self.slots.len() || !self.slots[user_index].connected {
            return ERROR_DEVICE_NOT_CONNECTED;
        }
        self.slots[user_index].vibration = *vibration;
        ERROR_SUCCESS
    }

    // Win32 XInputGetCapabilities API implementation.
    pub fn get_capabilities(&self, user_index: usize, _flags: u32, caps: &mut XInputCapabilities) -> u32 {
        if user_index >= self.slots.len() || !self.slots[user_index].connected {
            return ERROR_DEVICE_NOT_CONNECTED;
        }
        let slot = &self.slots[user_index];
        caps.Type = XINPUT_DEVTYPE_GAMEPAD;
        caps.SubType = XINPUT_DEVSUBTYPE_GAMEPAD;
        caps.Flags = XINPUT_FLAG_GAMEPAD;
        caps.Gamepad = slot.state.Gamepad;
        caps.Vibration = slot.vibration;
        ERROR_SUCCESS
    }

    // Win32 XInputGetBatteryInformation API implementation.
    pub fn get_battery_information(&self, user_index: usize, _dev_type: u8, battery: &mut XInputBatteryInformation) -> u32 {
        if user_index >= self.slots.len() || !self.slots[user_index].connected {
            return ERROR_DEVICE_NOT_CONNECTED;
        }
        *battery = self.slots[user_index].battery;
        ERROR_SUCCESS
    }
}

/// Verifies motion aiming injection, trigger haptic resistance evaluation, and runtime service routing.
pub fn verify_xinput_service_and_gyro_mapper() -> bool {
    let mut service = XInputRuntimeService::new();
    let report = RawGamepadReport {
        buttons: GamepadButtons::A.0 | GamepadButtons::START.0,
        left_stick_x: 0,
        left_stick_y: 0,
        right_stick_x: 10000,
        right_stick_y: 0,
        left_trigger: 128 << 8,
        right_trigger: 200 << 8,
        imu_sample: None,
        timestamp_us: 1000,
    };

    // Feed report with 0.5 rad/s yaw motion at dt = 0.01s (100Hz)
    let res = service.update_slot(0, &report, 0.5, 0.0, 0.01);
    if res != ERROR_SUCCESS {
        return false;
    }

    let mut state = XInputState::default();
    if service.get_state(0, &mut state) != ERROR_SUCCESS {
        return false;
    }

    if state.dwPacketNumber != 1 {
        return false;
    }
    if !state.Gamepad.has_button(XINPUT_GAMEPAD_A) || !state.Gamepad.has_button(XINPUT_GAMEPAD_START) {
        return false;
    }
    // Verify gyro yaw deflection added to right stick
    if state.Gamepad.sThumbRX <= 10000 {
        return false;
    }

    // Verify trigger haptic profiles
    let trigger_prof = XInputTriggerHapticProfile {
        mode: HapticTriggerMode::RigidStop,
        start_pos: 120,
        end_pos: 255,
        resistance_force: 200,
        vibration_hz: 0,
    };
    let (force_low, _) = trigger_prof.evaluate(100);
    let (force_high, _) = trigger_prof.evaluate(150);
    if force_low != 0 || force_high != 200 {
        return false;
    }

    true
}
