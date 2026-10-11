//! Win32 High-Resolution Performance Telemetry Ring Buffer & Real-Time ETW Provider.
//!
//! Implements Win32 ETW provider registration and trace session governor (EventRegister, EventWriteTransfer, EventUnregister),
//! lock-free SPSC telemetry ring buffer capturing frame boundaries, GPU present timings, and draw calls,
//! compact binary trace packet serializer with zero kernel heap allocations,
//! and handheld trace session buffer circular overwrite mode for minimal RAM footprint.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
pub use crate::pe::srw_futex::PVOID;
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

/// GUID representation for ETW provider identification.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GUID {
    pub Data1: u32,
    pub Data2: u16,
    pub Data3: u16,
    pub Data4: [u8; 8],
}

/// ETW Event Descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EVENT_DESCRIPTOR {
    pub Id: u16,
    pub Version: u8,
    pub Channel: u8,
    pub Level: u8,
    pub Opcode: u8,
    pub Task: u16,
    pub Keyword: u64,
}

pub type REGHANDLE = u64;

/// ETW Provider callback.
pub type PENABLECALLBACK = Option<
    unsafe extern "system" fn(
        SourceId: *const GUID,
        IsEnabled: u32,
        Level: u8,
        MatchAnyKeyword: u64,
        MatchAllKeyword: u64,
        FilterData: *const core::ffi::c_void,
        CallbackContext: *mut core::ffi::c_void,
    ),
>;

/// ETW Provider internal descriptor.
pub struct EtwProviderInner {
    pub reg_handle: REGHANDLE,
    pub provider_id: GUID,
    pub is_enabled: bool,
    pub level: u8,
    pub match_any_keyword: u64,
    pub callback: PENABLECALLBACK,
    pub context: usize,
}

static NEXT_REG_HANDLE: AtomicU64 = AtomicU64::new(0xE001);
static ETW_PROVIDERS: Spinlock<BTreeMap<REGHANDLE, EtwProviderInner>> = Spinlock::new(BTreeMap::new());

/// Registers a userland or driver Win32 ETW event provider (EventRegister).
pub unsafe extern "system" fn EventRegister(
    ProviderId: *const GUID,
    EnableCallback: PENABLECALLBACK,
    CallbackContext: *mut core::ffi::c_void,
    RegHandle: *mut REGHANDLE,
) -> u32 {
    if ProviderId.is_null() || RegHandle.is_null() {
        return 87; // ERROR_INVALID_PARAMETER
    }

    let prov_guid = *ProviderId;
    let handle = NEXT_REG_HANDLE.fetch_add(1, Ordering::SeqCst);

    let provider = EtwProviderInner {
        reg_handle: handle,
        provider_id: prov_guid,
        is_enabled: true,
        level: 5, // Verbose
        match_any_keyword: !0,
        callback: EnableCallback,
        context: CallbackContext as usize,
    };

    ETW_PROVIDERS.lock().insert(handle, provider);
    *RegHandle = handle;
    0 // ERROR_SUCCESS
}

/// Unregisters an ETW provider (EventUnregister).
pub unsafe extern "system" fn EventUnregister(RegHandle: REGHANDLE) -> u32 {
    if RegHandle == 0 {
        return 87; // ERROR_INVALID_PARAMETER
    }

    if ETW_PROVIDERS.lock().remove(&RegHandle).is_some() {
        0 // ERROR_SUCCESS
    } else {
        6 // ERROR_INVALID_HANDLE
    }
}

/// Returns whether the specified provider is enabled for events at given level and keyword.
pub fn win32_etw_provider_is_enabled(reg_handle: REGHANDLE, level: u8, keyword: u64) -> bool {
    let providers = ETW_PROVIDERS.lock();
    if let Some(p) = providers.get(&reg_handle) {
        p.is_enabled && (level <= p.level) && ((keyword & p.match_any_keyword) != 0 || keyword == 0)
    } else {
        false
    }
}

/// Telemetry event types for gaming and graphics profiling.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamingTelemetryEventType {
    FrameBoundary = 1,
    GpuPresent = 2,
    DrawCall = 3,
    ShaderDispatch = 4,
    InputPoll = 5,
}

/// Fixed-size telemetry sample record for lock-free ring buffer.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct GamingTelemetryRecord {
    pub timestamp_tsc: u64,
    pub event_type: u16,
    pub payload_u16: u16,
    pub payload_u32: u32,
    pub payload_u64: u64,
}

pub const TELEMETRY_RING_CAPACITY: usize = 1024;
pub const TELEMETRY_RING_MASK: usize = TELEMETRY_RING_CAPACITY - 1;

