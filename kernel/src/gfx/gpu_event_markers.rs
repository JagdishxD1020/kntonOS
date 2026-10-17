// Direct3D 12 Agility Execution Markers & PIX GPU Performance Event Instrumentation.
// Provides non-blocking execution markers, timestamp queries, and bottleneck telemetry.

use core::fmt;

// Maximum nesting level supported for hierarchical PIX begin/end events.
pub const MAX_PIX_EVENT_STACK_DEPTH: usize = 32;

// Maximum capacity of the marker event recording ring buffer per command list.
pub const MAX_RECORDED_EVENTS: usize = 256;

// Maximum byte length of an execution marker ASCII/UTF-8 label string.
pub const MAX_MARKER_NAME_LEN: usize = 64;

// 32-bit ARGB color packing for PIX execution event visualization.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PixColor(pub u32);

impl PixColor {
    // Default system cyan highlight for Agility command markers.
    pub const DEFAULT: Self = Self(0xFF00_D2FF);
    // Green highlight for geometry and vertex generation passes.
    pub const GEOMETRY: Self = Self(0xFF2E_CC71);
    // Amber highlight for pixel shading and rasterization passes.
    pub const PIXEL: Self = Self(0xFFF3_9C12);
    // Red highlight for memory bandwidth and resource copy passes.
    pub const MEMORY: Self = Self(0xFFE7_4C3C);

    // Creates a PIX color from 8-bit RGB components with full opacity.
    #[inline]
    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self(0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    // Creates a PIX color with explicit alpha and RGB components.
    #[inline]
    pub const fn from_argb(a: u8, r: u8, g: u8, b: u8) -> Self {
        Self(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }
}

// Marker event classifications conforming to D3D12 Agility instrumentation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum D3d12MarkerType {
    // Single-point execution timeline marker.
    SetMarker,
    // Beginning of a nested execution range.
    BeginEvent,
    // Termination of the most recent nested execution range.
    EndEvent,
}

// Fixed-capacity string buffer representing execution marker names without allocation.
#[derive(Copy, Clone, Eq)]
pub struct MarkerName {
    bytes: [u8; MAX_MARKER_NAME_LEN],
    len: usize,
}

impl MarkerName {
    // Creates an empty marker label buffer.
    pub const fn empty() -> Self {
        Self {
            bytes: [0; MAX_MARKER_NAME_LEN],
            len: 0,
        }
    }

    // Constructs a marker label from an ASCII string slice, clamping to buffer capacity.
    pub fn from_str(src: &str) -> Self {
        let mut name = Self::empty();
        let bytes = src.as_bytes();
        let copy_len = if bytes.len() > MAX_MARKER_NAME_LEN {
            MAX_MARKER_NAME_LEN
        } else {
            bytes.len()
        };
        name.bytes[..copy_len].copy_from_slice(&bytes[..copy_len]);
        name.len = copy_len;
        name
    }

    // Returns the marker label as a string slice.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("<invalid utf8>")
    }
}

impl PartialEq for MarkerName {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl fmt::Debug for MarkerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// Error conditions encountered during execution marker hierarchy manipulation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MarkerError {
    // Event stack exceeded maximum allowed nesting depth.
    StackOverflow,
    // Event stack attempted to pop when empty.
    StackUnderflow,
    // Recorder event storage capacity exhausted.
    RecorderBufferFull,
    // Query allocation requested when heap is fully committed.
    QueryHeapExhausted,
    // Query index outside valid range.
    InvalidQueryIndex,
}

// Represents a captured or scheduled PIX execution event.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PixEvent {
    pub name: MarkerName,
    pub color: PixColor,
    pub depth: u32,
    pub event_type: D3d12MarkerType,
    pub correlation_id: u64,
}

impl PixEvent {
    // Constructs an empty uninitialized event.
    pub const fn empty() -> Self {
        Self {
            name: MarkerName::empty(),
            color: PixColor::DEFAULT,
            depth: 0,
            event_type: D3d12MarkerType::SetMarker,
            correlation_id: 0,
        }
    }
}

// Fixed-capacity execution event stack tracking nested PIX scopes.
pub struct PixEventStack {
    stack: [PixEvent; MAX_PIX_EVENT_STACK_DEPTH],
    depth: usize,
}

impl PixEventStack {
    // Creates an empty PIX event hierarchy stack.
    pub const fn new() -> Self {
        Self {
            stack: [PixEvent::empty(); MAX_PIX_EVENT_STACK_DEPTH],
            depth: 0,
        }
    }

