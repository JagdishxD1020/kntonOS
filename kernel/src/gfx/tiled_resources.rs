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

// Bitmask flags configuring tile mapping operations
pub const D3D12_TILE_MAPPING_FLAG_NONE: u32 = 0x0;
pub const D3D12_TILE_MAPPING_FLAG_NO_HAZARD: u32 = 0x1;
pub const D3D12_TILE_MAPPING_FLAG_UNMAP: u32 = 0x2;

// Physical memory storage pool holding contiguous 64 KiB tiles
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_TILE_POOL {
    pub pool_id: u32,
    pub tile_count: u32,
    pub allocated_mask: Vec<bool>,
    pub active_mappings: usize,
}

impl D3D12_TILE_POOL {
    // Construct new 64 KiB physical tile pool
    pub fn new(pool_id: u32, tile_count: u32) -> Self {
        Self {
            pool_id,
            tile_count,
            allocated_mask: alloc::vec![false; tile_count as usize],
            active_mappings: 0,
        }
    }

    // Allocate an unused physical tile index
    pub fn allocate_tile(&mut self) -> Option<u32> {
        for (idx, alloc) in self.allocated_mask.iter_mut().enumerate() {
            if !*alloc {
                *alloc = true;
                return Some(idx as u32);
            }
        }
        None
    }

    // Release an allocated physical tile back to the pool
    pub fn free_tile(&mut self, tile_idx: u32) -> bool {
        if (tile_idx as usize) < self.allocated_mask.len() && self.allocated_mask[tile_idx as usize] {
            self.allocated_mask[tile_idx as usize] = false;
            true
        } else {
            false
        }
    }

    // Whether a physical tile index is valid within pool bounds
    #[inline]
    pub fn is_valid_tile(&self, tile_idx: u32) -> bool {
        tile_idx < self.tile_count
    }
}

// Sparse virtual texture page table manager with unmapped null-tile fallback
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_TILED_RESOURCE_MAPPING_MANAGER {
    pub resource_id: u32,
    pub total_virtual_tiles: u32,
    pub tile_table: Vec<Option<u32>>,
    pub null_tile_reads: u64,
    pub null_tile_writes_discarded: u64,
}

impl D3D12_TILED_RESOURCE_MAPPING_MANAGER {
    // Construct new sparse mapping manager initialized to fully unmapped (null) state
    pub fn new(resource_id: u32, total_virtual_tiles: u32) -> Self {
        Self {
            resource_id,
            total_virtual_tiles,
            tile_table: alloc::vec![None; total_virtual_tiles as usize],
            null_tile_reads: 0,
            null_tile_writes_discarded: 0,
        }
    }

    // Update range of virtual tiles with physical pool tiles or unmap
    pub fn update_tile_mappings(
        &mut self,
        start_virtual_tile: u32,
        tile_count: u32,
        pool_start_tile: Option<u32>,
        flags: u32,
    ) -> Result<(), &'static str> {
        let end_tile = start_virtual_tile + tile_count;
        if end_tile > self.total_virtual_tiles {
            return Err("Tile mapping range exceeds resource virtual tile count");
        }

        let is_unmap = (flags & D3D12_TILE_MAPPING_FLAG_UNMAP) != 0 || pool_start_tile.is_none();

        for i in 0..tile_count {
            let virt_idx = (start_virtual_tile + i) as usize;
            if is_unmap {
                self.tile_table[virt_idx] = None;
            } else {
                let phys_idx = pool_start_tile.unwrap() + i;
                self.tile_table[virt_idx] = Some(phys_idx);
            }
        }

        Ok(())
    }

    // Copy tile mappings from source sparse resource
    pub fn copy_tile_mappings(
        &mut self,
        dst_start: u32,
        src_manager: &D3D12_TILED_RESOURCE_MAPPING_MANAGER,
        src_start: u32,
        tile_count: u32,
    ) -> Result<(), &'static str> {
        if dst_start + tile_count > self.total_virtual_tiles {
            return Err("Destination tile range exceeds resource bounds");
        }
        if src_start + tile_count > src_manager.total_virtual_tiles {
            return Err("Source tile range exceeds source resource bounds");
        }

        for i in 0..tile_count {
            let src_idx = (src_start + i) as usize;
            let dst_idx = (dst_start + i) as usize;
            self.tile_table[dst_idx] = src_manager.tile_table[src_idx];
        }

        Ok(())
    }

    // Read 64 KiB tile with null-tile fallback (unmapped reads return 0 without faulting)
    pub fn read_tile(&mut self, virtual_tile: u32, out_buffer: &mut [u8]) -> bool {
        if (virtual_tile as usize) >= self.tile_table.len() {
            out_buffer.fill(0);
            self.null_tile_reads += 1;
            return false;
        }

        match self.tile_table[virtual_tile as usize] {
            Some(phys_tile) => {
                // Mapped tile: populate with deterministic simulated physical tile pattern
                let pattern = (phys_tile as u8).wrapping_add(0x5A);
                out_buffer.fill(pattern);
                true
            }
            None => {
                // Tier 2 null-tile fallback: read returns zero/clear color without page fault
                out_buffer.fill(0);
                self.null_tile_reads += 1;
                false
            }
        }
    }

    // Write 64 KiB tile with null-tile fallback (unmapped writes safely discarded)
    pub fn write_tile(&mut self, virtual_tile: u32, _in_buffer: &[u8]) -> bool {
        if (virtual_tile as usize) >= self.tile_table.len() {
            self.null_tile_writes_discarded += 1;
            return false;
        }

        match self.tile_table[virtual_tile as usize] {
            Some(_) => true,
            None => {
                // Writing to an unmapped tile is safely dropped
                self.null_tile_writes_discarded += 1;
                false
            }
        }
    }

    // Check if a range of virtual tiles is fully resident (SV_IsFullyMapped)
    pub fn is_fully_mapped(&self, start_virtual_tile: u32, tile_count: u32) -> bool {
        let end_tile = (start_virtual_tile + tile_count) as usize;
        if end_tile > self.tile_table.len() {
            return false;
        }
        for i in (start_virtual_tile as usize)..end_tile {
            if self.tile_table[i].is_none() {
                return false;
            }
        }
        true
    }
}
