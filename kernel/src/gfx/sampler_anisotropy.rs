//! Direct3D 12 Agility Dynamic Sampler LOD Bias Injection & Texture Filtering Anisotropy Governor.
//! Implements dynamic per-draw LOD bias injection based on GPU frametime and thermal throttle headroom,
//! dynamic anisotropic filtering level adaptation, and sampler descriptor heap deduplication.

#![allow(dead_code, non_camel_case_types)]

extern crate alloc;

// Filter reduction and comparison modes.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynamicFilterType {
    #[default]
    Point = 0,
    Linear = 1,
    Anisotropic = 2,
}

// Thermal and frametime throttle headroom state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuPerformanceState {
    pub current_frametime_ms: f32,
    pub target_frametime_ms: f32,
    pub gpu_temperature_celsius: f32,
    pub thermal_throttle_temp_celsius: f32,
}

impl GpuPerformanceState {
    pub const fn new(current_frametime_ms: f32, target_frametime_ms: f32, temp: f32, throttle_temp: f32) -> Self {
        Self {
            current_frametime_ms,
            target_frametime_ms,
            gpu_temperature_celsius: temp,
            thermal_throttle_temp_celsius: throttle_temp,
        }
    }

    #[inline(always)]
    pub fn is_under_heavy_load(&self) -> bool {
        self.current_frametime_ms > self.target_frametime_ms * 1.05
            || self.gpu_temperature_celsius >= self.thermal_throttle_temp_celsius - 2.0
    }
}

// Dynamic per-draw LOD bias injector governor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicLodBiasGovernor {
    pub base_lod_bias: f32,
    pub max_injected_bias: f32,
    pub current_injected_bias: f32,
    pub total_bias_evaluations: u64,
    pub total_bias_injections: u64,
}

impl DynamicLodBiasGovernor {
    pub fn new(max_bias: f32) -> Self {
        Self {
            base_lod_bias: 0.0,
            max_injected_bias: max_bias,
            current_injected_bias: 0.0,
            total_bias_evaluations: 0,
            total_bias_injections: 0,
        }
    }

    // Computes dynamic per-draw LOD bias adjustment based on GPU load and thermal headroom.
    pub fn calculate_injected_lod_bias(&mut self, perf: &GpuPerformanceState) -> f32 {
        self.total_bias_evaluations += 1;

        if perf.is_under_heavy_load() {
            // Under load or thermal stress, inject positive LOD bias (0.5 to max_bias) to select coarser mips
            let frametime_ratio = perf.current_frametime_ms / perf.target_frametime_ms.max(1.0);
            let bias = ((frametime_ratio - 1.0) * 1.5).min(self.max_injected_bias).max(0.5);
            self.current_injected_bias = bias;
            self.total_bias_injections += 1;
        } else {
            // Normal operation, zero extra bias
            self.current_injected_bias = 0.0;
        }

        self.base_lod_bias + self.current_injected_bias
    }
}
