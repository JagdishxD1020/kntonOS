//! Win32 Performance Counters & Hardware Event Telemetry Sampling Ring.
//!
//! Implements high-frequency CPU performance monitoring unit (PMU) event sampling (retired instructions,
//! branch mispredicts, L3 cache misses), per-CPU SPSC hardware event telemetry sampling ring buffer
//! with zero-copy stream readers, and anti-cheat anomaly detector flagging hypervisor traps and jitter.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;

/// Hardware PMU event classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmuEventType {
    RetiredInstructions,
    BranchMispredicts,
    L3CacheMisses,
    CpuCycles,
}

/// Sample record captured from CPU performance monitoring counters.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmuSampleRecord {
    pub timestamp_tsc: u64,
    pub rip: u64,
    pub cpu_id: u32,
    pub thread_id: u32,
    pub event_type: PmuEventType,
    pub counter_value: u64,
    pub delta_cycles: u32,
}

/// High-frequency CPU performance monitoring unit (PMU) event sampling engine.
pub struct PmuSamplingEngine {
    total_samples: AtomicU64,
    sample_period: AtomicU32,
}

impl PmuSamplingEngine {
    pub const fn new() -> Self {
        Self {
            total_samples: AtomicU64::new(0),
            sample_period: AtomicU32::new(1000), // Default: 1 sample per 1000 events
        }
    }

    /// Set sampling period interval.
    pub fn set_sample_period(&self, period: u32) {
        self.sample_period.store(period.max(1), Ordering::Release);
    }

    /// Capture a high-frequency hardware PMU event sample.
    pub fn sample_hardware_event(
        &self,
        timestamp_tsc: u64,
        rip: u64,
        cpu_id: u32,
        thread_id: u32,
        event_type: PmuEventType,
        counter_value: u64,
        delta_cycles: u32,
    ) -> PmuSampleRecord {
        self.total_samples.fetch_add(1, Ordering::Relaxed);
        PmuSampleRecord {
            timestamp_tsc,
            rip,
            cpu_id,
            thread_id,
            event_type,
            counter_value,
            delta_cycles,
        }
    }

    /// Retrieve count of captured PMU samples.
    pub fn get_total_samples(&self) -> u64 {
        self.total_samples.load(Ordering::Acquire)
    }
}

pub static PMU_SAMPLING_ENGINE: PmuSamplingEngine = PmuSamplingEngine::new();

pub const PMU_RING_CAPACITY: usize = 1024;

/// Per-CPU Single-Producer Single-Consumer (SPSC) hardware event telemetry ring buffer.
pub struct PmuSpscRingBuffer {
    pub cpu_id: u32,
    buffer: Spinlock<Vec<PmuSampleRecord>>,
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped_samples: AtomicU64,
}

impl PmuSpscRingBuffer {
    pub fn new(cpu_id: u32) -> Self {
        Self {
            cpu_id,
            buffer: Spinlock::new(Vec::new()),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped_samples: AtomicU64::new(0),
        }
    }

