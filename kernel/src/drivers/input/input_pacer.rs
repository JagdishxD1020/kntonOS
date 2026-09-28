//! Handheld Sub-Millisecond Input Aggregator & High-Poll-Rate HID Pacer.

#![allow(dead_code)]

/// Supported polling rates in Hertz.
pub const POLL_RATE_250HZ: u16 = 250;
pub const POLL_RATE_500HZ: u16 = 500;
pub const POLL_RATE_1000HZ: u16 = 1000;
pub const POLL_RATE_2000HZ: u16 = 2000;

/// Standard gamepad digital buttons bitmask.
pub const BTN_A: u16 = 1 << 0;
pub const BTN_B: u16 = 1 << 1;
pub const BTN_X: u16 = 1 << 2;
pub const BTN_Y: u16 = 1 << 3;
pub const BTN_DPAD_UP: u16 = 1 << 4;
pub const BTN_DPAD_DOWN: u16 = 1 << 5;
pub const BTN_DPAD_LEFT: u16 = 1 << 6;
pub const BTN_DPAD_RIGHT: u16 = 1 << 7;
pub const BTN_LB: u16 = 1 << 8;
pub const BTN_RB: u16 = 1 << 9;
pub const BTN_THUMB_L: u16 = 1 << 10;
pub const BTN_THUMB_R: u16 = 1 << 11;
pub const BTN_GRIP_L: u16 = 1 << 12;
pub const BTN_GRIP_R: u16 = 1 << 13;
pub const BTN_MENU: u16 = 1 << 14;
pub const BTN_VIEW: u16 = 1 << 15;

/// Aggregated sub-millisecond input packet captured from all handheld controllers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AggregatedInputPacket {
    /// Monotonically increasing packet sequence counter.
    pub sequence: u32,
    /// Hardware timestamp in microseconds (µs).
    pub timestamp_us: u64,
    /// Gamepad digital buttons bitmask.
    pub buttons: u16,
    /// Hall-effect Left Stick X and Y (-32768 to 32767).
    pub left_stick_x: i16,
    pub left_stick_y: i16,
    /// Hall-effect Right Stick X and Y (-32768 to 32767).
    pub right_stick_x: i16,
    pub right_stick_y: i16,
    /// Hall-effect Left and Right analog triggers (0 to 1023).
    pub left_trigger: u16,
    pub right_trigger: u16,
    /// Capacitive Left Trackpad relative delta (dx, dy) and pressure (0..255).
    pub left_trackpad_dx: i16,
    pub left_trackpad_dy: i16,
    pub left_trackpad_pressure: u8,
    pub left_trackpad_touch: bool,
    /// Capacitive Right Trackpad relative delta (dx, dy) and pressure (0..255).
    pub right_trackpad_dx: i16,
    pub right_trackpad_dy: i16,
    pub right_trackpad_pressure: u8,
    pub right_trackpad_touch: bool,
}

impl AggregatedInputPacket {
    pub const fn empty() -> Self {
        Self {
            sequence: 0,
            timestamp_us: 0,
            buttons: 0,
            left_stick_x: 0,
            left_stick_y: 0,
            right_stick_x: 0,
            right_stick_y: 0,
            left_trigger: 0,
            right_trigger: 0,
            left_trackpad_dx: 0,
            left_trackpad_dy: 0,
            left_trackpad_pressure: 0,
            left_trackpad_touch: false,
            right_trackpad_dx: 0,
            right_trackpad_dy: 0,
            right_trackpad_pressure: 0,
            right_trackpad_touch: false,
        }
    }
}

/// High-Polling Rate Engine aggregating inputs at 1000Hz or 2000Hz.
#[derive(Debug, Clone)]
pub struct HighPollRateAggregator {
    pub poll_rate_hz: u16,
    pub expected_interval_us: u32,
    pub next_sequence: u32,
    pub total_packets_generated: u64,
}

impl HighPollRateAggregator {
    pub const fn new(poll_rate_hz: u16) -> Self {
        let expected_interval_us = match poll_rate_hz {
            POLL_RATE_2000HZ => 500,
            POLL_RATE_1000HZ => 1000,
            POLL_RATE_500HZ => 2000,
            _ => 4000, // 250Hz default fallback
        };
        Self {
            poll_rate_hz,
            expected_interval_us,
            next_sequence: 1,
            total_packets_generated: 0,
        }
    }

    /// Sets operating polling frequency and updates micro-tick interval.
    pub fn set_poll_rate(&mut self, poll_rate_hz: u16) {
        self.poll_rate_hz = poll_rate_hz;
        self.expected_interval_us = match poll_rate_hz {
            POLL_RATE_2000HZ => 500,
            POLL_RATE_1000HZ => 1000,
            POLL_RATE_500HZ => 2000,
            _ => 4000,
        };
    }

