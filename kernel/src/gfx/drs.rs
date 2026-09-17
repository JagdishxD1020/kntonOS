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
