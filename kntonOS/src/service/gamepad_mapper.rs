// SteamDeck Virtual Gamepad Input Device Driver and Mapper.
// Implements XInput and DirectInput dual-mode emulation and haptic event pipelines.

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

pub const DECK_BUTTON_L4: u32 = 0x0001_0000;
pub const DECK_BUTTON_L5: u32 = 0x0002_0000;
pub const DECK_BUTTON_R4: u32 = 0x0004_0000;
pub const DECK_BUTTON_R5: u32 = 0x0008_0000;
pub const DECK_BUTTON_QAM: u32 = 0x0010_0000;
pub const DECK_BUTTON_STEAM: u32 = 0x0020_0000;

pub const DINPUT_POV_CENTER: u32 = 0xFFFF_FFFF;

// XInput standard gamepad state report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XInputGamepadState {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
    pub packet_number: u32,
}

impl XInputGamepadState {
    // Construct zeroed XInput state snapshot.
    pub const fn new() -> Self {
        Self {
            buttons: 0,
            left_trigger: 0,
            right_trigger: 0,
            thumb_lx: 0,
            thumb_ly: 0,
            thumb_rx: 0,
            thumb_ry: 0,
            packet_number: 0,
        }
    }
}

// DirectInput extended 32-button gamepad report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectInputGamepadState {
    pub buttons: u32,
    pub axis_x: i32,
    pub axis_y: i32,
    pub axis_z: i32,
    pub axis_rx: i32,
    pub axis_ry: i32,
    pub axis_rz: i32,
    pub pov_hat: u32,
}

impl DirectInputGamepadState {
    // Construct zeroed DirectInput state snapshot.
    pub const fn new() -> Self {
        Self {
            buttons: 0,
            axis_x: 0,
            axis_y: 0,
            axis_z: 0,
            axis_rx: 0,
            axis_ry: 0,
            axis_rz: 0,
            pov_hat: DINPUT_POV_CENTER,
        }
    }
}

// Controller protocol emulation profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmulationMode {
    XInputOnly,
    DirectInputOnly,
    DualMode,
}

// Virtual gamepad input device driver handling dual-mode protocol translation.
pub struct VirtualGamepadDriver {
    pub connected: bool,
    pub emulation_mode: EmulationMode,
    pub deck_extra_buttons: u32,
    pub xinput_state: XInputGamepadState,
    pub dinput_state: DirectInputGamepadState,
    pub rumble_left_motor: u16,
    pub rumble_right_motor: u16,
}

impl VirtualGamepadDriver {
    // Construct a new virtual gamepad driver instance.
    pub const fn new(mode: EmulationMode) -> Self {
        Self {
            connected: true,
            emulation_mode: mode,
            deck_extra_buttons: 0,
            xinput_state: XInputGamepadState::new(),
            dinput_state: DirectInputGamepadState::new(),
            rumble_left_motor: 0,
            rumble_right_motor: 0,
        }
    }

    // Set connection status of the virtual controller.
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    // Check if the virtual controller is currently reported as connected.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    // Press one or more standard XInput buttons and increment packet sequence.
    pub fn press_button(&mut self, mask: u16) {
        self.xinput_state.buttons |= mask;
        self.xinput_state.packet_number = self.xinput_state.packet_number.wrapping_add(1);
        self.sync_dinput();
    }

    // Release one or more standard XInput buttons.
    pub fn release_button(&mut self, mask: u16) {
        self.xinput_state.buttons &= !mask;
        self.xinput_state.packet_number = self.xinput_state.packet_number.wrapping_add(1);
        self.sync_dinput();
    }

    // Press SteamDeck rear grip or system button.
    pub fn press_deck_button(&mut self, mask: u32) {
        self.deck_extra_buttons |= mask;
        self.sync_dinput();
    }

    // Release SteamDeck rear grip or system button.
    pub fn release_deck_button(&mut self, mask: u32) {
        self.deck_extra_buttons &= !mask;
        self.sync_dinput();
    }

    // Set left analog trigger pressure [0..255].
    pub fn set_left_trigger(&mut self, value: u8) {
        self.xinput_state.left_trigger = value;
        self.sync_dinput();
    }