    /// Aggregates raw hardware sensors into a single timestamped packet.
    pub fn aggregate_sample(
        &mut self,
        timestamp_us: u64,
        buttons: u16,
        left_stick: (i16, i16),
        right_stick: (i16, i16),
        triggers: (u16, u16),
        left_trackpad: (i16, i16, u8, bool),
        right_trackpad: (i16, i16, u8, bool),
    ) -> AggregatedInputPacket {
        let pkt = AggregatedInputPacket {
            sequence: self.next_sequence,
            timestamp_us,
            buttons,
            left_stick_x: left_stick.0,
            left_stick_y: left_stick.1,
            right_stick_x: right_stick.0,
            right_stick_y: right_stick.1,
            left_trigger: triggers.0.min(1023),
            right_trigger: triggers.1.min(1023),
            left_trackpad_dx: left_trackpad.0,
            left_trackpad_dy: left_trackpad.1,
            left_trackpad_pressure: left_trackpad.2,
            left_trackpad_touch: left_trackpad.3,
            right_trackpad_dx: right_trackpad.0,
            right_trackpad_dy: right_trackpad.1,
            right_trackpad_pressure: right_trackpad.2,
            right_trackpad_touch: right_trackpad.3,
        };

        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.total_packets_generated += 1;
        pkt
    }
}

/// Unit test verifying high-polling rate 1000Hz/2000Hz input aggregator.
pub fn verify_high_poll_aggregator() -> bool {
    let mut agg = HighPollRateAggregator::new(POLL_RATE_1000HZ);
    if agg.expected_interval_us != 1000 {
        return false;
    }

    // 1. Generate 1000Hz sample (1ms micro-tick)
    let p1 = agg.aggregate_sample(
        1000,
        BTN_A | BTN_RB,
        (16000, -12000),
        (0, 0),
        (512, 1023),
        (5, -2, 128, true),
        (0, 0, 0, false),
    );
    if p1.sequence != 1 || p1.timestamp_us != 1000 {
        return false;
    }
    if p1.buttons != (BTN_A | BTN_RB) || p1.left_stick_x != 16000 || p1.left_trigger != 512 {
        return false;
    }
    if !p1.left_trackpad_touch || p1.left_trackpad_dx != 5 {
        return false;
    }

    // 2. Switch to 2000Hz (0.5ms / 500us micro-tick)
    agg.set_poll_rate(POLL_RATE_2000HZ);
    if agg.expected_interval_us != 500 {
        return false;
    }

    let p2 = agg.aggregate_sample(
        1500,
        BTN_B,
        (-8000, 4000),
        (100, -50),
        (0, 256),
        (0, 0, 0, false),
        (-3, 8, 200, true),
    );
    if p2.sequence != 2 || p2.timestamp_us != 1500 {
        return false;
    }
    if p2.right_trigger != 256 || !p2.right_trackpad_touch || p2.right_trackpad_dy != 8 {
        return false;
    }
    if agg.total_packets_generated != 2 {
        return false;
    }

    true
}


// ============================================================================
// Microsecond Hardware Timestamping & Circular Jitter Buffer
// ============================================================================

/// Maximum capacity of the input pacer circular jitter ring buffer.
pub const JITTER_BUFFER_CAPACITY: usize = 32;

/// Circular jitter smoothing queue decoupling USB interrupts from game engine pulls.
#[derive(Debug, Clone)]
pub struct InputJitterBuffer {
    pub ring: [AggregatedInputPacket; JITTER_BUFFER_CAPACITY],
    pub head: usize,
    pub tail: usize,
    pub count: usize,
    pub last_arrival_ts_us: u64,
    pub expected_interval_us: u32,
    pub jitter_samples: u32,
    pub total_jitter_us: u64,
    pub max_jitter_us: u32,
}

