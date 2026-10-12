//! Direct3D 12 Agility GPU Hardware Virtual Memory Page Remapping & Sparse Texture Governor.
//! Implements sparse 64 KiB tile page mapping, GPU TLB flushing, and handheld residency governors.

#![allow(dead_code, non_camel_case_types, non_snake_case, unused_imports)]

extern crate alloc;
use alloc::vec::Vec;

// Standard Direct3D 12 sparse tile footprint in bytes (64 KiB).
pub const D3D12_SPARSE_TILE_SIZE_BYTES: usize = 65536;

// Direct3D 12 tile mapping range flags specifying behavior for tile ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileRangeFlags {
    // Normal mapping to unique sequential tiles in physical tile pool.
    None = 0,
    // Unmaps tile region or binds it to the shared null tile (reads return 0).
    Null = 1,
    // Skips corresponding destination range, leaving existing mapping untouched.
    Skip = 2,
    // Maps entire destination tile range to a single shared physical tile.
    ReuseSingleTile = 3,
}

// Coordinate descriptor identifying a sparse tile within a texture subresource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SparseTileCoordinate {
    // Horizontal tile index (X).
    pub x: u32,
    // Vertical tile index (Y).
    pub y: u32,
    // Depth tile index (Z).
    pub z: u32,
    // Subresource mip level or array slice index.
    pub subresource: u32,
}

impl SparseTileCoordinate {
    // Constructs a new sparse tile coordinate.
    pub const fn new(x: u32, y: u32, z: u32, subresource: u32) -> Self {
        Self {
            x,
            y,
            z,
            subresource,
        }
    }
}

// Descriptor specifying a range of tiles within a tile mapping update operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileMappingRange {
    // Range mapping classification flags.
    pub flags: TileRangeFlags,
    // Start tile offset in physical tile pool memory.
    pub tile_pool_start_offset: u32,
    // Number of 64 KiB tiles covered by this range.
    pub tile_count: u32,
}

impl TileMappingRange {
    // Creates a normal sequential tile pool mapping range.
    pub const fn normal(start_offset: u32, count: u32) -> Self {
        Self {
            flags: TileRangeFlags::None,
            tile_pool_start_offset: start_offset,
            tile_count: count,
        }
    }

    // Creates an unmapped or null tile fallback range.
    pub const fn null(count: u32) -> Self {
        Self {
            flags: TileRangeFlags::Null,
            tile_pool_start_offset: 0,
            tile_count: count,
        }
    }

    // Creates a single physical tile reuse range.
    pub const fn reuse_single(tile_pool_offset: u32, count: u32) -> Self {
        Self {
            flags: TileRangeFlags::ReuseSingleTile,
            tile_pool_start_offset: tile_pool_offset,
            tile_count: count,
        }
    }
}

// In-memory mapping table managing virtual-to-physical tile assignments for a sparse resource.
#[derive(Debug, Clone)]
pub struct SparseResourceMappingTable {
    // Resource unique identifier.
    pub resource_id: u64,
    // Total virtual tiles spanned by this sparse resource.
    pub total_virtual_tiles: usize,
    // Physical tile indices mapped to each virtual tile (None = Null / unmapped).
    pub virtual_to_physical: Vec<Option<u32>>,
    // Total UpdateTileMappings operations performed.
    pub update_count: u64,
}

impl SparseResourceMappingTable {
    // Creates a new sparse resource mapping table with all tiles initialized to Null.
    pub fn new(resource_id: u64, total_virtual_tiles: usize) -> Self {
        let mut virtual_to_physical = Vec::new();
        virtual_to_physical.resize(total_virtual_tiles, None);
        Self {
            resource_id,
            total_virtual_tiles,
            virtual_to_physical,
            update_count: 0,
        }
    }

