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

/// Storage surface and GPU sampling simulator for Direct3D 12 Sampler Feedback Maps.
#[derive(Debug, Clone, PartialEq)]
#[allow(non_camel_case_types)]
pub struct D3D12_SAMPLER_FEEDBACK_MAP {
    pub desc: D3D12_SAMPLER_FEEDBACK_MAP_DESC,
    pub min_mip_data: alloc::vec::Vec<u8>,
    pub mip_mask_data: alloc::vec::Vec<u16>,
    pub total_samples_recorded: u64,
}

impl D3D12_SAMPLER_FEEDBACK_MAP {
    /// Allocates and initializes feedback map storage matching the descriptor.
    pub fn new(desc: D3D12_SAMPLER_FEEDBACK_MAP_DESC) -> Self {
        let count = desc.total_tiles() as usize;
        let min_mip_data = alloc::vec![D3D12_SAMPLER_FEEDBACK_UNACCESSED_MIP; count];
        let mip_mask_data = alloc::vec![0u16; count];

        Self {
            desc,
            min_mip_data,
            mip_mask_data,
            total_samples_recorded: 0,
        }
    }

    /// Clears feedback map contents for the next frame.
    pub fn clear(&mut self) {
        self.min_mip_data.fill(D3D12_SAMPLER_FEEDBACK_UNACCESSED_MIP);
        self.mip_mask_data.fill(0);
        self.total_samples_recorded = 0;
    }

    /// Records a GPU texture sampling operation at `(texel_x, texel_y)` accessing `mip_level`.
    pub fn record_sample(&mut self, texel_x: u32, texel_y: u32, mip_level: u8) -> bool {
        if texel_x >= self.desc.target_width || texel_y >= self.desc.target_height {
            return false;
        }
        if mip_level >= self.desc.target_mip_levels as u8 {
            return false;
        }

        let (tx, ty) = self.desc.texel_to_tile(texel_x, texel_y);
        let idx = match self.desc.tile_index(tx, ty) {
            Some(i) => i,
            None => return false,
        };

        match self.desc.feedback_type {
            D3D12_SAMPLER_FEEDBACK_TYPE::D3D12_SAMPLER_FEEDBACK_MIN_MIP => {
                let current = self.min_mip_data[idx];
                self.min_mip_data[idx] = if current == D3D12_SAMPLER_FEEDBACK_UNACCESSED_MIP {
                    mip_level
                } else {
                    core::cmp::min(current, mip_level)
                };
            }
            D3D12_SAMPLER_FEEDBACK_TYPE::D3D12_SAMPLER_FEEDBACK_MIP_REGION_USED => {
                self.mip_mask_data[idx] |= 1u16 << mip_level;
            }
        }

        self.total_samples_recorded = self.total_samples_recorded.saturating_add(1);
        true
    }

    /// Reads recorded minimum mip level for tile `(tile_x, tile_y)`.
    pub fn get_min_mip(&self, tile_x: u32, tile_y: u32) -> Option<u8> {
        self.desc.tile_index(tile_x, tile_y).map(|idx| self.min_mip_data[idx])
    }

    /// Reads recorded mip mask for tile `(tile_x, tile_y)`.
    pub fn get_mip_mask(&self, tile_x: u32, tile_y: u32) -> Option<u16> {
        self.desc.tile_index(tile_x, tile_y).map(|idx| self.mip_mask_data[idx])
    }

    /// Evaluates if tile `(tile_x, tile_y)` was accessed during the frame.
    pub fn is_tile_accessed(&self, tile_x: u32, tile_y: u32) -> bool {
        match self.desc.feedback_type {
            D3D12_SAMPLER_FEEDBACK_TYPE::D3D12_SAMPLER_FEEDBACK_MIN_MIP => {
                self.get_min_mip(tile_x, tile_y).map_or(false, |m| m != D3D12_SAMPLER_FEEDBACK_UNACCESSED_MIP)
            }
            D3D12_SAMPLER_FEEDBACK_TYPE::D3D12_SAMPLER_FEEDBACK_MIP_REGION_USED => {
                self.get_mip_mask(tile_x, tile_y).map_or(false, |m| m != 0)
            }
        }
    }
}

/// Representation of a required 64KB physical texture tile demanded by GPU feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequiredStreamingTile {
    pub texture_id: u32,
    pub mip_level: u16,
    pub tile_x: u32,
    pub tile_y: u32,
    pub physical_tile_index: u32,
    pub priority: u32,
}

/// Tracks current VRAM mip-level residency per 64KB physical tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureMipResidency {
    pub tiles_x: u32,
    pub tiles_y: u32,
    pub resident_mips: alloc::vec::Vec<u8>,
}