/// Lock-free single-producer single-consumer telemetry ring buffer.
pub struct SpscTelemetryRing {
    pub buffer: [GamingTelemetryRecord; TELEMETRY_RING_CAPACITY],
    pub head: AtomicUsize,
    pub tail: AtomicUsize,
    pub dropped_records: AtomicU64,
}

impl SpscTelemetryRing {
    pub const fn new() -> Self {
        Self {
            buffer: [GamingTelemetryRecord {
                timestamp_tsc: 0,
                event_type: 0,
                payload_u16: 0,
                payload_u32: 0,
                payload_u64: 0,
            }; TELEMETRY_RING_CAPACITY],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped_records: AtomicU64::new(0),
        }
    }

    /// Pushes a telemetry event record into the ring buffer (producer).
    pub fn push(&mut self, record: GamingTelemetryRecord) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);

        if (head.wrapping_sub(tail)) >= TELEMETRY_RING_CAPACITY {
            self.dropped_records.fetch_add(1, Ordering::Relaxed);
            return false;
        }

        self.buffer[head & TELEMETRY_RING_MASK] = record;
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// Pops a telemetry event record from the ring buffer (consumer).
    pub fn pop(&mut self) -> Option<GamingTelemetryRecord> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);

        if head == tail {
            return None;
        }

        let rec = self.buffer[tail & TELEMETRY_RING_MASK];
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(rec)
    }

    /// Returns the number of currently buffered records.
    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);
        head.saturating_sub(tail)
    }
}

static GLOBAL_TELEMETRY_RING: Spinlock<SpscTelemetryRing> = Spinlock::new(SpscTelemetryRing::new());

/// Records a frame boundary event with frame index and duration.
pub fn win32_etw_trace_frame_boundary(frame_index: u64, frame_time_us: u32) -> bool {
    let mut ring = GLOBAL_TELEMETRY_RING.lock();
    let rec = GamingTelemetryRecord {
        timestamp_tsc: crate::pe::perf_counter::win32_read_tsc(),
        event_type: GamingTelemetryEventType::FrameBoundary as u16,
        payload_u16: 0,
        payload_u32: frame_time_us,
        payload_u64: frame_index,
    };
    ring.push(rec)
}

/// Records a GPU Present event with sync interval and swapchain id.
pub fn win32_etw_trace_gpu_present(swapchain_id: u64, sync_interval: u16) -> bool {
    let mut ring = GLOBAL_TELEMETRY_RING.lock();
    let rec = GamingTelemetryRecord {
        timestamp_tsc: crate::pe::perf_counter::win32_read_tsc(),
        event_type: GamingTelemetryEventType::GpuPresent as u16,
        payload_u16: sync_interval,
        payload_u32: 0,
        payload_u64: swapchain_id,
    };
    ring.push(rec)
}

/// Records a draw call or compute dispatch count.
pub fn win32_etw_trace_draw_calls(draw_call_count: u32, triangle_count: u64) -> bool {
    let mut ring = GLOBAL_TELEMETRY_RING.lock();
    let rec = GamingTelemetryRecord {
        timestamp_tsc: crate::pe::perf_counter::win32_read_tsc(),
        event_type: GamingTelemetryEventType::DrawCall as u16,
        payload_u16: 0,
        payload_u32: draw_call_count,
        payload_u64: triangle_count,
    };
    ring.push(rec)
}

/// Pops next recorded telemetry event from global ring buffer.
pub fn win32_etw_pop_telemetry_event() -> Option<GamingTelemetryRecord> {
    let mut ring = GLOBAL_TELEMETRY_RING.lock();
    ring.pop()
}

/// Compact binary ETW packet serialization header.
pub const ETW_PACKET_MAGIC: u32 = 0x45545750; // 'ETWP'

/// Serializes an ETW event record into a compact 32-byte binary packet with zero heap allocations.
pub fn win32_etw_serialize_record(rec: &GamingTelemetryRecord, out_buf: &mut [u8; 32]) -> usize {
    out_buf[0..4].copy_from_slice(&ETW_PACKET_MAGIC.to_le_bytes());
    out_buf[4..12].copy_from_slice(&rec.timestamp_tsc.to_le_bytes());
    out_buf[12..14].copy_from_slice(&rec.event_type.to_le_bytes());
    out_buf[14..16].copy_from_slice(&rec.payload_u16.to_le_bytes());
    out_buf[16..20].copy_from_slice(&rec.payload_u32.to_le_bytes());
    out_buf[20..28].copy_from_slice(&rec.payload_u64.to_le_bytes());

    // Simple 4-byte CRC32 / XOR checksum
    let mut csum: u32 = 0;
    for i in 0..7 {
        let chunk = u32::from_le_bytes([out_buf[i * 4], out_buf[i * 4 + 1], out_buf[i * 4 + 2], out_buf[i * 4 + 3]]);
        csum ^= chunk.rotate_left((i as u32) + 1);
    }
    out_buf[28..32].copy_from_slice(&csum.to_le_bytes());
    32
}

