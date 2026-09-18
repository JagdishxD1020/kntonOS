//! Direct3D 12 Low-Latency Frame Pacing & Dynamic Resolution Scaling (DRS) Governor.
//! Provides frame budget evaluation, GPU timestamp delta tracking, and hysteretic resolution scaling.

#![allow(dead_code)]

extern crate alloc;
#[allow(unused_imports)]
use alloc::vec::Vec;

// Frame budget targets for fixed-rate and handheld gaming presentation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum D3D12_DRS_FRAME_BUDGET {
    Target120Fps, // 8333 microseconds (8.33ms)
    Target90Fps,  // 11111 microseconds (11.11ms)
    Target60Fps,  // 16666 microseconds (16.67ms)
    Target40Fps,  // 25000 microseconds (25.0ms - Steam Deck 40Hz)
    Target30Fps,  // 33333 microseconds (33.33ms)
    Custom(u32),  // Custom budget in microseconds
}

impl D3D12_DRS_FRAME_BUDGET {
    // Target duration in microseconds
    pub const fn target_microseconds(&self) -> u32 {
        match *self {
            Self::Target120Fps => 8333,
            Self::Target90Fps => 11111,
            Self::Target60Fps => 16666,
            Self::Target40Fps => 25000,
            Self::Target30Fps => 33333,
            Self::Custom(us) => us,
        }
    }

    // Safety headroom threshold (e.g. 90% of budget) triggering downscaling
    pub const fn upper_threshold_microseconds(&self) -> u32 {
        (self.target_microseconds() * 95) / 100
    }

    // Recovery threshold (e.g. 75% of budget) allowing upscaling
    pub const fn lower_threshold_microseconds(&self) -> u32 {
        (self.target_microseconds() * 75) / 100
    }
}

// Execution bottleneck classification between CPU generation and GPU rasterization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum D3D12_DRS_PIPELINE_BOTTLENECK {
    CpuBound,
    GpuBound,
    Balanced,
}

// Rolling microsecond timestamp delta tracker measuring CPU and GPU execution intervals
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_GPU_TIMESTAMP_DELTA_TRACKER {
    pub budget: D3D12_DRS_FRAME_BUDGET,
    pub history_gpu_us: [u32; 8],
    pub history_cpu_us: [u32; 8],
    pub history_cursor: usize,
    pub history_count: usize,
}

impl D3D12_GPU_TIMESTAMP_DELTA_TRACKER {
    // Construct tracker with frame budget
    pub const fn new(budget: D3D12_DRS_FRAME_BUDGET) -> Self {
        Self {
            budget,
            history_gpu_us: [0; 8],
            history_cpu_us: [0; 8],
            history_cursor: 0,
            history_count: 0,
        }
    }

    // Record frame timestamp deltas in microseconds
    pub fn record_frame(&mut self, cpu_duration_us: u32, gpu_duration_us: u32) {
        self.history_cpu_us[self.history_cursor] = cpu_duration_us;
        self.history_gpu_us[self.history_cursor] = gpu_duration_us;
        self.history_cursor = (self.history_cursor + 1) % 8;
        if self.history_count < 8 {
            self.history_count += 1;
        }
    }

    // Rolling average GPU duration in microseconds
    pub fn average_gpu_duration_us(&self) -> u32 {
        if self.history_count == 0 {
            return 0;
        }
        let sum: u32 = self.history_gpu_us.iter().take(self.history_count).sum();
        sum / (self.history_count as u32)
    }

    // Rolling average CPU duration in microseconds
    pub fn average_cpu_duration_us(&self) -> u32 {
        if self.history_count == 0 {
            return 0;
        }
        let sum: u32 = self.history_cpu_us.iter().take(self.history_count).sum();
        sum / (self.history_count as u32)
    }

    // Classify pipeline bottleneck
    pub fn evaluate_bottleneck(&self) -> D3D12_DRS_PIPELINE_BOTTLENECK {
        let avg_gpu = self.average_gpu_duration_us();
        let avg_cpu = self.average_cpu_duration_us();

        if avg_gpu > self.budget.target_microseconds() && avg_gpu > avg_cpu {
            D3D12_DRS_PIPELINE_BOTTLENECK::GpuBound
        } else if avg_cpu > self.budget.target_microseconds() && avg_cpu > avg_gpu {
            D3D12_DRS_PIPELINE_BOTTLENECK::CpuBound
        } else {
            D3D12_DRS_PIPELINE_BOTTLENECK::Balanced
        }
    }
}