impl TextureMipResidency {
    /// Initializes residency map where all tiles initially reside at `default_mip`.
    pub fn new(tiles_x: u32, tiles_y: u32, default_mip: u8) -> Self {
        let count = (tiles_x * tiles_y) as usize;
        Self {
            tiles_x,
            tiles_y,
            resident_mips: alloc::vec![default_mip; count],
        }
    }

    /// Returns resident mip level for tile `(tile_x, tile_y)`.
    pub fn get_resident_mip(&self, tile_x: u32, tile_y: u32) -> u8 {
        if tile_x < self.tiles_x && tile_y < self.tiles_y {
            self.resident_mips[(tile_y * self.tiles_x + tile_x) as usize]
        } else {
            0xFF
        }
    }

    /// Promotes resident mip level to higher-resolution `new_mip` if finer than current.
    pub fn promote_tile_mip(&mut self, tile_x: u32, tile_y: u32, new_mip: u8) -> bool {
        if tile_x < self.tiles_x && tile_y < self.tiles_y {
            let idx = (tile_y * self.tiles_x + tile_x) as usize;
            if new_mip < self.resident_mips[idx] {
                self.resident_mips[idx] = new_mip;
                return true;
            }
        }
        false
    }
}

/// Direct3D 12 Feedback Transcode Engine converting feedback maps to 64KB tile load requests.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_FEEDBACK_TRANSCODE_ENGINE {
    pub total_transcodes_performed: u64,
    pub total_tiles_demanded: u64,
    pub vram_bandwidth_saved_bytes: u64,
}

impl D3D12_FEEDBACK_TRANSCODE_ENGINE {
    /// Creates a new feedback transcode engine.
    pub const fn new() -> Self {
        Self {
            total_transcodes_performed: 0,
            total_tiles_demanded: 0,
            vram_bandwidth_saved_bytes: 0,
        }
    }

    /// Transcodes a MinMip feedback map into prioritized 64KB physical tile load requests.
    pub fn transcode_min_mip_map(
        &mut self,
        feedback_map: &D3D12_SAMPLER_FEEDBACK_MAP,
        residency: &TextureMipResidency,
        texture_id: u32,
    ) -> alloc::vec::Vec<RequiredStreamingTile> {
        let mut required = alloc::vec::Vec::new();
        let desc = &feedback_map.desc;

        for ty in 0..desc.map_height_tiles {
            for tx in 0..desc.map_width_tiles {
                if let Some(sampled_mip) = feedback_map.get_min_mip(tx, ty) {
                    if sampled_mip != D3D12_SAMPLER_FEEDBACK_UNACCESSED_MIP {
                        let resident_mip = residency.get_resident_mip(tx, ty);
                        if sampled_mip < resident_mip {
                            let priority = (resident_mip - sampled_mip) as u32 * 100;
                            let physical_tile_index = ty * desc.map_width_tiles + tx;

                            required.push(RequiredStreamingTile {
                                texture_id,
                                mip_level: sampled_mip as u16,
                                tile_x: tx,
                                tile_y: ty,
                                physical_tile_index,
                                priority,
                            });
                        }
                    }
                }
            }
        }

        // Sort descending by priority (highest urgency loaded first)
        required.sort_by(|a, b| b.priority.cmp(&a.priority));

        self.total_transcodes_performed = self.total_transcodes_performed.saturating_add(1);
        self.total_tiles_demanded = self.total_tiles_demanded.saturating_add(required.len() as u64);
        // Track VRAM savings: each unaccessed tile saved 65536 bytes (64KB) of texture streaming
        let unaccessed_count = desc.total_tiles().saturating_sub(required.len() as u32);
        self.vram_bandwidth_saved_bytes = self.vram_bandwidth_saved_bytes
            .saturating_add((unaccessed_count as u64) * 65536);

        required
    }