    // Set right analog trigger pressure [0..255].
    pub fn set_right_trigger(&mut self, value: u8) {
        self.xinput_state.right_trigger = value;
        self.sync_dinput();
    }

    // Set left analog thumbstick deflection coordinates.
    pub fn set_left_stick(&mut self, x: i16, y: i16) {
        self.xinput_state.thumb_lx = x;
        self.xinput_state.thumb_ly = y;
        self.sync_dinput();
    }

    // Set right analog thumbstick deflection coordinates.
    pub fn set_right_stick(&mut self, x: i16, y: i16) {
        self.xinput_state.thumb_rx = x;
        self.xinput_state.thumb_ry = y;
        self.sync_dinput();
    }

    // Update force-feedback rumble vibration motor speeds.
    pub fn set_vibration(&mut self, left_speed: u16, right_speed: u16) {
        self.rumble_left_motor = left_speed;
        self.rumble_right_motor = right_speed;
    }

    // Synchronize DirectInput state representation from XInput and Deck state.
    pub fn sync_dinput(&mut self) {
        if matches!(self.emulation_mode, EmulationMode::XInputOnly) {
            return;
        }

        // Map primary buttons: A -> Btn0, B -> Btn1, X -> Btn2, Y -> Btn3, LB -> Btn4, RB -> Btn5
        let mut d_buttons = 0u32;
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_A) != 0 { d_buttons |= 1 << 0; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_B) != 0 { d_buttons |= 1 << 1; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_X) != 0 { d_buttons |= 1 << 2; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_Y) != 0 { d_buttons |= 1 << 3; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_LEFT_SHOULDER) != 0 { d_buttons |= 1 << 4; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_RIGHT_SHOULDER) != 0 { d_buttons |= 1 << 5; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_BACK) != 0 { d_buttons |= 1 << 6; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_START) != 0 { d_buttons |= 1 << 7; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_LEFT_THUMB) != 0 { d_buttons |= 1 << 8; }
        if (self.xinput_state.buttons & XINPUT_GAMEPAD_RIGHT_THUMB) != 0 { d_buttons |= 1 << 9; }

        // Map SteamDeck rear grip buttons to buttons 10..13
        if (self.deck_extra_buttons & DECK_BUTTON_L4) != 0 { d_buttons |= 1 << 10; }
        if (self.deck_extra_buttons & DECK_BUTTON_R4) != 0 { d_buttons |= 1 << 11; }
        if (self.deck_extra_buttons & DECK_BUTTON_L5) != 0 { d_buttons |= 1 << 12; }
        if (self.deck_extra_buttons & DECK_BUTTON_R5) != 0 { d_buttons |= 1 << 13; }
        if (self.deck_extra_buttons & DECK_BUTTON_QAM) != 0 { d_buttons |= 1 << 14; }
        if (self.deck_extra_buttons & DECK_BUTTON_STEAM) != 0 { d_buttons |= 1 << 15; }

        self.dinput_state.buttons = d_buttons;
        self.dinput_state.axis_x = self.xinput_state.thumb_lx as i32;
        self.dinput_state.axis_y = -(self.xinput_state.thumb_ly as i32);
        self.dinput_state.axis_rx = self.xinput_state.thumb_rx as i32;
        self.dinput_state.axis_ry = -(self.xinput_state.thumb_ry as i32);
        self.dinput_state.axis_z = (self.xinput_state.left_trigger as i32) * 257 - 32768;
        self.dinput_state.axis_rz = (self.xinput_state.right_trigger as i32) * 257 - 32768;

