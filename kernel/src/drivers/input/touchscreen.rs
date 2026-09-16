//! Handheld Capacitive Touchscreen Multi-Touch & Gesture Engine.

#![allow(dead_code)]

/// Maximum number of simultaneous touch points supported.
pub const MAX_TS_CONTACTS: usize = 10;

/// Individual touch contact point descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TsContactPoint {
    pub contact_id: u8,
    pub x: u16,
    pub y: u16,
    pub pressure: u16,
    pub major_axis: u16,
    pub minor_axis: u16,
    pub active: bool,
}

impl TsContactPoint {
    /// Creates an empty/inactive touch contact.
    pub const fn empty() -> Self {
        Self {
            contact_id: 0,
            x: 0,
            y: 0,
            pressure: 0,
            major_axis: 0,
            minor_axis: 0,
            active: false,
        }
    }
}

/// Multi-touch contact tracking manager supporting up to 10 points.
#[derive(Debug, Clone, Copy)]
pub struct MultiTsContactPointTracker {
    pub contacts: [TsContactPoint; MAX_TS_CONTACTS],
    pub active_contact_count: usize,
}

impl MultiTsContactPointTracker {
    /// Creates a new empty contact tracker.
    pub const fn new() -> Self {
        Self {
            contacts: [TsContactPoint::empty(); MAX_TS_CONTACTS],
            active_contact_count: 0,
        }
    }

    /// Updates or inserts a contact point by contact ID.
    pub fn update_contact(&mut self, id: u8, x: u16, y: u16, pressure: u16, major: u16, minor: u16) -> bool {
        // Look for existing contact with same ID
        for c in self.contacts.iter_mut() {
            if c.active && c.contact_id == id {
                c.x = x;
                c.y = y;
                c.pressure = pressure;
                c.major_axis = major;
                c.minor_axis = minor;
                return true;
            }
        }

        // Insert into first free slot
        for c in self.contacts.iter_mut() {
            if !c.active {
                *c = TsContactPoint {
                    contact_id: id,
                    x,
                    y,
                    pressure,
                    major_axis: major,
                    minor_axis: minor,
                    active: true,
                };
                self.active_contact_count += 1;
                return true;
            }
        }

        false
    }

    /// Releases a contact by ID.
    pub fn release_contact(&mut self, id: u8) -> bool {
        for c in self.contacts.iter_mut() {
            if c.active && c.contact_id == id {
                c.active = false;
                if self.active_contact_count > 0 {
                    self.active_contact_count -= 1;
                }
                return true;
            }
        }
        false
    }
}

/// Unit verification for multi-touch contact tracking.
pub fn verify_multi_touch_tracking() -> bool {
    let mut tracker = MultiTsContactPointTracker::new();

    // Register 3 simultaneous contacts
    if !tracker.update_contact(1, 100, 200, 500, 15, 10) {
        return false;
    }
    if !tracker.update_contact(2, 300, 400, 600, 18, 12) {
        return false;
    }
    if !tracker.update_contact(3, 500, 600, 700, 20, 14) {
        return false;
    }

    if tracker.active_contact_count != 3 {
        return false;
    }

    // Release contact 2
    if !tracker.release_contact(2) || tracker.active_contact_count != 2 {
        return false;
    }

    true
}

/// Screen bezel deadzone margin in pixels.
pub const BEZEL_DEADZONE_PX: u16 = 40;
/// Large contact area threshold indicative of palm resting (major axis in px).
pub const PALM_MAJOR_AXIS_THRESHOLD_PX: u16 = 60;

/// Handheld palm rejection filter preventing accidental touches while holding grips.
#[derive(Debug, Clone, Copy)]
pub struct HandheldPalmRejectionFilter {
    pub display_width: u16,
    pub display_height: u16,
    pub bezel_margin_px: u16,
    pub rejected_palm_events: u32,
}

impl HandheldPalmRejectionFilter {
    /// Creates a new palm rejection filter for a 1920x1080 handheld screen.
    pub const fn new(width: u16, height: u16) -> Self {
        Self {
            display_width: width,
            display_height: height,
            bezel_margin_px: BEZEL_DEADZONE_PX,
            rejected_palm_events: 0,
        }
    }

    /// Evaluates whether a contact point is an accidental palm or thumb touch.
    pub fn is_palm_contact(&mut self, contact: &TsContactPoint) -> bool {
        if !contact.active {
            return false;
        }

        // Rule 1: Large contact surface area (major axis >= 60px) characteristic of palm resting
        if contact.major_axis >= PALM_MAJOR_AXIS_THRESHOLD_PX {
            self.rejected_palm_events += 1;
            return true;
        }

        // Rule 2: Contact within left/right bezel grip zones while also having significant contact area
        let in_left_bezel = contact.x < self.bezel_margin_px;
        let in_right_bezel = contact.x > self.display_width.saturating_sub(self.bezel_margin_px);

        if (in_left_bezel || in_right_bezel) && contact.major_axis > 25 {
            self.rejected_palm_events += 1;
            return true;
        }

        false
    }
}

/// Unit verification for handheld palm rejection filter.
pub fn verify_palm_rejection_filter() -> bool {
    let mut filter = HandheldPalmRejectionFilter::new(1920, 1080);

    // Normal finger tap: x=500, y=500, major=15px -> valid touch
    let finger = TsContactPoint {
        contact_id: 1,
        x: 500,
        y: 500,
        pressure: 400,
        major_axis: 15,
        minor_axis: 12,
        active: true,
    };
    if filter.is_palm_contact(&finger) {
        return false;
    }

    // Large palm resting: major=80px -> rejected
    let palm = TsContactPoint {
        contact_id: 2,
        x: 800,
        y: 600,
        pressure: 800,
        major_axis: 80,
        minor_axis: 50,
        active: true,
    };
    if !filter.is_palm_contact(&palm) {
        return false;
    }

    // Thumb resting on bezel edge: x=20 (< 40), major=30px -> rejected
    let bezel_thumb = TsContactPoint {
        contact_id: 3,
        x: 20,
        y: 540,
        pressure: 600,
        major_axis: 30,
        minor_axis: 20,
        active: true,
    };
    if !filter.is_palm_contact(&bezel_thumb) {
        return false;
    }

    true
}
