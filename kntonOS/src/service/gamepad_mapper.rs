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

// SteamDeck capacitive trackpad hardware identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackpadId {
    Left,
    Right,
}

// Operational emulation mode for capacitive trackpads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackpadMode {
    MouseCursor,
    ScrollWheel,
    RadialMenu,
    VirtualDpad,
    PrecisionJoystick,
}

// Raw sensor sample reported by SteamDeck capacitive touch sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackpadSample {
    pub x: i16,
    pub y: i16,
    pub pressure: u16,
    pub touching: bool,
    pub clicked: bool,
}

impl TrackpadSample {
    // Construct empty trackpad sample.
    pub const fn new() -> Self {
        Self {
            x: 0,
            y: 0,
            pressure: 0,
            touching: false,
            clicked: false,
        }
    }
}

// Haptic feedback event queued for voice-coil LRA actuator motors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HapticFeedbackEvent {
    pub trackpad: TrackpadId,
    pub intensity_pct: u8,
    pub frequency_hz: u16,
    pub duration_ms: u16,
    pub pulses: u8,
}

impl HapticFeedbackEvent {
    // Construct click haptic pulse event.
    pub const fn click(trackpad: TrackpadId) -> Self {
        Self {
            trackpad,
            intensity_pct: 90,
            frequency_hz: 240,
            duration_ms: 12,
            pulses: 1,
        }
    }

    // Construct detent tick haptic pulse event.
    pub const fn tick(trackpad: TrackpadId) -> Self {
        Self {
            trackpad,
            intensity_pct: 60,
            frequency_hz: 180,
            duration_ms: 6,
            pulses: 1,
        }
    }
}

// Capacitive touch filter and mode processor for an individual trackpad.
pub struct TrackpadFilterEngine {
    pub id: TrackpadId,
    pub mode: TrackpadMode,
    pub filter_alpha: u16,
    pub filtered_x: i32,
    pub filtered_y: i32,
    pub prev_filtered_x: i32,
    pub prev_filtered_y: i32,
    pub delta_x: i16,
    pub delta_y: i16,
    pub was_touching: bool,
    pub was_clicked: bool,
    pub accumulated_detent_dist: i32,
    pub detent_threshold: i32,
}

impl TrackpadFilterEngine {
    // Construct a new trackpad coordinate filter engine.
    pub const fn new(id: TrackpadId, mode: TrackpadMode) -> Self {
        Self {
            id,
            mode,
            filter_alpha: 192,
            filtered_x: 0,
            filtered_y: 0,
            prev_filtered_x: 0,
            prev_filtered_y: 0,
            delta_x: 0,
            delta_y: 0,
            was_touching: false,
            was_clicked: false,
            accumulated_detent_dist: 0,
            detent_threshold: 400,
        }
    }

    // Process a new raw capacitive sample and produce optional haptic feedback.
    pub fn process_sample(
        &mut self,
        sample: TrackpadSample,
    ) -> (i16, i16, Option<HapticFeedbackEvent>) {
        let mut haptic = None;

        if !sample.touching {
            self.was_touching = false;
            self.was_clicked = false;
            self.delta_x = 0;
            self.delta_y = 0;
            self.accumulated_detent_dist = 0;
            return (0, 0, None);
        }

        // Tactile bottom click detection triggers strong haptic confirmation
        if sample.clicked && !self.was_clicked {
            haptic = Some(HapticFeedbackEvent::click(self.id));
        }
        self.was_clicked = sample.clicked;

        if !self.was_touching {
            self.filtered_x = sample.x as i32;
            self.filtered_y = sample.y as i32;
            self.prev_filtered_x = sample.x as i32;
            self.prev_filtered_y = sample.y as i32;
            self.delta_x = 0;
            self.delta_y = 0;
            self.was_touching = true;
            if haptic.is_none() {
                haptic = Some(HapticFeedbackEvent::tick(self.id));
            }
            return (self.filtered_x as i16, self.filtered_y as i16, haptic);
        }

        // Apply EMA low-pass filter to smooth capacitive coordinate jitter
        let alpha = self.filter_alpha as i32;
        self.filtered_x = (alpha * sample.x as i32 + (256 - alpha) * self.filtered_x) / 256;
        self.filtered_y = (alpha * sample.y as i32 + (256 - alpha) * self.filtered_y) / 256;

        self.delta_x = (self.filtered_x - self.prev_filtered_x).clamp(-32768, 32767) as i16;
        self.delta_y = (self.filtered_y - self.prev_filtered_y).clamp(-32768, 32767) as i16;
        self.prev_filtered_x = self.filtered_x;
        self.prev_filtered_y = self.filtered_y;

        // Check detent thresholds for scroll wheel and virtual boundaries
        match self.mode {
            TrackpadMode::ScrollWheel => {
                self.accumulated_detent_dist += self.delta_y as i32;
                if self.accumulated_detent_dist.abs() >= self.detent_threshold {
                    self.accumulated_detent_dist = 0;
                    if haptic.is_none() {
                        haptic = Some(HapticFeedbackEvent::tick(self.id));
                    }
                }
            }
            TrackpadMode::VirtualDpad => {
                let dist_sq = self.filtered_x * self.filtered_x + self.filtered_y * self.filtered_y;
                if dist_sq >= 3000 * 3000 && !self.was_touching && haptic.is_none() {
                    haptic = Some(HapticFeedbackEvent::tick(self.id));
                }
            }
            _ => {}
        }

        (self.filtered_x as i16, self.filtered_y as i16, haptic)
    }
}

