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
