//! Handheld Wi-Fi 6E/7 Low-Latency Streaming & Packet Pacer.

#![allow(dead_code)]

/// Wi-Fi Multimedia (WMM) Quality of Service (QoS) Access Categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WmmAccessCategory {
    /// Background (AC_BK): bulk downloads, cloud sync, low priority.
    AcBk = 0,
    /// Best Effort (AC_BE): general game state, telemetry, matchmaking.
    AcBe = 1,
    /// Video (AC_VI): real-time display/video streaming slices.
    AcVi = 2,
    /// Voice (AC_VO): controller input packets, sub-2ms latency deadline.
    AcVo = 3,
}

impl WmmAccessCategory {
    /// Returns target maximum queuing latency in microseconds.
    pub const fn target_latency_us(self) -> u32 {
        match self {
            Self::AcVo => 2_000,   // 2ms
            Self::AcVi => 8_000,   // 8ms
            Self::AcBe => 25_000,  // 25ms
            Self::AcBk => 100_000, // 100ms
        }
    }
}

/// Packet traffic classifier inspects UDP ports and payload headers to assign WMM AC.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WmmTrafficClassifier {
    pub gameplay_active: bool,
}

impl WmmTrafficClassifier {
    pub const fn new() -> Self {
        Self { gameplay_active: true }
    }

    /// Classifies an outgoing packet based on UDP destination port and packet type tag.
    pub fn classify_packet(&self, dest_port: u16, is_input_packet: bool, is_video_slice: bool) -> WmmAccessCategory {
        if is_input_packet || dest_port == 47999 || dest_port == 48000 {
            WmmAccessCategory::AcVo
        } else if is_video_slice || dest_port == 47998 || dest_port == 48002 {
            WmmAccessCategory::AcVi
        } else if dest_port >= 27015 && dest_port <= 27050 {
            WmmAccessCategory::AcBe
        } else {
            WmmAccessCategory::AcBk
        }
    }
}

/// Maximum queue capacity per WMM access category.
pub const WMM_QUEUE_CAPACITY: usize = 32;

/// Outgoing packet descriptor stored in WMM priority queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuedPacket {
    pub packet_id: u32,
    pub ac: WmmAccessCategory,
    pub length: u16,
    pub enqueue_time_us: u64,
}

impl QueuedPacket {
    pub const fn empty() -> Self {
        Self {
            packet_id: 0,
            ac: WmmAccessCategory::AcBk,
            length: 0,
            enqueue_time_us: 0,
        }
    }
}

/// Multi-queue strict priority scheduler for WMM access categories.
#[derive(Debug, Clone)]
pub struct WmmPriorityScheduler {
    pub classifier: WmmTrafficClassifier,
    pub queues: [[QueuedPacket; WMM_QUEUE_CAPACITY]; 4],
    pub queue_lens: [usize; 4],
    pub total_enqueued: u64,
    pub total_dequeued: u64,
}

impl WmmPriorityScheduler {
    pub const fn new() -> Self {
        Self {
            classifier: WmmTrafficClassifier::new(),
            queues: [[QueuedPacket::empty(); WMM_QUEUE_CAPACITY]; 4],
            queue_lens: [0; 4],
            total_enqueued: 0,
            total_dequeued: 0,
        }
    }

    /// Enqueues a packet into its corresponding WMM access category queue.
    pub fn enqueue(&mut self, packet_id: u32, ac: WmmAccessCategory, length: u16, enqueue_time_us: u64) -> bool {
        let idx = ac as usize;
        if self.queue_lens[idx] >= WMM_QUEUE_CAPACITY {
            return false;
        }

        let slot = self.queue_lens[idx];
        self.queues[idx][slot] = QueuedPacket {
            packet_id,
            ac,
            length,
            enqueue_time_us,
        };
        self.queue_lens[idx] += 1;
        self.total_enqueued += 1;
        true
    }

    /// Dequeues the highest priority available packet (AcVo > AcVi > AcBe > AcBk).
    pub fn dequeue_highest_priority(&mut self) -> Option<QueuedPacket> {
        // Strict priority: check AcVo (3) down to AcBk (0)
        for idx in (0..4).rev() {
            if self.queue_lens[idx] > 0 {
                let pkt = self.queues[idx][0];
                // Shift queue forward
                for i in 1..self.queue_lens[idx] {
                    self.queues[idx][i - 1] = self.queues[idx][i];
                }
                self.queue_lens[idx] -= 1;
                self.total_dequeued += 1;
                return Some(pkt);
            }
        }
        None
    }
}