/// Deserializes a compact 32-byte binary ETW packet into a GamingTelemetryRecord.
pub fn win32_etw_deserialize_record(packet: &[u8; 32]) -> Option<GamingTelemetryRecord> {
    let magic = u32::from_le_bytes([packet[0], packet[1], packet[2], packet[3]]);
    if magic != ETW_PACKET_MAGIC {
        return None;
    }

    let mut expected_csum: u32 = 0;
    for i in 0..7 {
        let chunk = u32::from_le_bytes([packet[i * 4], packet[i * 4 + 1], packet[i * 4 + 2], packet[i * 4 + 3]]);
        expected_csum ^= chunk.rotate_left((i as u32) + 1);
    }
    let actual_csum = u32::from_le_bytes([packet[28], packet[29], packet[30], packet[31]]);
    if expected_csum != actual_csum {
        return None;
    }

    let timestamp_tsc = u64::from_le_bytes([
        packet[4], packet[5], packet[6], packet[7], packet[8], packet[9], packet[10], packet[11],
    ]);
    let event_type = u16::from_le_bytes([packet[12], packet[13]]);
    let payload_u16 = u16::from_le_bytes([packet[14], packet[15]]);
    let payload_u32 = u32::from_le_bytes([packet[16], packet[17], packet[18], packet[19]]);
    let payload_u64 = u64::from_le_bytes([
        packet[20], packet[21], packet[22], packet[23], packet[24], packet[25], packet[26], packet[27],
    ]);

    Some(GamingTelemetryRecord {
        timestamp_tsc,
        event_type,
        payload_u16,
        payload_u32,
        payload_u64,
    })
}

/// Writes an event transfer via EventWriteTransfer API.
pub unsafe extern "system" fn EventWriteTransfer(
    RegHandle: REGHANDLE,
    EventDescriptor: *const EVENT_DESCRIPTOR,
    _ActivityId: *const GUID,
    _RelatedActivityId: *const GUID,
    UserDataCount: u32,
    _UserData: *const core::ffi::c_void,
) -> u32 {
    if RegHandle == 0 || EventDescriptor.is_null() {
        return 87; // ERROR_INVALID_PARAMETER
    }

    let desc = *EventDescriptor;
    if !win32_etw_provider_is_enabled(RegHandle, desc.Level, desc.Keyword) {
        return 0; // Filtered out
    }

    let rec = GamingTelemetryRecord {
        timestamp_tsc: crate::pe::perf_counter::win32_read_tsc(),
        event_type: desc.Id,
        payload_u16: desc.Task,
        payload_u32: UserDataCount,
        payload_u64: desc.Keyword,
    };

    let mut ring = GLOBAL_TELEMETRY_RING.lock();
    ring.push(rec);
    0 // ERROR_SUCCESS
}

/// Circular overwrite mode for handheld gaming profile sessions with strict RAM budgets.
pub struct HandheldCircularOverwriteRing {
    pub buffer: [GamingTelemetryRecord; 256],
    pub cursor: AtomicUsize,
    pub total_written: AtomicU64,
}

impl HandheldCircularOverwriteRing {
    pub const fn new() -> Self {
        Self {
            buffer: [GamingTelemetryRecord {
                timestamp_tsc: 0,
                event_type: 0,
                payload_u16: 0,
                payload_u32: 0,
                payload_u64: 0,
            }; 256],
            cursor: AtomicUsize::new(0),
            total_written: AtomicU64::new(0),
        }
    }

    /// Overwrites the oldest telemetry sample once 256 records are reached.
    pub fn push_overwrite(&mut self, record: GamingTelemetryRecord) {
        let idx = self.cursor.load(Ordering::Relaxed) % 256;
        self.buffer[idx] = record;
        self.cursor.store((idx + 1) % 256, Ordering::Release);
        self.total_written.fetch_add(1, Ordering::Relaxed);
    }

    /// Reads latest N records from circular buffer.
    pub fn read_latest(&self, out: &mut [GamingTelemetryRecord]) -> usize {
        let count = out.len().min(256).min(self.total_written.load(Ordering::Relaxed) as usize);
        let cur = self.cursor.load(Ordering::Acquire);

        for i in 0..count {
            let idx = (cur + 256 - count + i) % 256;
            out[i] = self.buffer[idx];
        }
        count
    }
}

