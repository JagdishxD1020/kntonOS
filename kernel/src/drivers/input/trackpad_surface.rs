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
