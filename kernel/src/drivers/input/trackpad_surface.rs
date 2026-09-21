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
