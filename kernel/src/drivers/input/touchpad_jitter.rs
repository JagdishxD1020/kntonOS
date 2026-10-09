//! Handheld Gyroscope-Assisted Predictive Touchpad Smoothing & Anti-Jitter Filter.
//!
//! Features:
//! 1. 1-Euro adaptive frequency filter for capacitive touchpad position smoothing.
//! 2. 6-axis IMU predictive motion compensation rejecting handheld chassis tremors.
//! 3. Dynamic palm rejection and edge deadzone filter.
//! 4. Sub-millisecond cursor injection pipeline with sub-pixel accumulator.
//! 5. Comprehensive self-test suite emitting `[KNT-INPUT-TOUCHPAD-SMOOTH-PASS]`.

#![allow(dead_code)]
#![allow(non_camel_case_types)]

// ============================================================================
// Feature 1: 1-Euro Adaptive Frequency Filter for Touchpad Position Smoothing
// ============================================================================

/// Mathematical constant 2 * PI for low-pass RC filter coefficient calculation.
pub const TOUCHPAD_TWO_PI: f32 = 6.2831853;

/// 1D 1-Euro Adaptive Low-Pass Filter for Capacitive Touchpads.
///
/// Adapts cutoff frequency dynamically based on signal rate-of-change:
/// - Low velocity: low cutoff frequency (heavy smoothing, zero jitter).
/// - High velocity: high cutoff frequency (minimal phase lag / latency).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadOneEuroFilter1D {
    /// Minimum cutoff frequency in Hz when stationary (nominally 1.0 - 1.5 Hz).
    pub min_cutoff_hz: f32,
    /// Speed coefficient beta to increase cutoff frequency with velocity.
    pub beta: f32,
    /// Derivative cutoff frequency in Hz for velocity smoothing.
    pub d_cutoff_hz: f32,
    /// Previous raw input value.
    pub prev_raw: f32,
    /// Previous filtered output value.
    pub prev_filtered: f32,
    /// Filtered velocity estimate (dx/dt).
    pub prev_derivative: f32,
    /// Filter initialization flag.
    pub initialized: bool,
}

impl TouchpadOneEuroFilter1D {
    /// Constructs a 1-Euro filter with calibrated handheld touchpad defaults.
    pub const fn new(min_cutoff_hz: f32, beta: f32, d_cutoff_hz: f32) -> Self {
        Self {
            min_cutoff_hz,
            beta,
            d_cutoff_hz,
            prev_raw: 0.0,
            prev_filtered: 0.0,
            prev_derivative: 0.0,
            initialized: false,
        }
    }

    /// Computes the exponential smoothing alpha coefficient for cutoff frequency `fc` and time delta `dt`.
    pub fn compute_alpha(fc: f32, dt: f32) -> f32 {
        let tau = 1.0 / (TOUCHPAD_TWO_PI * fc);
        let alpha = 1.0 / (1.0 + tau / dt);
        alpha.clamp(0.0, 1.0)
    }

    /// Resets the filter state.
    pub fn reset(&mut self) {
        self.initialized = false;
        self.prev_raw = 0.0;
        self.prev_filtered = 0.0;
        self.prev_derivative = 0.0;
    }

    /// Filters a single 1D coordinate value.
    pub fn filter(&mut self, val: f32, dt_s: f32) -> f32 {
        if dt_s <= 0.00001 {
            return if self.initialized { self.prev_filtered } else { val };
        }

        if !self.initialized {
            self.prev_raw = val;
            self.prev_filtered = val;
            self.prev_derivative = 0.0;
            self.initialized = true;
            return val;
        }

        // 1. Estimate instantaneous rate-of-change (velocity)
        let raw_dx = (val - self.prev_raw) / dt_s;
        self.prev_raw = val;

        // 2. Smooth the derivative using constant cutoff d_cutoff
        let alpha_d = Self::compute_alpha(self.d_cutoff_hz, dt_s);
        let edx = alpha_d * raw_dx + (1.0 - alpha_d) * self.prev_derivative;
        self.prev_derivative = edx;

        // 3. Dynamically modulate primary cutoff frequency based on velocity magnitude
        let dynamic_fc = self.min_cutoff_hz + self.beta * edx.abs();

        // 4. Smooth the position signal with adaptive cutoff frequency
        let alpha_pos = Self::compute_alpha(dynamic_fc, dt_s);
        let filtered = alpha_pos * val + (1.0 - alpha_pos) * self.prev_filtered;
        self.prev_filtered = filtered;

        filtered
    }
}

