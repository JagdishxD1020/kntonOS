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

// Tiling layout metrics for an individual subresource (mip level)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_SUBRESOURCE_TILING {
    pub width_in_tiles: u32,
    pub height_in_tiles: u16,
    pub depth_in_tiles: u16,
    pub start_tile_index_in_overall_resource: u32,
}

impl D3D12_SUBRESOURCE_TILING {
    // Total number of 64 KiB tiles spanning this subresource
    #[inline]
    pub fn total_tiles(&self) -> u32 {
        self.width_in_tiles * self.height_in_tiles as u32 * self.depth_in_tiles as u32
    }

    // Convert local (x, y, z) tile coordinate to linear index within this subresource
    pub fn coord_to_local_index(&self, x: u32, y: u32, z: u32) -> Option<u32> {
        if x >= self.width_in_tiles || y >= self.height_in_tiles as u32 || z >= self.depth_in_tiles as u32 {
            return None;
        }
        let slice_tiles = self.width_in_tiles * self.height_in_tiles as u32;
        Some(z * slice_tiles + y * self.width_in_tiles + x)
    }

    // Convert local linear index within subresource to 3D tile coordinate (x, y, z)
    pub fn local_index_to_coord(&self, index: u32) -> Option<(u32, u32, u32)> {
        if index >= self.total_tiles() {
            return None;
        }
        let slice_tiles = self.width_in_tiles * self.height_in_tiles as u32;
        let z = index / slice_tiles;
        let rem = index % slice_tiles;
        let y = rem / self.width_in_tiles;
        let x = rem % self.width_in_tiles;
        Some((x, y, z))
    }
}

// Complete tiled resource descriptor specifying dimensions, format, and tiling geometry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_TILED_RESOURCE_DESC {
    pub tier: D3D12_TILED_RESOURCES_TIER,
    pub width: u32,
    pub height: u32,
    pub depth_or_array_size: u16,
    pub mip_levels: u16,
    pub bytes_per_texel: u32,
    pub sample_count: u32,
    pub is_3d: bool,
}

impl D3D12_TILED_RESOURCE_DESC {
    // Standard 64 KiB tile shape for this resource
    pub fn tile_shape(&self) -> D3D12_TILE_SHAPE {
        if self.is_3d {
            D3D12_TILE_SHAPE::for_3d_format(self.bytes_per_texel)
        } else {
            D3D12_TILE_SHAPE::for_2d_format(self.bytes_per_texel)
        }
    }

    // Texel dimensions for a specific mip level
    pub fn mip_dimensions(&self, mip: u16) -> (u32, u32, u16) {
        let w = (self.width >> mip).max(1);
        let h = (self.height >> mip).max(1);
        let d = if self.is_3d {
            (self.depth_or_array_size >> mip).max(1)
        } else {
            self.depth_or_array_size
        };
        (w, h, d)
    }

    // Calculate subresource tiling metrics for a given mip level
    pub fn calculate_subresource_tiling(&self, mip: u16, start_tile_index: u32) -> D3D12_SUBRESOURCE_TILING {
        let (w, h, d) = self.mip_dimensions(mip);
        let shape = self.tile_shape();

        let w_tiles = (w + shape.width_in_texels - 1) / shape.width_in_texels;
        let h_tiles = (h + shape.height_in_texels - 1) / shape.height_in_texels;
        let d_tiles = if self.is_3d {
            (d as u32 + shape.depth_in_texels - 1) / shape.depth_in_texels
        } else {
            d as u32
        };

        D3D12_SUBRESOURCE_TILING {
            width_in_tiles: w_tiles,
            height_in_tiles: h_tiles as u16,
            depth_in_tiles: d_tiles as u16,
            start_tile_index_in_overall_resource: start_tile_index,
        }
    }

    // Convert full tiled coordinate to overall resource linear tile index
    pub fn coordinate_to_overall_index(&self, coord: &D3D12_TILED_RESOURCE_COORDINATE) -> Option<u32> {
        if coord.subresource >= self.mip_levels as u32 {
            return None;
        }

        let mut current_start = 0u32;
        for m in 0..coord.subresource as u16 {
            let tiling = self.calculate_subresource_tiling(m, current_start);
            current_start += tiling.total_tiles();
        }

        let tiling = self.calculate_subresource_tiling(coord.subresource as u16, current_start);
        let local_idx = tiling.coord_to_local_index(coord.x, coord.y, coord.z)?;
        Some(current_start + local_idx)
    }
}