    // Pushes a new nested scope onto the event stack.
    pub fn push_event(
        &mut self,
        name: MarkerName,
        color: PixColor,
        correlation_id: u64,
    ) -> Result<u32, MarkerError> {
        if self.depth >= MAX_PIX_EVENT_STACK_DEPTH {
            return Err(MarkerError::StackOverflow);
        }
        let current_depth = self.depth as u32;
        self.stack[self.depth] = PixEvent {
            name,
            color,
            depth: current_depth,
            event_type: D3d12MarkerType::BeginEvent,
            correlation_id,
        };
        self.depth += 1;
        Ok(current_depth)
    }

    // Pops the topmost nested scope from the event stack.
    pub fn pop_event(&mut self) -> Result<PixEvent, MarkerError> {
        if self.depth == 0 {
            return Err(MarkerError::StackUnderflow);
        }
        self.depth -= 1;
        let mut evt = self.stack[self.depth];
        evt.event_type = D3d12MarkerType::EndEvent;
        Ok(evt)
    }

    // Returns current nesting depth of the active scope.
    #[inline]
    pub fn current_depth(&self) -> u32 {
        self.depth as u32
    }

    // Indicates whether the stack is currently empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.depth == 0
    }
}

// Command list execution marker recorder embedding instrumentation events into draw streams.
pub struct CommandListMarkerRecorder {
    stack: PixEventStack,
    events: [PixEvent; MAX_RECORDED_EVENTS],
    event_count: usize,
    next_correlation_id: u64,
}

impl CommandListMarkerRecorder {
    // Initializes an execution marker recorder with empty event buffers.
    pub const fn new() -> Self {
        Self {
            stack: PixEventStack::new(),
            events: [PixEvent::empty(); MAX_RECORDED_EVENTS],
            event_count: 0,
            next_correlation_id: 1,
        }
    }

    // Records a point-in-time execution marker into the active command stream.
    pub fn set_marker(&mut self, label: &str, color: PixColor) -> Result<u64, MarkerError> {
        if self.event_count >= MAX_RECORDED_EVENTS {
            return Err(MarkerError::RecorderBufferFull);
        }
        let id = self.next_correlation_id;
        self.next_correlation_id += 1;

        let evt = PixEvent {
            name: MarkerName::from_str(label),
            color,
            depth: self.stack.current_depth(),
            event_type: D3d12MarkerType::SetMarker,
            correlation_id: id,
        };
        self.events[self.event_count] = evt;
        self.event_count += 1;
        Ok(id)
    }

    // Begins a nested instrumentation event scope in the command stream.
    pub fn begin_event(&mut self, label: &str, color: PixColor) -> Result<u64, MarkerError> {
        if self.event_count >= MAX_RECORDED_EVENTS {
            return Err(MarkerError::RecorderBufferFull);
        }
        let id = self.next_correlation_id;
        self.next_correlation_id += 1;

        let name = MarkerName::from_str(label);
        let depth = self.stack.push_event(name, color, id)?;

        let evt = PixEvent {
            name,
            color,
            depth,
            event_type: D3d12MarkerType::BeginEvent,
            correlation_id: id,
        };
        self.events[self.event_count] = evt;
        self.event_count += 1;
        Ok(id)
    }

    // Ends the topmost nested instrumentation event scope in the command stream.
    pub fn end_event(&mut self) -> Result<PixEvent, MarkerError> {
        if self.event_count >= MAX_RECORDED_EVENTS {
            return Err(MarkerError::RecorderBufferFull);
        }
        let popped = self.stack.pop_event()?;
        self.events[self.event_count] = popped;
        self.event_count += 1;
        Ok(popped)
    }

    // PIX standard convenience wrapper: begins an event scope.
    #[inline]
    pub fn pix_begin_event(&mut self, color: PixColor, label: &str) -> Result<u64, MarkerError> {
        self.begin_event(label, color)
    }

    // PIX standard convenience wrapper: ends the current event scope.
    #[inline]
    pub fn pix_end_event(&mut self) -> Result<PixEvent, MarkerError> {
        self.end_event()
    }

    // PIX standard convenience wrapper: records a discrete execution marker.
    #[inline]
    pub fn pix_set_marker(&mut self, color: PixColor, label: &str) -> Result<u64, MarkerError> {
        self.set_marker(label, color)
    }

    // Returns a slice of all recorded execution events in this command list.
    #[inline]
    pub fn recorded_events(&self) -> &[PixEvent] {
        &self.events[..self.event_count]
    }