        // Map D-pad to POV hat angles (in hundredths of degrees)
        let dpad = self.xinput_state.buttons & 0x000F;
        self.dinput_state.pov_hat = match dpad {
            XINPUT_GAMEPAD_DPAD_UP => 0,
            x if x == (XINPUT_GAMEPAD_DPAD_UP | XINPUT_GAMEPAD_DPAD_RIGHT) => 4500,
            XINPUT_GAMEPAD_DPAD_RIGHT => 9000,
            x if x == (XINPUT_GAMEPAD_DPAD_DOWN | XINPUT_GAMEPAD_DPAD_RIGHT) => 13500,
            XINPUT_GAMEPAD_DPAD_DOWN => 18000,
            x if x == (XINPUT_GAMEPAD_DPAD_DOWN | XINPUT_GAMEPAD_DPAD_LEFT) => 22500,
            XINPUT_GAMEPAD_DPAD_LEFT => 27000,
            x if x == (XINPUT_GAMEPAD_DPAD_UP | XINPUT_GAMEPAD_DPAD_LEFT) => 31500,
            _ => DINPUT_POV_CENTER,
        };
    }
}

// Fast integer square root algorithm for `#![no_std]` stick radius calculations.
pub const fn isqrt(n: u64) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut x0 = (n >> 1) as u32;
    if x0 == 0 {
        x0 = 1;
    }
    let mut x1 = ((x0 as u64 + n / x0 as u64) >> 1) as u32;
    while x1 < x0 {
        x0 = x1;
        x1 = ((x0 as u64 + n / x0 as u64) >> 1) as u32;
    }
    x0
}

// Geometric shape profile for analog deadzone boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadzoneShape {
    Radial,
    Axial,
    Cross,
}

// Non-linear transfer function curve for stick deflection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseCurve {
    Linear,
    Exponential,
    Aggressive,
    WideAim,
}

// Adaptive rolling filter eliminating analog thumbstick zero-point resting drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicDriftFilter {
    pub accum_x: i32,
    pub accum_y: i32,
    pub sample_count: u16,
    pub bias_x: i16,
    pub bias_y: i16,
    pub max_drift_threshold: u16,
}

impl DynamicDriftFilter {
    // Construct drift filter with specified drift clamp window.
    pub const fn new(max_drift: u16) -> Self {
        Self {
            accum_x: 0,
            accum_y: 0,
            sample_count: 0,
            bias_x: 0,
            bias_y: 0,
            max_drift_threshold: max_drift,
        }
    }

    // Feed stick sample when stick is near center resting position.
    pub fn feed_idle_sample(&mut self, raw_x: i16, raw_y: i16) {
        let abs_x = raw_x.unsigned_abs();
        let abs_y = raw_y.unsigned_abs();
        if abs_x <= self.max_drift_threshold && abs_y <= self.max_drift_threshold {
            self.accum_x += raw_x as i32;
            self.accum_y += raw_y as i32;
            self.sample_count += 1;
            if self.sample_count >= 64 {
                self.bias_x = (self.accum_x / 64) as i16;
                self.bias_y = (self.accum_y / 64) as i16;
                self.accum_x = 0;
                self.accum_y = 0;
                self.sample_count = 0;
            }
        }
    }

    // Reset accumulated drift calibration state.
    pub fn reset(&mut self) {
        self.accum_x = 0;
        self.accum_y = 0;
        self.sample_count = 0;
        self.bias_x = 0;
        self.bias_y = 0;
    }
}

// Calibration settings configuring deadzone, response curve, and anti-drift parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StickCalibration {
    pub inner_deadzone: u16,
    pub outer_deadzone: u16,
    pub shape: DeadzoneShape,
    pub curve: ResponseCurve,
    pub drift_filter: DynamicDriftFilter,
}

impl StickCalibration {
    // Construct custom stick calibration profile.
    pub const fn new(
        inner_deadzone: u16,
        outer_deadzone: u16,
        shape: DeadzoneShape,
        curve: ResponseCurve,
        max_drift: u16,
    ) -> Self {
        Self {
            inner_deadzone,
            outer_deadzone,
            shape,
            curve,
            drift_filter: DynamicDriftFilter::new(max_drift),
        }
    }

    // Default configuration optimized for SteamDeck left movement stick.
    pub const fn default_left() -> Self {
        Self::new(2800, 31500, DeadzoneShape::Radial, ResponseCurve::Linear, 1500)
    }

    // Default configuration optimized for SteamDeck right aim stick with exponential curve.
    pub const fn default_right() -> Self {
        Self::new(2400, 31800, DeadzoneShape::Radial, ResponseCurve::Exponential, 1200)
    }
}

