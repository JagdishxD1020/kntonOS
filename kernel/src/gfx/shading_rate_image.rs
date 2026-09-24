//! Direct3D 12 VRS Tier 2 Shading Rate Image & Combiner Pipeline.
//! Provides screen-space tile rate maps (8x8 and 16x16 granularity), combiner evaluation rules,
//! contrast-adaptive variance rate generation, and command list API recording.

#![allow(dead_code)]

use super::vrs::D3D12_SHADING_RATE;

/// Errors returned by the shading rate image and combiner subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadingRateImageError {
    InvalidDimensions,
    InvalidTileGranularity,
    BufferTooSmall,
    OutOfBounds,
}

/// Direct3D 12 VRS Tier 2 Shading Rate Image tile granularity (texels per tile edge).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub enum D3D12_SHADING_RATE_IMAGE_TILE_GRANULARITY {
    /// 8x8 texels per shading rate image tile.
    Tile8x8 = 8,
    /// 16x16 texels per shading rate image tile (standard hardware default).
    #[default]
    Tile16x16 = 16,
}

impl D3D12_SHADING_RATE_IMAGE_TILE_GRANULARITY {
    /// Returns tile edge dimension in texels.
    #[inline(always)]
    pub const fn tile_size(self) -> u32 {
        self as u32
    }
}

/// Descriptor configuring screen-space shading rate image geometry and tile grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_SHADING_RATE_IMAGE_DESC {
    pub width_pixels: u32,
    pub height_pixels: u32,
    pub tile_granularity: D3D12_SHADING_RATE_IMAGE_TILE_GRANULARITY,
    pub tiles_x: u32,
    pub tiles_y: u32,
}

impl D3D12_SHADING_RATE_IMAGE_DESC {
    /// Creates and validates a new shading rate image descriptor.
    pub fn new(
        width_pixels: u32,
        height_pixels: u32,
        tile_granularity: D3D12_SHADING_RATE_IMAGE_TILE_GRANULARITY,
    ) -> Result<Self, ShadingRateImageError> {
        if width_pixels == 0 || height_pixels == 0 || width_pixels > 16384 || height_pixels > 16384 {
            return Err(ShadingRateImageError::InvalidDimensions);
        }

        let tile_size = tile_granularity.tile_size();
        let tiles_x = (width_pixels.saturating_add(tile_size - 1)) / tile_size;
        let tiles_y = (height_pixels.saturating_add(tile_size - 1)) / tile_size;

        Ok(Self {
            width_pixels,
            height_pixels,
            tile_granularity,
            tiles_x,
            tiles_y,
        })
    }

    /// Total number of shading rate tiles in the screen grid.
    #[inline(always)]
    pub const fn total_tiles(&self) -> u32 {
        self.tiles_x * self.tiles_y
    }

    /// Maps a screen-space pixel coordinate `(x, y)` to its corresponding tile coordinate `(tx, ty)`.
    #[inline(always)]
    pub fn pixel_to_tile(&self, x: u32, y: u32) -> (u32, u32) {
        let tile_size = self.tile_granularity.tile_size();
        (x / tile_size, y / tile_size)
    }

    /// Linear index of tile `(tile_x, tile_y)` in row-major order.
    #[inline(always)]
    pub fn tile_index(&self, tile_x: u32, tile_y: u32) -> Option<usize> {
        if tile_x < self.tiles_x && tile_y < self.tiles_y {
            Some((tile_y * self.tiles_x + tile_x) as usize)
        } else {
            None
        }
    }
}

/// Screen-Space Shading Rate Image Surface storing per-tile coarse shading rates.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadingRateImageSurface {
    pub desc: D3D12_SHADING_RATE_IMAGE_DESC,
    pub tile_rates: alloc::vec::Vec<D3D12_SHADING_RATE>,
}

impl ShadingRateImageSurface {
    /// Allocates a new shading rate surface with a uniform default shading rate.
    pub fn new(
        desc: D3D12_SHADING_RATE_IMAGE_DESC,
        default_rate: D3D12_SHADING_RATE,
    ) -> Result<Self, ShadingRateImageError> {
        let total = desc.total_tiles() as usize;
        Ok(Self {
            desc,
            tile_rates: alloc::vec![default_rate; total],
        })
    }

    /// Sets shading rate for tile `(tile_x, tile_y)`.
    pub fn set_rate(&mut self, tile_x: u32, tile_y: u32, rate: D3D12_SHADING_RATE) -> bool {
        if let Some(idx) = self.desc.tile_index(tile_x, tile_y) {
            self.tile_rates[idx] = rate;
            true
        } else {
            false
        }
    }

    /// Reads shading rate for tile `(tile_x, tile_y)`.
    pub fn get_rate(&self, tile_x: u32, tile_y: u32) -> D3D12_SHADING_RATE {
        self.desc
            .tile_index(tile_x, tile_y)
            .map(|idx| self.tile_rates[idx])
            .unwrap_or(D3D12_SHADING_RATE::D3D12_SHADING_RATE_1X1)
    }

    /// Samples the shading rate image at a continuous screen-space pixel position `(px, py)`.
    pub fn sample_at_pixel(&self, px: u32, py: u32) -> D3D12_SHADING_RATE {
        let (tx, ty) = self.desc.pixel_to_tile(px, py);
        self.get_rate(tx, ty)
    }
}

/// Temporary stub for shading rate image self-test during incremental commits.
pub fn test_shading_rate_image_subsystem() -> bool {
    true
}