    // Resets recorder state and event counters for command buffer recycling.
    pub fn reset(&mut self) {
        self.stack = PixEventStack::new();
        self.event_count = 0;
    }
}

// Maximum number of timestamp queries simultaneously managed in a query heap.
pub const MAX_TIMESTAMP_QUERIES: usize = 512;

// Standard D3D12 query type for high-precision timeline timestamps.
pub const D3D12_QUERY_TYPE_TIMESTAMP: u32 = 2;

// Fixed-capacity GPU timestamp query heap conforming to D3D12 specifications.
pub struct D3d12TimestampQueryHeap {
    timestamps: [u64; MAX_TIMESTAMP_QUERIES],
    allocated_bitmap: [u64; MAX_TIMESTAMP_QUERIES / 64],
    capacity: usize,
    active_count: usize,
}

impl D3d12TimestampQueryHeap {
    // Initializes an empty timestamp query heap.
    pub const fn new() -> Self {
        Self {
            timestamps: [0; MAX_TIMESTAMP_QUERIES],
            allocated_bitmap: [0; MAX_TIMESTAMP_QUERIES / 64],
            capacity: MAX_TIMESTAMP_QUERIES,
            active_count: 0,
        }
    }

    // Allocates a discrete query slot for timestamp recording.
    pub fn allocate_query(&mut self) -> Result<u32, MarkerError> {
        for word_idx in 0..self.allocated_bitmap.len() {
            let word = self.allocated_bitmap[word_idx];
            if word != u64::MAX {
                let bit_idx = word.trailing_ones() as usize;
                let query_idx = word_idx * 64 + bit_idx;
                if query_idx < self.capacity {
                    self.allocated_bitmap[word_idx] |= 1 << bit_idx;
                    self.timestamps[query_idx] = 0;
                    self.active_count += 1;
                    return Ok(query_idx as u32);
                }
            }
        }
        Err(MarkerError::QueryHeapExhausted)
    }

    // Releases a previously allocated query slot back to the heap.
    pub fn free_query(&mut self, query_idx: u32) -> Result<(), MarkerError> {
        let idx = query_idx as usize;
        if idx >= self.capacity {
            return Err(MarkerError::InvalidQueryIndex);
        }
        let word_idx = idx / 64;
        let bit_idx = idx % 64;
        if (self.allocated_bitmap[word_idx] & (1 << bit_idx)) == 0 {
            return Err(MarkerError::InvalidQueryIndex);
        }
        self.allocated_bitmap[word_idx] &= !(1 << bit_idx);
        self.timestamps[idx] = 0;
        self.active_count -= 1;
        Ok(())
    }

    // Simulates or records a hardware GPU timestamp into the specified slot.
    pub fn record_timestamp(&mut self, query_idx: u32, gpu_tick: u64) -> Result<(), MarkerError> {
        let idx = query_idx as usize;
        if idx >= self.capacity {
            return Err(MarkerError::InvalidQueryIndex);
        }
        let word_idx = idx / 64;
        let bit_idx = idx % 64;
        if (self.allocated_bitmap[word_idx] & (1 << bit_idx)) == 0 {
            return Err(MarkerError::InvalidQueryIndex);
        }
        self.timestamps[idx] = gpu_tick;
        Ok(())
    }

    // Reads the captured timestamp for the query slot.
    pub fn read_timestamp(&self, query_idx: u32) -> Result<u64, MarkerError> {
        let idx = query_idx as usize;
        if idx >= self.capacity {
            return Err(MarkerError::InvalidQueryIndex);
        }
        Ok(self.timestamps[idx])
    }

    // Resolves a range of query timestamps into a destination readback buffer.
    pub fn resolve_query_data(
        &self,
        start_idx: u32,
        count: u32,
        destination: &mut [u64],
    ) -> Result<(), MarkerError> {
        let start = start_idx as usize;
        let n = count as usize;
        if start + n > self.capacity || destination.len() < n {
            return Err(MarkerError::InvalidQueryIndex);
        }
        destination[..n].copy_from_slice(&self.timestamps[start..start + n]);
        Ok(())
    }

    // Returns the total count of currently allocated queries in the heap.
    #[inline]
    pub fn active_count(&self) -> usize {
        self.active_count
    }
}

// Calibrates GPU timeline clock counters against CPU RDTSC cycles for synchronized profiling.
pub struct GpuCpuTimestampCalibrator {
    gpu_frequency_hz: u64,
    cpu_frequency_hz: u64,
    base_gpu_timestamp: u64,
    base_cpu_rdtsc: u64,
}