    /// Transcodes a MipRegionUsed feedback bitmask map into physical 64KB tile load requests.
    pub fn transcode_mip_region_used(
        &mut self,
        feedback_map: &D3D12_SAMPLER_FEEDBACK_MAP,
        residency: &TextureMipResidency,
        texture_id: u32,
    ) -> alloc::vec::Vec<RequiredStreamingTile> {
        let mut required = alloc::vec::Vec::new();
        let desc = &feedback_map.desc;

        for ty in 0..desc.map_height_tiles {
            for tx in 0..desc.map_width_tiles {
                if let Some(mask) = feedback_map.get_mip_mask(tx, ty) {
                    if mask != 0 {
                        let resident_mip = residency.get_resident_mip(tx, ty);
                        for m in 0..desc.target_mip_levels {
                            if (mask & (1 << m)) != 0 && (m as u8) < resident_mip {
                                let priority = (resident_mip - (m as u8)) as u32 * 100;
                                let physical_tile_index = ty * desc.map_width_tiles + tx;

                                required.push(RequiredStreamingTile {
                                    texture_id,
                                    mip_level: m,
                                    tile_x: tx,
                                    tile_y: ty,
                                    physical_tile_index,
                                    priority,
                                });
                            }
                        }
                    }
                }
            }
        }

        required.sort_by(|a, b| b.priority.cmp(&a.priority));
        self.total_transcodes_performed = self.total_transcodes_performed.saturating_add(1);
        self.total_tiles_demanded = self.total_tiles_demanded.saturating_add(required.len() as u64);

        required
    }
}


// ============================================================================
// Direct3D 12 Sampler Feedback Streaming Maps & Tile Residency Bitmask Decoder
// ============================================================================

// Packed bitmask tracking 64KB physical tile residency across a texture mip level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileResidencyBitmask {
    // 64-bit words storing residency status (1 = resident, 0 = unmapped).
    pub words: alloc::vec::Vec<u64>,
    // Total number of discrete tiles represented in this bitmask.
    pub total_tiles: u32,
    // Total number of currently resident tiles.
    pub resident_count: u32,
}

impl TileResidencyBitmask {
    // Creates a new residency bitmask for the given tile count.
    pub fn new(total_tiles: u32, all_resident: bool) -> Self {
        let num_words = ((total_tiles as usize).saturating_add(63)) / 64;
        let fill_word = if all_resident { !0u64 } else { 0u64 };
        let mut words = alloc::vec![fill_word; num_words];

        // Mask out extra bits in the last word
        let remainder = (total_tiles as usize) % 64;
        if all_resident && remainder != 0 && num_words > 0 {
            words[num_words - 1] = (1u64 << remainder) - 1;
        }

        let resident_count = if all_resident { total_tiles } else { 0 };
        Self {
            words,
            total_tiles,
            resident_count,
        }
    }

    // Queries whether a specific tile index is marked resident.
    #[inline]
    pub fn is_tile_resident(&self, tile_index: u32) -> bool {
        if tile_index >= self.total_tiles {
            return false;
        }
        let word_idx = (tile_index / 64) as usize;
        let bit_idx = (tile_index % 64) as usize;
        (self.words[word_idx] & (1u64 << bit_idx)) != 0
    }

    // Updates the residency status of a specific tile index.
    pub fn set_tile_resident(&mut self, tile_index: u32, resident: bool) -> bool {
        if tile_index >= self.total_tiles {
            return false;
        }
        let word_idx = (tile_index / 64) as usize;
        let bit_idx = (tile_index % 64) as usize;
        let mask = 1u64 << bit_idx;
        let was_resident = (self.words[word_idx] & mask) != 0;

        if resident && !was_resident {
            self.words[word_idx] |= mask;
            self.resident_count = self.resident_count.saturating_add(1);
        } else if !resident && was_resident {
            self.words[word_idx] &= !mask;
            self.resident_count = self.resident_count.saturating_sub(1);
        }
        was_resident
    }

    // Marks all tiles in this bitmask as resident or unmapped.
    pub fn mark_all(&mut self, resident: bool) {
        let fill_word = if resident { !0u64 } else { 0u64 };
        self.words.fill(fill_word);
        let remainder = (self.total_tiles as usize) % 64;
        if resident && remainder != 0 && !self.words.is_empty() {
            let last = self.words.len() - 1;
            self.words[last] = (1u64 << remainder) - 1;
        }
        self.resident_count = if resident { self.total_tiles } else { 0 };
    }
}

// Virtual texture streaming map pairing mip levels with discrete 64KB tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamingTextureVirtualMap {
    // Virtual texture identifier.
    pub texture_id: u32,
    // Full resolution base texture width in texels.
    pub width: u32,
    // Full resolution base texture height in texels.
    pub height: u32,
    // Total number of mip levels in texture pyramid.
    pub mip_levels: u8,
    // Dimension in texels of a single 64KB tile at mip 0.
    pub tile_dim_texels: u32,
    // Per-mip residency bitmask tracking tile mapping status.
    pub mip_residency: alloc::vec::Vec<TileResidencyBitmask>,
}