// Analog stick processor applying anti-drift, deadzone normalization, and curve shaping.
pub struct AnalogStickProcessor {
    pub calibration: StickCalibration,
}

impl AnalogStickProcessor {
    // Initialize processor with active calibration parameters.
    pub const fn new(calibration: StickCalibration) -> Self {
        Self { calibration }
    }

    // Apply response curve transfer function to normalized value [0..32767].
    fn apply_curve(val: u32, curve: ResponseCurve) -> u32 {
        match curve {
            ResponseCurve::Linear => val,
            ResponseCurve::Exponential => ((val as u64 * val as u64) / 32767) as u32,
            ResponseCurve::Aggressive => isqrt(val as u64 * 32767),
            ResponseCurve::WideAim => {
                let v = val as u64;
                ((v * v + v * 16384) / 49151) as u32
            }
        }
    }

    // Transform raw analog stick input into calibrated anti-drift output coordinates.
    pub fn process(&mut self, raw_x: i16, raw_y: i16) -> (i16, i16) {
        self.calibration.drift_filter.feed_idle_sample(raw_x, raw_y);

        // Correct hardware resting bias
        let xc = (raw_x as i32 - self.calibration.drift_filter.bias_x as i32).clamp(-32768, 32767);
        let yc = (raw_y as i32 - self.calibration.drift_filter.bias_y as i32).clamp(-32768, 32767);

        let inner = self.calibration.inner_deadzone as u32;
        let outer = self.calibration.outer_deadzone as u32;

        match self.calibration.shape {
            DeadzoneShape::Radial => {
                let radius_sq = (xc as i64 * xc as i64 + yc as i64 * yc as i64) as u64;
                let radius = isqrt(radius_sq);

                if radius <= inner {
                    return (0, 0);
                }

                let span = if outer > inner { outer - inner } else { 1 };
                let clamped_r = if radius >= outer { outer } else { radius };
                let norm_r = ((clamped_r - inner) as u64 * 32767 / span as u64) as u32;
                let curved_r = Self::apply_curve(norm_r, self.calibration.curve);

                let out_x = ((xc as i64 * curved_r as i64) / radius as i64).clamp(-32768, 32767) as i16;
                let out_y = ((yc as i64 * curved_r as i64) / radius as i64).clamp(-32768, 32767) as i16;
                (out_x, out_y)
            }
            DeadzoneShape::Axial => {
                let scale_axis = |val: i32| -> i16 {
                    let sign = if val < 0 { -1i64 } else { 1i64 };
                    let mag = val.unsigned_abs() as u32;
                    if mag <= inner {
                        return 0;
                    }
                    let span = if outer > inner { outer - inner } else { 1 };
                    let clamped = if mag >= outer { outer } else { mag };
                    let norm = ((clamped - inner) as u64 * 32767 / span as u64) as u32;
                    let curved = Self::apply_curve(norm, self.calibration.curve);
                    (sign * curved as i64).clamp(-32768, 32767) as i16
                };
                (scale_axis(xc), scale_axis(yc))
            }
            DeadzoneShape::Cross => {
                let mag_x = xc.unsigned_abs() as u32;
                let mag_y = yc.unsigned_abs() as u32;
                let ax = if mag_x <= inner { 0 } else { xc };
                let ay = if mag_y <= inner { 0 } else { yc };
                let r = isqrt((ax as i64 * ax as i64 + ay as i64 * ay as i64) as u64);
                if r == 0 {
                    return (0, 0);
                }
                let span = if outer > inner { outer - inner } else { 1 };
                let clamped_r = if r >= outer { outer } else { r };
                let norm_r = ((clamped_r - inner) as u64 * 32767 / span as u64) as u32;
                let curved_r = Self::apply_curve(norm_r, self.calibration.curve);
                let out_x = ((ax as i64 * curved_r as i64) / r as i64).clamp(-32768, 32767) as i16;
                let out_y = ((ay as i64 * curved_r as i64) / r as i64).clamp(-32768, 32767) as i16;
                (out_x, out_y)
            }
        }
    }
}