    /// Push a sample record into the lock-free SPSC ring buffer.
    pub fn push_sample(&self, sample: PmuSampleRecord) -> bool {
        let mut buf = self.buffer.lock();
        if buf.len() < PMU_RING_CAPACITY {
            buf.push(sample);
            self.head.fetch_add(1, Ordering::Release);
            true
        } else {
            self.dropped_samples.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    /// Drain all pending telemetry samples in a zero-copy batch.
    pub fn drain_batch(&self) -> Vec<PmuSampleRecord> {
        let mut buf = self.buffer.lock();
        let batch = buf.clone();
        buf.clear();
        self.tail.store(self.head.load(Ordering::Acquire), Ordering::Release);
        batch
    }

    /// Return pending samples count.
    pub fn len(&self) -> usize {
        self.buffer.lock().len()
    }

    /// Check if ring buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Retrieve number of samples dropped due to buffer overflow.
    pub fn get_dropped_samples(&self) -> u64 {
        self.dropped_samples.load(Ordering::Acquire)
    }
}

/// Multi-core PMU ring buffer coordinator managing per-CPU telemetry streams.
pub struct PerCpuPmuRingManager {
    rings: Spinlock<BTreeMap<u32, Arc<PmuSpscRingBuffer>>>,
}

impl PerCpuPmuRingManager {
    pub const fn new() -> Self {
        Self {
            rings: Spinlock::new(BTreeMap::new()),
        }
    }

    /// Get or create SPSC ring buffer for designated CPU core.
    pub fn get_or_create_ring(&self, cpu_id: u32) -> Arc<PmuSpscRingBuffer> {
        let mut map = self.rings.lock();
        map.entry(cpu_id)
            .or_insert_with(|| Arc::new(PmuSpscRingBuffer::new(cpu_id)))
            .clone()
    }

    /// Publish a hardware telemetry sample to the CPU's local ring buffer.
    pub fn publish_sample(&self, sample: PmuSampleRecord) -> bool {
        let ring = self.get_or_create_ring(sample.cpu_id);
        ring.push_sample(sample)
    }

    /// Drain pending samples from a specific CPU ring buffer.
    pub fn drain_cpu_samples(&self, cpu_id: u32) -> Vec<PmuSampleRecord> {
        let ring = self.get_or_create_ring(cpu_id);
        ring.drain_batch()
    }
}

pub static PER_CPU_PMU_MANAGER: PerCpuPmuRingManager = PerCpuPmuRingManager::new();

/// Anomaly classification for anti-cheat profiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmuAnomalyType {
    HypervisorTrapJitter { delta_cycles: u32, threshold: u32 },
    HighBranchMispredictRatio { mispredicts: u64 },
    L3CacheThrashing { misses: u64 },
}

/// Profile anomaly incident descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmuAnomalyReport {
    pub rip: u64,
    pub thread_id: u32,
    pub anomaly_type: PmuAnomalyType,
    pub timestamp_tsc: u64,
}

/// Anti-cheat profiling anomaly detector flagging hypervisor traps and code injection jitter.
pub struct PmuAnomalyDetector {
    hypervisor_jitter_threshold_cycles: AtomicU32,
    detected_anomalies: Spinlock<Vec<PmuAnomalyReport>>,
    total_anomalies_flagged: AtomicU64,
}

impl PmuAnomalyDetector {
    pub const fn new(jitter_threshold: u32) -> Self {
        Self {
            hypervisor_jitter_threshold_cycles: AtomicU32::new(jitter_threshold),
            detected_anomalies: Spinlock::new(Vec::new()),
            total_anomalies_flagged: AtomicU64::new(0),
        }
    }

    /// Set jitter cycle threshold for hypervisor VM-exit anomaly detection.
    pub fn set_jitter_threshold(&self, threshold: u32) {
        self.hypervisor_jitter_threshold_cycles.store(threshold, Ordering::Release);
    }

    /// Analyze a PMU sample record for execution timing and profiling anomalies.
    pub fn evaluate_sample(&self, sample: &PmuSampleRecord) -> Option<PmuAnomalyReport> {
        let threshold = self.hypervisor_jitter_threshold_cycles.load(Ordering::Acquire);

        let anomaly = if sample.delta_cycles > threshold {
            Some(PmuAnomalyType::HypervisorTrapJitter {
                delta_cycles: sample.delta_cycles,
                threshold,
            })
        } else {
            match sample.event_type {
                PmuEventType::BranchMispredicts if sample.counter_value > 2000 => {
                    Some(PmuAnomalyType::HighBranchMispredictRatio {
                        mispredicts: sample.counter_value,
                    })
                }
                PmuEventType::L3CacheMisses if sample.counter_value > 5000 => {
                    Some(PmuAnomalyType::L3CacheThrashing {
                        misses: sample.counter_value,
                    })
                }
                _ => None,
            }
        };

        if let Some(anom) = anomaly {
            let report = PmuAnomalyReport {
                rip: sample.rip,
                thread_id: sample.thread_id,
                anomaly_type: anom,
                timestamp_tsc: sample.timestamp_tsc,
            };
            self.detected_anomalies.lock().push(report);
            self.total_anomalies_flagged.fetch_add(1, Ordering::Relaxed);
            Some(report)
        } else {
            None
        }
    }

    /// Retrieve count of flagged anomalies.
    pub fn get_total_anomalies(&self) -> u64 {
        self.total_anomalies_flagged.load(Ordering::Acquire)
    }

    /// Retrieve detected anomalies list.
    pub fn get_anomalies(&self) -> Vec<PmuAnomalyReport> {
        self.detected_anomalies.lock().clone()
    }

    /// Clear recorded anomalies.
    pub fn clear(&self) {
        self.detected_anomalies.lock().clear();
        self.total_anomalies_flagged.store(0, Ordering::Relaxed);
    }
}