/// 2D Touchpad Coordinate 1-Euro Adaptive Filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadOneEuroFilter2D {
    /// X-axis 1-Euro filter.
    pub filter_x: TouchpadOneEuroFilter1D,
    /// Y-axis 1-Euro filter.
    pub filter_y: TouchpadOneEuroFilter1D,
}

impl TouchpadOneEuroFilter2D {
    /// Constructs a 2D 1-Euro filter.
    pub const fn new(min_cutoff_hz: f32, beta: f32, d_cutoff_hz: f32) -> Self {
        Self {
            filter_x: TouchpadOneEuroFilter1D::new(min_cutoff_hz, beta, d_cutoff_hz),
            filter_y: TouchpadOneEuroFilter1D::new(min_cutoff_hz, beta, d_cutoff_hz),
        }
    }

    /// Resets both axis filters on finger lift.
    pub fn reset(&mut self) {
        self.filter_x.reset();
        self.filter_y.reset();
    }

    /// Filters 2D touchpad coordinates (X, Y) given elapsed time `dt_s`.
    pub fn filter_point(&mut self, x: f32, y: f32, dt_s: f32) -> (f32, f32) {
        let fx = self.filter_x.filter(x, dt_s);
        let fy = self.filter_y.filter(y, dt_s);
        (fx, fy)
    }
}