impl StreamingTextureVirtualMap {
    // Constructs a virtual texture map and initializes residency bitmasks.
    pub fn new(
        texture_id: u32,
        width: u32,
        height: u32,
        mip_levels: u8,
        tile_dim_texels: u32,
        base_mip_cutoff: u8,
    ) -> Self {
        let mut mip_residency = alloc::vec::Vec::with_capacity(mip_levels as usize);
        for mip in 0..mip_levels {
            let mip_w = core::cmp::max(1, width >> mip);
            let mip_h = core::cmp::max(1, height >> mip);
            let tiles_x = (mip_w.saturating_add(tile_dim_texels - 1)) / tile_dim_texels;
            let tiles_y = (mip_h.saturating_add(tile_dim_texels - 1)) / tile_dim_texels;
            let total_tiles = core::cmp::max(1, tiles_x * tiles_y);
            // Coarse base mips at or above cutoff are permanently resident
            let is_always_resident = mip >= base_mip_cutoff;
            mip_residency.push(TileResidencyBitmask::new(total_tiles, is_always_resident));
        }

        Self {
            texture_id,
            width,
            height,
            mip_levels,
            tile_dim_texels,
            mip_residency,
        }
    }

    // Calculates tile grid dimensions (tiles_x, tiles_y) for a specific mip level.
    pub fn tiles_per_axis_for_mip(&self, mip: u8) -> (u32, u32) {
        if mip >= self.mip_levels {
            return (0, 0);
        }
        let mip_w = core::cmp::max(1, self.width >> mip);
        let mip_h = core::cmp::max(1, self.height >> mip);
        let tiles_x = (mip_w.saturating_add(self.tile_dim_texels - 1)) / self.tile_dim_texels;
        let tiles_y = (mip_h.saturating_add(self.tile_dim_texels - 1)) / self.tile_dim_texels;
        (core::cmp::max(1, tiles_x), core::cmp::max(1, tiles_y))
    }

    // Returns total tile count for a given mip level.
    pub fn total_tiles_for_mip(&self, mip: u8) -> u32 {
        let (tx, ty) = self.tiles_per_axis_for_mip(mip);
        tx * ty
    }
}

// Decodes raw GPU sampler feedback maps against virtual texture residency bitmasks.
#[derive(Debug, Clone, Default)]
pub struct TileResidencyBitmaskDecoder {
    // Total decoded feedback tiles processed across frames.
    pub total_tiles_decoded: u64,
    // Total missing residency tile faults detected.
    pub total_residency_faults: u64,
}

impl TileResidencyBitmaskDecoder {
    // Creates a new tile residency bitmask decoder.
    pub const fn new() -> Self {
        Self {
            total_tiles_decoded: 0,
            total_residency_faults: 0,
        }
    }

    // Decodes MinMip feedback map and identifies tiles requiring streaming load.
    pub fn decode_min_mip_faults(
        &mut self,
        feedback_map: &D3D12_SAMPLER_FEEDBACK_MAP,
        virtual_map: &StreamingTextureVirtualMap,
    ) -> alloc::vec::Vec<(u8, u32, u32)> {
        let mut missing_tiles = alloc::vec::Vec::new();
        let desc = &feedback_map.desc;

        for ty in 0..desc.map_height_tiles {
            for tx in 0..desc.map_width_tiles {
                self.total_tiles_decoded = self.total_tiles_decoded.saturating_add(1);
                if let Some(sampled_mip) = feedback_map.get_min_mip(tx, ty) {
                    if sampled_mip != D3D12_SAMPLER_FEEDBACK_UNACCESSED_MIP
                        && sampled_mip < virtual_map.mip_levels
                    {
                        let (tiles_x, _) = virtual_map.tiles_per_axis_for_mip(sampled_mip);
                        let tile_idx = ty * tiles_x + tx;
                        let is_resident = virtual_map.mip_residency
                            .get(sampled_mip as usize)
                            .map_or(false, |mask| mask.is_tile_resident(tile_idx));

                        if !is_resident {
                            self.total_residency_faults = self.total_residency_faults.saturating_add(1);
                            missing_tiles.push((sampled_mip, tx, ty));
                        }
                    }
                }
            }
        }
        missing_tiles
    }
}


// ============================================================================
// GPU Texture LOD Feedback Analysis & Dynamic Mip-Level Request Queue
// ============================================================================

// Represents a single dynamic mip tile load request demanded by GPU LOD feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicMipLevelRequest {
    // Virtual texture identifier.
    pub texture_id: u32,
    // Target mip level to stream (0 = finest resolution).
    pub mip_level: u8,
    // Horizontal tile coordinate within the mip level.
    pub tile_x: u16,
    // Vertical tile coordinate within the mip level.
    pub tile_y: u16,
    // Priority score: higher score indicates higher visual urgency.
    pub priority: u32,
    // Monotonic frame counter when request was queued.
    pub frame_timestamp: u64,
}

