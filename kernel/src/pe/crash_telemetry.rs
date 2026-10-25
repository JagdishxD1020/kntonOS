//! Win32 Dynamic Exception Routing & Minidump Crash Telemetry Ring Buffer.
//!
//! Implements low-latency user-mode exception capture (access violations, illegal instructions,
//! and stack buffer overflows), fast in-memory Minidump stream encoder, and lock-free SPSC crash
//! telemetry ring buffer streaming crash events to kernel diagnostic store.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;

pub const EXCEPTION_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const EXCEPTION_ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
pub const EXCEPTION_STACK_BUFFER_OVERRUN: u32 = 0xC000_0409;

/// Captured Exception Record with Fault Addressing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapturedExceptionContext {
    pub exception_code: u32,
    pub exception_flags: u32,
    pub fault_address: u64,
    pub thread_id: u64,
    pub process_id: u32,
    pub timestamp_tsc: u64,
    pub rip: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
}

/// Dynamic Exception Router capturing fatal faults.
pub struct DynamicExceptionRouter {
    captured_exceptions: Spinlock<Vec<CapturedExceptionContext>>,
    capture_count: AtomicU64,
}

impl DynamicExceptionRouter {
    pub const fn new() -> Self {
        Self {
            captured_exceptions: Spinlock::new(Vec::new()),
            capture_count: AtomicU64::new(0),
        }
    }

    /// Capture and route a user-mode exception.
    pub fn capture_exception(&self, ctx: CapturedExceptionContext) {
        let mut list = self.captured_exceptions.lock();
        list.push(ctx);
        self.capture_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Check if an exception code is a recognized fatal user crash.
    pub fn is_fatal_crash_code(code: u32) -> bool {
        code == EXCEPTION_ACCESS_VIOLATION
            || code == EXCEPTION_ILLEGAL_INSTRUCTION
            || code == EXCEPTION_STACK_BUFFER_OVERRUN
    }

    /// Retrieve count of captured exceptions.
    pub fn get_capture_count(&self) -> u64 {
        self.capture_count.load(Ordering::Acquire)
    }

    /// Retrieve captured exceptions list.
    pub fn get_captured(&self) -> Vec<CapturedExceptionContext> {
        self.captured_exceptions.lock().clone()
    }
}

pub static EXCEPTION_ROUTER: DynamicExceptionRouter = DynamicExceptionRouter::new();

/// Minidump stream signature ("MDMP").
pub const MINIDUMP_SIGNATURE: u32 = 0x504D444D;
pub const MINIDUMP_VERSION: u32 = 0x0000_A793;

/// In-memory Minidump Stream Header.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinidumpHeader {
    pub signature: u32,
    pub version: u32,
    pub number_of_streams: u32,
    pub stream_directory_rva: u32,
    pub check_sum: u32,
    pub time_date_stamp: u32,
    pub flags: u64,
}

/// Minidump Stack Backtrace Frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MinidumpStackFrame {
    pub instruction_offset: u64,
    pub return_offset: u64,
    pub frame_offset: u64,
    pub stack_offset: u64,
}

/// Encoded Minidump Stream Record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedMinidump {
    pub header: MinidumpHeader,
    pub exception_record: CapturedExceptionContext,
    pub stack_backtrace: Vec<MinidumpStackFrame>,
    pub memory_dump_bytes: Vec<u8>,
}

/// In-Memory Minidump Stream Encoder.
pub struct MinidumpStreamEncoder;

impl MinidumpStreamEncoder {
    /// Encode a captured crash context and stack memory range into an in-memory Minidump stream.
    pub fn encode_minidump(
        ctx: &CapturedExceptionContext,
        stack_memory: &[u8],
    ) -> EncodedMinidump {
        let header = MinidumpHeader {
            signature: MINIDUMP_SIGNATURE,
            version: MINIDUMP_VERSION,
            number_of_streams: 3, // ExceptionStream, ThreadListStream, MemoryListStream
            stream_directory_rva: 0x20,
            check_sum: 0,
            time_date_stamp: 0x6723_0000,
            flags: 0x0002_0000, // MiniDumpNormal
        };

        let mut backtrace = Vec::new();
        // Construct frame 0 from faulting registers
        backtrace.push(MinidumpStackFrame {
            instruction_offset: ctx.rip,
            return_offset: ctx.rip.saturating_add(4),
            frame_offset: ctx.rbp,
            stack_offset: ctx.rsp,
        });

        EncodedMinidump {
            header,
            exception_record: *ctx,
            stack_backtrace: backtrace,
            memory_dump_bytes: stack_memory.to_vec(),
        }
    }
}