static CIRCULAR_OVERWRITE_RING: Spinlock<HandheldCircularOverwriteRing> =
    Spinlock::new(HandheldCircularOverwriteRing::new());

/// Appends record to compact circular overwrite ring buffer.
pub fn win32_etw_push_circular_record(record: GamingTelemetryRecord) {
    let mut ring = CIRCULAR_OVERWRITE_RING.lock();
    ring.push_overwrite(record);
}

/// Reads recent records from circular overwrite buffer.
pub fn win32_etw_read_circular_records(out: &mut [GamingTelemetryRecord]) -> usize {
    let ring = CIRCULAR_OVERWRITE_RING.lock();
    ring.read_latest(out)
}

/// Self-test suite verifying Win32 ETW providers, lock-free telemetry ring buffer, and compact serialization.
pub fn test_win32_etw_subsystem() -> bool {
    crate::serial_write_str("[KNT-TEST] Initializing Win32 Performance Telemetry Ring Buffer & ETW self-test...\r\n");

    // 1. Test EventRegister
    let prov_guid = GUID {
        Data1: 0x12345678,
        Data2: 0x9ABC,
        Data3: 0xDEF0,
        Data4: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
    };

    let mut reg_handle: REGHANDLE = 0;
    let status = unsafe {
        EventRegister(
            &prov_guid,
            None,
            core::ptr::null_mut(),
            &mut reg_handle,
        )
    };
    if status != 0 || reg_handle == 0 {
        crate::serial_write_str("[KNT-ERROR] EventRegister failed\r\n");
        return false;
    }

    if !win32_etw_provider_is_enabled(reg_handle, 3, 0) {
        crate::serial_write_str("[KNT-ERROR] win32_etw_provider_is_enabled returned false\r\n");
        return false;
    }

    // 2. Test EventWriteTransfer
    let desc = EVENT_DESCRIPTOR {
        Id: 101,
        Version: 1,
        Channel: 0,
        Level: 4,
        Opcode: 0,
        Task: 1,
        Keyword: 0x01,
    };
    let write_status = unsafe {
        EventWriteTransfer(
            reg_handle,
            &desc,
            core::ptr::null(),
            core::ptr::null(),
            42,
            core::ptr::null(),
        )
    };
    if write_status != 0 {
        crate::serial_write_str("[KNT-ERROR] EventWriteTransfer failed\r\n");
        return false;
    }

    // 3. Test telemetry ring buffer push & pop
    win32_etw_trace_frame_boundary(1, 16_666);
    win32_etw_trace_gpu_present(0x1000, 1);
    win32_etw_trace_draw_calls(150, 450_000);

    let mut popped_count = 0;
    while let Some(evt) = win32_etw_pop_telemetry_event() {
        popped_count += 1;
        // 4. Test compact binary packet serialization
        let mut packet = [0u8; 32];
        let bytes = win32_etw_serialize_record(&evt, &mut packet);
        if bytes != 32 {
            crate::serial_write_str("[KNT-ERROR] Serialization byte count mismatch\r\n");
            return false;
        }

        let deserialized = match win32_etw_deserialize_record(&packet) {
            Some(d) => d,
            None => {
                crate::serial_write_str("[KNT-ERROR] Deserialization failed or checksum mismatch\r\n");
                return false;
            }
        };

        if deserialized.event_type != evt.event_type || deserialized.payload_u64 != evt.payload_u64 {
            crate::serial_write_str("[KNT-ERROR] Deserialized record field mismatch\r\n");
            return false;
        }
    }

    if popped_count < 4 {
        crate::serial_write_str("[KNT-ERROR] Popped telemetry records fewer than expected\r\n");
        return false;
    }

    // 5. Test circular overwrite mode with 300 samples (exceeding 256 capacity)
    for i in 0..300 {
        win32_etw_push_circular_record(GamingTelemetryRecord {
            timestamp_tsc: i,
            event_type: 1,
            payload_u16: 0,
            payload_u32: i as u32,
            payload_u64: i * 10,
        });
    }

    let mut latest = [GamingTelemetryRecord::default(); 16];
    let n_read = win32_etw_read_circular_records(&mut latest);
    if n_read != 16 || latest[15].payload_u32 != 299 {
        crate::serial_write_str("[KNT-ERROR] Circular overwrite buffer read mismatch\r\n");
        return false;
    }

    // 6. Test EventUnregister
    let unreg_status = unsafe { EventUnregister(reg_handle) };
    if unreg_status != 0 {
        crate::serial_write_str("[KNT-ERROR] EventUnregister failed\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-ETW-PASS] Win32 high-resolution ETW telemetry provider and SPSC ring buffer verified\r\n");
    true
}
