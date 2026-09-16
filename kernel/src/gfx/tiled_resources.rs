//! Direct3D 12 Tiled Resources & Sparse Virtual Texture Engine.
//! Provides 64 KiB tile coordinate math, tile pool mapping, and sparse residency tracking.

#![allow(dead_code)]

extern crate alloc;
#[allow(unused_imports)]
use alloc::vec::Vec;
#[allow(unused_imports)]
use crate::gfx::tile_compact::D3D12_TILED_RESOURCE_TILE_SIZE_IN_BYTES;

// Direct3D 12 Tiled Resource Hardware Support Tiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum D3D12_TILED_RESOURCES_TIER {
    #[default]
    D3D12_TILED_RESOURCES_TIER_NOT_SUPPORTED = 0,
    D3D12_TILED_RESOURCES_TIER_1 = 1,
    D3D12_TILED_RESOURCES_TIER_2 = 2,
    D3D12_TILED_RESOURCES_TIER_3 = 3,
}

// 64 KiB tile dimensions in texels for a specific surface format and dimension
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_TILE_SHAPE {
    pub width_in_texels: u32,
    pub height_in_texels: u32,
    pub depth_in_texels: u32,
}

impl D3D12_TILE_SHAPE {
    // Construct standard 64 KiB tile shape for 2D textures based on format byte size
    pub fn for_2d_format(bytes_per_texel: u32) -> Self {
        match bytes_per_texel {
            1 => Self { width_in_texels: 256, height_in_texels: 256, depth_in_texels: 1 },
            2 => Self { width_in_texels: 256, height_in_texels: 128, depth_in_texels: 1 },
            4 => Self { width_in_texels: 128, height_in_texels: 128, depth_in_texels: 1 },
            8 => Self { width_in_texels: 128, height_in_texels: 64, depth_in_texels: 1 },
            16 => Self { width_in_texels: 64, height_in_texels: 64, depth_in_texels: 1 },
            _ => Self { width_in_texels: 128, height_in_texels: 128, depth_in_texels: 1 },
        }
    }

    // Construct standard 64 KiB tile shape for 3D volume textures based on format byte size
    pub fn for_3d_format(bytes_per_texel: u32) -> Self {
        match bytes_per_texel {
            1 => Self { width_in_texels: 64, height_in_texels: 32, depth_in_texels: 32 },
            2 => Self { width_in_texels: 32, height_in_texels: 32, depth_in_texels: 32 },
            4 => Self { width_in_texels: 32, height_in_texels: 32, depth_in_texels: 16 },
            8 => Self { width_in_texels: 32, height_in_texels: 16, depth_in_texels: 16 },
            16 => Self { width_in_texels: 16, height_in_texels: 16, depth_in_texels: 16 },
            _ => Self { width_in_texels: 32, height_in_texels: 32, depth_in_texels: 16 },
        }
    }

    // Total number of texels contained in a single 64 KiB tile
    #[inline]
    pub const fn texel_count(&self) -> u32 {
        self.width_in_texels * self.height_in_texels * self.depth_in_texels
    }
}

// Coordinate specifying a tile within a subresource
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_TILED_RESOURCE_COORDINATE {
    pub x: u32,
    pub y: u32,
    pub z: u32,
    pub subresource: u32,
}

impl D3D12_TILED_RESOURCE_COORDINATE {
    // Construct new tiled resource coordinate
    pub const fn new(x: u32, y: u32, z: u32, subresource: u32) -> Self {
        Self { x, y, z, subresource }
    }
}

// Region size specifying a 3D box or linear run of tiles for mapping updates
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_TILE_REGION_SIZE {
    pub num_tiles: u32,
    pub use_box: bool,
    pub width: u32,
    pub height: u16,
    pub depth: u16,
}

impl D3D12_TILE_REGION_SIZE {
    // Create a 1D linear run of tiles
    pub const fn linear(num_tiles: u32) -> Self {
        Self {
            num_tiles,
            use_box: false,
            width: num_tiles,
            height: 1,
            depth: 1,
        }
    }

    // Create a 3D rectangular box region of tiles
    pub const fn r#box(width: u32, height: u16, depth: u16) -> Self {
        Self {
            num_tiles: width * height as u32 * depth as u32,
            use_box: true,
            width,
            height,
            depth,
        }
    }
}
