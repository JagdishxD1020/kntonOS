// Direct3D 12 Agility Enhanced Sampler Feedback Minification & Streaming Tessellation Optimizer.
// Evaluates GPU sampler feedback minification maps to dynamically drive LOD streaming and adaptive tessellation.

// Tile dimension in texels/pixels represented by a single sampler feedback record.
pub const SAMPLER_FEEDBACK_TILE_SIZE: usize = 16;

// Maximum dimensions of the feedback minification grid.
pub const FEEDBACK_GRID_WIDTH: usize = 32;
pub const FEEDBACK_GRID_HEIGHT: usize = 32;

// Value representing an unvisited or unreferenced feedback tile (0xFF).
pub const UNREFERENCED_FEEDBACK_MIP: u8 = 0xFF;

// Maximum mipmap levels supported for streaming virtual textures.
pub const MAX_MIP_LEVELS: usize = 16;

// GPU hardware sampler feedback minification map tracking minimum sampled mip levels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedbackMinificationMap {
    pub tiles: [[u8; FEEDBACK_GRID_WIDTH]; FEEDBACK_GRID_HEIGHT],
}

impl FeedbackMinificationMap {
    // Initializes an empty feedback map with all tiles marked as unreferenced.
    pub const fn new() -> Self {
        Self {
            tiles: [[UNREFERENCED_FEEDBACK_MIP; FEEDBACK_GRID_WIDTH]; FEEDBACK_GRID_HEIGHT],
        }
    }

    // Records a shader texture sampling event into the specified feedback tile.
    pub fn record_sample(&mut self, tile_x: usize, tile_y: usize, sampled_mip: u8) {
        if tile_x < FEEDBACK_GRID_WIDTH && tile_y < FEEDBACK_GRID_HEIGHT {
            let current = self.tiles[tile_y][tile_x];
            self.tiles[tile_y][tile_x] = current.min(sampled_mip);
        }
    }

    // Returns the minimum sampled mipmap level across the entire feedback map.
    pub fn min_sampled_mip(&self) -> u8 {
        let mut min_mip = UNREFERENCED_FEEDBACK_MIP;
        for y in 0..FEEDBACK_GRID_HEIGHT {
            for x in 0..FEEDBACK_GRID_WIDTH {
                let m = self.tiles[y][x];
                if m < min_mip {
                    min_mip = m;
                }
            }
        }
        min_mip
    }

    // Returns total count of tiles touched by GPU texture sampling operations.
    pub fn referenced_tile_count(&self) -> usize {
        let mut count = 0usize;
        for y in 0..FEEDBACK_GRID_HEIGHT {
            for x in 0..FEEDBACK_GRID_WIDTH {
                if self.tiles[y][x] != UNREFERENCED_FEEDBACK_MIP {
                    count += 1;
                }
            }
        }
        count
    }

    // Calculates histogram distribution of sampled mip levels across referenced tiles.
    pub fn compute_mip_histogram(&self) -> [u32; MAX_MIP_LEVELS] {
        let mut histogram = [0u32; MAX_MIP_LEVELS];
        for y in 0..FEEDBACK_GRID_HEIGHT {
            for x in 0..FEEDBACK_GRID_WIDTH {
                let m = self.tiles[y][x];
                if (m as usize) < MAX_MIP_LEVELS {
                    histogram[m as usize] += 1;
                }
            }
        }
        histogram
    }

    // Clears all feedback tiles back to unreferenced sentinel.
    pub fn clear(&mut self) {
        self.tiles = [[UNREFERENCED_FEEDBACK_MIP; FEEDBACK_GRID_WIDTH]; FEEDBACK_GRID_HEIGHT];
    }
}

// Mipmap residency tracker managing dynamic VRAM residency masks for streaming textures.
pub struct MipmapResidencyTracker {
    total_mips: u8,
    resident_mask: u16,
}

impl MipmapResidencyTracker {
    // Creates a new residency tracker with baseline tail mips loaded into VRAM.
    pub const fn new(total_mips: u8, initial_resident_mask: u16) -> Self {
        Self {
            total_mips,
            resident_mask: initial_resident_mask,
        }
    }

    // Evaluates feedback map to determine required mip loads and evictions.
    pub fn evaluate_residency_demands(&self, feedback: &FeedbackMinificationMap) -> (u16, u16) {
        let mut required_mask = 0u16;
        let min_mip = feedback.min_sampled_mip();

        if min_mip != UNREFERENCED_FEEDBACK_MIP {
            for mip in min_mip..self.total_mips {
                if (mip as usize) < MAX_MIP_LEVELS {
                    required_mask |= 1 << mip;
                }
            }
        }

        // Mips needed that are not resident.
        let to_load = required_mask & !self.resident_mask;
        // High-detail mips currently resident that are no longer referenced.
        let to_evict = self.resident_mask & !required_mask;

        (to_load, to_evict)
    }

    // Applies completed residency transition masks.
    pub fn apply_residency_update(&mut self, loaded: u16, evicted: u16) {
        self.resident_mask = (self.resident_mask | loaded) & !evicted;
    }

    // Checks if a specific mip level is currently resident in VRAM.
    #[inline]
    pub fn is_mip_resident(&self, mip: u8) -> bool {
        if (mip as usize) < MAX_MIP_LEVELS {
            (self.resident_mask & (1 << mip)) != 0
        } else {
            false
        }
    }

    // Returns the most detailed mip level currently resident in VRAM.
    pub fn min_resident_mip(&self) -> u8 {
        for mip in 0..self.total_mips {
            if self.is_mip_resident(mip) {
                return mip;
            }
        }
        self.total_mips.saturating_sub(1)
    }
}