impl GpuCpuTimestampCalibrator {
    // Initializes the timestamp calibrator with hardware clock frequencies.
    pub const fn new(gpu_frequency_hz: u64, cpu_frequency_hz: u64) -> Self {
        Self {
            gpu_frequency_hz,
            cpu_frequency_hz,
            base_gpu_timestamp: 0,
            base_cpu_rdtsc: 0,
        }
    }

    // Synchronizes the baseline timeline correlation points between GPU and CPU.
    pub fn calibrate(&mut self, gpu_timestamp: u64, cpu_rdtsc: u64) {
        self.base_gpu_timestamp = gpu_timestamp;
        self.base_cpu_rdtsc = cpu_rdtsc;
    }

    // Converts raw GPU clock ticks to nanoseconds based on GPU clock frequency.
    pub fn gpu_ticks_to_nanoseconds(&self, ticks: u64) -> u64 {
        if self.gpu_frequency_hz == 0 {
            return 0;
        }
        ((ticks as u128 * 1_000_000_000) / self.gpu_frequency_hz as u128) as u64
    }

    // Converts GPU tick delta into equivalent CPU RDTSC cycle delta.
    pub fn gpu_delta_to_cpu_cycles(&self, gpu_delta_ticks: u64) -> u64 {
        if self.gpu_frequency_hz == 0 {
            return 0;
        }
        ((gpu_delta_ticks as u128 * self.cpu_frequency_hz as u128)
            / self.gpu_frequency_hz as u128) as u64
    }

    // Projects a GPU timeline timestamp to CPU RDTSC domain using baseline calibration.
    pub fn correlate_gpu_timestamp_to_cpu_rdtsc(&self, gpu_timestamp: u64) -> u64 {
        if gpu_timestamp >= self.base_gpu_timestamp {
            let delta = gpu_timestamp - self.base_gpu_timestamp;
            self.base_cpu_rdtsc.saturating_add(self.gpu_delta_to_cpu_cycles(delta))
        } else {
            let delta = self.base_gpu_timestamp - gpu_timestamp;
            self.base_cpu_rdtsc.saturating_sub(self.gpu_delta_to_cpu_cycles(delta))
        }
    }

    // Projects a CPU RDTSC tick value back to the GPU timestamp domain.
    pub fn correlate_cpu_rdtsc_to_gpu_timestamp(&self, cpu_rdtsc: u64) -> u64 {
        if self.cpu_frequency_hz == 0 {
            return 0;
        }
        if cpu_rdtsc >= self.base_cpu_rdtsc {
            let delta = cpu_rdtsc - self.base_cpu_rdtsc;
            let gpu_delta = ((delta as u128 * self.gpu_frequency_hz as u128)
                / self.cpu_frequency_hz as u128) as u64;
            self.base_gpu_timestamp.saturating_add(gpu_delta)
        } else {
            let delta = self.base_cpu_rdtsc - cpu_rdtsc;
            let gpu_delta = ((delta as u128 * self.gpu_frequency_hz as u128)
                / self.cpu_frequency_hz as u128) as u64;
            self.base_gpu_timestamp.saturating_sub(gpu_delta)
        }
    }
}

// Maximum number of draw pass performance records stored in the instrumentation ring buffer.
pub const MAX_PASS_INSTRUMENTATION_CAPACITY: usize = 128;

// Per-draw pass performance telemetry capturing GPU timeline, draw counts, and pipeline metrics.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DrawPassMetrics {
    pub pass_id: u32,
    pub pass_name: MarkerName,
    pub draw_call_count: u32,
    pub dispatch_count: u32,
    pub primitive_count: u64,
    pub vertex_invocations: u64,
    pub pixel_invocations: u64,
    pub vram_read_bytes: u64,
    pub vram_write_bytes: u64,
    pub start_query_idx: u32,
    pub end_query_idx: u32,
    pub duration_gpu_ticks: u64,
    pub duration_nanoseconds: u64,
    pub color: PixColor,
    pub completed: bool,
}

impl DrawPassMetrics {
    // Creates an empty draw pass metric record.
    pub const fn empty() -> Self {
        Self {
            pass_id: 0,
            pass_name: MarkerName::empty(),
            draw_call_count: 0,
            dispatch_count: 0,
            primitive_count: 0,
            vertex_invocations: 0,
            pixel_invocations: 0,
            vram_read_bytes: 0,
            vram_write_bytes: 0,
            start_query_idx: 0,
            end_query_idx: 0,
            duration_gpu_ticks: 0,
            duration_nanoseconds: 0,
            color: PixColor::DEFAULT,
            completed: false,
        }
    }
}