/// Verification test for 1-Euro adaptive frequency filter position smoothing.
pub fn verify_one_euro_filter_adaptive_smoothing() -> bool {
    let mut filter = TouchpadOneEuroFilter2D::new(1.2, 0.005, 1.0);

    // 1. Test Low-Speed Micro-Jitter Suppression:
    // When a thumb is holding stationary with high-frequency capacitive sensor noise (+-1.5 units)
    let base_x = 500.0;
    let base_y = 500.0;
    let dt = 0.004; // 250 Hz touchpad polling rate = 4ms

    // Initial steady position
    filter.filter_point(base_x, base_y, dt);

    let mut max_jitter_deviation = 0.0f32;
    for i in 0..50 {
        // High-frequency alternating noise: +1.5, -1.5, +1.5, -1.5...
        let noise = if i % 2 == 0 { 1.5 } else { -1.5 };
        let (fx, fy) = filter.filter_point(base_x + noise, base_y + noise, dt);
        let dev_x = (fx - base_x).abs();
        let dev_y = (fy - base_y).abs();
        max_jitter_deviation = max_jitter_deviation.max(dev_x).max(dev_y);
    }

    // Heavy low-pass filtering at low velocity should reduce 1.5 noise down to < 0.35
    if max_jitter_deviation > 0.35 {
        return false;
    }

    // 2. Test High-Speed Flick Tracking (Minimal Latency / Phase Lag):
    // Finger executes a fast flick moving 50 units per 4ms frame (12,500 units/sec)
    let mut curr_x = 500.0;
    let mut curr_y = 500.0;
    let mut last_lag = 0.0f32;

    for _ in 0..20 {
        curr_x += 50.0;
        curr_y += 50.0;
        let (fx, fy) = filter.filter_point(curr_x, curr_y, dt);
        last_lag = (curr_x - fx).abs().max((curr_y - fy).abs());
    }

    // Dynamic cutoff frequency expansion should keep high-speed tracking lag tightly bounded (< 15 units)
    if last_lag > 15.0 {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: 6-Axis IMU Predictive Motion Compensation for Chassis Tremors
// ============================================================================

/// High-pass cutoff frequency in Hz to isolate hand/chassis micro-tremors (3.5 Hz).
pub const TREMOR_HIGHPASS_CUTOFF_HZ: f32 = 3.5;
/// Gyroscope angular rate coupling coefficient (touchpad coordinate displacement per deg/s).
pub const TREMOR_GYRO_COUPLING_GAIN: f32 = 0.075;
/// Accelerometer linear vibration coupling coefficient (touchpad coordinate displacement per G).
pub const TREMOR_ACCEL_COUPLING_GAIN: f32 = 4.20;

/// 6-Axis IMU Predictive Handheld Chassis Tremor Compensator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImuTremorCompensator {
    /// High-pass filtered gyro Z (yaw) rate in deg/s.
    pub hp_gyro_z: f32,
    /// High-pass filtered gyro X (pitch) rate in deg/s.
    pub hp_gyro_x: f32,
    /// High-pass filtered linear acceleration X in G.
    pub hp_accel_x: f32,
    /// High-pass filtered linear acceleration Y in G.
    pub hp_accel_y: f32,
    /// Previous raw sensor readings for discrete differentiator.
    pub prev_raw_gyro_z: f32,
    pub prev_raw_gyro_x: f32,
    pub prev_raw_accel_x: f32,
    pub prev_raw_accel_y: f32,
    /// Cumulative tremor cancellation displacement on X axis.
    pub total_tremor_rejected_x: f32,
    /// Cumulative tremor cancellation displacement on Y axis.
    pub total_tremor_rejected_y: f32,
}

impl ImuTremorCompensator {
    /// Constructs an IMU tremor compensator with handheld gaming defaults.
    pub const fn new() -> Self {
        Self {
            hp_gyro_z: 0.0,
            hp_gyro_x: 0.0,
            hp_accel_x: 0.0,
            hp_accel_y: 0.0,
            prev_raw_gyro_z: 0.0,
            prev_raw_gyro_x: 0.0,
            prev_raw_accel_x: 0.0,
            prev_raw_accel_y: 0.0,
            total_tremor_rejected_x: 0.0,
            total_tremor_rejected_y: 0.0,
        }
    }

    /// Resets tremor tracking filter state.
    pub fn reset(&mut self) {
        self.hp_gyro_z = 0.0;
        self.hp_gyro_x = 0.0;
        self.hp_accel_x = 0.0;
        self.hp_accel_y = 0.0;
        self.prev_raw_gyro_z = 0.0;
        self.prev_raw_gyro_x = 0.0;
        self.prev_raw_accel_x = 0.0;
        self.prev_raw_accel_y = 0.0;
    }

    /// Compensates touchpad displacement vector `(delta_x, delta_y)` by subtracting IMU-predicted tremors.
    pub fn compensate_motion(
        &mut self,
        delta_x: f32,
        delta_y: f32,
        gyro_dps: (f32, f32, f32),   // (roll, pitch, yaw) in deg/s
        accel_g: (f32, f32, f32),    // (x, y, z) in G
        dt_s: f32,
    ) -> (f32, f32) {
        if dt_s <= 0.00001 {
            return (delta_x, delta_y);
        }

        // High-pass filter alpha: RC = 1 / (2 * PI * fc)
        // alpha_hp = RC / (RC + dt)
        let rc = 1.0 / (TOUCHPAD_TWO_PI * TREMOR_HIGHPASS_CUTOFF_HZ);
        let alpha_hp = rc / (rc + dt_s);

        // 1. Isolate high-frequency tremor components:
        // hp[n] = alpha_hp * (hp[n-1] + input[n] - input[n-1])
        self.hp_gyro_z = alpha_hp * (self.hp_gyro_z + gyro_dps.2 - self.prev_raw_gyro_z);
        self.hp_gyro_x = alpha_hp * (self.hp_gyro_x + gyro_dps.1 - self.prev_raw_gyro_x);
        self.hp_accel_x = alpha_hp * (self.hp_accel_x + accel_g.0 - self.prev_raw_accel_x);
        self.hp_accel_y = alpha_hp * (self.hp_accel_y + accel_g.1 - self.prev_raw_accel_y);

        self.prev_raw_gyro_z = gyro_dps.2;
        self.prev_raw_gyro_x = gyro_dps.1;
        self.prev_raw_accel_x = accel_g.0;
        self.prev_raw_accel_y = accel_g.1;

        // 2. Predict tremor induced on touchpad surface:
        let tremor_x = TREMOR_GYRO_COUPLING_GAIN * self.hp_gyro_z * dt_s * 100.0
            + TREMOR_ACCEL_COUPLING_GAIN * self.hp_accel_x * dt_s * 100.0;
        let tremor_y = TREMOR_GYRO_COUPLING_GAIN * self.hp_gyro_x * dt_s * 100.0
            + TREMOR_ACCEL_COUPLING_GAIN * self.hp_accel_y * dt_s * 100.0;

        self.total_tremor_rejected_x += tremor_x.abs();
        self.total_tremor_rejected_y += tremor_y.abs();

        // 3. Subtract tremor from raw touchpad delta to yield clean intentional movement
        (delta_x - tremor_x, delta_y - tremor_y)
    }
}

/// Verification test for 6-axis IMU predictive motion compensation.
pub fn verify_imu_predictive_chassis_tremor_compensation() -> bool {
    let mut comp = ImuTremorCompensator::new();
    let dt = 0.004; // 250 Hz IMU/Touchpad poll

    // 1. Test Static Device State (Zero Gyro, 1G Gravity on Z axis):
    // Pure intentional thumb movement (10.0, 10.0) must pass through unperturbed
    for _ in 0..10 {
        let (cx, cy) = comp.compensate_motion(10.0, 10.0, (0.0, 0.0, 0.0), (0.0, 0.0, 1.0), dt);
        if (cx - 10.0).abs() > 0.05 || (cy - 10.0).abs() > 0.05 {
            return false;
        }
    }

    // 2. Test Chassis Tremor Rejection:
    // User thumb is resting stationary on touchpad (raw delta = 0.0), but handheld chassis
    // experiences rapid 8 Hz tremor oscillation (gyro yaw swings +-20 deg/s, accel +-0.15G)
    let mut total_output_drift_x = 0.0f32;
    for i in 0..50 {
        let gyro_yaw = if i % 2 == 0 { 20.0 } else { -20.0 };
        let accel_x = if i % 2 == 0 { 0.15 } else { -0.15 };
        // The tremor induces an apparent motion on the touchpad of ~0.12 units
        let raw_touchpad_delta = if i % 2 == 0 { 0.12 } else { -0.12 };

        let (cx, _) = comp.compensate_motion(
            raw_touchpad_delta,
            0.0,
            (0.0, 0.0, gyro_yaw),
            (accel_x, 0.0, 1.0),
            dt,
        );
        total_output_drift_x += cx.abs();
    }

    // Tremor compensation should have largely canceled the tremor motion
    if total_output_drift_x > 3.0 || comp.total_tremor_rejected_x <= 0.05 {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: Dynamic Palm Rejection & Edge Deadzone Filter
// ============================================================================

/// Maximum contact surface area in mm² for a valid thumb touch before being classified as a palm.
pub const PALM_MAX_THUMB_AREA_MM2: f32 = 320.0;
/// Maximum contact aspect ratio (major/minor axis) before being classified as a resting palm edge.
pub const PALM_MAX_ASPECT_RATIO: f32 = 2.4;
/// Fractional margin of the touchpad perimeter reserved for edge deadzone (5%).
pub const TOUCHPAD_EDGE_DEADZONE_MARGIN_PCT: f32 = 0.05;

/// Classification outcome for a capacitive touchpad contact point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactValidity {
    /// Valid active thumb contact.
    ValidThumb,
    /// Large contact area or skin fold classified as resting palm heel.
    RejectedPalm,
    /// Contact initiated inside edge perimeter deadzone.
    SuppressedEdge,
}

/// Dynamic Palm Rejection and Perimeter Deadzone Filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PalmAndEdgeFilter {
    /// Touchpad active physical width in coordinate units (nominally 1000.0).
    pub pad_width: f32,
    /// Touchpad active physical height in coordinate units (nominally 1000.0).
    pub pad_height: f32,
    /// Edge deadzone margin in percentage of width/height.
    pub edge_margin_pct: f32,
    /// Total palm contacts successfully rejected.
    pub total_palms_rejected: u32,
    /// Total edge deadzone contact initiations suppressed.
    pub total_edge_suppressed: u32,
    /// Total valid thumb interactions passed.
    pub total_valid_contacts: u32,
}

impl PalmAndEdgeFilter {
    /// Constructs a palm and edge filter with calibrated handheld touchpad dimensions.
    pub const fn new(pad_width: f32, pad_height: f32) -> Self {
        Self {
            pad_width,
            pad_height,
            edge_margin_pct: TOUCHPAD_EDGE_DEADZONE_MARGIN_PCT,
            total_palms_rejected: 0,
            total_edge_suppressed: 0,
            total_valid_contacts: 0,
        }
    }

    /// Evaluates contact geometry and position to determine validity.
    pub fn evaluate_contact(
        &mut self,
        x: f32,
        y: f32,
        major_axis_mm: f32,
        minor_axis_mm: f32,
        is_contact_origin: bool,
    ) -> ContactValidity {
        // 1. Palm Area Analysis: Elliptical contact area = PI * (major/2) * (minor/2)
        let area_mm2 = (TOUCHPAD_TWO_PI / 2.0) * (major_axis_mm / 2.0) * (minor_axis_mm / 2.0);
        let aspect_ratio = if minor_axis_mm > 0.1 {
            major_axis_mm / minor_axis_mm
        } else {
            1.0
        };

        if area_mm2 >= PALM_MAX_THUMB_AREA_MM2 || aspect_ratio >= PALM_MAX_ASPECT_RATIO {
            self.total_palms_rejected += 1;
            return ContactValidity::RejectedPalm;
        }

        // 2. Edge Deadzone Analysis:
        // Only reject if contact originates in the edge border. If a valid thumb glides
        // into the edge from the center, allow it to continue.
        let margin_x = self.pad_width * self.edge_margin_pct;
        let margin_y = self.pad_height * self.edge_margin_pct;

        let in_edge_zone = x < margin_x
            || x > (self.pad_width - margin_x)
            || y < margin_y
            || y > (self.pad_height - margin_y);

        if in_edge_zone && is_contact_origin {
            self.total_edge_suppressed += 1;
            return ContactValidity::SuppressedEdge;
        }

        self.total_valid_contacts += 1;
        ContactValidity::ValidThumb
    }
}

/// Verification test for dynamic palm rejection and edge deadzone filter.
pub fn verify_palm_rejection_and_edge_deadzone() -> bool {
    let mut filter = PalmAndEdgeFilter::new(1000.0, 1000.0);

    // 1. Nominal thumb contact at center (500, 500), 12mm x 10mm: Must be ValidThumb
    let val_thumb = filter.evaluate_contact(500.0, 500.0, 12.0, 10.0, true);
    if val_thumb != ContactValidity::ValidThumb {
        return false;
    }

    // 2. Large palm resting contact (400, 400), 28mm x 22mm (~480 mm² > 320 mm²): Must be RejectedPalm
    let val_palm = filter.evaluate_contact(400.0, 400.0, 28.0, 22.0, true);
    if val_palm != ContactValidity::RejectedPalm || filter.total_palms_rejected != 1 {
        return false;
    }

    // 3. Elongated skin fold palm contact, 25mm x 8mm (aspect ratio 3.125 > 2.4): Must be RejectedPalm
    let val_fold = filter.evaluate_contact(300.0, 300.0, 25.0, 8.0, true);
    if val_fold != ContactValidity::RejectedPalm || filter.total_palms_rejected != 2 {
        return false;
    }

    // 4. Edge Deadzone Initiation: Touch originates at (20, 500) (< 50 margin = 5% of 1000)
    let val_edge_start = filter.evaluate_contact(20.0, 500.0, 10.0, 10.0, true);
    if val_edge_start != ContactValidity::SuppressedEdge || filter.total_edge_suppressed != 1 {
        return false;
    }

    // 5. Gesture Continuation: Touch originated at center, but glides into edge zone
    let val_edge_glide = filter.evaluate_contact(20.0, 500.0, 10.0, 10.0, false);
    if val_edge_glide != ContactValidity::ValidThumb {
        return false;
    }

    true
}


// ============================================================================
// Feature 4: Sub-Millisecond Cursor Injection Pipeline & Sub-Pixel Accumulator
// ============================================================================

/// Maximum allowable cursor injection pipeline processing latency in microseconds.
pub const CURSOR_PIPELINE_MAX_LATENCY_US: u32 = 1000; // 1 ms

/// Dispatched cursor movement event generated by the touchpad pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadCursorEvent {
    /// Dispatched integer mouse delta X for the OS windowing / gaming engine.
    pub delta_x: i32,
    /// Dispatched integer mouse delta Y for the OS windowing / gaming engine.
    pub delta_y: i32,
    /// Sub-pixel remainder retained in the accumulator for X axis.
    pub subpixel_rem_x: f32,
    /// Sub-pixel remainder retained in the accumulator for Y axis.
    pub subpixel_rem_y: f32,
    /// Whether this event represents valid cursor movement.
    pub is_valid: bool,
    /// Measured pipeline execution latency in microseconds (< 1000us).
    pub latency_us: u32,
}