pub const MAX_HAPTIC_QUEUE: usize = 16;

// Dual capacitive trackpad manager coordinating left/right pads and haptic queue.
pub struct DualTrackpadManager {
    pub left_pad: TrackpadFilterEngine,
    pub right_pad: TrackpadFilterEngine,
    pub haptic_queue: [Option<HapticFeedbackEvent>; MAX_HAPTIC_QUEUE],
    pub queue_head: usize,
    pub queue_tail: usize,
}

impl DualTrackpadManager {
    // Construct dual trackpad manager with default Left=ScrollWheel, Right=MouseCursor modes.
    pub const fn new() -> Self {
        Self {
            left_pad: TrackpadFilterEngine::new(TrackpadId::Left, TrackpadMode::ScrollWheel),
            right_pad: TrackpadFilterEngine::new(TrackpadId::Right, TrackpadMode::MouseCursor),
            haptic_queue: [None; MAX_HAPTIC_QUEUE],
            queue_head: 0,
            queue_tail: 0,
        }
    }

    // Push haptic feedback event into delivery queue.
    pub fn push_haptic(&mut self, event: HapticFeedbackEvent) {
        let next_head = (self.queue_head + 1) % MAX_HAPTIC_QUEUE;
        if next_head != self.queue_tail {
            self.haptic_queue[self.queue_head] = Some(event);
            self.queue_head = next_head;
        }
    }

    // Pop oldest pending haptic event for actuator dispatch.
    pub fn pop_haptic(&mut self) -> Option<HapticFeedbackEvent> {
        if self.queue_tail == self.queue_head {
            return None;
        }
        let event = self.haptic_queue[self.queue_tail].take();
        self.queue_tail = (self.queue_tail + 1) % MAX_HAPTIC_QUEUE;
        event
    }

    // Process dual trackpad samples and automatically queue generated haptic pulses.
    pub fn process(
        &mut self,
        left_sample: TrackpadSample,
        right_sample: TrackpadSample,
    ) -> ((i16, i16, i16, i16), (i16, i16, i16, i16)) {
        let (lx, ly, l_hap) = self.left_pad.process_sample(left_sample);
        let (rx, ry, r_hap) = self.right_pad.process_sample(right_sample);

        if let Some(h) = l_hap {
            self.push_haptic(h);
        }
        if let Some(h) = r_hap {
            self.push_haptic(h);
        }

        (
            (lx, ly, self.left_pad.delta_x, self.left_pad.delta_y),
            (rx, ry, self.right_pad.delta_x, self.right_pad.delta_y),
        )
    }
}

// Unified SteamDeck Gamepad Mapper coordinating virtual input, calibrated sticks, and haptic trackpads.
pub struct SteamDeckGamepadMapper {
    pub virtual_driver: VirtualGamepadDriver,
    pub left_stick: AnalogStickProcessor,
    pub right_stick: AnalogStickProcessor,
    pub trackpads: DualTrackpadManager,
}