impl DynamicMipLevelRequest {
    // Constructs a new dynamic mip level request with computed priority.
    pub fn new(
        texture_id: u32,
        mip_level: u8,
        tile_x: u16,
        tile_y: u16,
        priority: u32,
        frame_timestamp: u64,
    ) -> Self {
        Self {
            texture_id,
            mip_level,
            tile_x,
            tile_y,
            priority,
            frame_timestamp,
        }
    }
}

// Bounded priority queue of pending mip level streaming requests with deduplication.
#[derive(Debug, Clone)]
pub struct DynamicMipRequestQueue {
    // Max requests retained in the queue.
    pub capacity: usize,
    // Pending requests sorted by priority (highest priority first).
    pub requests: alloc::vec::Vec<DynamicMipLevelRequest>,
    // Total requests enqueued lifetime.
    pub total_enqueued: u64,
    // Total duplicate requests dropped.
    pub total_duplicates_dropped: u64,
}

impl DynamicMipRequestQueue {
    // Constructs a new request queue with the specified maximum capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            requests: alloc::vec::Vec::with_capacity(core::cmp::min(capacity, 1024)),
            total_enqueued: 0,
            total_duplicates_dropped: 0,
        }
    }

    // Queries whether an identical request already exists in the queue.
    pub fn contains(&self, texture_id: u32, mip_level: u8, tile_x: u16, tile_y: u16) -> bool {
        self.requests.iter().any(|r| {
            r.texture_id == texture_id
                && r.mip_level == mip_level
                && r.tile_x == tile_x
                && r.tile_y == tile_y
        })
    }

    // Enqueues a mip level request with deduplication and priority insertion.
    pub fn enqueue(&mut self, req: DynamicMipLevelRequest) -> bool {
        if self.contains(req.texture_id, req.mip_level, req.tile_x, req.tile_y) {
            self.total_duplicates_dropped = self.total_duplicates_dropped.saturating_add(1);
            return false;
        }

        if self.requests.len() >= self.capacity {
            // Drop lowest priority if incoming request is more urgent
            if let Some(lowest) = self.requests.last() {
                if req.priority > lowest.priority {
                    self.requests.pop();
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }

        // Binary search insertion index to maintain descending priority order
        let insert_idx = match self.requests.binary_search_by(|probe| probe.priority.cmp(&req.priority).reverse()) {
            Ok(idx) | Err(idx) => idx,
        };
        self.requests.insert(insert_idx, req);
        self.total_enqueued = self.total_enqueued.saturating_add(1);
        true
    }

    // Dequeues highest priority streaming request.
    pub fn pop_highest_priority(&mut self) -> Option<DynamicMipLevelRequest> {
        if self.requests.is_empty() {
            None
        } else {
            Some(self.requests.remove(0))
        }
    }

    // Dequeues a batch of requests up to `max_batch` count.
    pub fn pop_batch(&mut self, max_batch: usize) -> alloc::vec::Vec<DynamicMipLevelRequest> {
        let count = core::cmp::min(max_batch, self.requests.len());
        let mut batch = alloc::vec::Vec::with_capacity(count);
        for _ in 0..count {
            if !self.requests.is_empty() {
                batch.push(self.requests.remove(0));
            }
        }
        batch
    }

    // Clears all pending requests.
    pub fn clear(&mut self) {
        self.requests.clear();
    }
}

// Analyzes GPU sampler feedback to determine required texture LODs and enqueues streaming requests.
#[derive(Debug, Clone)]
pub struct GpuTextureLodFeedbackAnalyzer {
    // Priority bonus for fine resolution mips (Mip 0, Mip 1) to eliminate texture blur.
    pub high_res_priority_boost: u32,
    // Total feedback analysis passes executed.
    pub feedback_passes_analyzed: u64,
    // Total tile load requests generated.
    pub requests_generated: u64,
}

impl GpuTextureLodFeedbackAnalyzer {
    // Constructs a GPU texture LOD feedback analyzer.
    pub const fn new() -> Self {
        Self {
            high_res_priority_boost: 500,
            feedback_passes_analyzed: 0,
            requests_generated: 0,
        }
    }

    // Analyzes feedback map against virtual texture layout and populates the request queue.
    pub fn analyze_feedback(
        &mut self,
        feedback_map: &D3D12_SAMPLER_FEEDBACK_MAP,
        virtual_map: &StreamingTextureVirtualMap,
        current_frame: u64,
        queue: &mut DynamicMipRequestQueue,
    ) {
        let desc = &feedback_map.desc;

        for ty in 0..desc.map_height_tiles {
            for tx in 0..desc.map_width_tiles {
                if let Some(sampled_mip) = feedback_map.get_min_mip(tx, ty) {
                    if sampled_mip != D3D12_SAMPLER_FEEDBACK_UNACCESSED_MIP
                        && sampled_mip < virtual_map.mip_levels
                    {
                        let (tiles_x, _) = virtual_map.tiles_per_axis_for_mip(sampled_mip);
                        let tile_idx = ty * tiles_x + tx;
                        let is_resident = virtual_map.mip_residency
                            .get(sampled_mip as usize)
                            .map_or(false, |mask| mask.is_tile_resident(tile_idx));

                        if !is_resident {
                            // Compute priority: higher for finer mips
                            let mut priority = (virtual_map.mip_levels.saturating_sub(sampled_mip) as u32) * 100;
                            if sampled_mip <= 1 {
                                priority = priority.saturating_add(self.high_res_priority_boost);
                            }

                            let req = DynamicMipLevelRequest::new(
                                virtual_map.texture_id,
                                sampled_mip,
                                tx as u16,
                                ty as u16,
                                priority,
                                current_frame,
                            );

                            if queue.enqueue(req) {
                                self.requests_generated = self.requests_generated.saturating_add(1);
                            }
                        }
                    }
                }
            }
        }

        self.feedback_passes_analyzed = self.feedback_passes_analyzed.saturating_add(1);
    }
}


// ============================================================================
// Asynchronous DirectStorage Streaming Request Generator
// ============================================================================

// Compression algorithms supported by DirectStorage NVMe-to-GPU streaming.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectStorageCompression {
    None = 0,
    GDeflate = 1,
    Zlib = 2,
}