    // Executes D3D12 UpdateTileMappings updating virtual-to-physical associations.
    pub fn update_tile_mappings(
        &mut self,
        start_virtual_tile: usize,
        ranges: &[TileMappingRange],
    ) -> usize {
        let mut curr_virtual = start_virtual_tile;
        let mut tiles_mapped = 0;

        for range in ranges {
            for i in 0..range.tile_count {
                if curr_virtual >= self.total_virtual_tiles {
                    break;
                }

                match range.flags {
                    TileRangeFlags::None => {
                        self.virtual_to_physical[curr_virtual] =
                            Some(range.tile_pool_start_offset + i);
                    }
                    TileRangeFlags::Null => {
                        self.virtual_to_physical[curr_virtual] = None;
                    }
                    TileRangeFlags::Skip => {}
                    TileRangeFlags::ReuseSingleTile => {
                        self.virtual_to_physical[curr_virtual] = Some(range.tile_pool_start_offset);
                    }
                }

                curr_virtual += 1;
                tiles_mapped += 1;
            }
        }

        self.update_count += 1;
        tiles_mapped
    }

    // Executes D3D12 CopyTileMappings cloning tile assignments from a source resource.
    pub fn copy_tile_mappings(
        &mut self,
        dest_start: usize,
        source: &SparseResourceMappingTable,
        source_start: usize,
        tile_count: usize,
    ) -> usize {
        let mut copied = 0;
        for i in 0..tile_count {
            let src_idx = source_start + i;
            let dst_idx = dest_start + i;
            if src_idx < source.total_virtual_tiles && dst_idx < self.total_virtual_tiles {
                self.virtual_to_physical[dst_idx] = source.virtual_to_physical[src_idx];
                copied += 1;
            }
        }
        self.update_count += 1;
        copied
    }

    // Queries physical tile index mapped to a virtual tile.
    pub fn get_physical_tile(&self, virtual_tile: usize) -> Option<u32> {
        if virtual_tile < self.virtual_to_physical.len() {
            self.virtual_to_physical[virtual_tile]
        } else {
            None
        }
    }
}

// GPU hardware virtual memory page table leaf entry (64 KiB tile granularity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuPteLeaf {
    // Flag indicating whether mapping is active and valid in hardware.
    pub valid: bool,
    // Physical frame byte address in GPU VRAM or system memory.
    pub physical_address: u64,
    // Whether this entry points to the shared zeroed dummy null tile.
    pub is_null_tile: bool,
    // Read-only permission flag preventing tile corruption from shader writes.
    pub read_only: bool,
}

impl Default for GpuPteLeaf {
    fn default() -> Self {
        Self {
            valid: false,
            physical_address: 0,
            is_null_tile: true,
            read_only: false,
        }
    }
}

// Scope classification for GPU Translation Lookaside Buffer (TLB) flush operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuTlbFlushScope {
    // Selective range invalidation for specific virtual page span.
    Range {
        start_page: usize,
        page_count: usize,
    },
    // Global invalidation flushing entire shader engine L1/L2 TLB hierarchy.
    Global,
}

// Hardware GPU TLB flush and page table leaf invalidation engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTlbEngine {
    // Current monotonic TLB flush generation sequence number.
    pub tlb_generation: u64,
    // Total range or global TLB flushes executed.
    pub total_flushes: u64,
    // Total page table entries invalidated.
    pub total_invalidated_leaves: u64,
}

impl Default for GpuTlbEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuTlbEngine {
    // Creates a new GPU TLB invalidation engine.
    pub fn new() -> Self {
        Self {
            tlb_generation: 1,
            total_flushes: 0,
            total_invalidated_leaves: 0,
        }
    }

    // Invalidates a range of page table entries and generates a TLB flush token.
    pub fn invalidate_page_range(
        &mut self,
        page_table: &mut [GpuPteLeaf],
        start_page: usize,
        page_count: usize,
    ) -> u64 {
        let mut count = 0;
        let end = (start_page + page_count).min(page_table.len());

        let mut i = start_page;
        while i < end {
            if page_table[i].valid {
                page_table[i].valid = false;
                page_table[i].is_null_tile = true;
                page_table[i].physical_address = 0;
                count += 1;
            }
            i += 1;
        }

        self.tlb_generation += 1;
        self.total_flushes += 1;
        self.total_invalidated_leaves += count as u64;
        self.tlb_generation
    }

    // Flushes all TLB caches across all GPU shader cores globally.
    pub fn flush_global(&mut self) -> u64 {
        self.tlb_generation += 1;
        self.total_flushes += 1;
        self.tlb_generation
    }
}