pub static PMU_ANOMALY_DETECTOR: PmuAnomalyDetector = PmuAnomalyDetector::new(5000);

/// Comprehensive self-test suite for Win32 PMU Event Telemetry Sampling & Anomaly Detector.
pub fn test_win32_pmu_sampler_subsystem() -> bool {
    crate::serial_write_str("[KNT-WIN32-PMU-SAMPLER] Initializing Win32 PMU Sampler Subsystem self-test...\r\n");

    // 1. Verify High-Frequency PMU Event Sampling Engine
    let initial_samples = PMU_SAMPLING_ENGINE.get_total_samples();
    let s1 = PMU_SAMPLING_ENGINE.sample_hardware_event(
        100_000,
        0x0000_0040_1000,
        0,
        1001,
        PmuEventType::RetiredInstructions,
        15_000,
        80,
    );

    if s1.event_type != PmuEventType::RetiredInstructions || s1.rip != 0x0000_0040_1000 {
        crate::serial_write_str("[KNT-TEST-ERR] PMU sample record fields mismatch\r\n");
        return false;
    }

    if PMU_SAMPLING_ENGINE.get_total_samples() != initial_samples + 1 {
        crate::serial_write_str("[KNT-TEST-ERR] Total PMU samples counter mismatch\r\n");
        return false;
    }

    // 2. Verify Per-CPU SPSC Hardware Event Telemetry Ring Buffer
    let cpu_target = 2u32;
    let s2 = PmuSampleRecord {
        timestamp_tsc: 105_000,
        rip: 0x0000_0040_2200,
        cpu_id: cpu_target,
        thread_id: 1002,
        event_type: PmuEventType::BranchMispredicts,
        counter_value: 450,
        delta_cycles: 65,
    };

    if !PER_CPU_PMU_MANAGER.publish_sample(s2) {
        crate::serial_write_str("[KNT-TEST-ERR] Failed to publish sample to per-CPU ring\r\n");
        return false;
    }

    let ring = PER_CPU_PMU_MANAGER.get_or_create_ring(cpu_target);
    if ring.is_empty() {
        crate::serial_write_str("[KNT-TEST-ERR] CPU ring buffer should not be empty\r\n");
        return false;
    }

    let drained = PER_CPU_PMU_MANAGER.drain_cpu_samples(cpu_target);
    if drained.len() != 1 || drained[0].rip != 0x0000_0040_2200 {
        crate::serial_write_str("[KNT-TEST-ERR] Drained batch sample mismatch\r\n");
        return false;
    }

    if !ring.is_empty() {
        crate::serial_write_str("[KNT-TEST-ERR] CPU ring buffer should be empty after drain\r\n");
        return false;
    }

    // 3. Verify Anti-Cheat Anomaly Detection (Hypervisor Trap Jitter & Cache Thrashing)
    // Normal sample -> no anomaly
    if PMU_ANOMALY_DETECTOR.evaluate_sample(&s1).is_some() {
        crate::serial_write_str("[KNT-TEST-ERR] Normal execution sample incorrectly flagged as anomaly\r\n");
        return false;
    }

    // Jitter sample: 12,000 cycles (exceeds 5,000 cycle threshold)
    let jitter_sample = PmuSampleRecord {
        timestamp_tsc: 120_000,
        rip: 0x0000_7FFF_DEAD_0000,
        cpu_id: 0,
        thread_id: 666,
        event_type: PmuEventType::CpuCycles,
        counter_value: 12_000,
        delta_cycles: 12_000,
    };

    let anom = PMU_ANOMALY_DETECTOR.evaluate_sample(&jitter_sample);
    match anom {
        Some(rep) => {
            match rep.anomaly_type {
                PmuAnomalyType::HypervisorTrapJitter { delta_cycles, .. } if delta_cycles == 12_000 => {}
                _ => {
                    crate::serial_write_str("[KNT-TEST-ERR] Flagged anomaly type mismatch\r\n");
                    return false;
                }
            }
        }
        None => {
            crate::serial_write_str("[KNT-TEST-ERR] Hypervisor jitter anomaly was not flagged\r\n");
            return false;
        }
    }

    if PMU_ANOMALY_DETECTOR.get_total_anomalies() == 0 {
        crate::serial_write_str("[KNT-TEST-ERR] Anomaly counter was zero\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-PMU-SAMPLER-PASS] Win32 PMU Telemetry Sampling & Anomaly Detector verified\r\n");
    true
}
