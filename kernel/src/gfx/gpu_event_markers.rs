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