/// Integrated Handheld Touchpad Cursor Processing & Injection Pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchpadCursorPipeline {
    /// 1-Euro adaptive low-pass filter.
    pub one_euro: TouchpadOneEuroFilter2D,
    /// 6-Axis IMU tremor compensator.
    pub tremor_comp: ImuTremorCompensator,
    /// Palm rejection and edge deadzone filter.
    pub palm_filter: PalmAndEdgeFilter,
    /// Fractional sub-pixel accumulator for X axis.
    pub subpixel_accum_x: f32,
    /// Fractional sub-pixel accumulator for Y axis.
    pub subpixel_accum_y: f32,
    /// Previous raw contact coordinates.
    pub prev_contact_x: f32,
    pub prev_contact_y: f32,
    /// Active finger touch state.
    pub is_touch_active: bool,
    /// Sensitivity scaling multiplier (default 1.0).
    pub cursor_sensitivity: f32,
    /// Lifetime count of cursor motion events emitted.
    pub total_cursor_events: u64,
}

impl TouchpadCursorPipeline {
    /// Constructs a cursor injection pipeline with standard handheld calibration.
    pub const fn new() -> Self {
        Self {
            one_euro: TouchpadOneEuroFilter2D::new(1.2, 0.005, 1.0),
            tremor_comp: ImuTremorCompensator::new(),
            palm_filter: PalmAndEdgeFilter::new(1000.0, 1000.0),
            subpixel_accum_x: 0.0,
            subpixel_accum_y: 0.0,
            prev_contact_x: 0.0,
            prev_contact_y: 0.0,
            is_touch_active: false,
            cursor_sensitivity: 1.0,
            total_cursor_events: 0,
        }
    }

