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

// Maximum hardware tessellation factor conforming to Direct3D 12 Agility limits.
pub const MAX_HARDWARE_TESSELLATION_FACTOR: f32 = 64.0;
pub const MIN_HARDWARE_TESSELLATION_FACTOR: f32 = 1.0;

// Continuous fractional tessellation factors for quad and triangle patches.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PatchTessellationFactors {
    pub edge_factors: [f32; 4],
    pub inside_factors: [f32; 2],
    pub is_culled: bool,
}

impl PatchTessellationFactors {
    // Creates a culled patch state requiring zero geometry amplification.
    pub const fn culled() -> Self {
        Self {
            edge_factors: [0.0; 4],
            inside_factors: [0.0; 2],
            is_culled: true,
        }
    }

    // Creates uniform tessellation factors across all edges and interior.
    pub fn uniform(factor: f32) -> Self {
        let f = factor.clamp(MIN_HARDWARE_TESSELLATION_FACTOR, MAX_HARDWARE_TESSELLATION_FACTOR);
        Self {
            edge_factors: [f; 4],
            inside_factors: [f; 2],
            is_culled: false,
        }
    }
}

// Evaluates continuous tessellation factors combining sampler feedback density and camera distance.
pub struct ContinuousTessellationCalculator {
    pub near_distance: f32,
    pub far_distance: f32,
    pub max_feedback_mip: f32,
}

impl ContinuousTessellationCalculator {
    // Initializes the continuous tessellation calculator with view distance boundaries.
    pub const fn new(near_distance: f32, far_distance: f32) -> Self {
        Self {
            near_distance,
            far_distance,
            max_feedback_mip: 8.0,
        }
    }

    // Calculates continuous tessellation factor for a geometry patch.
    pub fn calculate_tessellation_factor(&self, camera_distance: f32, feedback_mip: u8) -> f32 {
        if camera_distance > self.far_distance {
            return MIN_HARDWARE_TESSELLATION_FACTOR;
        }

        // Distance attenuation factor [0.0, 1.0].
        let dist_span = (self.far_distance - self.near_distance).max(0.001);
        let dist_factor = 1.0 - ((camera_distance - self.near_distance) / dist_span).clamp(0.0, 1.0);

        // Feedback sampling density factor [0.0, 1.0].
        let feedback_factor = if feedback_mip == UNREFERENCED_FEEDBACK_MIP {
            0.0
        } else {
            1.0 - ((feedback_mip as f32) / self.max_feedback_mip).clamp(0.0, 1.0)
        };

        // Weighted continuous synthesis (60% distance importance, 40% sampler feedback detail).
        let combined = 0.6 * dist_factor + 0.4 * feedback_factor;
        let factor = MIN_HARDWARE_TESSELLATION_FACTOR + (MAX_HARDWARE_TESSELLATION_FACTOR - MIN_HARDWARE_TESSELLATION_FACTOR) * combined;

        factor.clamp(MIN_HARDWARE_TESSELLATION_FACTOR, MAX_HARDWARE_TESSELLATION_FACTOR)
    }

    // Evaluates quad patch tessellation factors from corner distances and feedback tile.
    pub fn evaluate_quad_patch(
        &self,
        corner_distances: [f32; 4],
        feedback_mip: u8,
    ) -> PatchTessellationFactors {
        let e0 = self.calculate_tessellation_factor((corner_distances[0] + corner_distances[1]) * 0.5, feedback_mip);
        let e1 = self.calculate_tessellation_factor((corner_distances[1] + corner_distances[2]) * 0.5, feedback_mip);
        let e2 = self.calculate_tessellation_factor((corner_distances[2] + corner_distances[3]) * 0.5, feedback_mip);
        let e3 = self.calculate_tessellation_factor((corner_distances[3] + corner_distances[0]) * 0.5, feedback_mip);

        let inside_x = (e1 + e3) * 0.5;
        let inside_y = (e0 + e2) * 0.5;

        PatchTessellationFactors {
            edge_factors: [e0, e1, e2, e3],
            inside_factors: [inside_x, inside_y],
            is_culled: false,
        }
    }
}

// Default per-frame VRAM streaming transfer budget (64 Megabytes per frame at 60 FPS).
pub const DEFAULT_MAX_STREAMING_BANDWIDTH_BYTES: usize = 64 * 1024 * 1024;

// Texture residency transfer request tracking staging uploads or evictions.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ResidencyTransferRequest {
    pub texture_id: u32,
    pub mip_level: u8,
    pub size_bytes: usize,
    pub is_upload: bool,
}

