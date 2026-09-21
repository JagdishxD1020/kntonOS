//! Handheld Dual Trackpad Haptic Surface & Mouse Emulation Engine.

#![allow(dead_code)]

use crate::graphics::sqrt_f32;
#[allow(unused_imports)]
use super::imu::atan2_f32;
#[allow(unused_imports)]
use super::haptic_dsp::TactilePattern;

/// Identifies the physical trackpad on the handheld device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackpadId {
    LeftTrackpad,
    RightTrackpad,
}

/// Trackpad operational mode defining gesture translation and input semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackpadMode {
    /// Virtual trackball with momentum physics and friction decay for mouse cursor aiming.
    TrackballMouse,
    /// Circular thumb motion for scrolling or weapon cycling.
    ScrollWheel,
    /// 16-slice touch menu for quick inventory / macro selection.
    RadialMenu,
    /// 4-way or 8-way directional pad emulation.
    DirectionalPad,
}

/// Normalized touch contact on a circular trackpad surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackpadContact {
    /// Normalized horizontal position [-1.0 .. +1.0], 0.0 at center.
    pub x: f32,
    /// Normalized vertical position [-1.0 .. +1.0], 0.0 at center.
    pub y: f32,
    /// Contact pressure [0.0 .. 1.0].
    pub pressure: f32,
    /// Contact active (finger touching surface).
    pub is_touching: bool,
    /// Hardware timestamp in microseconds.
    pub timestamp_us: u64,
}

impl TrackpadContact {
    pub const fn empty() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            is_touching: false,
            timestamp_us: 0,
        }
    }
}

/// Velocity vector of thumb movement on trackpad surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThumbVelocity {
    pub vx: f32,
    pub vy: f32,
    pub speed: f32,
}

impl ThumbVelocity {
    pub const fn zero() -> Self {
        Self { vx: 0.0, vy: 0.0, speed: 0.0 }
    }
}

/// State tracker for an individual trackpad surface.
#[derive(Debug, Clone)]
pub struct TrackpadSurface {
    pub id: TrackpadId,
    pub mode: TrackpadMode,
    pub current_contact: TrackpadContact,
    pub prev_contact: TrackpadContact,
    pub velocity: ThumbVelocity,
    pub sample_count: u64,
}

impl TrackpadSurface {
    pub const fn new(id: TrackpadId, mode: TrackpadMode) -> Self {
        Self {
            id,
            mode,
            current_contact: TrackpadContact::empty(),
            prev_contact: TrackpadContact::empty(),
            velocity: ThumbVelocity::zero(),
            sample_count: 0,
        }
    }

    /// Updates touch contact point and calculates velocity vector.
    pub fn update_touch(&mut self, x: f32, y: f32, pressure: f32, touching: bool, timestamp_us: u64) {
        self.prev_contact = self.current_contact;
        self.current_contact = TrackpadContact {
            x: x.clamp(-1.0, 1.0),
            y: y.clamp(-1.0, 1.0),
            pressure: pressure.clamp(0.0, 1.0),
            is_touching: touching,
            timestamp_us,
        };

        if touching && self.prev_contact.is_touching && timestamp_us > self.prev_contact.timestamp_us {
            let dt_s = ((timestamp_us - self.prev_contact.timestamp_us) as f32) / 1_000_000.0;
            if dt_s > 0.0001 && dt_s < 0.1 {
                let dx = self.current_contact.x - self.prev_contact.x;
                let dy = self.current_contact.y - self.prev_contact.y;
                let vx = dx / dt_s;
                let vy = dy / dt_s;
                let speed = sqrt_f32(vx * vx + vy * vy);
                self.velocity = ThumbVelocity { vx, vy, speed };
            }
        } else if !touching {
            // Keep velocity on release for momentum calculation
        } else {
            self.velocity = ThumbVelocity::zero();
        }

        self.sample_count += 1;
    }
}