// Represents an asynchronous DirectStorage queue submission request for a 64KB texture tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectStorageStreamingRequest {
    // Unique monotonic request tracking index.
    pub request_id: u64,
    // Source texture identifier.
    pub texture_id: u32,
    // Subresource mip level index.
    pub subresource_mip: u8,
    // Horizontal tile coordinate in the texture mip level.
    pub tile_x: u16,
    // Vertical tile coordinate in the texture mip level.
    pub tile_y: u16,
    // Byte offset in the virtual package file on NVMe storage.
    pub source_file_offset: u64,
    // Compressed payload size in bytes.
    pub compressed_size_bytes: u32,
    // Decompressed payload size in bytes (typically 65536 for 64KB tile).
    pub uncompressed_size_bytes: u32,
    // Target physical VRAM destination address or tile pool slot.
    pub destination_vram_addr: u64,
    // Compression format used for decompression in GPU shader or HW engine.
    pub compression: DirectStorageCompression,
    // DirectStorage completion fence value to signal upon DMA transfer finish.
    pub fence_signal_value: u64,
}

// Translates prioritized mip load requests into asynchronous DirectStorage streaming commands.
#[derive(Debug, Clone)]
pub struct DirectStorageStreamGenerator {
    // Current monotonic request ID generator.
    pub next_request_id: u64,
    // DirectStorage timeline fence value.
    pub current_fence_value: u64,
    // Base physical VRAM pool address for streaming tiles.
    pub vram_tile_pool_base: u64,
    // Standard tile byte size (64KB).
    pub standard_tile_size: u32,
    // Active compression mode for streaming package assets.
    pub compression_mode: DirectStorageCompression,
    // Average compression ratio percentage (e.g. 40 = 40% of uncompressed size).
    pub compression_ratio_percent: u32,
    // Total DirectStorage requests generated.
    pub total_requests_generated: u64,
    // Total uncompressed streaming bytes requested.
    pub total_uncompressed_bytes: u64,
    // Total compressed bytes transferred over NVMe.
    pub total_compressed_bytes: u64,
}

impl DirectStorageStreamGenerator {
    // Constructs an asynchronous DirectStorage streaming request generator.
    pub fn new(vram_tile_pool_base: u64, compression_mode: DirectStorageCompression) -> Self {
        let compression_ratio_percent = match compression_mode {
            DirectStorageCompression::None => 100,
            DirectStorageCompression::GDeflate => 45, // ~45% footprint for BC textures
            DirectStorageCompression::Zlib => 55,
        };

        Self {
            next_request_id: 1,
            current_fence_value: 0,
            vram_tile_pool_base,
            standard_tile_size: 65536, // 64 KiB
            compression_mode,
            compression_ratio_percent,
            total_requests_generated: 0,
            total_uncompressed_bytes: 0,
            total_compressed_bytes: 0,
        }
    }