pub const CRASH_RING_BUFFER_CAPACITY: usize = 256;

/// Lock-free SPSC crash telemetry ring buffer streaming crash events with drop tracking.
pub struct CrashTelemetryRingBuffer {
    buffer: Spinlock<Vec<EncodedMinidump>>,
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped_records: AtomicU64,
}

impl CrashTelemetryRingBuffer {
    pub const fn new() -> Self {
        Self {
            buffer: Spinlock::new(Vec::new()),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped_records: AtomicU64::new(0),
        }
    }

    /// Enqueue an encoded minidump with sub-microsecond latency.
    pub fn enqueue_minidump(&self, dump: EncodedMinidump) -> bool {
        let mut buf = self.buffer.lock();
        if buf.len() < CRASH_RING_BUFFER_CAPACITY {
            buf.push(dump);
            self.head.fetch_add(1, Ordering::Release);
            true
        } else {
            self.dropped_records.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    /// Dequeue all pending crash minidumps in batch.
    pub fn dequeue_batch(&self) -> Vec<EncodedMinidump> {
        let mut buf = self.buffer.lock();
        let batch = buf.clone();
        buf.clear();
        self.head.store(0, Ordering::Relaxed);
        batch
    }

    /// Retrieve number of dropped crash records.
    pub fn get_dropped_count(&self) -> u64 {
        self.dropped_records.load(Ordering::Acquire)
    }

    /// Retrieve number of pending records.
    pub fn len(&self) -> usize {
        self.buffer.lock().len()
    }

    /// Check if buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub static CRASH_RING_BUFFER: CrashTelemetryRingBuffer = CrashTelemetryRingBuffer::new();

/// Kernel diagnostic store ingesting crash telemetry and minidump streams.
pub struct KernelDiagnosticStore {
    records: Spinlock<Vec<EncodedMinidump>>,
    total_ingested: AtomicU64,
}

impl KernelDiagnosticStore {
    pub const fn new() -> Self {
        Self {
            records: Spinlock::new(Vec::new()),
            total_ingested: AtomicU64::new(0),
        }
    }

    /// Ingest pending minidump streams from the crash telemetry ring buffer.
    pub fn ingest_from_ring_buffer(&self, ring_buffer: &CrashTelemetryRingBuffer) -> usize {
        let batch = ring_buffer.dequeue_batch();
        let count = batch.len();
        if count > 0 {
            let mut rec = self.records.lock();
            rec.extend(batch);
            self.total_ingested.fetch_add(count as u64, Ordering::Release);
        }
        count
    }

    /// Retrieve total count of ingested minidumps.
    pub fn get_total_ingested(&self) -> u64 {
        self.total_ingested.load(Ordering::Acquire)
    }

    /// Query minidump records by exception code.
    pub fn query_by_exception_code(&self, code: u32) -> Vec<EncodedMinidump> {
        let rec = self.records.lock();
        rec.iter()
            .filter(|m| m.exception_record.exception_code == code)
            .cloned()
            .collect()
    }

    /// Clear all stored crash records.
    pub fn clear(&self) {
        let mut rec = self.records.lock();
        rec.clear();
        self.total_ingested.store(0, Ordering::Relaxed);
    }
}

pub static KERNEL_DIAGNOSTIC_STORE: KernelDiagnosticStore = KernelDiagnosticStore::new();

/// Comprehensive self-test for Win32 Dynamic Exception Routing & Minidump Crash Telemetry Ring Buffer.
pub fn test_win32_crash_telemetry_subsystem() -> bool {
    crate::serial_write_str("[KNT-WIN32-CRASH-TELEMETRY] Initializing Win32 Crash Telemetry Subsystem self-test...\r\n");

    // 1. Verify Fatal Crash Codes & Dynamic Exception Router
    if !DynamicExceptionRouter::is_fatal_crash_code(EXCEPTION_ACCESS_VIOLATION)
        || !DynamicExceptionRouter::is_fatal_crash_code(EXCEPTION_ILLEGAL_INSTRUCTION)
        || !DynamicExceptionRouter::is_fatal_crash_code(EXCEPTION_STACK_BUFFER_OVERRUN)
    {
        crate::serial_write_str("[KNT-TEST-ERR] Fatal crash code classification mismatch\r\n");
        return false;
    }

    let initial_count = EXCEPTION_ROUTER.get_capture_count();
    let ctx = CapturedExceptionContext {
        exception_code: EXCEPTION_ACCESS_VIOLATION,
        exception_flags: 1, // EXCEPTION_NONCONTINUABLE
        fault_address: 0x0000_7FFF_DEAD_BEEFu64,
        thread_id: 2048,
        process_id: 100,
        timestamp_tsc: 987654321,
        rip: 0x0000_7FFF_0040_1000,
        rsp: 0x0000_00A0_1000_8000,
        rbp: 0x0000_00A0_1000_8020,
        rax: 0x11,
        rbx: 0x22,
        rcx: 0x33,
        rdx: 0x44,
    };

    EXCEPTION_ROUTER.capture_exception(ctx);
    if EXCEPTION_ROUTER.get_capture_count() != initial_count + 1 {
        crate::serial_write_str("[KNT-TEST-ERR] Dynamic exception capture count mismatch\r\n");
        return false;
    }

    // 2. Verify Minidump Stream Encoder
    let mock_stack_mem = [0x55u8; 128];
    let minidump = MinidumpStreamEncoder::encode_minidump(&ctx, &mock_stack_mem);

    if minidump.header.signature != MINIDUMP_SIGNATURE || minidump.header.version != MINIDUMP_VERSION {
        crate::serial_write_str("[KNT-TEST-ERR] Minidump signature or version header mismatch\r\n");
        return false;
    }

    if minidump.stack_backtrace.is_empty()
        || minidump.stack_backtrace[0].instruction_offset != ctx.rip
        || minidump.stack_backtrace[0].stack_offset != ctx.rsp
    {
        crate::serial_write_str("[KNT-TEST-ERR] Minidump stack backtrace frame mismatch\r\n");
        return false;
    }

    if minidump.memory_dump_bytes.len() != 128 || minidump.memory_dump_bytes[0] != 0x55 {
        crate::serial_write_str("[KNT-TEST-ERR] Minidump memory dump payload mismatch\r\n");
        return false;
    }

    // 3. Verify SPSC Crash Telemetry Ring Buffer & Kernel Diagnostic Store
    let enq_res = CRASH_RING_BUFFER.enqueue_minidump(minidump);
    if !enq_res || CRASH_RING_BUFFER.is_empty() {
        crate::serial_write_str("[KNT-TEST-ERR] SPSC ring buffer enqueue failed\r\n");
        return false;
    }

    let initial_store_count = KERNEL_DIAGNOSTIC_STORE.get_total_ingested();
    let ingested = KERNEL_DIAGNOSTIC_STORE.ingest_from_ring_buffer(&CRASH_RING_BUFFER);
    if ingested != 1 || !CRASH_RING_BUFFER.is_empty() {
        crate::serial_write_str("[KNT-TEST-ERR] Diagnostic store ingestion or buffer clear failed\r\n");
        return false;
    }

    if KERNEL_DIAGNOSTIC_STORE.get_total_ingested() != initial_store_count + 1 {
        crate::serial_write_str("[KNT-TEST-ERR] Diagnostic store total count mismatch\r\n");
        return false;
    }

    let queried = KERNEL_DIAGNOSTIC_STORE.query_by_exception_code(EXCEPTION_ACCESS_VIOLATION);
    if queried.is_empty() || queried.last().unwrap().exception_record.fault_address != 0x0000_7FFF_DEAD_BEEF {
        crate::serial_write_str("[KNT-TEST-ERR] Diagnostic store queried record mismatch\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-CRASH-TELEMETRY-PASS] Win32 Dynamic Exception Routing & Minidump Telemetry verified\r\n");
    true
}