// Discrete DRS resolution scale percentage steps (50% to 100%)
pub const D3D12_DRS_SCALE_STEPS: [u32; 7] = [50, 58, 67, 75, 83, 91, 100];

// Dynamic viewport and scissor scaler with hysteretic thresholding to prevent oscillation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_DRS_VIEWPORT_SCALER {
    pub native_width: u32,
    pub native_height: u32,
    pub current_step_index: usize,
    pub consecutive_over_budget_frames: u32,
    pub consecutive_under_budget_frames: u32,
    pub downscale_threshold_frames: u32,
    pub upscale_threshold_frames: u32,
}

impl D3D12_DRS_VIEWPORT_SCALER {
    // Construct resolution scaler initialized to 100% native resolution
    pub const fn new(native_width: u32, native_height: u32) -> Self {
        Self {
            native_width,
            native_height,
            current_step_index: 6, // 100% native
            consecutive_over_budget_frames: 0,
            consecutive_under_budget_frames: 0,
            downscale_threshold_frames: 2, // Rapid downscale on sustained drop
            upscale_threshold_frames: 10,  // Hysteretic upscale delay prevents oscillation
        }
    }

    // Current render resolution percentage (e.g. 50, 75, 100)
    #[inline]
    pub fn current_scale_percentage(&self) -> u32 {
        D3D12_DRS_SCALE_STEPS[self.current_step_index]
    }

    // Current floating-point render scale factor [0.5, 1.0]
    #[inline]
    pub fn current_scale_factor(&self) -> f32 {
        self.current_scale_percentage() as f32 / 100.0
    }

    // Scaled render dimensions aligned to 2-pixel boundaries
    pub fn scaled_dimensions(&self) -> (u32, u32) {
        let pct = self.current_scale_percentage();
        let w = ((self.native_width * pct) / 100) & !1;
        let h = ((self.native_height * pct) / 100) & !1;
        (w.max(2), h.max(2))
    }

    // Evaluate GPU timestamp telemetry and update resolution scale factor
    pub fn update(&mut self, tracker: &D3D12_GPU_TIMESTAMP_DELTA_TRACKER) -> bool {
        let avg_gpu = tracker.average_gpu_duration_us();
        let upper = tracker.budget.upper_threshold_microseconds();
        let lower = tracker.budget.lower_threshold_microseconds();

        if avg_gpu > upper {
            self.consecutive_under_budget_frames = 0;
            self.consecutive_over_budget_frames += 1;
            if self.consecutive_over_budget_frames >= self.downscale_threshold_frames {
                if self.current_step_index > 0 {
                    self.current_step_index -= 1;
                    self.consecutive_over_budget_frames = 0;
                    return true;
                }
            }
        } else if avg_gpu < lower {
            self.consecutive_over_budget_frames = 0;
            self.consecutive_under_budget_frames += 1;
            if self.consecutive_under_budget_frames >= self.upscale_threshold_frames {
                if self.current_step_index < D3D12_DRS_SCALE_STEPS.len() - 1 {
                    self.current_step_index += 1;
                    self.consecutive_under_budget_frames = 0;
                    return true;
                }
            }
        } else {
            self.consecutive_over_budget_frames = 0;
            self.consecutive_under_budget_frames = 0;
        }

        false
    }

    // Viewport descriptor (X, Y, Width, Height, MinDepth, MaxDepth)
    pub fn viewport(&self) -> (f32, f32, f32, f32, f32, f32) {
        let (w, h) = self.scaled_dimensions();
        (0.0, 0.0, w as f32, h as f32, 0.0, 1.0)
    }

    // Scissor rect descriptor (Left, Top, Right, Bottom)
    pub fn scissor_rect(&self) -> (i32, i32, i32, i32) {
        let (w, h) = self.scaled_dimensions();
        (0, 0, w as i32, h as i32)
    }
}

// Low-latency queue pacer holding CPU submission until GPU completes previous work
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_LOW_LATENCY_QUEUE_PACER {
    pub max_queue_depth: u32,
    pub submitted_cpu_frame: u64,
    pub completed_gpu_frame: u64,
    pub low_latency_mode_enabled: bool,
    pub boost_enabled: bool,
    pub target_frame_budget_us: u32,
    pub total_stalls_injected: u64,
}

