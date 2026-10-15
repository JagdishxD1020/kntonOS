// Direct3D 12 Agility Enhanced Sync Intervals & Variable Refresh Rate (VRR) Frame Pacing Governor.
// Implements tear-free frame pacing, 1-frame latency regulation, and presentation telemetry.

extern crate alloc;
use alloc::vec::Vec;

// DXGI presentation flags for enhanced synchronization and tearing control.
pub const DXGI_PRESENT_DO_NOT_WAIT: u32 = 0x0000_0001;
pub const DXGI_PRESENT_RESTART: u32 = 0x0000_0002;
pub const DXGI_PRESENT_ALLOW_TEARING: u32 = 0x0000_0200;

// DXGI presentation return status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxgiPresentResult {
    Success,
    WasStillDrawing,
    TearingAllowed,
    DeviceRemoved,
}

// Individual swapchain backbuffer tracking GPU completion fence and timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrrBackbuffer {
    pub buffer_id: usize,
    pub in_flight_fence: u64,
    pub present_timestamp_ns: u64,
    pub is_ready: bool,
}

impl VrrBackbuffer {
    // Construct a new backbuffer instance.
    pub fn new(buffer_id: usize) -> Self {
        Self {
            buffer_id,
            in_flight_fence: 0,
            present_timestamp_ns: 0,
            is_ready: true,
        }
    }
}

// Enhanced DXGI swapchain with variable sync intervals, tearing control, and flip model.
pub struct VrrEnhancedSwapChain {
    pub buffer_count: usize,
    pub current_buffer_index: usize,
    pub buffers: Vec<VrrBackbuffer>,
    pub completed_gpu_fence: u64,
    pub current_gpu_fence: u64,
    pub tearing_supported: bool,
    pub total_presents: u64,
    pub total_tearing_presents: u64,
    pub total_still_drawing_drops: u64,
}

impl VrrEnhancedSwapChain {
    // Construct a new swapchain with specified buffer count and tearing capability.
    pub fn new(buffer_count: usize, tearing_supported: bool) -> Self {
        let count = buffer_count.max(2);
        let mut buffers = Vec::with_capacity(count);
        for i in 0..count {
            buffers.push(VrrBackbuffer::new(i));
        }
        Self {
            buffer_count: count,
            current_buffer_index: 0,
            buffers,
            completed_gpu_fence: 0,
            current_gpu_fence: 0,
            tearing_supported,
            total_presents: 0,
            total_tearing_presents: 0,
            total_still_drawing_drops: 0,
        }
    }

    // Advance GPU fence progress to simulate hardware rendering completion.
    pub fn advance_gpu_fence(&mut self, completed_fence: u64) {
        if completed_fence > self.completed_gpu_fence {
            self.completed_gpu_fence = completed_fence;
            for buf in &mut self.buffers {
                if buf.in_flight_fence <= self.completed_gpu_fence {
                    buf.is_ready = true;
                }
            }
        }
    }

    // Execute Present operation supporting sync intervals and enhanced presentation flags.
    pub fn present(
        &mut self,
        sync_interval: u32,
        flags: u32,
        current_time_ns: u64,
    ) -> DxgiPresentResult {
        let next_index = (self.current_buffer_index + 1) % self.buffer_count;
        let target_buffer = &self.buffers[next_index];

        if (flags & DXGI_PRESENT_DO_NOT_WAIT) != 0 {
            if !target_buffer.is_ready && target_buffer.in_flight_fence > self.completed_gpu_fence {
                self.total_still_drawing_drops += 1;
                return DxgiPresentResult::WasStillDrawing;
            }
        }

        self.current_gpu_fence += 1;
        let new_fence = self.current_gpu_fence;
        self.current_buffer_index = next_index;

        let buf = &mut self.buffers[self.current_buffer_index];
        buf.in_flight_fence = new_fence;
        buf.present_timestamp_ns = current_time_ns;
        buf.is_ready = false;
        self.total_presents += 1;

        if sync_interval == 0 && (flags & DXGI_PRESENT_ALLOW_TEARING) != 0 && self.tearing_supported {
            self.total_tearing_presents += 1;
            DxgiPresentResult::TearingAllowed
        } else {
            DxgiPresentResult::Success
        }
    }

    // Retrieve current backbuffer index.
    pub fn current_index(&self) -> usize {
        self.current_buffer_index
    }
}
