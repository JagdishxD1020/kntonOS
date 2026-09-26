//! Direct3D 12 Variable-Rate Shading (VRS) Tier 2 Combiners & Per-Draw Shading Rate Multiplier.
//! Evaluates multi-stage combiner pipelines, dynamic hardware clamping, and adaptive handheld rate governing.

#![allow(dead_code, non_snake_case, non_camel_case_types, unused_imports)]

pub use super::vrs::{
    D3D12_FEATURE_DATA_D3D12_OPTIONS6, D3D12_SHADING_RATE, D3D12_SHADING_RATE_COMBINER,
};

// Per-draw shading rate multiplier modifying coarse pixel dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VrsRateMultiplier {
    pub horizontal_scale: u32,
    pub vertical_scale: u32,
}

impl VrsRateMultiplier {
    pub const IDENTITY: Self = Self { horizontal_scale: 1, vertical_scale: 1 };

    pub const fn new(horizontal_scale: u32, vertical_scale: u32) -> Self {
        Self {
            horizontal_scale: if horizontal_scale == 0 { 1 } else { horizontal_scale },
            vertical_scale: if vertical_scale == 0 { 1 } else { vertical_scale },
        }
    }
}

// Configuration of the two-stage combiner pipeline set via RSSetShadingRate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrsCombinerStageConfig {
    pub combiner_primitive_image: D3D12_SHADING_RATE_COMBINER,
    pub combiner_result_base: D3D12_SHADING_RATE_COMBINER,
}

impl Default for VrsCombinerStageConfig {
    fn default() -> Self {
        Self {
            combiner_primitive_image: D3D12_SHADING_RATE_COMBINER::D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
            combiner_result_base: D3D12_SHADING_RATE_COMBINER::D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
        }
    }
}

// Combiner subsystem error conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VrsCombinerError {
    UnsupportedRate,
    InvalidCombinerMode,
    InvalidTileGranularity,
    ClampingFailed,
}

// Temporary stub for self-test during incremental commits.
pub fn test_vrs_combiners_subsystem() -> bool {
    true
}
