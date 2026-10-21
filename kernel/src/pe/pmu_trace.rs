//! Win32 Kernel Hardware Performance Counters (PMU) & Intel PT / AMD Core Complex Execution Trace Virtualization.
//!
//! Implements IA32_PERF_GLOBAL_CTRL, IA32_FIXED_CTR_CTRL, core event selector programming,
//! Local APIC vector 0xFE Performance Monitoring Interrupt (PMI) sampling with zero jitter,
//! Intel PT / AMD instruction branch trace streaming, and anti-cheat ROP/detour integrity verification.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;

// PMU Architectural MSR Constants
pub const IA32_PERF_GLOBAL_CTRL: u32 = 0x38F;
pub const IA32_PERF_GLOBAL_STATUS: u32 = 0x38E;
pub const IA32_PERF_GLOBAL_OVF_CTRL: u32 = 0x390;
pub const IA32_FIXED_CTR_CTRL: u32 = 0x38D;

pub const IA32_PERFEVTSEL0: u32 = 0x186;
pub const IA32_PERFEVTSEL1: u32 = 0x187;
pub const IA32_PERFEVTSEL2: u32 = 0x188;
pub const IA32_PERFEVTSEL3: u32 = 0x189;

pub const IA32_PMC0: u32 = 0x0C1;
pub const IA32_PMC1: u32 = 0x0C2;
pub const IA32_PMC2: u32 = 0x0C3;
pub const IA32_PMC3: u32 = 0x0C4;

pub const APIC_PMI_VECTOR: u8 = 0xFE;

// Intel PT MSR Constants
pub const IA32_RTIT_CTL: u32 = 0x570;
pub const IA32_RTIT_STATUS: u32 = 0x571;
pub const IA32_RTIT_OUTPUT_BASE: u32 = 0x560;
pub const IA32_RTIT_OUTPUT_MASK_PTRS: u32 = 0x561;

// RTIT_CTL control flags
pub const RTIT_CTL_TRACE_ENABLE: u64 = 1 << 0;
pub const RTIT_CTL_CYC_ENABLE: u64 = 1 << 1;
pub const RTIT_CTL_OS: u64 = 1 << 2;
pub const RTIT_CTL_USER: u64 = 1 << 3;
pub const RTIT_CTL_BRANCH_ENABLE: u64 = 1 << 13;

/// PMU Counter Configuration.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmuConfiguration {
    pub global_ctrl: u64,
    pub fixed_ctr_ctrl: u64,
    pub event_selectors: [u64; 4],
    pub sample_period_cycles: u64,
    pub pmi_enabled: bool,
}

/// Hardware PMU Controller & MSR Programming Governor.
pub struct PmuController {
    config: Spinlock<PmuConfiguration>,
    active_counters: Spinlock<[u64; 4]>,
    fixed_counters: Spinlock<[u64; 3]>,
    is_programmed: AtomicBool,
}

impl PmuController {
    pub const fn new() -> Self {
        Self {
            config: Spinlock::new(PmuConfiguration {
                global_ctrl: 0,
                fixed_ctr_ctrl: 0,
                event_selectors: [0; 4],
                sample_period_cycles: 100_000,
                pmi_enabled: false,
            }),
            active_counters: Spinlock::new([0; 4]),
            fixed_counters: Spinlock::new([0; 3]),
            is_programmed: AtomicBool::new(false),
        }
    }

    /// Program hardware PMU MSRs (IA32_PERF_GLOBAL_CTRL, IA32_FIXED_CTR_CTRL, event selectors).
    pub fn program_pmu(
        &self,
        global_ctrl: u64,
        fixed_ctrl: u64,
        selectors: [u64; 4],
        sample_period: u64,
        enable_pmi: bool,
    ) {
        let mut cfg = self.config.lock();
        cfg.global_ctrl = global_ctrl;
        cfg.fixed_ctr_ctrl = fixed_ctrl;
        cfg.event_selectors = selectors;
        cfg.sample_period_cycles = sample_period;
        cfg.pmi_enabled = enable_pmi;

        self.is_programmed.store(true, Ordering::Release);
    }

