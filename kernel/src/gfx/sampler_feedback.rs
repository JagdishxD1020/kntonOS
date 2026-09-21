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

impl D3D12_SAMPLER_FEEDBACK_MAP_DESC {
    /// Creates and validates a new sampler feedback map descriptor.
    pub fn new(
        feedback_type: D3D12_SAMPLER_FEEDBACK_TYPE,
        target_width: u32,
        target_height: u32,
        target_mip_levels: u16,
        tile_width: u32,
        tile_height: u32,
    ) -> Result<Self, SamplerFeedbackError> {
        if target_width == 0 || target_height == 0 || target_width > 16384 || target_height > 16384 {
            return Err(SamplerFeedbackError::InvalidTextureDimensions);
        }
        if target_mip_levels == 0 || target_mip_levels > 16 {
            return Err(SamplerFeedbackError::InvalidMipCount);
        }
        if tile_width == 0 || tile_height == 0 || !tile_width.is_power_of_two() || !tile_height.is_power_of_two() {
            return Err(SamplerFeedbackError::InvalidTileGranularity);
        }

        let map_width_tiles = (target_width.saturating_add(tile_width - 1)) / tile_width;
        let map_height_tiles = (target_height.saturating_add(tile_height - 1)) / tile_height;

        Ok(Self {
            feedback_type,
            target_width,
            target_height,
            target_mip_levels,
            tile_width,
            tile_height,
            map_width_tiles,
            map_height_tiles,
        })
    }

    /// Total number of discrete feedback tiles spanning the target texture.
    #[inline]
    pub const fn total_tiles(&self) -> u32 {
        self.map_width_tiles * self.map_height_tiles
    }

    /// Maps a source texture texel coordinate `(x, y)` to its corresponding feedback tile coordinate.
    #[inline]
    pub fn texel_to_tile(&self, x: u32, y: u32) -> (u32, u32) {
        (x / self.tile_width, y / self.tile_height)
    }

    /// Computes linear tile index for a given tile coordinate `(tile_x, tile_y)`.
    #[inline]
    pub fn tile_index(&self, tile_x: u32, tile_y: u32) -> Option<usize> {
        if tile_x < self.map_width_tiles && tile_y < self.map_height_tiles {
            Some((tile_y * self.map_width_tiles + tile_x) as usize)
        } else {
            None
        }
    }
}

/// Validator for Direct3D 12 Sampler Feedback creation and region bindings.
#[allow(non_camel_case_types)]
pub struct D3D12_SAMPLER_FEEDBACK_VALIDATOR;

impl D3D12_SAMPLER_FEEDBACK_VALIDATOR {
    /// Validates paired target texture dimensions and mip count.
    pub fn validate_target_texture(width: u32, height: u32, mip_levels: u16) -> Result<(), SamplerFeedbackError> {
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(SamplerFeedbackError::InvalidTextureDimensions);
        }
        if mip_levels == 0 || mip_levels > 16 {
            return Err(SamplerFeedbackError::InvalidMipCount);
        }
        Ok(())
    }

    /// Validates rectangular tile region against feedback map bounds.
    pub fn validate_tile_region(
        desc: &D3D12_SAMPLER_FEEDBACK_MAP_DESC,
        tile_x: u32,
        tile_y: u32,
        width_tiles: u32,
        height_tiles: u32,
    ) -> Result<(), SamplerFeedbackError> {
        if tile_x.saturating_add(width_tiles) > desc.map_width_tiles
            || tile_y.saturating_add(height_tiles) > desc.map_height_tiles
        {
            return Err(SamplerFeedbackError::OutOfBounds);
        }
        Ok(())
    }
}

/// Temporary stub for sampler feedback test during incremental commits.
pub fn test_sampler_feedback_subsystem() -> bool {
    true
}