/// Unit test verifying dual-trackpad operating modes and touch tracking.
pub fn verify_trackpad_modes_and_touch_tracking() -> bool {
    let left_pad = TrackpadSurface::new(TrackpadId::LeftTrackpad, TrackpadMode::DirectionalPad);
    let mut right_pad = TrackpadSurface::new(TrackpadId::RightTrackpad, TrackpadMode::TrackballMouse);

    if left_pad.mode != TrackpadMode::DirectionalPad || right_pad.mode != TrackpadMode::TrackballMouse {
        return false;
    }

    // Touch down at (0.0, 0.0)
    right_pad.update_touch(0.0, 0.0, 0.5, true, 1_000_000);
    if !right_pad.current_contact.is_touching || right_pad.current_contact.x != 0.0 {
        return false;
    }

    // Swipe right to (0.1, 0.0) in 10ms (0.01s) -> vx = 10.0 units/s
    right_pad.update_touch(0.1, 0.0, 0.6, true, 1_010_000);
    if (right_pad.velocity.vx - 10.0).abs() > 0.1 || right_pad.velocity.vy.abs() > 0.01 {
        return false;
    }
    if (right_pad.velocity.speed - 10.0).abs() > 0.1 {
        return false;
    }

    // Lift finger
    right_pad.update_touch(0.1, 0.0, 0.0, false, 1_020_000);
    if right_pad.current_contact.is_touching {
        return false;
    }

    true
}



// Virtual Trackball Momentum Physics & Friction Decay Engine.

/// Virtual trackball physics configuration and active momentum state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackpadVirtualTrackball {
    /// Friction decay coefficient (decay per second, nominally 3.5 .. 5.0).
    pub friction_coeff: f32,
    /// Minimum speed threshold below which momentum halts to zero.
    pub stop_threshold: f32,
    /// Active velocity along X in normalized units per second.
    pub vx: f32,
    /// Active velocity along Y in normalized units per second.
    pub vy: f32,
    /// Momentum active (ball currently spinning post-release).
    pub is_spinning: bool,
}

impl TrackpadVirtualTrackball {
    pub const fn new(friction_coeff: f32, stop_threshold: f32) -> Self {
        Self {
            friction_coeff,
            stop_threshold,
            vx: 0.0,
            vy: 0.0,
            is_spinning: false,
        }
    }

    /// Default trackball setup tuned for gaming precision and smooth flicking.
    pub const fn default_gaming() -> Self {
        Self::new(4.0, 0.05)
    }

    /// Imparts velocity upon thumb release (fling gesture).
    pub fn launch(&mut self, vx: f32, vy: f32) {
        let speed = sqrt_f32(vx * vx + vy * vy);
        if speed >= self.stop_threshold {
            self.vx = vx;
            self.vy = vy;
            self.is_spinning = true;
        } else {
            self.stop();
        }
    }

    /// Halts momentum immediately when thumb touches down on trackpad.
    pub fn stop(&mut self) {
        self.vx = 0.0;
        self.vy = 0.0;
        self.is_spinning = false;
    }

    /// Advances momentum simulation by time step dt_s and returns displacement (dx, dy).
    pub fn step_simulation(&mut self, dt_s: f32) -> (f32, f32) {
        if !self.is_spinning || dt_s <= 0.0 {
            return (0.0, 0.0);
        }

        // Displacement during this time step
        let dx = self.vx * dt_s;
        let dy = self.vy * dt_s;

        // Exponential friction decay
        let decay_factor = (1.0 - self.friction_coeff * dt_s).max(0.0);
        self.vx *= decay_factor;
        self.vy *= decay_factor;

        let current_speed = sqrt_f32(self.vx * self.vx + self.vy * self.vy);
        if current_speed < self.stop_threshold {
            self.stop();
        }

        (dx, dy)
    }
}

/// Unit test verifying virtual trackball momentum physics and friction decay.
pub fn verify_trackball_momentum_decay() -> bool {
    let mut trackball = TrackpadVirtualTrackball::new(4.0, 0.05);

    // Launch with fling velocity (10.0, 0.0)
    trackball.launch(10.0, 0.0);
    if !trackball.is_spinning || trackball.vx != 10.0 {
        return false;
    }

    // Step 10ms (0.01s): displacement dx ~ 0.1, decay factor 1 - 0.04 = 0.96 -> vx ~ 9.6
    let (dx1, dy1) = trackball.step_simulation(0.01);
    if (dx1 - 0.1).abs() > 0.01 || dy1.abs() > 0.001 {
        return false;
    }
    if (trackball.vx - 9.6).abs() > 0.05 {
        return false;
    }

    // Simulate decay over 1.5 seconds (150 steps of 10ms)
    for _ in 0..150 {
        trackball.step_simulation(0.01);
    }

    // After 1.5 seconds with friction=4.0, momentum should have cleanly stopped
    if trackball.is_spinning || trackball.vx != 0.0 || trackball.vy != 0.0 {
        return false;
    }

    // Touchdown brake test
    trackball.launch(8.0, 8.0);
    if !trackball.is_spinning {
        return false;
    }
    trackball.stop();
    if trackball.is_spinning || trackball.vx != 0.0 || trackball.vy != 0.0 {
        return false;
    }

    true
}



