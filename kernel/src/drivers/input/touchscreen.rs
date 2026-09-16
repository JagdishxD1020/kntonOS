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

/// Exponential moving average (EMA) jitter filter for touch contact coordinates.
#[derive(Debug, Clone, Copy)]
pub struct TouchCoordinateJitterFilter {
    pub smoothed_x_q8: u32,
    pub smoothed_y_q8: u32,
    pub alpha_q8: u32, // Smoothing factor in Q8 (e.g. 192 = 0.75 weight to new sample)
    pub initialized: bool,
}

impl TouchCoordinateJitterFilter {
    /// Creates a new touch jitter filter with alpha=0.75 for low-latency touch aiming.
    pub const fn new() -> Self {
        Self {
            smoothed_x_q8: 0,
            smoothed_y_q8: 0,
            alpha_q8: 192, // 0.75
            initialized: false,
        }
    }

    /// Resets the filter state when a new contact begins.
    pub fn reset(&mut self, initial_x: u16, initial_y: u16) {
        self.smoothed_x_q8 = (initial_x as u32) << 8;
        self.smoothed_y_q8 = (initial_y as u32) << 8;
        self.initialized = true;
    }

    /// Filters a new coordinate sample, returning smoothed integer coordinates.
    pub fn filter_sample(&mut self, raw_x: u16, raw_y: u16) -> (u16, u16) {
        if !self.initialized {
            self.reset(raw_x, raw_y);
            return (raw_x, raw_y);
        }

        // EMA: smoothed = alpha * raw + (1 - alpha) * smoothed
        let target_x = (raw_x as u32) << 8;
        let target_y = (raw_y as u32) << 8;

        self.smoothed_x_q8 = ((self.smoothed_x_q8 * (256 - self.alpha_q8)) + (target_x * self.alpha_q8)) >> 8;
        self.smoothed_y_q8 = ((self.smoothed_y_q8 * (256 - self.alpha_q8)) + (target_y * self.alpha_q8)) >> 8;

        ((self.smoothed_x_q8 >> 8) as u16, (self.smoothed_y_q8 >> 8) as u16)
    }
}

/// Unit verification for touch coordinate jitter filter.
pub fn verify_touch_jitter_filter() -> bool {
    let mut filter = TouchCoordinateJitterFilter::new();

    // Initial touch at (100, 100)
    let (x0, y0) = filter.filter_sample(100, 100);
    if x0 != 100 || y0 != 100 {
        return false;
    }

    // High frequency 1-pixel noise spike: (101, 99)
    let (x1, y1) = filter.filter_sample(101, 99);
    // 0.75 * 101 + 0.25 * 100 = 75.75 + 25 = 100.75 -> 100
    if x1 != 100 || y1 != 99 {
        return false;
    }

    true
}

/// Recognized gaming touchscreen gesture types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamingTouchGesture {
    None,
    Tap,
    DoubleTap,
    PinchToZoom,
    ThreeFingerSwipeHorizontal,
    VirtualKeyboardSwipe,
}

/// Gaming multi-touch gesture recognizer.
#[derive(Debug, Clone, Copy)]
pub struct GamingGestureRecognizer {
    pub last_tap_time_ms: u32,
    pub tap_count: u8,
    pub initial_pinch_distance: u32,
    pub active_gesture: GamingTouchGesture,
}

impl GamingGestureRecognizer {
    /// Creates a new gesture recognizer.
    pub const fn new() -> Self {
        Self {
            last_tap_time_ms: 0,
            tap_count: 0,
            initial_pinch_distance: 0,
            active_gesture: GamingTouchGesture::None,
        }
    }

    /// Evaluates single or double tap based on tap timestamp (within 300ms window).
    pub fn evaluate_tap(&mut self, current_time_ms: u32) -> GamingTouchGesture {
        if self.last_tap_time_ms > 0 && current_time_ms.saturating_sub(self.last_tap_time_ms) <= 300 {
            self.tap_count = 0;
            self.last_tap_time_ms = 0;
            self.active_gesture = GamingTouchGesture::DoubleTap;
        } else {
            self.tap_count = 1;
            self.last_tap_time_ms = current_time_ms;
            self.active_gesture = GamingTouchGesture::Tap;
        }
        self.active_gesture
    }

    /// Evaluates 2-finger distance delta for pinch-to-zoom (game minimap zoom).
    pub fn evaluate_pinch(&mut self, p1: (u16, u16), p2: (u16, u16)) -> GamingTouchGesture {
        let dx = (p1.0 as i32 - p2.0 as i32).abs();
        let dy = (p1.1 as i32 - p2.1 as i32).abs();
        let dist = (dx * dx + dy * dy) as u32; // Squared distance

        if self.initial_pinch_distance == 0 {
            self.initial_pinch_distance = dist;
            self.active_gesture = GamingTouchGesture::None;
        } else {
            let diff = if dist > self.initial_pinch_distance {
                dist - self.initial_pinch_distance
            } else {
                self.initial_pinch_distance - dist
            };
            if diff > 100 {
                self.active_gesture = GamingTouchGesture::PinchToZoom;
            }
        }
        self.active_gesture
    }

    /// Evaluates 3-finger swipe gesture for game overlay invocation.
    pub fn evaluate_three_finger_swipe(&mut self, active_contacts: usize, delta_x: i32) -> GamingTouchGesture {
        if active_contacts == 3 && delta_x.abs() > 50 {
            self.active_gesture = GamingTouchGesture::ThreeFingerSwipeHorizontal;
        } else {
            self.active_gesture = GamingTouchGesture::None;
        }
        self.active_gesture
    }
}

/// Unit verification for gaming gesture recognizer.
pub fn verify_gaming_gesture_recognizer() -> bool {
    let mut rec = GamingGestureRecognizer::new();

    // Tap and double-tap within 150ms
    let g1 = rec.evaluate_tap(1000);
    if g1 != GamingTouchGesture::Tap {
        return false;
    }
    let g2 = rec.evaluate_tap(1150);
    if g2 != GamingTouchGesture::DoubleTap {
        return false;
    }

    // 2-finger pinch
    rec.evaluate_pinch((100, 100), (200, 200));
    let g_pinch = rec.evaluate_pinch((50, 50), (300, 300));
    if g_pinch != GamingTouchGesture::PinchToZoom {
        return false;
    }

    // 3-finger swipe
    let g_swipe = rec.evaluate_three_finger_swipe(3, 80);
    if g_swipe != GamingTouchGesture::ThreeFingerSwipeHorizontal {
        return false;
    }

    true
}
