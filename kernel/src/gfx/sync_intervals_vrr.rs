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

// VRR display timing profile for handheld displays (e.g., 40 Hz - 120 Hz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrrDisplayProfile {
    pub min_refresh_hz: u32,
    pub max_refresh_hz: u32,
    pub min_frame_time_ns: u64,
    pub max_frame_time_ns: u64,
    pub vrr_active: bool,
}

impl VrrDisplayProfile {
    // Construct display profile with verified refresh rate bounds.
    pub fn new(min_hz: u32, max_hz: u32) -> Self {
        let min_r = min_hz.max(20);
        let max_r = max_hz.max(min_r);
        let min_ft = 1_000_000_000u64 / (max_r as u64);
        let max_ft = 1_000_000_000u64 / (min_r as u64);
        Self {
            min_refresh_hz: min_r,
            max_refresh_hz: max_r,
            min_frame_time_ns: min_ft,
            max_frame_time_ns: max_ft,
            vrr_active: true,
        }
    }
}

// Handheld VRR tear-free frame pacing governor regulating frame deadlines and micro-sleeps.
pub struct HandheldVrrFramePacingGovernor {
    pub profile: VrrDisplayProfile,
    pub history_durations_ns: [u64; 8],
    pub history_index: usize,
    pub history_count: usize,
    pub estimated_frame_time_ns: u64,
    pub target_frame_rate_hz: u32,
    pub last_present_time_ns: u64,
    pub total_pacing_sleeps: u64,
    pub accumulated_sleep_ns: u64,
}

impl HandheldVrrFramePacingGovernor {
    // Construct frame pacing governor with target frame rate and display profile.
    pub fn new(min_hz: u32, max_hz: u32, target_hz: u32) -> Self {
        let profile = VrrDisplayProfile::new(min_hz, max_hz);
        let default_target_ft = 1_000_000_000u64 / (target_hz.max(min_hz).min(max_hz) as u64);
        Self {
            profile,
            history_durations_ns: [default_target_ft; 8],
            history_index: 0,
            history_count: 0,
            estimated_frame_time_ns: default_target_ft,
            target_frame_rate_hz: target_hz,
            last_present_time_ns: 0,
            total_pacing_sleeps: 0,
            accumulated_sleep_ns: 0,
        }
    }

    // Record rendered frame duration into exponential moving average.
    pub fn record_frame_rendered(&mut self, render_duration_ns: u64) {
        self.history_durations_ns[self.history_index] = render_duration_ns;
        self.history_index = (self.history_index + 1) % 8;
        if self.history_count < 8 {
            self.history_count += 1;
        }

        let mut sum = 0u64;
        for i in 0..self.history_count {
            sum += self.history_durations_ns[i];
        }
        self.estimated_frame_time_ns = sum / (self.history_count as u64);
    }

    // Estimate next presentation deadline based on target frame rate and VRR envelope.
    pub fn estimate_next_frame_deadline(&self, frame_start_ns: u64) -> u64 {
        let target_interval = 1_000_000_000u64 / (self.target_frame_rate_hz as u64);
        let clamped_interval = target_interval
            .max(self.profile.min_frame_time_ns)
            .min(self.profile.max_frame_time_ns);
        frame_start_ns + clamped_interval
    }

    // Compute micro-sleep duration to align command submission with scanout window.
    pub fn compute_micro_sleep_ns(&mut self, current_time_ns: u64, predicted_render_ns: u64) -> u64 {
        if self.last_present_time_ns == 0 {
            self.last_present_time_ns = current_time_ns;
            return 0;
        }

        let target_interval = 1_000_000_000u64 / (self.target_frame_rate_hz as u64);
        let target_deadline = self.last_present_time_ns + target_interval;

        if target_deadline > current_time_ns + predicted_render_ns {
            let sleep_needed = target_deadline - (current_time_ns + predicted_render_ns);
            self.total_pacing_sleeps += 1;
            self.accumulated_sleep_ns += sleep_needed;
            sleep_needed
        } else {
            0
        }
    }

    // Update last presentation timestamp.
    pub fn notify_present(&mut self, present_time_ns: u64) {
        self.last_present_time_ns = present_time_ns;
    }
}