// GPU memory bandwidth throttle governor preventing VRAM thrashing during texture streaming.
pub struct VramBandwidthThrottleGovernor {
    max_bytes_per_frame: usize,
    current_frame_bytes: usize,
    thrash_counter: u32,
    active_lod_clamp_bias: u8,
    total_transfers_throttled: u64,
}

impl VramBandwidthThrottleGovernor {
    // Initializes the throttle governor with target per-frame transfer bandwidth ceiling.
    pub const fn new(max_bytes_per_frame: usize) -> Self {
        Self {
            max_bytes_per_frame,
            current_frame_bytes: 0,
            thrash_counter: 0,
            active_lod_clamp_bias: 0,
            total_transfers_throttled: 0,
        }
    }

    // Default governor configured for handheld unified APU bus constraints.
    pub const fn default_handheld() -> Self {
        Self::new(DEFAULT_MAX_STREAMING_BANDWIDTH_BYTES)
    }

    // Requests bandwidth budget to stage a texture residency transfer, throttling if budget is full.
    pub fn request_transfer(&mut self, request: &ResidencyTransferRequest) -> bool {
        // Evictions do not consume PCIe/storage upload bandwidth.
        if !request.is_upload {
            return true;
        }

        // Check if adding request exceeds frame budget.
        if self.current_frame_bytes.saturating_add(request.size_bytes) > self.max_bytes_per_frame {
            self.total_transfers_throttled += 1;
            self.thrash_counter = self.thrash_counter.saturating_add(1);
            if self.thrash_counter > 5 {
                // Engage anti-thrashing LOD clamp bias under continuous saturation.
                self.active_lod_clamp_bias = (self.active_lod_clamp_bias + 1).min(3);
            }
            return false;
        }

        self.current_frame_bytes += request.size_bytes;
        true
    }

    // Advances the governor timeline at frame boundary, resetting per-frame bandwidth counter.
    pub fn advance_frame(&mut self) {
        if self.current_frame_bytes < (self.max_bytes_per_frame / 2) {
            // Relieve thrash pressure when bandwidth demand drops.
            self.thrash_counter = self.thrash_counter.saturating_sub(1);
            if self.thrash_counter == 0 {
                self.active_lod_clamp_bias = self.active_lod_clamp_bias.saturating_sub(1);
            }
        }
        self.current_frame_bytes = 0;
    }

    // Returns the active anti-thrashing LOD clamp bias (0 = unconstrained).
    #[inline]
    pub fn active_lod_clamp_bias(&self) -> u8 {
        self.active_lod_clamp_bias
    }

    // Returns current bandwidth utilization percentage for the active frame [0, 100].
    pub fn frame_utilization_percent(&self) -> u32 {
        if self.max_bytes_per_frame == 0 {
            0
        } else {
            ((self.current_frame_bytes as u128 * 100) / self.max_bytes_per_frame as u128) as u32
        }
    }

    // Returns cumulative number of transfer operations delayed by bandwidth throttling.
    #[inline]
    pub fn total_transfers_throttled(&self) -> u64 {
        self.total_transfers_throttled
    }
}

// Evaluated tessellation parameters and geometry expansion for a streaming meshlet.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TessellatedMeshletLod {
    pub meshlet_id: u32,
    pub base_primitives: u32,
    pub amplified_primitives: u32,
    pub tessellation_factor: f32,
    pub feedback_min_mip: u8,
}

// Integrated streaming tessellation and feedback minification optimizer.
pub struct StreamingTessellationOptimizer {
    pub tess_calculator: ContinuousTessellationCalculator,
    pub throttle_governor: VramBandwidthThrottleGovernor,
}

impl StreamingTessellationOptimizer {
    // Initializes the streaming tessellation optimizer.
    pub const fn new(near_dist: f32, far_dist: f32, max_bandwidth_bytes: usize) -> Self {
        Self {
            tess_calculator: ContinuousTessellationCalculator::new(near_dist, far_dist),
            throttle_governor: VramBandwidthThrottleGovernor::new(max_bandwidth_bytes),
        }
    }

    // Default optimizer configured for standard handheld hardware profiles.
    pub const fn default_handheld() -> Self {
        Self::new(2.0, 100.0, DEFAULT_MAX_STREAMING_BANDWIDTH_BYTES)
    }