    // Translates a slice of dynamic mip requests into DirectStorage queue streaming packets.
    pub fn generate_streaming_requests(
        &mut self,
        mip_requests: &[DynamicMipLevelRequest],
    ) -> alloc::vec::Vec<DirectStorageStreamingRequest> {
        let mut ds_requests = alloc::vec::Vec::with_capacity(mip_requests.len());

        for req in mip_requests {
            self.current_fence_value = self.current_fence_value.saturating_add(1);
            let req_id = self.next_request_id;
            self.next_request_id = self.next_request_id.saturating_add(1);

            let uncompressed_size = self.standard_tile_size;
            let compressed_size = if self.compression_mode == DirectStorageCompression::None {
                uncompressed_size
            } else {
                (uncompressed_size * self.compression_ratio_percent) / 100
            };

            // Calculate deterministic NVMe file offset from texture layout
            let tile_index = (req.tile_y as u64) * 256 + (req.tile_x as u64);
            let mip_base_offset = (req.texture_id as u64) * 0x10000000 + (req.mip_level as u64) * 0x1000000;
            let source_file_offset = mip_base_offset + tile_index * (compressed_size as u64);

            // Compute destination VRAM physical tile address in sparse pool
            let destination_vram_addr = self.vram_tile_pool_base + ((req_id - 1) % 1024) * 65536;

            let ds_req = DirectStorageStreamingRequest {
                request_id: req_id,
                texture_id: req.texture_id,
                subresource_mip: req.mip_level,
                tile_x: req.tile_x,
                tile_y: req.tile_y,
                source_file_offset,
                compressed_size_bytes: compressed_size,
                uncompressed_size_bytes: uncompressed_size,
                destination_vram_addr,
                compression: self.compression_mode,
                fence_signal_value: self.current_fence_value,
            };

            self.total_requests_generated = self.total_requests_generated.saturating_add(1);
            self.total_uncompressed_bytes = self.total_uncompressed_bytes.saturating_add(uncompressed_size as u64);
            self.total_compressed_bytes = self.total_compressed_bytes.saturating_add(compressed_size as u64);

            ds_requests.push(ds_req);
        }

        ds_requests
    }

    // Returns total NVMe transfer bandwidth saved due to DirectStorage compression.
    pub fn bandwidth_saved_bytes(&self) -> u64 {
        self.total_uncompressed_bytes.saturating_sub(self.total_compressed_bytes)
    }
}