// Non-stalling per-draw pass performance instrumentation ring buffer.
pub struct InstrumentationRingBuffer {
    records: [DrawPassMetrics; MAX_PASS_INSTRUMENTATION_CAPACITY],
    head: usize,
    tail: usize,
    count: usize,
    total_recorded_passes: u64,
}

impl InstrumentationRingBuffer {
    // Initializes a new draw pass instrumentation ring buffer.
    pub const fn new() -> Self {
        Self {
            records: [DrawPassMetrics::empty(); MAX_PASS_INSTRUMENTATION_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
            total_recorded_passes: 0,
        }
    }

    // Begins instrumentation of a draw pass, allocating a ring slot without GPU pipeline stalls.
    pub fn begin_pass(
        &mut self,
        pass_id: u32,
        name: &str,
        color: PixColor,
        start_query_idx: u32,
    ) -> usize {
        let slot = self.tail;
        if self.count == MAX_PASS_INSTRUMENTATION_CAPACITY {
            // Non-stalling overwrite: advance head pointer to drop oldest record.
            self.head = (self.head + 1) % MAX_PASS_INSTRUMENTATION_CAPACITY;
        } else {
            self.count += 1;
        }
        self.tail = (self.tail + 1) % MAX_PASS_INSTRUMENTATION_CAPACITY;
        self.total_recorded_passes += 1;

        let mut metric = DrawPassMetrics::empty();
        metric.pass_id = pass_id;
        metric.pass_name = MarkerName::from_str(name);
        metric.color = color;
        metric.start_query_idx = start_query_idx;
        metric.completed = false;

        self.records[slot] = metric;
        slot
    }

    // Ends instrumentation for an active draw pass, recording draw counts and pipeline counters.
    #[allow(clippy::too_many_arguments)]
    pub fn end_pass(
        &mut self,
        slot: usize,
        end_query_idx: u32,
        draw_calls: u32,
        dispatches: u32,
        primitives: u64,
        vertex_invocations: u64,
        pixel_invocations: u64,
        vram_read_bytes: u64,
        vram_write_bytes: u64,
    ) -> Result<(), MarkerError> {
        if slot >= MAX_PASS_INSTRUMENTATION_CAPACITY {
            return Err(MarkerError::InvalidQueryIndex);
        }
        let record = &mut self.records[slot];
        record.end_query_idx = end_query_idx;
        record.draw_call_count = draw_calls;
        record.dispatch_count = dispatches;
        record.primitive_count = primitives;
        record.vertex_invocations = vertex_invocations;
        record.pixel_invocations = pixel_invocations;
        record.vram_read_bytes = vram_read_bytes;
        record.vram_write_bytes = vram_write_bytes;
        record.completed = true;
        Ok(())
    }

    // Resolves asynchronous query timestamps into pass execution duration without CPU waiting.
    pub fn resolve_pass_timing(
        &mut self,
        slot: usize,
        heap: &D3d12TimestampQueryHeap,
        calibrator: &GpuCpuTimestampCalibrator,
    ) -> Result<(), MarkerError> {
        if slot >= MAX_PASS_INSTRUMENTATION_CAPACITY {
            return Err(MarkerError::InvalidQueryIndex);
        }
        let record = &mut self.records[slot];
        if !record.completed {
            return Ok(());
        }

        let t_start = heap.read_timestamp(record.start_query_idx)?;
        let t_end = heap.read_timestamp(record.end_query_idx)?;
        let delta_ticks = if t_end >= t_start {
            t_end - t_start
        } else {
            0
        };

        record.duration_gpu_ticks = delta_ticks;
        record.duration_nanoseconds = calibrator.gpu_ticks_to_nanoseconds(delta_ticks);
        Ok(())
    }

    // Returns a reference to the pass metrics record at the specified ring slot.
    #[inline]
    pub fn get_record(&self, slot: usize) -> Option<&DrawPassMetrics> {
        if slot < MAX_PASS_INSTRUMENTATION_CAPACITY && self.records[slot].completed {
            Some(&self.records[slot])
        } else {
            None
        }
    }

    // Returns the total number of valid records currently stored in the ring buffer.
    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }

    // Returns the cumulative count of passes processed across the lifetime of the engine.
    #[inline]
    pub fn total_recorded_passes(&self) -> u64 {
        self.total_recorded_passes
    }
}