// Circular Scroll Wheel & 16-Slice Radial Menu Engine.

/// Circular scroll wheel engine tracking rotational angle for scrolling and weapon cycling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CircularScrollWheel {
    /// Angular step per discrete scroll notch in radians (e.g. 22.5 deg = 0.3927 rad).
    pub step_angle_rad: f32,
    /// Minimum radius threshold from center to engage scrolling (deadzone).
    pub min_radius: f32,
    /// Previous touch angle in radians (-PI .. +PI).
    pub prev_angle_rad: f32,
    /// Accumulated angular delta since last discrete scroll tick.
    pub accumulated_angle_rad: f32,
    /// Total discrete scroll clicks emitted.
    pub total_scroll_clicks: i32,
    /// Whether wheel tracking is currently active.
    pub is_active: bool,
}

impl CircularScrollWheel {
    pub const fn new(step_angle_rad: f32, min_radius: f32) -> Self {
        Self {
            step_angle_rad,
            min_radius,
            prev_angle_rad: 0.0,
            accumulated_angle_rad: 0.0,
            total_scroll_clicks: 0,
            is_active: false,
        }
    }

    /// Default 16-notch scroll wheel (22.5 degrees per tick, 0.20 deadzone).
    pub const fn default_16_notch() -> Self {
        Self::new(0.3926991, 0.20)
    }

    /// Updates touch point and computes discrete scroll tick output (+1 clockwise, -1 counter-clockwise).
    pub fn update(&mut self, x: f32, y: f32, touching: bool) -> i32 {
        if !touching {
            self.is_active = false;
            self.accumulated_angle_rad = 0.0;
            return 0;
        }

        let r = sqrt_f32(x * x + y * y);
        if r < self.min_radius {
            return 0;
        }

        let current_angle = atan2_f32(y, x);

        if !self.is_active {
            self.prev_angle_rad = current_angle;
            self.is_active = true;
            return 0;
        }

        // Compute shortest angular delta across [-PI, +PI] boundary
        let mut d_theta = current_angle - self.prev_angle_rad;
        let pi = 3.14159265;
        while d_theta > pi {
            d_theta -= 2.0 * pi;
        }
        while d_theta < -pi {
            d_theta += 2.0 * pi;
        }

        self.prev_angle_rad = current_angle;
        self.accumulated_angle_rad += d_theta;

        let mut ticks = 0;
        while self.accumulated_angle_rad >= self.step_angle_rad {
            self.accumulated_angle_rad -= self.step_angle_rad;
            ticks += 1;
        }
        while self.accumulated_angle_rad <= -self.step_angle_rad {
            self.accumulated_angle_rad += self.step_angle_rad;
            ticks -= 1;
        }

        self.total_scroll_clicks += ticks;
        ticks
    }
}

/// 16-Slice Radial Menu touch classifier for quick weapon/inventory selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadialMenuClassifier {
    /// Inner deadzone radius below which no sector is highlighted.
    pub deadzone_radius: f32,
    /// Currently highlighted sector index (0..15), or None if in deadzone.
    pub active_sector: Option<u8>,
}

impl RadialMenuClassifier {
    pub const fn new(deadzone_radius: f32) -> Self {
        Self {
            deadzone_radius,
            active_sector: None,
        }
    }

    /// Evaluates touch point and returns active sector (0..15).
    pub fn classify(&mut self, x: f32, y: f32, touching: bool) -> Option<u8> {
        if !touching {
            self.active_sector = None;
            return None;
        }

        let r = sqrt_f32(x * x + y * y);
        if r < self.deadzone_radius {
            self.active_sector = None;
            return None;
        }

        // Angle in radians [-PI .. +PI]
        let angle = atan2_f32(y, x);
        let pi = 3.14159265;
        // Shift to [0 .. 2*PI]
        let normalized_angle = if angle < 0.0 { angle + 2.0 * pi } else { angle };

        // 16 sectors: sector size = 2*PI / 16 = PI / 8
        let sector_size = (2.0 * pi) / 16.0;
        // Add half sector offset so sector 0 is centered on positive X axis
        let offset_angle = (normalized_angle + sector_size * 0.5) % (2.0 * pi);
        let sector = (offset_angle / sector_size) as u8;
        let clamped_sector = sector.min(15);

        self.active_sector = Some(clamped_sector);
        self.active_sector
    }
}