impl InputJitterBuffer {
    pub const fn new(expected_interval_us: u32) -> Self {
        Self {
            ring: [AggregatedInputPacket::empty(); JITTER_BUFFER_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
            last_arrival_ts_us: 0,
            expected_interval_us,
            jitter_samples: 0,
            total_jitter_us: 0,
            max_jitter_us: 0,
        }
    }

    /// Updates expected interval when polling rate changes.
    pub fn set_expected_interval(&mut self, interval_us: u32) {
        self.expected_interval_us = interval_us;
    }

    /// Pushes a newly aggregated packet into the ring buffer with jitter tracking.
    pub fn push(&mut self, packet: AggregatedInputPacket) {
        if self.last_arrival_ts_us > 0 && packet.timestamp_us > self.last_arrival_ts_us {
            let actual_delta_us = (packet.timestamp_us - self.last_arrival_ts_us) as u32;
            let jitter = if actual_delta_us > self.expected_interval_us {
                actual_delta_us - self.expected_interval_us
            } else {
                self.expected_interval_us - actual_delta_us
            };

            self.total_jitter_us += jitter as u64;
            self.jitter_samples += 1;
            if jitter > self.max_jitter_us {
                self.max_jitter_us = jitter;
            }
        }
        self.last_arrival_ts_us = packet.timestamp_us;

        // If ring buffer is full, advance tail to drop oldest frame (drop-oldest policy for low latency)
        if self.count == JITTER_BUFFER_CAPACITY {
            self.tail = (self.tail + 1) % JITTER_BUFFER_CAPACITY;
        } else {
            self.count += 1;
        }

        self.ring[self.head] = packet;
        self.head = (self.head + 1) % JITTER_BUFFER_CAPACITY;
    }

    /// Pops the oldest smoothed input packet for game engine dispatch.
    pub fn pop(&mut self) -> Option<AggregatedInputPacket> {
        if self.count == 0 {
            return None;
        }

        let pkt = self.ring[self.tail];
        self.tail = (self.tail + 1) % JITTER_BUFFER_CAPACITY;
        self.count -= 1;
        Some(pkt)
    }

    /// Peeks at the next available packet without consuming it.
    pub fn peek(&self) -> Option<&AggregatedInputPacket> {
        if self.count == 0 {
            None
        } else {
            Some(&self.ring[self.tail])
        }
    }

    /// Computes average inter-arrival jitter in microseconds.
    pub fn average_jitter_us(&self) -> u32 {
        if self.jitter_samples == 0 {
            0
        } else {
            (self.total_jitter_us / self.jitter_samples as u64) as u32
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn len(&self) -> usize {
        self.count
    }
}

/// Unit test verifying microsecond hardware timestamping and circular jitter buffer.
pub fn verify_jitter_buffer() -> bool {
    let mut jb = InputJitterBuffer::new(1000); // 1000us (1000Hz) expected interval
    if !jb.is_empty() || jb.len() != 0 {
        return false;
    }

    // 1. Push 4 packets with synthetic USB jitter (expected 1000us delta)
    let mut agg = HighPollRateAggregator::new(POLL_RATE_1000HZ);
    let p1 = agg.aggregate_sample(1000, BTN_A, (0, 0), (0, 0), (0, 0), (0, 0, 0, false), (0, 0, 0, false));
    let p2 = agg.aggregate_sample(1950, BTN_B, (0, 0), (0, 0), (0, 0), (0, 0, 0, false), (0, 0, 0, false)); // delta 950 (jitter 50)
    let p3 = agg.aggregate_sample(3100, BTN_X, (0, 0), (0, 0), (0, 0), (0, 0, 0, false), (0, 0, 0, false)); // delta 1150 (jitter 150)
    let p4 = agg.aggregate_sample(4080, BTN_Y, (0, 0), (0, 0), (0, 0), (0, 0, 0, false), (0, 0, 0, false)); // delta 980 (jitter 20)

    jb.push(p1);
    jb.push(p2);
    jb.push(p3);
    jb.push(p4);

    if jb.len() != 4 || jb.max_jitter_us != 150 {
        return false;
    }
    // Average jitter: (50 + 150 + 20) / 3 = 220 / 3 = 73 us
    if jb.average_jitter_us() != 73 {
        return false;
    }

    // 2. FIFO order verification
    let out1 = match jb.pop() { Some(p) => p, None => return false };
    let out2 = match jb.pop() { Some(p) => p, None => return false };
    if out1.buttons != BTN_A || out2.buttons != BTN_B || jb.len() != 2 {
        return false;
    }

    // 3. Ring buffer wrap-around test
    for i in 0..40 {
        let pkt = agg.aggregate_sample((5000 + i * 1000) as u64, BTN_A, (0, 0), (0, 0), (0, 0), (0, 0, 0, false), (0, 0, 0, false));
        jb.push(pkt);
    }
    if jb.len() != JITTER_BUFFER_CAPACITY {
        return false;
    }

    true
}


// ============================================================================
// Power-Aware Dynamic Polling Scaler
// ============================================================================

/// Low-battery polling rate throttling threshold percentage.
pub const LOW_BATTERY_THROTTLE_SOC: u8 = 20;
/// Critical battery polling rate throttling threshold percentage.
pub const CRITICAL_BATTERY_THROTTLE_SOC: u8 = 10;
/// SoC hysteresis buffer percentage to prevent rapid frequency oscillation.
pub const BATTERY_HYSTERESIS_DELTA_SOC: u8 = 3;

/// Dynamic Polling Scaler adjusting HID poll rates based on thermal and battery state.
#[derive(Debug, Clone)]
pub struct PowerAwarePollingScaler {
    pub preferred_rate_hz: u16,
    pub current_rate_hz: u16,
    pub low_power_downscaled: bool,
    pub scale_transitions_count: u32,
}

impl PowerAwarePollingScaler {
    pub const fn new(preferred_rate_hz: u16) -> Self {
        Self {
            preferred_rate_hz,
            current_rate_hz: preferred_rate_hz,
            low_power_downscaled: false,
            scale_transitions_count: 0,
        }
    }

    /// Evaluates system power metrics and determines optimal polling frequency.
    pub fn evaluate_rate(
        &mut self,
        battery_soc_pct: u8,
        whisper_mode: bool,
        ac_connected: bool,
    ) -> u16 {
        let target_rate = if ac_connected {
            if whisper_mode {
                POLL_RATE_500HZ
            } else {
                self.preferred_rate_hz
            }
        } else if whisper_mode {
            if battery_soc_pct < CRITICAL_BATTERY_THROTTLE_SOC {
                POLL_RATE_250HZ
            } else {
                POLL_RATE_500HZ
            }
        } else if battery_soc_pct < CRITICAL_BATTERY_THROTTLE_SOC {
            POLL_RATE_250HZ
        } else if self.low_power_downscaled {
            // Hysteresis: must recover above (LOW_BATTERY_THROTTLE_SOC + BATTERY_HYSTERESIS_DELTA_SOC)
            if battery_soc_pct >= (LOW_BATTERY_THROTTLE_SOC + BATTERY_HYSTERESIS_DELTA_SOC) {
                self.low_power_downscaled = false;
                self.preferred_rate_hz
            } else {
                POLL_RATE_500HZ
            }
        } else if battery_soc_pct < LOW_BATTERY_THROTTLE_SOC {
            self.low_power_downscaled = true;
            POLL_RATE_500HZ
        } else {
            self.preferred_rate_hz
        };

        if target_rate != self.current_rate_hz {
            self.current_rate_hz = target_rate;
            self.scale_transitions_count += 1;
        }

        self.current_rate_hz
    }

    pub fn is_downscaled(&self) -> bool {
        self.current_rate_hz < self.preferred_rate_hz
    }
}

/// Unit test verifying power-aware dynamic polling rate scaler.
pub fn verify_power_aware_scaler() -> bool {
    let mut scaler = PowerAwarePollingScaler::new(POLL_RATE_1000HZ);

    // 1. AC connected, normal battery: full 1000Hz
    let r1 = scaler.evaluate_rate(80, false, true);
    if r1 != POLL_RATE_1000HZ || scaler.is_downscaled() {
        return false;
    }

    // 2. Unplugged on battery, drops below 20% (18%): downscales to 500Hz
    let r2 = scaler.evaluate_rate(18, false, false);
    if r2 != POLL_RATE_500HZ || !scaler.is_downscaled() {
        return false;
    }

    // 3. Drops to critical battery (8% < 10%): downscales to 250Hz
    let r3 = scaler.evaluate_rate(8, false, false);
    if r3 != POLL_RATE_250HZ || !scaler.is_downscaled() {
        return false;
    }

    // 4. Charges up slightly to 21% (between 20% and 23% recovery): remains 500Hz due to hysteresis
    let r4 = scaler.evaluate_rate(21, false, false);
    if r4 != POLL_RATE_500HZ {
        return false;
    }

    // 5. Charges past 23% (25%): recovers full 1000Hz
    let r5 = scaler.evaluate_rate(25, false, false);
    if r5 != POLL_RATE_1000HZ || scaler.is_downscaled() {
        return false;
    }

    // 6. Whisper mode active on AC: throttles to 500Hz for acoustic governor
    let r_whisper = scaler.evaluate_rate(100, true, true);
    if r_whisper != POLL_RATE_500HZ || !scaler.is_downscaled() {
        return false;
    }

    true
}


// ============================================================================
// Integrated HID Input Pacer Dispatching to Win32 Raw Input Buffers
// ============================================================================

/// Win32 Raw Input device class constant for HID Gamepad / Joystick.
pub const RIM_TYPEHID: u32 = 2;

/// Standardized Win32 Raw Input packet structure consumed by game loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Win32RawInputPacket {
    /// RAWINPUTHEADER dwType (RIM_TYPEHID = 2).
    pub header_type: u32,
    /// Handle to simulated hardware input device.
    pub device_handle: u64,
    /// wParam input message code (e.g. RIM_INPUT = 0).
    pub wparam: usize,
    /// Aggregated handheld input payload.
    pub payload: AggregatedInputPacket,
}

/// Integrated Sub-Millisecond Input Aggregator & High-Poll-Rate HID Pacer.
#[derive(Debug, Clone)]
pub struct InputPacerManager {
    pub aggregator: HighPollRateAggregator,
    pub jitter_buffer: InputJitterBuffer,
    pub scaler: PowerAwarePollingScaler,
    pub total_dispatched_to_raw_input: u64,
    pub device_handle: u64,
}

impl InputPacerManager {
    pub const fn new(preferred_rate_hz: u16) -> Self {
        let expected_interval_us = match preferred_rate_hz {
            POLL_RATE_2000HZ => 500,
            POLL_RATE_1000HZ => 1000,
            POLL_RATE_500HZ => 2000,
            _ => 4000,
        };

        Self {
            aggregator: HighPollRateAggregator::new(preferred_rate_hz),
            jitter_buffer: InputJitterBuffer::new(expected_interval_us),
            scaler: PowerAwarePollingScaler::new(preferred_rate_hz),
            total_dispatched_to_raw_input: 0,
            device_handle: 0xDEAD_BEEF_0001,
        }
    }

    /// Evaluates system power state and dynamically adjusts aggregator and jitter buffer.
    pub fn update_power_policy(&mut self, battery_soc_pct: u8, whisper_mode: bool, ac_connected: bool) {
        let effective_rate = self.scaler.evaluate_rate(battery_soc_pct, whisper_mode, ac_connected);
        if effective_rate != self.aggregator.poll_rate_hz {
            self.aggregator.set_poll_rate(effective_rate);
            self.jitter_buffer.set_expected_interval(self.aggregator.expected_interval_us);
        }
    }

    /// Ingests raw hardware controller event at sub-millisecond precision.
    pub fn ingest_sample(
        &mut self,
        timestamp_us: u64,
        buttons: u16,
        left_stick: (i16, i16),
        right_stick: (i16, i16),
        triggers: (u16, u16),
        left_trackpad: (i16, i16, u8, bool),
        right_trackpad: (i16, i16, u8, bool),
    ) {
        let packet = self.aggregator.aggregate_sample(
            timestamp_us,
            buttons,
            left_stick,
            right_stick,
            triggers,
            left_trackpad,
            right_trackpad,
        );
        self.jitter_buffer.push(packet);
    }

    /// Dispatches the next smoothed packet formatted for Win32 Raw Input API.
    pub fn dispatch_to_raw_input(&mut self) -> Option<Win32RawInputPacket> {
        let pkt = self.jitter_buffer.pop()?;
        self.total_dispatched_to_raw_input += 1;
        Some(Win32RawInputPacket {
            header_type: RIM_TYPEHID,
            device_handle: self.device_handle,
            wparam: 0,
            payload: pkt,
        })
    }
}

/// Unit test verifying integrated HID input pacer dispatching to Win32 Raw Input.
pub fn verify_input_pacer_manager() -> bool {
    let mut pacer = InputPacerManager::new(POLL_RATE_1000HZ);

    // 1. Ingest 5 packets at 1000Hz (1ms spacing)
    for i in 1..=5 {
        pacer.ingest_sample(
            (i * 1000) as u64,
            BTN_A | BTN_X,
            (1000 * i as i16, -1000 * i as i16),
            (0, 0),
            (250 * i as u16, 500),
            (10, 5, 100, true),
            (0, 0, 0, false),
        );
    }

    if pacer.jitter_buffer.len() != 5 {
        return false;
    }

    // 2. Dispatch all 5 to Win32 Raw Input
    for i in 1..=5 {
        let raw = match pacer.dispatch_to_raw_input() {
            Some(r) => r,
            None => return false,
        };
        if raw.header_type != RIM_TYPEHID || raw.payload.sequence != i {
            return false;
        }
        if raw.payload.left_stick_x != (1000 * i as i16) {
            return false;
        }
    }

    if !pacer.jitter_buffer.is_empty() || pacer.total_dispatched_to_raw_input != 5 {
        return false;
    }

    // 3. Power downscaling to 500Hz on low battery (15% SoC on battery)
    pacer.update_power_policy(15, false, false);
    if pacer.aggregator.poll_rate_hz != POLL_RATE_500HZ || pacer.jitter_buffer.expected_interval_us != 2000 {
        return false;
    }

    true
}