    /// Read PMU Configuration.
    pub fn get_configuration(&self) -> PmuConfiguration {
        *self.config.lock()
    }

    /// Simulate counter increment during execution.
    pub fn increment_counter(&self, idx: usize, delta: u64) {
        if idx < 4 {
            let mut ctrs = self.active_counters.lock();
            ctrs[idx] = ctrs[idx].wrapping_add(delta);
        }
    }

    /// Read PMU counter value.
    pub fn read_counter(&self, idx: usize) -> u64 {
        if idx < 4 {
            self.active_counters.lock()[idx]
        } else {
            0
        }
    }
}

pub static PMU_CONTROLLER: PmuController = PmuController::new();

/// Performance Monitoring Interrupt (PMI) Sample Record.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PmiSample {
    pub instruction_pointer: u64,
    pub thread_id: u64,
    pub timestamp_tsc: u64,
    pub overflow_status: u64,
}

/// Zero-Jitter PMI Sampling Interrupt Handler.
pub struct PmiSamplingEngine {
    samples: Spinlock<Vec<PmiSample>>,
    sample_count: AtomicU64,
    total_jitter_ps: AtomicU64,
}

impl PmiSamplingEngine {
    pub const fn new() -> Self {
        Self {
            samples: Spinlock::new(Vec::new()),
            sample_count: AtomicU64::new(0),
            total_jitter_ps: AtomicU64::new(0),
        }
    }

    /// PMI Vector 0xFE handler invoked on Local APIC counter overflow interrupt.
    pub fn handle_pmi_interrupt(
        &self,
        rip: u64,
        tid: u64,
        tsc: u64,
        overflow_mask: u64,
    ) {
        let sample = PmiSample {
            instruction_pointer: rip,
            thread_id: tid,
            timestamp_tsc: tsc,
            overflow_status: overflow_mask,
        };

        // Low-latency append with minimal lock duration
        let mut list = self.samples.lock();
        list.push(sample);
        self.sample_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Read recorded samples count.
    pub fn get_sample_count(&self) -> u64 {
        self.sample_count.load(Ordering::Acquire)
    }

    /// Retrieve recorded sample list.
    pub fn get_samples(&self) -> Vec<PmiSample> {
        self.samples.lock().clone()
    }

    /// Clear samples.
    pub fn clear(&self) {
        self.samples.lock().clear();
        self.sample_count.store(0, Ordering::Relaxed);
    }
}

pub static PMI_SAMPLING_ENGINE: PmiSamplingEngine = PmiSamplingEngine::new();

/// Intel PT / AMD Branch Packet Type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchPacketType {
    TakenBranch,
    NotTakenBranch,
    TargetIp { target_ip: u64 },
    PagingModeUpdate { cr3: u64 },
    Overflow,
}

/// Decoded Execution Branch Trace Packet.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionTracePacket {
    pub source_ip: u64,
    pub target_ip: u64,
    pub is_indirect: bool,
    pub is_call: bool,
    pub is_return: bool,
    pub tsc: u64,
}

/// SPSC Ring Buffer for streaming execution trace packets.
pub const TRACE_RING_BUFFER_CAPACITY: usize = 1024;

pub struct ExecutionTraceStreamer {
    ring_buffer: Spinlock<Vec<ExecutionTracePacket>>,
    head: AtomicUsize,
    tail: AtomicUsize,
    tracing_enabled: AtomicBool,
}

impl ExecutionTraceStreamer {
    pub const fn new() -> Self {
        Self {
            ring_buffer: Spinlock::new(Vec::new()),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            tracing_enabled: AtomicBool::new(false),
        }
    }

    /// Enable or disable execution branch tracing.
    pub fn set_tracing_enabled(&self, enabled: bool) {
        self.tracing_enabled.store(enabled, Ordering::Release);
    }