/// Unit test verifying circular scroll wheel and 16-slice radial menu sector mapping.
pub fn verify_scroll_wheel_and_radial_menu() -> bool {
    // 1. Scroll Wheel verification
    let mut wheel = CircularScrollWheel::default_16_notch();

    // Touch down at (1.0, 0.0) -> angle 0
    let t0 = wheel.update(1.0, 0.0, true);
    if t0 != 0 || !wheel.is_active {
        return false;
    }

    // Rotate clockwise: (0.92388, 0.38268) -> angle 22.5 deg (0.3927 rad)
    let t1 = wheel.update(0.92388, 0.38268, true);
    if t1 != 1 || wheel.total_scroll_clicks != 1 {
        return false;
    }

    // Deadzone check: inside 0.20 radius -> outputs 0 ticks
    let t_dz = wheel.update(0.05, 0.05, true);
    if t_dz != 0 {
        return false;
    }

    // 2. Radial Menu verification
    let mut radial = RadialMenuClassifier::new(0.20);

    // Deadzone check: (0.1, 0.0) with r=0.1 < 0.2 -> None
    if radial.classify(0.1, 0.0, true).is_some() {
        return false;
    }

    // East / 0 deg: (0.8, 0.0) -> Sector 0
    let s_east = radial.classify(0.8, 0.0, true);
    if s_east != Some(0) {
        return false;
    }

    // North / 90 deg: (0.0, 0.8) -> Sector 4
    let s_north = radial.classify(0.0, 0.8, true);
    if s_north != Some(4) {
        return false;
    }

    // West / 180 deg: (-0.8, 0.0) -> Sector 8
    let s_west = radial.classify(-0.8, 0.0, true);
    if s_west != Some(8) {
        return false;
    }

    // South / 270 deg: (0.0, -0.8) -> Sector 12
    let s_south = radial.classify(0.0, -0.8, true);
    if s_south != Some(12) {
        return false;
    }

    true
}



// Rotational Haptic Detent Generator & Win32 Mouse Displacement Synthesizer.

/// Rotational and displacement haptic detent generator using tactile patterns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackpadHapticGenerator {
    /// Linear distance traveled before triggering a detent tick.
    pub step_distance: f32,
    /// Distance accumulated since last haptic tick.
    pub accumulated_distance: f32,
    /// Previous sector index for boundary crossing detents.
    pub prev_sector: Option<u8>,
    /// Total haptic pulses emitted.
    pub total_pulses_emitted: u32,
}

impl TrackpadHapticGenerator {
    pub const fn new(step_distance: f32) -> Self {
        Self {
            step_distance,
            accumulated_distance: 0.0,
            prev_sector: None,
            total_pulses_emitted: 0,
        }
    }

    /// Evaluates motion displacement and triggers haptic tick if threshold exceeded.
    pub fn process_displacement(&mut self, dx: f32, dy: f32) -> Option<TactilePattern> {
        let dist = sqrt_f32(dx * dx + dy * dy);
        self.accumulated_distance += dist;

        if self.accumulated_distance >= self.step_distance {
            self.accumulated_distance -= self.step_distance;
            self.total_pulses_emitted += 1;
            Some(TactilePattern::Tick)
        } else {
            None
        }
    }

    /// Evaluates radial sector transitions and triggers a crisp click on crossing.
    pub fn process_sector_transition(&mut self, current_sector: Option<u8>) -> Option<TactilePattern> {
        let pulse = match (self.prev_sector, current_sector) {
            (Some(prev), Some(curr)) if prev != curr => {
                self.total_pulses_emitted += 1;
                Some(TactilePattern::CrispClick)
            }
            (None, Some(_)) => {
                self.total_pulses_emitted += 1;
                Some(TactilePattern::CrispClick)
            }
            _ => None,
        };

        self.prev_sector = current_sector;
        pulse
    }
}

/// Win32 sub-pixel cursor displacement synthesizer with fractional residual accumulator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Win32MouseSynthesizer {
    /// DPI / Sensitivity multiplier (counts per normalized unit, nominally 800.0 .. 1600.0).
    pub sensitivity_dpi: f32,
    /// Fractional sub-pixel X remainder from previous frames.
    pub residual_x: f32,
    /// Fractional sub-pixel Y remainder from previous frames.
    pub residual_y: f32,
}

impl Win32MouseSynthesizer {
    pub const fn new(sensitivity_dpi: f32) -> Self {
        Self {
            sensitivity_dpi,
            residual_x: 0.0,
            residual_y: 0.0,
        }
    }

    /// Default 1200 DPI gaming sensitivity.
    pub const fn default_gaming() -> Self {
        Self::new(1200.0)
    }