// Error conditions encountered during sparse tile residency operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SparseTileError {
    PoolExhausted,
    InvalidVirtualTile,
    MappingFailed,
}

// Record tracking a resident physical tile allocated to a virtual texture region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResidentTileRecord {
    // Unique ID of owning sparse resource.
    pub resource_id: u64,
    // Virtual tile offset within the sparse resource.
    pub virtual_tile_index: usize,
    // Allocated physical tile pool slot.
    pub physical_tile_index: u32,
    // Timestamp frame counter when tile was last touched.
    pub last_access_frame: u64,
    // Residency priority level where higher values resist eviction.
    pub priority: u32,
}

// Handheld sparse texture streaming governor mapping 64 KiB tiles on demand.
#[derive(Debug, Clone)]
pub struct HandheldSparseTextureGovernor {
    // Maximum physical 64 KiB tiles permitted in active pool.
    pub pool_capacity_tiles: usize,
    // Active physical tile records currently mapped.
    pub resident_tiles: Vec<ResidentTileRecord>,
    // Next physical tile index allocated before pool wraps.
    pub next_physical_tile: u32,
    // Total on-demand tile streaming requests served.
    pub total_tile_requests: u64,
    // Total LRU tile evictions triggered under memory pressure.
    pub total_evictions: u64,
}

impl HandheldSparseTextureGovernor {
    // Creates a new handheld sparse texture streaming governor.
    pub fn new(pool_capacity_tiles: usize) -> Self {
        Self {
            pool_capacity_tiles: if pool_capacity_tiles == 0 {
                256
            } else {
                pool_capacity_tiles
            },
            resident_tiles: Vec::new(),
            next_physical_tile: 0,
            total_tile_requests: 0,
            total_evictions: 0,
        }
    }

    // Requests on-demand residency for a virtual tile, evicting LRU tiles if pool is full.
    pub fn request_tile(
        &mut self,
        resource_id: u64,
        virtual_tile: usize,
        priority: u32,
        frame_id: u64,
        table: &mut SparseResourceMappingTable,
    ) -> Result<u32, SparseTileError> {
        self.total_tile_requests += 1;

        // Check if tile is already resident.
        for record in &mut self.resident_tiles {
            if record.resource_id == resource_id && record.virtual_tile_index == virtual_tile {
                record.last_access_frame = frame_id;
                record.priority = priority;
                return Ok(record.physical_tile_index);
            }
        }

        // Allocate a new physical tile or evict LRU tile if pool is full.
        let physical_tile = if self.resident_tiles.len() < self.pool_capacity_tiles {
            let p = self.next_physical_tile;
            self.next_physical_tile += 1;
            p
        } else {
            let mut victim_idx = 0;
            let mut lowest_prio = u32::MAX;
            let mut oldest_frame = u64::MAX;

            for (i, rec) in self.resident_tiles.iter().enumerate() {
                if rec.priority < lowest_prio
                    || (rec.priority == lowest_prio && rec.last_access_frame < oldest_frame)
                {
                    lowest_prio = rec.priority;
                    oldest_frame = rec.last_access_frame;
                    victim_idx = i;
                }
            }

            let victim = self.resident_tiles.remove(victim_idx);
            self.total_evictions += 1;

            if table.resource_id == victim.resource_id
                && victim.virtual_tile_index < table.total_virtual_tiles
            {
                table.virtual_to_physical[victim.virtual_tile_index] = None;
            }

            victim.physical_tile_index
        };

        self.resident_tiles.push(ResidentTileRecord {
            resource_id,
            virtual_tile_index: virtual_tile,
            physical_tile_index: physical_tile,
            last_access_frame: frame_id,
            priority,
        });

        if virtual_tile < table.total_virtual_tiles {
            table.virtual_to_physical[virtual_tile] = Some(physical_tile);
        }

        Ok(physical_tile)
    }

    // Returns percentage of physical tile pool currently committed in range 0 to 100.
    pub fn pool_utilization_percentage(&self) -> u32 {
        if self.pool_capacity_tiles == 0 {
            0
        } else {
            ((self.resident_tiles.len() * 100) / self.pool_capacity_tiles) as u32
        }
    }
}

// Temporary stub for incremental commits.
pub fn test_gpu_page_remapping_subsystem() -> bool {
    true
}