impl D3D12_LOW_LATENCY_QUEUE_PACER {
    // Construct pacer with target frame budget and single-frame queue depth clamp
    pub const fn new(target_frame_budget_us: u32) -> Self {
        Self {
            max_queue_depth: 1, // Reflex / Anti-Lag strict 1-frame queue depth
            submitted_cpu_frame: 0,
            completed_gpu_frame: 0,
            low_latency_mode_enabled: true,
            boost_enabled: false,
            target_frame_budget_us,
            total_stalls_injected: 0,
        }
    }

    // Number of frames currently pending execution in the GPU queue
    #[inline]
    pub fn current_queue_depth(&self) -> u32 {
        self.submitted_cpu_frame.saturating_sub(self.completed_gpu_frame) as u32
    }

    // Whether CPU submission should be held back to eliminate input lag buffering
    pub fn should_hold_cpu_submission(&mut self) -> bool {
        if !self.low_latency_mode_enabled {
            return false;
        }

        if self.current_queue_depth() >= self.max_queue_depth {
            self.total_stalls_injected += 1;
            true
        } else {
            false
        }
    }

    // Calculate microsecond pacing sleep delay before sampling input and starting render
    pub fn compute_pacing_sleep_us(&self, estimated_gpu_remaining_us: u32) -> u32 {
        if !self.low_latency_mode_enabled || self.boost_enabled {
            return 0; // In boost mode or when pacing disabled, start immediately
        }

        if self.current_queue_depth() >= self.max_queue_depth {
            // Sleep until GPU finishes remaining work on current frame
            estimated_gpu_remaining_us.min(self.target_frame_budget_us)
        } else {
            0
        }
    }

    // Notify pacer that CPU has submitted a new frame to the GPU queue
    pub fn on_cpu_frame_submit(&mut self) {
        self.submitted_cpu_frame += 1;
    }

    // Notify pacer that GPU timeline fence signaled completion of a frame
    pub fn on_gpu_frame_complete(&mut self, completed_frame: u64) {
        if completed_frame > self.completed_gpu_frame {
            self.completed_gpu_frame = completed_frame;
        }
    }

    // Estimate input-to-display latency reduction in microseconds compared to traditional 3-frame queue
    pub fn estimated_latency_reduction_us(&self) -> u32 {
        // Traditional queue buffers 2 extra frames ahead (2 * budget)
        2 * self.target_frame_budget_us
    }
}

// Precomputed 16-phase 2D Halton (2, 3) low-discrepancy sub-pixel jitter sequence [-0.5, 0.5]
pub const D3D12_HALTON_2_3_JITTER_16: [[f32; 2]; 16] = [
    [0.0, -0.16666667],
    [-0.25, 0.16666667],
    [0.25, -0.3888889],
    [-0.375, -0.055555552],
    [0.125, 0.2777778],
    [-0.125, -0.2777778],
    [0.375, 0.055555552],
    [-0.4375, 0.3888889],
    [0.0625, -0.462963],
    [-0.1875, -0.12962963],
    [0.3125, 0.2037037],
    [-0.3125, -0.35185185],
    [0.1875, -0.018518519],
    [-0.0625, 0.3148148],
    [0.4375, -0.24074075],
    [-0.46875, 0.09259259],
];

// Spatial upscaler reconstruction metadata payload for FSR / DLSS passes
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(non_camel_case_types)]
pub struct D3D12_SPATIAL_UPSCALER_METADATA {
    pub jitter_offset: [f32; 2],
    pub render_scale: [f32; 2],
    pub render_dimensions: (u32, u32),
    pub native_dimensions: (u32, u32),
    pub sharpness: f32,
    pub reset_history: bool,
    pub phase_index: u32,
}

// Spatial upscaler metadata provider coordinating DRS scale with temporal jitter sequences
#[derive(Debug, Clone, PartialEq)]
#[allow(non_camel_case_types)]
pub struct D3D12_DRS_UPSCALER_PROVIDER {
    pub scaler: D3D12_DRS_VIEWPORT_SCALER,
    pub current_phase: u32,
    pub sharpness: f32,
    pub history_reset_pending: bool,
}

impl D3D12_DRS_UPSCALER_PROVIDER {
    // Construct upscaler provider with native display dimensions
    pub fn new(native_width: u32, native_height: u32) -> Self {
        Self {
            scaler: D3D12_DRS_VIEWPORT_SCALER::new(native_width, native_height),
            current_phase: 0,
            sharpness: 0.8,
            history_reset_pending: false,
        }
    }