    /// Converts normalized float displacement (dx, dy) into integer Win32 mouse units, preserving fractions.
    pub fn synthesize_motion(&mut self, dx: f32, dy: f32) -> (i32, i32) {
        let total_x = dx * self.sensitivity_dpi + self.residual_x;
        let total_y = dy * self.sensitivity_dpi + self.residual_y;

        let int_x = total_x as i32;
        let int_y = total_y as i32;

        self.residual_x = total_x - (int_x as f32);
        self.residual_y = total_y - (int_y as f32);

        (int_x, int_y)
    }
}

/// Unit test verifying rotational haptic detents and Win32 mouse displacement synthesizer.
pub fn verify_haptic_detents_and_mouse_synthesizer() -> bool {
    // 1. Haptic detent displacement test
    let mut haptics = TrackpadHapticGenerator::new(0.05);

    // Move 0.02 units -> no pulse
    if haptics.process_displacement(0.02, 0.0).is_some() || haptics.total_pulses_emitted != 0 {
        return false;
    }

    // Move another 0.04 units (total 0.06 >= 0.05) -> triggers Tick
    let p1 = haptics.process_displacement(0.04, 0.0);
    if p1 != Some(TactilePattern::Tick) || haptics.total_pulses_emitted != 1 {
        return false;
    }

    // Radial sector transition -> CrispClick
    let p_sec = haptics.process_sector_transition(Some(2));
    if p_sec != Some(TactilePattern::CrispClick) || haptics.total_pulses_emitted != 2 {
        return false;
    }
    // Staying in same sector -> no click
    if haptics.process_sector_transition(Some(2)).is_some() {
        return false;
    }
    // Changing sector 2 -> 3 -> triggers CrispClick
    if haptics.process_sector_transition(Some(3)) != Some(TactilePattern::CrispClick) {
        return false;
    }

    // 2. Win32 Mouse Synthesizer test
    let mut synth = Win32MouseSynthesizer::new(1000.0);

    // Small sub-pixel micro-aiming: dx = 0.0004 -> total_x = 0.4 -> int_x = 0, residual = 0.4
    let (mx0, my0) = synth.synthesize_motion(0.0004, 0.0);
    if mx0 != 0 || my0 != 0 || (synth.residual_x - 0.4).abs() > 0.001 {
        return false;
    }

    // Second micro-aiming frame: dx = 0.0008 -> total_x = 0.8 + 0.4 = 1.2 -> int_x = 1, residual = 0.2
    let (mx1, my1) = synth.synthesize_motion(0.0008, 0.0);
    if mx1 != 1 || my1 != 0 || (synth.residual_x - 0.2).abs() > 0.001 {
        return false;
    }

    // Large flick: dx = 0.05 -> total_x = 50.0 + 0.2 = 50.2 -> int_x = 50, residual = 0.2
    let (mx2, my2) = synth.synthesize_motion(0.05, 0.0);
    if mx2 != 50 || my2 != 0 {
        return false;
    }

    true
}



/// Comprehensive verification self-test suite for Handheld Dual Trackpad Haptic Surface & Mouse Emulation Engine.
pub fn test_trackpad_surface_subsystem() -> bool {
    crate::serial_write_str("[KNT-INPUT] Testing Handheld Dual Trackpad Haptic Surface & Mouse Emulation Engine...\r\n");

    // 1. Verify Dual-Trackpad Touch Tracking & Operating Modes
    if !verify_trackpad_modes_and_touch_tracking() {
        crate::serial_write_str("[KNT-PANIC] Trackpad modes and touch tracking verification failed!\r\n");
        return false;
    }

    // 2. Verify Virtual Trackball Momentum Physics & Friction Decay
    if !verify_trackball_momentum_decay() {
        crate::serial_write_str("[KNT-PANIC] Trackball momentum physics and friction decay verification failed!\r\n");
        return false;
    }

    // 3. Verify Circular Scroll Wheel & 16-Slice Radial Menu Sector Classifier
    if !verify_scroll_wheel_and_radial_menu() {
        crate::serial_write_str("[KNT-PANIC] Scroll wheel and radial menu sector mapping verification failed!\r\n");
        return false;
    }

    // 4. Verify Rotational Haptic Detent Generator & Win32 Mouse Displacement Synthesizer
    if !verify_haptic_detents_and_mouse_synthesizer() {
        crate::serial_write_str("[KNT-PANIC] Haptic detents and mouse displacement synthesizer verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-TRACKPAD-PASS] Handheld dual trackpad haptic surface and mouse emulation engine verified\r\n");
    true
}