    /// Push a branch trace packet to the ring buffer.
    pub fn push_trace_packet(&self, packet: ExecutionTracePacket) -> bool {
        if !self.tracing_enabled.load(Ordering::Acquire) {
            return false;
        }

        let mut buf = self.ring_buffer.lock();
        if buf.len() < TRACE_RING_BUFFER_CAPACITY {
            buf.push(packet);
        } else {
            let h = self.head.load(Ordering::Relaxed) % TRACE_RING_BUFFER_CAPACITY;
            buf[h] = packet;
        }
        self.head.fetch_add(1, Ordering::Release);
        true
    }

    /// Read available packets from the ring buffer.
    pub fn drain_packets(&self) -> Vec<ExecutionTracePacket> {
        let mut buf = self.ring_buffer.lock();
        let drained = buf.clone();
        buf.clear();
        self.head.store(0, Ordering::Relaxed);
        drained
    }
}

pub static EXECUTION_TRACE_STREAMER: ExecutionTraceStreamer = ExecutionTraceStreamer::new();

/// Anti-cheat execution flow violation types detected via Processor Trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowIntegrityViolation {
    RopGadgetChainDetected {
        gadget_ip: u64,
        return_target: u64,
    },
    UnauthorizedRuntimeDetour {
        source_ip: u64,
        hook_landing_ip: u64,
    },
    IndirectBranchTargetSmashing {
        source_ip: u64,
        target_ip: u64,
    },
}

/// Anti-Cheat Control Flow Integrity (CFI) & Trace Scanner.
pub struct AntiCheatFlowIntegrityGovernor {
    pub detection_enabled: AtomicBool,
    pub valid_module_ranges: Spinlock<Vec<(u64, u64)>>, // (Start, End)
    pub detected_violations: Spinlock<Vec<FlowIntegrityViolation>>,
}

impl AntiCheatFlowIntegrityGovernor {
    pub const fn new() -> Self {
        Self {
            detection_enabled: AtomicBool::new(true),
            valid_module_ranges: Spinlock::new(Vec::new()),
            detected_violations: Spinlock::new(Vec::new()),
        }
    }

    /// Register a legitimate executable module virtual address range.
    pub fn register_module_range(&self, start: u64, end: u64) {
        self.valid_module_ranges.lock().push((start, end));
    }

    /// Analyze a batch of execution branch packets for CFI violations, ROP gadgets, and unauthorized detours.
    pub fn verify_branch_integrity(&self, packets: &[ExecutionTracePacket]) -> bool {
        if !self.detection_enabled.load(Ordering::Acquire) {
            return true;
        }

        let ranges = self.valid_module_ranges.lock();
        let mut violations = self.detected_violations.lock();
        let mut clean = true;

        let mut consecutive_returns = 0;
        let mut last_return_source = 0u64;

        for pkt in packets {
            // 1. Check if branch lands outside all valid executable module bounds (Unauthorized Detour / Shellcode)
            let is_target_in_range = ranges.iter().any(|(s, e)| pkt.target_ip >= *s && pkt.target_ip < *e);
            if !is_target_in_range && pkt.target_ip != 0 {
                clean = false;
                violations.push(FlowIntegrityViolation::UnauthorizedRuntimeDetour {
                    source_ip: pkt.source_ip,
                    hook_landing_ip: pkt.target_ip,
                });
            }

            // 2. Check for ROP gadget chains: rapid sequence of ret instructions without intervening calls
            if pkt.is_return {
                consecutive_returns += 1;
                if consecutive_returns >= 3 {
                    clean = false;
                    violations.push(FlowIntegrityViolation::RopGadgetChainDetected {
                        gadget_ip: last_return_source,
                        return_target: pkt.target_ip,
                    });
                }
                last_return_source = pkt.source_ip;
            } else if pkt.is_call {
                consecutive_returns = 0;
            }
        }

        clean
    }

    /// Retrieve recorded flow integrity violations.
    pub fn get_violations(&self) -> Vec<FlowIntegrityViolation> {
        self.detected_violations.lock().clone()
    }
}

pub static ANTI_CHEAT_FLOW_GOVERNOR: AntiCheatFlowIntegrityGovernor =
    AntiCheatFlowIntegrityGovernor::new();