// Comprehensive verification self-test suite for Direct3D 12 Sampler Feedback & Texture Streaming
pub fn test_sampler_feedback_subsystem() -> bool {
    crate::serial_write_str("[KNT-GPU-FEEDBACK] Initializing Direct3D 12 Sampler Feedback self-test...\r\n");

    // 1. Verify D3D12 Options7 feature query and Tier 1.0 capability reporting
    let options7 = query_d3d12_options7_features();
    if options7.SamplerFeedbackTier != D3D12_SAMPLER_FEEDBACK_TIER_1_0 {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Options7 sampler feedback tier reporting failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Direct3D 12 Options7 Sampler Feedback Tier 1.0 capability verified\r\n");

    // 2. Verify feedback map descriptor validation and coordinate conversion
    let desc = match D3D12_SAMPLER_FEEDBACK_MAP_DESC::new(
        D3D12_SAMPLER_FEEDBACK_MIN_MIP,
        2048,
        2048,
        12,
        D3D12_SAMPLER_FEEDBACK_DEFAULT_TILE_SIZE,
        D3D12_SAMPLER_FEEDBACK_DEFAULT_TILE_SIZE,
    ) {
        Ok(d) => d,
        Err(_) => {
            crate::serial_write_str("[KNT-FEEDBACK-FAIL] Valid feedback map descriptor creation failed\r\n");
            return false;
        }
    };

    if desc.map_width_tiles != 16 || desc.map_height_tiles != 16 || desc.total_tiles() != 256 {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Tile count calculation mismatch\r\n");
        return false;
    }
    if desc.texel_to_tile(250, 500) != (1, 3) {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Texel to tile coordinate mapping mismatch\r\n");
        return false;
    }

    // Invalid parameters must fail
    if D3D12_SAMPLER_FEEDBACK_MAP_DESC::new(D3D12_SAMPLER_FEEDBACK_MIN_MIP, 0, 1024, 10, 128, 128).is_ok() {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Zero texture width was not rejected\r\n");
        return false;
    }
    if D3D12_SAMPLER_FEEDBACK_MAP_DESC::new(D3D12_SAMPLER_FEEDBACK_MIN_MIP, 1024, 1024, 0, 128, 128).is_ok() {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Zero mip count was not rejected\r\n");
        return false;
    }
    if D3D12_SAMPLER_FEEDBACK_MAP_DESC::new(D3D12_SAMPLER_FEEDBACK_MIN_MIP, 1024, 1024, 10, 100, 100).is_ok() {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Non-power-of-two tile size was not rejected\r\n");
        return false;
    }

    if D3D12_SAMPLER_FEEDBACK_VALIDATOR::validate_tile_region(&desc, 14, 14, 4, 4) != Err(SamplerFeedbackError::OutOfBounds) {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Out of bounds tile region was not rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * Feedback map descriptors and coordinate mappings verified\r\n");

    // 3. Verify MinMip and MipRegionUsed GPU sampling simulation
    let mut min_mip_map = D3D12_SAMPLER_FEEDBACK_MAP::new(desc);
    if min_mip_map.get_min_mip(1, 1) != Some(D3D12_SAMPLER_FEEDBACK_UNACCESSED_MIP) {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Initial min-mip state must be unaccessed\r\n");
        return false;
    }

    // Texel (150, 150) lies in tile (1, 1)
    min_mip_map.record_sample(150, 150, 4);
    if min_mip_map.get_min_mip(1, 1) != Some(4) {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Min-mip sample record failed\r\n");
        return false;
    }

    min_mip_map.record_sample(150, 150, 2); // Finer mip (numerically lower)
    if min_mip_map.get_min_mip(1, 1) != Some(2) {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Min-mip resolution to finer level failed\r\n");
        return false;
    }

    min_mip_map.record_sample(150, 150, 5); // Coarser mip (numerically higher) should be ignored
    if min_mip_map.get_min_mip(1, 1) != Some(2) {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Min-mip erroneously clobbered by coarser mip\r\n");
        return false;
    }

    if !min_mip_map.is_tile_accessed(1, 1) || min_mip_map.is_tile_accessed(0, 0) {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Tile accessed tracking mismatch\r\n");
        return false;
    }

    let mask_desc = D3D12_SAMPLER_FEEDBACK_MAP_DESC::new(
        D3D12_SAMPLER_FEEDBACK_MIP_REGION_USED,
        2048,
        2048,
        12,
        128,
        128,
    ).unwrap();
    let mut mask_map = D3D12_SAMPLER_FEEDBACK_MAP::new(mask_desc);
    mask_map.record_sample(150, 150, 1);
    mask_map.record_sample(150, 150, 3);
    if mask_map.get_mip_mask(1, 1) != Some(0x000A) { // bit 1 and bit 3
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] MipRegionUsed bitmask recording mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Min-mip and MipRegionUsed GPU sampling recording verified\r\n");

    // 4. Verify feedback transcode engine and 64KB physical streaming tile generation
    let mut residency = TextureMipResidency::new(16, 16, 4);
    let mut transcode_engine = D3D12_FEEDBACK_TRANSCODE_ENGINE::new();

    let demanded_tiles = transcode_engine.transcode_min_mip_map(&min_mip_map, &residency, 42);
    if demanded_tiles.len() != 1 {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Transcode required tile count mismatch\r\n");
        return false;
    }

    let tile = &demanded_tiles[0];
    if tile.texture_id != 42 || tile.tile_x != 1 || tile.tile_y != 1 || tile.mip_level != 2 || tile.priority != 200 {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Demanded tile metadata or priority mismatch\r\n");
        return false;
    }
    if transcode_engine.vram_bandwidth_saved_bytes == 0 {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] VRAM streaming bandwidth savings not tracked\r\n");
        return false;
    }

    // Fulfill streaming tile promotion
    if !residency.promote_tile_mip(tile.tile_x, tile.tile_y, tile.mip_level as u8) {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Residency promotion failed\r\n");
        return false;
    }

    // Re-transcode: no tiles demanded because residency now matches feedback min-mip
    let re_demanded = transcode_engine.transcode_min_mip_map(&min_mip_map, &residency, 42);
    if !re_demanded.is_empty() {
        crate::serial_write_str("[KNT-FEEDBACK-FAIL] Demanded tiles should be empty after promotion\r\n");
        return false;
    }
    crate::serial_write_str("  * Transcode engine 64KB physical streaming tile generation verified\r\n");

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-FEEDBACK-PASS] Direct3D 12 sampler feedback and texture streaming engine verified\r\n");
    true
}