impl SteamDeckGamepadMapper {
    // Construct default unified gamepad mapper instance.
    pub const fn new() -> Self {
        Self {
            virtual_driver: VirtualGamepadDriver::new(EmulationMode::DualMode),
            left_stick: AnalogStickProcessor::new(StickCalibration::default_left()),
            right_stick: AnalogStickProcessor::new(StickCalibration::default_right()),
            trackpads: DualTrackpadManager::new(),
        }
    }

    // Process physical analog stick inputs and update virtual controller axes.
    pub fn update_sticks(&mut self, raw_lx: i16, raw_ly: i16, raw_rx: i16, raw_ry: i16) {
        let (lx, ly) = self.left_stick.process(raw_lx, raw_ly);
        let (rx, ry) = self.right_stick.process(raw_rx, raw_ry);
        self.virtual_driver.set_left_stick(lx, ly);
        self.virtual_driver.set_right_stick(rx, ry);
    }
}

// Comprehensive self-test suite verifying SteamDeck gamepad mapping, stick calibration and trackpads.
pub fn verify_gamepad_mapper_subsystem() -> bool {
    let mut mapper = SteamDeckGamepadMapper::new();

    // 1. Verify Virtual Gamepad Driver (XInput & DirectInput dual mode)
    if !mapper.virtual_driver.is_connected() {
        return false;
    }
    mapper.virtual_driver.press_button(XINPUT_GAMEPAD_A | XINPUT_GAMEPAD_DPAD_UP);
    mapper.virtual_driver.set_left_trigger(200);
    mapper.virtual_driver.press_deck_button(DECK_BUTTON_L4 | DECK_BUTTON_R4);

    let xstate = mapper.virtual_driver.xinput_state;
    if (xstate.buttons & XINPUT_GAMEPAD_A) == 0 || (xstate.buttons & XINPUT_GAMEPAD_DPAD_UP) == 0 {
        return false;
    }
    if xstate.left_trigger != 200 {
        return false;
    }

    let dstate = mapper.virtual_driver.dinput_state;
    if (dstate.buttons & (1 << 0)) == 0 || (dstate.buttons & (1 << 10)) == 0 || (dstate.buttons & (1 << 11)) == 0 {
        return false;
    }
    if dstate.pov_hat != 0 {
        return false;
    }

    // 2. Verify Analog Stick Deadzone & Anti-Drift Calibration
    let (lx0, ly0) = mapper.left_stick.process(1000, 1000);
    if lx0 != 0 || ly0 != 0 {
        return false;
    }

    let (lx_full, _) = mapper.left_stick.process(32000, 0);
    if lx_full < 30000 {
        return false;
    }

    let mut drift_filter = DynamicDriftFilter::new(1500);
    for _ in 0..70 {
        drift_filter.feed_idle_sample(400, -300);
    }
    if drift_filter.bias_x != 400 || drift_filter.bias_y != -300 {
        return false;
    }

    // 3. Verify Dual Capacitive Trackpad Filtering & Haptic Detents
    let mut left_touch = TrackpadSample::new();
    left_touch.x = 1000;
    left_touch.y = 1000;
    left_touch.touching = true;

    let mut right_touch = TrackpadSample::new();
    right_touch.x = 500;
    right_touch.y = 500;
    right_touch.touching = true;
    right_touch.clicked = true;

    mapper.trackpads.process(left_touch, right_touch);

    let mut found_click = false;
    while let Some(hap) = mapper.trackpads.pop_haptic() {
        if hap.trackpad == TrackpadId::Right && hap.intensity_pct == 90 {
            found_click = true;
        }
    }
    if !found_click {
        return false;
    }

    // 4. Verify End-to-End Gamepad Mapper Pipeline
    mapper.update_sticks(0, 0, 15000, -15000);
    let st = mapper.virtual_driver.xinput_state;
    if st.thumb_lx != 0 || st.thumb_ly != 0 {
        return false;
    }
    if st.thumb_rx == 0 || st.thumb_ry == 0 {
        return false;
    }

    true
}