    // Evaluates dynamic tessellation factors for a meshlet taking into account bandwidth constraints.
    pub fn evaluate_meshlet(
        &self,
        meshlet_id: u32,
        base_primitives: u32,
        camera_distance: f32,
        feedback_map: &FeedbackMinificationMap,
        tile_x: usize,
        tile_y: usize,
    ) -> TessellatedMeshletLod {
        let feedback_mip = if tile_x < FEEDBACK_GRID_WIDTH && tile_y < FEEDBACK_GRID_HEIGHT {
            feedback_map.tiles[tile_y][tile_x]
        } else {
            UNREFERENCED_FEEDBACK_MIP
        };

        let mut raw_factor = self.tess_calculator.calculate_tessellation_factor(camera_distance, feedback_mip);

        // Apply anti-thrashing LOD clamp bias if active.
        let bias = self.throttle_governor.active_lod_clamp_bias();
        if bias > 0 {
            raw_factor = (raw_factor / ((1 << bias) as f32)).max(MIN_HARDWARE_TESSELLATION_FACTOR);
        }

        let expansion = (raw_factor * raw_factor) as u32;
        let amplified = base_primitives.saturating_mul(expansion.max(1));

        TessellatedMeshletLod {
            meshlet_id,
            base_primitives,
            amplified_primitives: amplified,
            tessellation_factor: raw_factor,
            feedback_min_mip: feedback_mip,
        }
    }
}

// Comprehensive self-test suite for Direct3D 12 Agility Sampler Feedback and Streaming Tessellation.
pub fn test_sampler_tessellation_subsystem() -> bool {
    crate::serial_write_str("[KNT-SAMPLER-TESS] Initializing Direct3D 12 Agility sampler feedback & tessellation test...\r\n");

    // 1. Verify sampler feedback minification map decoding and mip residency tracking.
    let mut feedback = FeedbackMinificationMap::new();
    feedback.record_sample(0, 0, 2);
    feedback.record_sample(1, 0, 4);
    feedback.record_sample(2, 2, 1);

    if feedback.min_sampled_mip() != 1 || feedback.referenced_tile_count() != 3 {
        return false;
    }

    let hist = feedback.compute_mip_histogram();
    if hist[1] != 1 || hist[2] != 1 || hist[4] != 1 || hist[0] != 0 {
        return false;
    }

    // Residency tracking with initial tail mips 4..8 resident (0x00F0).
    let mut tracker = MipmapResidencyTracker::new(8, 0x00F0);
    let (to_load, to_evict) = tracker.evaluate_residency_demands(&feedback);
    if to_load != 0x000E || to_evict != 0 {
        return false;
    }

    tracker.apply_residency_update(to_load, to_evict);
    if !tracker.is_mip_resident(1) || !tracker.is_mip_resident(2) || !tracker.is_mip_resident(4) {
        return false;
    }
    if tracker.min_resident_mip() != 1 {
        return false;
    }

    // 2. Verify continuous tessellation factor calculation from feedback density and camera distance.
    let calc = ContinuousTessellationCalculator::new(2.0, 100.0);
    let close_factor = calc.calculate_tessellation_factor(5.0, 0);
    if close_factor < 40.0 || close_factor > MAX_HARDWARE_TESSELLATION_FACTOR {
        return false;
    }

    let far_factor = calc.calculate_tessellation_factor(120.0, 5);
    if far_factor != MIN_HARDWARE_TESSELLATION_FACTOR {
        return false;
    }

    let quad_factors = calc.evaluate_quad_patch([5.0, 5.0, 5.0, 5.0], 1);
    if quad_factors.is_culled || quad_factors.edge_factors[0] < 20.0 {
        return false;
    }

    // 3. Verify VRAM bandwidth throttle governor and anti-thrashing controller.
    let mut governor = VramBandwidthThrottleGovernor::new(10_000_000);
    let req1 = ResidencyTransferRequest {
        texture_id: 101,
        mip_level: 1,
        size_bytes: 6_000_000,
        is_upload: true,
    };
    if !governor.request_transfer(&req1) || governor.frame_utilization_percent() != 60 {
        return false;
    }

    let req2 = ResidencyTransferRequest {
        texture_id: 102,
        mip_level: 0,
        size_bytes: 6_000_000,
        is_upload: true,
    };
    // Exceeds 10MB budget: must be throttled.
    if governor.request_transfer(&req2) || governor.total_transfers_throttled() != 1 {
        return false;
    }

    governor.advance_frame();
    if governor.frame_utilization_percent() != 0 {
        return false;
    }

    // 4. Verify streaming tessellation optimizer and meshlet LOD evaluation.
    let optimizer = StreamingTessellationOptimizer::default_handheld();
    let meshlet = optimizer.evaluate_meshlet(1001, 64, 8.0, &feedback, 2, 2);
    if meshlet.meshlet_id != 1001 || meshlet.feedback_min_mip != 1 || meshlet.amplified_primitives <= 64 {
        return false;
    }

    // 5. Emit deterministic boot sentinel.
    crate::serial_write_str("[KNT-D3D12-SAMPLER-TESSELLATION-PASS] Direct3D 12 Agility sampler feedback & tessellation verified\r\n");
    true
}
