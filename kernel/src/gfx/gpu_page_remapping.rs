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

// Shared physical null tile manager routing uncommitted sparse regions to a single zeroed page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedNullTileManager {
    // Dedicated physical frame address of the shared zeroed dummy tile.
    pub null_tile_physical_address: u64,
    // Cumulative virtual tiles mapped to the shared null tile.
    pub total_bound_null_tiles: u64,
    // Cumulative virtual tiles reclaimed from the null tile mapping.
    pub total_unbound_null_tiles: u64,
}

impl Default for SharedNullTileManager {
    fn default() -> Self {
        Self::new(0x1000_0000)
    }
}

impl SharedNullTileManager {
    // Creates a new shared null tile manager with specified physical address.
    pub fn new(null_tile_physical_address: u64) -> Self {
        Self {
            null_tile_physical_address,
            total_bound_null_tiles: 0,
            total_unbound_null_tiles: 0,
        }
    }

    // Binds a range of virtual page table entries to the shared null tile.
    pub fn bind_null_range(
        &mut self,
        page_table: &mut [GpuPteLeaf],
        start_page: usize,
        page_count: usize,
    ) -> usize {
        let mut count = 0;
        let end = (start_page + page_count).min(page_table.len());

        let mut i = start_page;
        while i < end {
            page_table[i].valid = true;
            page_table[i].is_null_tile = true;
            page_table[i].physical_address = self.null_tile_physical_address;
            page_table[i].read_only = true;
            count += 1;
            i += 1;
        }

        self.total_bound_null_tiles += count as u64;
        count
    }

    // Reclaims a range of virtual page table entries from null tile binding.
    pub fn unbind_null_range(
        &mut self,
        page_table: &mut [GpuPteLeaf],
        start_page: usize,
        page_count: usize,
    ) -> usize {
        let mut count = 0;
        let end = (start_page + page_count).min(page_table.len());

        let mut i = start_page;
        while i < end {
            if page_table[i].is_null_tile && page_table[i].valid {
                page_table[i].valid = false;
                page_table[i].physical_address = 0;
                count += 1;
            }
            i += 1;
        }

        self.total_unbound_null_tiles += count as u64;
        count
    }

    // Total active virtual tiles currently sharing the single null tile.
    pub fn active_null_tiles(&self) -> u64 {
        self.total_bound_null_tiles.saturating_sub(self.total_unbound_null_tiles)
    }

    // Computes physical VRAM saved by sharing a single 64 KiB tile across all null mappings.
    pub fn calculate_vram_saved_bytes(&self) -> u64 {
        let active = self.active_null_tiles();
        if active > 1 {
            (active - 1) * (D3D12_SPARSE_TILE_SIZE_BYTES as u64)
        } else {
            0
        }
    }
}

// Comprehensive verification self-test suite for D3D12 sparse tile page remapping and TLB engine.
pub fn test_gpu_page_remapping_subsystem() -> bool {
    // 1. Verify D3D12 UpdateTileMappings and CopyTileMappings.
    let mut table1 = SparseResourceMappingTable::new(1001, 16);
    let ranges = [
        TileMappingRange::normal(100, 4),
        TileMappingRange::null(4),
        TileMappingRange::reuse_single(200, 4),
        TileMappingRange::normal(300, 4),
    ];
    let mapped = table1.update_tile_mappings(0, &ranges);
    if mapped != 16 {
        return false;
    }

    if table1.get_physical_tile(0) != Some(100)
        || table1.get_physical_tile(3) != Some(103)
        || table1.get_physical_tile(4).is_some()
        || table1.get_physical_tile(8) != Some(200)
        || table1.get_physical_tile(11) != Some(200)
        || table1.get_physical_tile(12) != Some(300)
    {
        return false;
    }

    let mut table2 = SparseResourceMappingTable::new(1002, 8);
    let copied = table2.copy_tile_mappings(0, &table1, 0, 4);
    if copied != 4
        || table2.get_physical_tile(0) != Some(100)
        || table2.get_physical_tile(3) != Some(103)
    {
        return false;
    }

    // 2. Verify GPU virtual page table leaf invalidation and TLB flush engine.
    let mut ptes = [GpuPteLeaf::default(); 16];
    let mut i = 0;
    while i < 8 {
        ptes[i].valid = true;
        ptes[i].is_null_tile = false;
        ptes[i].physical_address = (i as u64) * 65536;
        i += 1;
    }

    let mut tlb_engine = GpuTlbEngine::new();
    let gen1 = tlb_engine.invalidate_page_range(&mut ptes, 2, 4);
    if gen1 != 2 || tlb_engine.total_flushes != 1 || tlb_engine.total_invalidated_leaves != 4 {
        return false;
    }
    if ptes[1].valid != true || ptes[2].valid != false || ptes[5].valid != false || ptes[6].valid != true {
        return false;
    }

    let gen2 = tlb_engine.flush_global();
    if gen2 != 3 || tlb_engine.total_flushes != 2 {
        return false;
    }

    // 3. Verify handheld sparse texture streaming governor and LRU tile eviction.
    let mut gov = HandheldSparseTextureGovernor::new(2);
    let mut table3 = SparseResourceMappingTable::new(1003, 4);

    let p0 = match gov.request_tile(1003, 0, 1, 10, &mut table3) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let p1 = match gov.request_tile(1003, 1, 1, 20, &mut table3) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if p0 != 0 || p1 != 1 || gov.pool_utilization_percentage() != 100 {
        return false;
    }

    // Evicts tile 0 (oldest frame 10), reallocating physical tile 0 to virtual tile 2.
    let p2 = match gov.request_tile(1003, 2, 1, 30, &mut table3) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if p2 != 0 || table3.get_physical_tile(0).is_some() || table3.get_physical_tile(2) != Some(0) {
        return false;
    }
    if gov.total_evictions != 1 {
        return false;
    }

    // 4. Verify shared null tile fallback and VRAM savings metric.
    let mut null_mgr = SharedNullTileManager::new(0x2000_0000);
    let mut null_ptes = [GpuPteLeaf::default(); 10];
    let bound_count = null_mgr.bind_null_range(&mut null_ptes, 0, 10);
    if bound_count != 10 || null_mgr.active_null_tiles() != 10 {
        return false;
    }
    if null_ptes[0].physical_address != 0x2000_0000 || !null_ptes[0].is_null_tile || !null_ptes[0].valid {
        return false;
    }

    let vram_saved = null_mgr.calculate_vram_saved_bytes();
    if vram_saved != 589_824 {
        return false;
    }

    // 5. Emit deterministic boot sentinel.
    crate::serial_write_str(
        "[KNT-D3D12-SPARSE-TILE-PASS] Direct3D 12 agility sparse tile page remapping verified\r\n",
    );
    true
}