    // Step DRS governor and generate upscaler reconstruction metadata for current frame
    pub fn generate_frame_metadata(
        &mut self,
        tracker: &D3D12_GPU_TIMESTAMP_DELTA_TRACKER,
    ) -> D3D12_SPATIAL_UPSCALER_METADATA {
        let scale_changed = self.scaler.update(tracker);
        if scale_changed {
            self.history_reset_pending = true;
        }

        let jitter = D3D12_HALTON_2_3_JITTER_16[(self.current_phase % 16) as usize];
        let (render_w, render_h) = self.scaler.scaled_dimensions();
        let scale_factor = self.scaler.current_scale_factor();

        let metadata = D3D12_SPATIAL_UPSCALER_METADATA {
            jitter_offset: jitter,
            render_scale: [scale_factor, scale_factor],
            render_dimensions: (render_w, render_h),
            native_dimensions: (self.scaler.native_width, self.scaler.native_height),
            sharpness: self.sharpness,
            reset_history: self.history_reset_pending,
            phase_index: self.current_phase,
        };

        // History reset consumed for this frame
        self.history_reset_pending = false;
        self.current_phase = (self.current_phase + 1) % 16;

        metadata
    }
}

// Comprehensive verification self-test suite for D3D12 DRS governor and low-latency pacing
pub fn test_drs_governor_subsystem() -> bool {
    crate::serial_write_str("[KNT-GPU-DRS] Initializing Direct3D 12 DRS Governor & Low-Latency Pacing self-test...\r\n");

    // 1. Verify frame budgets and bottleneck classification
    let budget_60 = D3D12_DRS_FRAME_BUDGET::Target60Fps;
    if budget_60.target_microseconds() != 16666
        || budget_60.upper_threshold_microseconds() != 15832
        || budget_60.lower_threshold_microseconds() != 12499
    {
        crate::serial_write_str("[KNT-DRS-FAIL] 60fps frame budget calculations mismatch\r\n");
        return false;
    }

    let mut tracker = D3D12_GPU_TIMESTAMP_DELTA_TRACKER::new(budget_60);
    // Simulate GPU bound state (GPU duration 19000 us > 16666 us target)
    for _ in 0..8 {
        tracker.record_frame(8000, 19000);
    }
    if tracker.average_gpu_duration_us() != 19000
        || tracker.average_cpu_duration_us() != 8000
        || tracker.evaluate_bottleneck() != D3D12_DRS_PIPELINE_BOTTLENECK::GpuBound
    {
        crate::serial_write_str("[KNT-DRS-FAIL] GPU bound bottleneck classification failed\r\n");
        return false;
    }

    // Simulate CPU bound state (CPU duration 20000 us > 16666 us target)
    for _ in 0..8 {
        tracker.record_frame(20000, 8000);
    }
    if tracker.evaluate_bottleneck() != D3D12_DRS_PIPELINE_BOTTLENECK::CpuBound {
        crate::serial_write_str("[KNT-DRS-FAIL] CPU bound bottleneck classification failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Frame budget thresholds and microsecond bottleneck analysis verified\r\n");

    // 2. Verify dynamic viewport scaling and hysteretic stability
    let mut scaler = D3D12_DRS_VIEWPORT_SCALER::new(1920, 1080);
    if scaler.current_scale_percentage() != 100 || scaler.scaled_dimensions() != (1920, 1080) {
        crate::serial_write_str("[KNT-DRS-FAIL] Initial scaler resolution mismatch\r\n");
        return false;
    }

    // Feed GPU bound tracker (19000 us): should require 2 consecutive frames before downscaling
    let mut heavy_tracker = D3D12_GPU_TIMESTAMP_DELTA_TRACKER::new(budget_60);
    for _ in 0..8 {
        heavy_tracker.record_frame(8000, 19000);
    }

    let changed1 = scaler.update(&heavy_tracker);
    if changed1 || scaler.current_scale_percentage() != 100 {
        crate::serial_write_str("[KNT-DRS-FAIL] Downscaled prematurely on single frame drop\r\n");
        return false;
    }

    let changed2 = scaler.update(&heavy_tracker);
    if !changed2 || scaler.current_scale_percentage() != 91 {
        crate::serial_write_str("[KNT-DRS-FAIL] Failed to downscale to 91% on second frame drop\r\n");
        return false;
    }
    let (scaled_w, scaled_h) = scaler.scaled_dimensions();
    if scaled_w != 1746 || scaled_h != 982 {
        crate::serial_write_str("[KNT-DRS-FAIL] 91% scaled dimensions calculation mismatch\r\n");
        return false;
    }

    // Fast-downscale to minimum step (50%)
    for _ in 0..20 {
        scaler.update(&heavy_tracker);
    }
    if scaler.current_scale_percentage() != 50 || scaler.scaled_dimensions() != (960, 540) {
        crate::serial_write_str("[KNT-DRS-FAIL] Failed to clamp downscale at 50% floor\r\n");
        return false;
    }

    // Test hysteretic upscaling: feed light GPU load (10000 us < 12499 us lower threshold)
    let mut light_tracker = D3D12_GPU_TIMESTAMP_DELTA_TRACKER::new(budget_60);
    for _ in 0..8 {
        light_tracker.record_frame(8000, 10000);
    }

    // Frames 1..9 must NOT upscale (prevent oscillation)
    for _ in 0..9 {
        if scaler.update(&light_tracker) {
            crate::serial_write_str("[KNT-DRS-FAIL] Upscaled prematurely before hysteresis threshold reached\r\n");
            return false;
        }
    }
    if scaler.current_scale_percentage() != 50 {
        crate::serial_write_str("[KNT-DRS-FAIL] Scale changed during hysteresis hold period\r\n");
        return false;
    }

    // Frame 10 must trigger upscale to 58%
    if !scaler.update(&light_tracker) || scaler.current_scale_percentage() != 58 {
        crate::serial_write_str("[KNT-DRS-FAIL] Failed to upscale to 58% after 10 stable under-budget frames\r\n");
        return false;
    }
    crate::serial_write_str("  * Dynamic viewport scaling and hysteretic anti-oscillation verified\r\n");

    // 3. Verify low-latency queue pacing (Reflex / Anti-Lag clamp)
    let mut pacer = D3D12_LOW_LATENCY_QUEUE_PACER::new(16666);
    if pacer.current_queue_depth() != 0 || pacer.should_hold_cpu_submission() {
        crate::serial_write_str("[KNT-DRS-FAIL] Initial pacer state should not hold CPU submission\r\n");
        return false;
    }

    // Submit frame 1 (queue depth = 1): should hold back CPU submission to prevent queue buffering
    pacer.on_cpu_frame_submit();
    if pacer.current_queue_depth() != 1 || !pacer.should_hold_cpu_submission() {
        crate::serial_write_str("[KNT-DRS-FAIL] Pacer did not clamp submission at queue depth 1\r\n");
        return false;
    }
    let sleep_us = pacer.compute_pacing_sleep_us(5000);
    if sleep_us != 5000 {
        crate::serial_write_str("[KNT-DRS-FAIL] Computed pacing sleep duration mismatch\r\n");
        return false;
    }

    // GPU completes frame 1 -> queue depth drops to 0, CPU unblocked
    pacer.on_gpu_frame_complete(1);
    if pacer.current_queue_depth() != 0 || pacer.should_hold_cpu_submission() {
        crate::serial_write_str("[KNT-DRS-FAIL] Pacer failed to unblock CPU after GPU frame completion\r\n");
        return false;
    }
    if pacer.estimated_latency_reduction_us() != 33332 {
        crate::serial_write_str("[KNT-DRS-FAIL] Latency reduction metric mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Low-latency queue pacer and Reflex/Anti-Lag latency clamp verified\r\n");

    // 4. Verify spatial upscaler metadata provider and Halton jitter sequence
    let mut upscaler_provider = D3D12_DRS_UPSCALER_PROVIDER::new(1920, 1080);
    let meta0 = upscaler_provider.generate_frame_metadata(&light_tracker);
    if meta0.phase_index != 0 || meta0.render_dimensions != (1920, 1080) {
        crate::serial_write_str("[KNT-DRS-FAIL] Upscaler metadata initial frame mismatch\r\n");
        return false;
    }

    let meta1 = upscaler_provider.generate_frame_metadata(&light_tracker);
    if meta1.phase_index != 1 || meta1.jitter_offset != [-0.25, 0.16666667] {
        crate::serial_write_str("[KNT-DRS-FAIL] Halton (2, 3) jitter phase 1 mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Spatial upscaler metadata provider and Halton jitter sequence verified\r\n");

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-DRS-PASS] Direct3D 12 low-latency frame pacing and dynamic resolution scaling governor verified\r\n");
    true
}
