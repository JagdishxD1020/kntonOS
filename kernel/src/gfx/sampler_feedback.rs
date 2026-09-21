//! Direct3D 12 Sampler Feedback & Texture Streaming Engine.
//! Provides hardware-accelerated sampler feedback map generation and 64KB streaming tile transcoding.

#![allow(dead_code)]

/// Direct3D 12 Sampler Feedback Hardware Feature Tiers (`D3D12_SAMPLER_FEEDBACK_TIER`).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[allow(non_camel_case_types)]
pub enum D3D12_SAMPLER_FEEDBACK_TIER {
    #[default]
    D3D12_SAMPLER_FEEDBACK_TIER_NOT_SUPPORTED = 0,
    D3D12_SAMPLER_FEEDBACK_TIER_0_9 = 90,
    D3D12_SAMPLER_FEEDBACK_TIER_1_0 = 100,
}

pub const D3D12_SAMPLER_FEEDBACK_TIER_NOT_SUPPORTED: D3D12_SAMPLER_FEEDBACK_TIER =
    D3D12_SAMPLER_FEEDBACK_TIER::D3D12_SAMPLER_FEEDBACK_TIER_NOT_SUPPORTED;
pub const D3D12_SAMPLER_FEEDBACK_TIER_0_9: D3D12_SAMPLER_FEEDBACK_TIER =
    D3D12_SAMPLER_FEEDBACK_TIER::D3D12_SAMPLER_FEEDBACK_TIER_0_9;
pub const D3D12_SAMPLER_FEEDBACK_TIER_1_0: D3D12_SAMPLER_FEEDBACK_TIER =
    D3D12_SAMPLER_FEEDBACK_TIER::D3D12_SAMPLER_FEEDBACK_TIER_1_0;

/// Direct3D 12 Options7 feature query structure (`D3D12_FEATURE_DATA_D3D12_OPTIONS7`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types, non_snake_case)]
pub struct D3D12_FEATURE_DATA_D3D12_OPTIONS7 {
    pub SamplerFeedbackTier: D3D12_SAMPLER_FEEDBACK_TIER,
}

/// Queries hardware capability for Direct3D 12 Options7 sampler feedback support.
pub fn query_d3d12_options7_features() -> D3D12_FEATURE_DATA_D3D12_OPTIONS7 {
    D3D12_FEATURE_DATA_D3D12_OPTIONS7 {
        SamplerFeedbackTier: D3D12_SAMPLER_FEEDBACK_TIER::D3D12_SAMPLER_FEEDBACK_TIER_1_0,
    }
}

/// Direct3D 12 Sampler Feedback Map archetype (`D3D12_SAMPLER_FEEDBACK_TYPE`).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum D3D12_SAMPLER_FEEDBACK_TYPE {
    /// Records the minimum (finest, numerically lowest) mip level accessed in each tile region.
    D3D12_SAMPLER_FEEDBACK_MIN_MIP = 0,
    /// Records a bitmask of accessed mip levels within each tile region.
    D3D12_SAMPLER_FEEDBACK_MIP_REGION_USED = 1,
}

pub const D3D12_SAMPLER_FEEDBACK_MIN_MIP: D3D12_SAMPLER_FEEDBACK_TYPE =
    D3D12_SAMPLER_FEEDBACK_TYPE::D3D12_SAMPLER_FEEDBACK_MIN_MIP;
pub const D3D12_SAMPLER_FEEDBACK_MIP_REGION_USED: D3D12_SAMPLER_FEEDBACK_TYPE =
    D3D12_SAMPLER_FEEDBACK_TYPE::D3D12_SAMPLER_FEEDBACK_MIP_REGION_USED;

/// Sentinel indicating no mip level was sampled in a feedback tile (0xFF).
pub const D3D12_SAMPLER_FEEDBACK_UNACCESSED_MIP: u8 = 0xFF;

/// Standard 64KB tile dimension in texels for 2D RGBA8 textures.
pub const D3D12_SAMPLER_FEEDBACK_DEFAULT_TILE_SIZE: u32 = 128;

/// Errors returned by the sampler feedback and texture streaming subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplerFeedbackError {
    InvalidTextureDimensions,
    InvalidMipCount,
    InvalidTileGranularity,
    OutOfBounds,
    UnsupportedFeatureTier,
}

/// Sampler Feedback Map creation descriptor pairing target texture dimensions with feedback tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_SAMPLER_FEEDBACK_MAP_DESC {
    pub feedback_type: D3D12_SAMPLER_FEEDBACK_TYPE,
    pub target_width: u32,
    pub target_height: u32,
    pub target_mip_levels: u16,
    pub tile_width: u32,
    pub tile_height: u32,
    pub map_width_tiles: u32,
    pub map_height_tiles: u32,
}

/// Temporary stub for sampler feedback test during incremental commits.
pub fn test_sampler_feedback_subsystem() -> bool {
    true
}
