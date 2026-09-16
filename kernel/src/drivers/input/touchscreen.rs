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