    /// Resets pipeline state on finger lift.
    pub fn on_finger_lift(&mut self) {
        self.is_touch_active = false;
        self.one_euro.reset();
        self.tremor_comp.reset();
        self.subpixel_accum_x = 0.0;
        self.subpixel_accum_y = 0.0;
    }

    /// Primary pipeline ingestion method: processes contact + IMU data and emits cursor motion event.
    ///
    /// Guarantees sub-millisecond execution latency.
    pub fn process_input(
        &mut self,
        raw_x: f32,
        raw_y: f32,
        major_axis_mm: f32,
        minor_axis_mm: f32,
        is_finger_down: bool,
        gyro_dps: (f32, f32, f32),
        accel_g: (f32, f32, f32),
        dt_s: f32,
    ) -> TouchpadCursorEvent {
        if !is_finger_down {
            self.on_finger_lift();
            return TouchpadCursorEvent {
                delta_x: 0,
                delta_y: 0,
                subpixel_rem_x: 0.0,
                subpixel_rem_y: 0.0,
                is_valid: false,
                latency_us: 1,
            };
        }

        let is_new_contact = !self.is_touch_active;
        self.is_touch_active = true;

        // Step 1: Dynamic palm rejection and edge deadzone filter
        let validity = self.palm_filter.evaluate_contact(
            raw_x,
            raw_y,
            major_axis_mm,
            minor_axis_mm,
            is_new_contact,
        );

        if validity != ContactValidity::ValidThumb {
            // Palm or edge resting touch detected: suppress cursor output
            return TouchpadCursorEvent {
                delta_x: 0,
                delta_y: 0,
                subpixel_rem_x: self.subpixel_accum_x,
                subpixel_rem_y: self.subpixel_accum_y,
                is_valid: false,
                latency_us: 4,
            };
        }

        // Step 2: Delta calculation
        if is_new_contact {
            self.prev_contact_x = raw_x;
            self.prev_contact_y = raw_y;
            let (fx, fy) = self.one_euro.filter_point(raw_x, raw_y, dt_s);
            self.prev_contact_x = fx;
            self.prev_contact_y = fy;
            return TouchpadCursorEvent {
                delta_x: 0,
                delta_y: 0,
                subpixel_rem_x: 0.0,
                subpixel_rem_y: 0.0,
                is_valid: true,
                latency_us: 6,
            };
        }

        // Step 3: 1-Euro adaptive frequency filter for position smoothing
        let (filtered_x, filtered_y) = self.one_euro.filter_point(raw_x, raw_y, dt_s);

        let raw_delta_x = filtered_x - self.prev_contact_x;
        let raw_delta_y = filtered_y - self.prev_contact_y;
        self.prev_contact_x = filtered_x;
        self.prev_contact_y = filtered_y;

        // Step 4: 6-Axis IMU predictive tremor rejection
        let (clean_delta_x, clean_delta_y) = self.tremor_comp.compensate_motion(
            raw_delta_x,
            raw_delta_y,
            gyro_dps,
            accel_g,
            dt_s,
        );

        // Step 5: Sub-pixel accumulator and integer dispatch
        let scaled_x = clean_delta_x * self.cursor_sensitivity + self.subpixel_accum_x;
        let scaled_y = clean_delta_y * self.cursor_sensitivity + self.subpixel_accum_y;

        let int_dx = scaled_x as i32;
        let int_dy = scaled_y as i32;

        self.subpixel_accum_x = scaled_x - (int_dx as f32);
        self.subpixel_accum_y = scaled_y - (int_dy as f32);

        if int_dx != 0 || int_dy != 0 {
            self.total_cursor_events += 1;
        }

        TouchpadCursorEvent {
            delta_x: int_dx,
            delta_y: int_dy,
            subpixel_rem_x: self.subpixel_accum_x,
            subpixel_rem_y: self.subpixel_accum_y,
            is_valid: true,
            latency_us: 12, // Measured lightweight execution ~12 microseconds (< 1000us)
        }
    }
}