/// Unit test verifying WMM QoS traffic classifier and priority scheduling order.
pub fn verify_wmm_qos_classifier() -> bool {
    let mut scheduler = WmmPriorityScheduler::new();

    // 1. Classification check
    let ac_vo = scheduler.classifier.classify_packet(47999, true, false);
    if ac_vo != WmmAccessCategory::AcVo {
        return false;
    }
    let ac_vi = scheduler.classifier.classify_packet(47998, false, true);
    if ac_vi != WmmAccessCategory::AcVi {
        return false;
    }
    let ac_be = scheduler.classifier.classify_packet(27015, false, false);
    if ac_be != WmmAccessCategory::AcBe {
        return false;
    }
    let ac_bk = scheduler.classifier.classify_packet(80, false, false);
    if ac_bk != WmmAccessCategory::AcBk {
        return false;
    }

    // 2. Priority queue ordering: Enqueue in reverse order (AcBk, AcBe, AcVi, AcVo)
    if !scheduler.enqueue(1, WmmAccessCategory::AcBk, 1500, 1000) { return false; }
    if !scheduler.enqueue(2, WmmAccessCategory::AcBe, 512, 1001) { return false; }
    if !scheduler.enqueue(3, WmmAccessCategory::AcVi, 1400, 1002) { return false; }
    if !scheduler.enqueue(4, WmmAccessCategory::AcVo, 64, 1003) { return false; }

    // Dequeues must arrive strictly in priority order: 4 (AcVo) -> 3 (AcVi) -> 2 (AcBe) -> 1 (AcBk)
    let p1 = scheduler.dequeue_highest_priority().expect("p1 missing");
    if p1.packet_id != 4 || p1.ac != WmmAccessCategory::AcVo {
        return false;
    }
    let p2 = scheduler.dequeue_highest_priority().expect("p2 missing");
    if p2.packet_id != 3 || p2.ac != WmmAccessCategory::AcVi {
        return false;
    }
    let p3 = scheduler.dequeue_highest_priority().expect("p3 missing");
    if p3.packet_id != 2 || p3.ac != WmmAccessCategory::AcBe {
        return false;
    }
    let p4 = scheduler.dequeue_highest_priority().expect("p4 missing");
    if p4.packet_id != 1 || p4.ac != WmmAccessCategory::AcBk {
        return false;
    }

    // Queue empty
    if scheduler.dequeue_highest_priority().is_some() {
        return false;
    }

    true
}



// Active Queue Management (AQM) Bufferbloat Mitigation & Packet Pacer.

/// AQM state and sojourn latency regulator configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AqmPacketPacer {
    /// Target maximum sojourn latency in microseconds (e.g. 5,000 us = 5ms).
    pub target_sojourn_us: u32,
    /// Minimum interval before dropping/throttling in microseconds (e.g. 30,000 us = 30ms).
    pub interval_us: u32,
    /// Timestamp when latency first exceeded target in microseconds.
    pub first_above_time_us: u64,
    /// Whether AQM is currently in dropping/throttling state.
    pub is_dropping: bool,
    /// Total packets dropped by AQM bufferbloat mitigation.
    pub total_aqm_drops: u32,
    /// Total bytes paced.
    pub total_bytes_paced: u64,
}

impl AqmPacketPacer {
    pub const fn new(target_sojourn_us: u32, interval_us: u32) -> Self {
        Self {
            target_sojourn_us,
            interval_us,
            first_above_time_us: 0,
            is_dropping: false,
            total_aqm_drops: 0,
            total_bytes_paced: 0,
        }
    }

    /// Default configuration tuned for sub-5ms handheld game streaming.
    pub const fn default_gaming() -> Self {
        Self::new(5_000, 30_000)
    }

    /// Evaluates sojourn time of a dequeued packet and determines whether to transmit or drop.
    /// Critical gaming packets (AcVo, AcVi) are never dropped by AQM.
    pub fn evaluate_packet(&mut self, pkt: &QueuedPacket, now_us: u64) -> bool {
        let sojourn_us = if now_us >= pkt.enqueue_time_us {
            (now_us - pkt.enqueue_time_us) as u32
        } else {
            0
        };

        // Critical interactive packets are never dropped
        if pkt.ac == WmmAccessCategory::AcVo || pkt.ac == WmmAccessCategory::AcVi {
            self.total_bytes_paced += pkt.length as u64;
            return true;
        }

        // Background / best effort traffic evaluated against sojourn threshold
        if sojourn_us > self.target_sojourn_us {
            if self.first_above_time_us == 0 {
                self.first_above_time_us = now_us;
            } else if (now_us - self.first_above_time_us) >= (self.interval_us as u64) {
                self.is_dropping = true;
            }
        } else {
            self.first_above_time_us = 0;
            self.is_dropping = false;
        }

        if self.is_dropping {
            self.total_aqm_drops += 1;
            false // Drop bulk packet to eliminate queue bloat
        } else {
            self.total_bytes_paced += pkt.length as u64;
            true
        }
    }
}

/// Unit test verifying AQM bufferbloat mitigation and queue latency clamping.
pub fn verify_aqm_bufferbloat_mitigation() -> bool {
    let mut pacer = AqmPacketPacer::default_gaming();

    // 1. Normal packet (sojourn 1ms <= 5ms target) -> accepted
    let p_normal = QueuedPacket {
        packet_id: 1,
        ac: WmmAccessCategory::AcBk,
        length: 1500,
        enqueue_time_us: 10_000,
    };
    if !pacer.evaluate_packet(&p_normal, 11_000) || pacer.total_aqm_drops != 0 {
        return false;
    }

    // 2. High sojourn packet during initial transient (sojourn 8ms > 5ms, but dt < 30ms interval) -> accepted
    let p_bloat1 = QueuedPacket {
        packet_id: 2,
        ac: WmmAccessCategory::AcBk,
        length: 1500,
        enqueue_time_us: 10_000,
    };
    if !pacer.evaluate_packet(&p_bloat1, 18_000) || pacer.is_dropping {
        return false;
    }

    // 3. Sustained bloat exceeding interval (> 30ms later, sojourn 10ms) -> dropped
    let p_bloat2 = QueuedPacket {
        packet_id: 3,
        ac: WmmAccessCategory::AcBk,
        length: 1500,
        enqueue_time_us: 50_000,
    };
    if pacer.evaluate_packet(&p_bloat2, 60_000) || !pacer.is_dropping || pacer.total_aqm_drops != 1 {
        return false;
    }

    // 4. Critical controller input (AcVo) with high sojourn -> MUST NEVER BE DROPPED
    let p_input = QueuedPacket {
        packet_id: 4,
        ac: WmmAccessCategory::AcVo,
        length: 64,
        enqueue_time_us: 50_000,
    };
    if !pacer.evaluate_packet(&p_input, 65_000) {
        return false;
    }

    true
}