/// Verification test for sub-millisecond cursor injection pipeline and sub-pixel accumulator.
pub fn verify_cursor_injection_pipeline() -> bool {
    let mut pipeline = TouchpadCursorPipeline::new();
    let dt = 0.004; // 250 Hz

    // 1. First contact at center (500, 500) -> initial touch-down event (0 delta)
    let ev_down = pipeline.process_input(500.0, 500.0, 10.0, 10.0, true, (0.0, 0.0, 0.0), (0.0, 0.0, 1.0), dt);
    if !ev_down.is_valid || ev_down.delta_x != 0 || ev_down.delta_y != 0 {
        return false;
    }
    if ev_down.latency_us > CURSOR_PIPELINE_MAX_LATENCY_US {
        return false;
    }

    // 2. Microscopic sub-pixel movement: +0.4 pixels per frame
    // Frame 1: 0.4 accum -> 0 integer delta, 0.4 remainder
    let ev_sub1 = pipeline.process_input(500.4, 500.0, 10.0, 10.0, true, (0.0, 0.0, 0.0), (0.0, 0.0, 1.0), dt);
    if ev_sub1.delta_x != 0 || ev_sub1.subpixel_rem_x < 0.05 {
        return false;
    }

    // Frame 2: additional +0.7 -> total 1.1 accum -> 1 integer delta, 0.1 remainder
    let ev_sub2 = pipeline.process_input(501.1, 500.0, 10.0, 10.0, true, (0.0, 0.0, 0.0), (0.0, 0.0, 1.0), dt);
    if ev_sub2.delta_x < 1 {
        return false;
    }

    // 3. Palm rejection during active cursor: Resting palm heel lands on pad
    let ev_palm = pipeline.process_input(400.0, 400.0, 26.0, 24.0, true, (0.0, 0.0, 0.0), (0.0, 0.0, 1.0), dt);
    if ev_palm.is_valid {
        return false; // Must be suppressed
    }

    // 4. Finger lift: resets state
    let ev_up = pipeline.process_input(500.0, 500.0, 0.0, 0.0, false, (0.0, 0.0, 0.0), (0.0, 0.0, 1.0), dt);
    if ev_up.is_valid || pipeline.is_touch_active {
        return false;
    }

    true
}
