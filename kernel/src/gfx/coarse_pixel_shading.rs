//! Direct3D 12 Agility Coarse Pixel Shading (Tier 2 Shading Rate Palette & Tile Mask Compiler).
//! Provides hardware-accelerated coarse pixel shading rates, tile masks, and peripheral foveation.

#![allow(dead_code, non_camel_case_types, non_snake_case, unused_imports)]

extern crate alloc;
use alloc::vec::Vec;

// Direct3D 12 coarse pixel shading rates supported in Tier 2 hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CoarseShadingRate {
    Rate1x1 = 0,
    Rate1x2 = 1,
    Rate2x1 = 2,
    Rate2x2 = 3,
    Rate2x4 = 4,
    Rate4x2 = 5,
    Rate4x4 = 6,
}

impl CoarseShadingRate {
    // Width dimension in pixels for coarse shading rate.
    pub const fn width(self) -> u32 {
        match self {
            Self::Rate1x1 | Self::Rate1x2 => 1,
            Self::Rate2x1 | Self::Rate2x2 | Self::Rate2x4 => 2,
            Self::Rate4x2 | Self::Rate4x4 => 4,
        }
    }

    // Height dimension in pixels for coarse shading rate.
    pub const fn height(self) -> u32 {
        match self {
            Self::Rate1x1 | Self::Rate2x1 => 1,
            Self::Rate1x2 | Self::Rate2x2 | Self::Rate4x2 => 2,
            Self::Rate2x4 | Self::Rate4x4 => 4,
        }
    }

    // Pixel area covered by a single coarse fragment invocation.
    pub const fn pixel_area(self) -> u32 {
        self.width() * self.height()
    }

    // D3D12 standard hardware bitwise encoding for the shading rate.
    pub const fn to_d3d12_encoding(self) -> u32 {
        match self {
            Self::Rate1x1 => 0x0,
            Self::Rate1x2 => 0x1,
            Self::Rate2x1 => 0x4,
            Self::Rate2x2 => 0x5,
            Self::Rate2x4 => 0x6,
            Self::Rate4x2 => 0x9,
            Self::Rate4x4 => 0xa,
        }
    }

    // Constructs coarse rate from pixel dimensions if valid.
    pub const fn from_dimensions(width: u32, height: u32) -> Option<Self> {
        match (width, height) {
            (1, 1) => Some(Self::Rate1x1),
            (1, 2) => Some(Self::Rate1x2),
            (2, 1) => Some(Self::Rate2x1),
            (2, 2) => Some(Self::Rate2x2),
            (2, 4) => Some(Self::Rate2x4),
            (4, 2) => Some(Self::Rate4x2),
            (4, 4) => Some(Self::Rate4x4),
            _ => None,
        }
    }

    // Constructs coarse rate from D3D12 hardware bitwise encoding.
    pub const fn from_d3d12_encoding(encoding: u32) -> Option<Self> {
        match encoding {
            0x0 => Some(Self::Rate1x1),
            0x1 => Some(Self::Rate1x2),
            0x4 => Some(Self::Rate2x1),
            0x5 => Some(Self::Rate2x2),
            0x6 => Some(Self::Rate2x4),
            0x9 => Some(Self::Rate4x2),
            0xa => Some(Self::Rate4x4),
            _ => None,
        }
    }
}

// Maximum entries in a Direct3D 12 hardware shading rate palette.
pub const D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES: usize = 16;

// Direct3D 12 16-entry shading rate palette mapping 4-bit indices to coarse rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadingRatePalette {
    // Array of palette entries storing coarse rates.
    pub entries: [CoarseShadingRate; D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES],
    // Number of active programmed palette entries.
    pub active_count: u32,
}

impl Default for ShadingRatePalette {
    fn default() -> Self {
        Self::new_standard()
    }
}

impl ShadingRatePalette {
    // Creates standard default palette populated with all standard rates.
    pub const fn new_standard() -> Self {
        let mut entries = [CoarseShadingRate::Rate1x1; D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES];
        entries[0] = CoarseShadingRate::Rate1x1;
        entries[1] = CoarseShadingRate::Rate1x2;
        entries[2] = CoarseShadingRate::Rate2x1;
        entries[3] = CoarseShadingRate::Rate2x2;
        entries[4] = CoarseShadingRate::Rate2x4;
        entries[5] = CoarseShadingRate::Rate4x2;
        entries[6] = CoarseShadingRate::Rate4x4;
        Self {
            entries,
            active_count: 7,
        }
    }

    // Looks up coarse shading rate for given palette index.
    pub fn lookup(&self, index: u8) -> CoarseShadingRate {
        let idx = (index as usize) % D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES;
        self.entries[idx]
    }

    // Finds palette index for a given rate if present.
    pub fn find_index(&self, rate: CoarseShadingRate) -> Option<u8> {
        let mut i = 0usize;
        while i < (self.active_count as usize) && i < D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES {
            if self.entries[i] == rate {
                return Some(i as u8);
            }
            i += 1;
        }
        None
    }

    // Programs a specific palette slot with a coarse shading rate.
    pub fn set_entry(&mut self, slot: usize, rate: CoarseShadingRate) -> bool {
        if slot >= D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES {
            return false;
        }
        self.entries[slot] = rate;
        if (slot as u32) >= self.active_count {
            self.active_count = (slot as u32) + 1;
        }
        true
    }
}

// Thresholds for edge-preserving contrast detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContrastThresholds {
    // Contrast delta above which fine shading (1x1) is enforced to avoid edge aliasing.
    pub high_contrast_threshold: u8,
    // Contrast delta above which medium shading (2x2) is used instead of coarse.
    pub medium_contrast_threshold: u8,
}

impl Default for ContrastThresholds {
    fn default() -> Self {
        Self {
            high_contrast_threshold: 64,
            medium_contrast_threshold: 24,
        }
    }
}

// Edge-preserving contrast detector classifying tile complexity from luminance deltas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContrastEdgeDetector {
    // Configured contrast thresholds.
    pub thresholds: ContrastThresholds,
}

impl ContrastEdgeDetector {
    // Creates a new contrast edge detector with default thresholds.
    pub fn new() -> Self {
        Self {
            thresholds: ContrastThresholds::default(),
        }
    }

    // Creates detector with custom contrast sensitivity thresholds.
    pub const fn with_thresholds(high: u8, medium: u8) -> Self {
        Self {
            thresholds: ContrastThresholds {
                high_contrast_threshold: high,
                medium_contrast_threshold: medium,
            },
        }
    }

    // Computes max-min luminance delta across tile samples.
    pub fn compute_tile_contrast(&self, samples: &[u8]) -> u8 {
        if samples.is_empty() {
            return 0;
        }
        let mut min_val = samples[0];
        let mut max_val = samples[0];
        let mut i = 1;
        while i < samples.len() {
            let v = samples[i];
            if v < min_val {
                min_val = v;
            }
            if v > max_val {
                max_val = v;
            }
            i += 1;
        }
        max_val.saturating_sub(min_val)
    }

    // Classifies appropriate coarse shading rate according to detected contrast delta.
    pub fn classify_rate(&self, contrast: u8) -> CoarseShadingRate {
        if contrast >= self.thresholds.high_contrast_threshold {
            CoarseShadingRate::Rate1x1
        } else if contrast >= self.thresholds.medium_contrast_threshold {
            CoarseShadingRate::Rate2x2
        } else {
            CoarseShadingRate::Rate4x4
        }
    }
}

// Compiled 2D tile shading rate image mask mapping screen tiles to palette indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileShadingRateMask {
    // Number of horizontal tiles across screen.
    pub width_tiles: u32,
    // Number of vertical tiles down screen.
    pub height_tiles: u32,
    // Screen pixel width and height per tile (e.g. 8 or 16).
    pub tile_size: u32,
    // Shading rate palette used by this mask.
    pub palette: ShadingRatePalette,
    // Compact array of palette indices in row-major order.
    pub tile_indices: alloc::vec::Vec<u8>,
}

impl TileShadingRateMask {
    // Allocates a new tile mask with initial uniform rate.
    pub fn new(
        width_tiles: u32,
        height_tiles: u32,
        tile_size: u32,
        palette: ShadingRatePalette,
    ) -> Self {
        let total_tiles = (width_tiles as usize) * (height_tiles as usize);
        let default_idx = palette.find_index(CoarseShadingRate::Rate1x1).unwrap_or(0);
        let mut tile_indices = alloc::vec::Vec::new();
        tile_indices.resize(total_tiles, default_idx);
        Self {
            width_tiles,
            height_tiles,
            tile_size,
            palette,
            tile_indices,
        }
    }

    // Queries coarse shading rate at specified tile coordinate.
    pub fn get_rate(&self, tile_x: u32, tile_y: u32) -> Option<CoarseShadingRate> {
        if tile_x >= self.width_tiles || tile_y >= self.height_tiles {
            return None;
        }
        let idx = (tile_y * self.width_tiles + tile_x) as usize;
        let palette_idx = self.tile_indices[idx];
        Some(self.palette.lookup(palette_idx))
    }

    // Sets coarse shading rate at specified tile coordinate.
    pub fn set_rate(&mut self, tile_x: u32, tile_y: u32, rate: CoarseShadingRate) -> bool {
        if tile_x >= self.width_tiles || tile_y >= self.height_tiles {
            return false;
        }
        let palette_idx = match self.palette.find_index(rate) {
            Some(i) => i,
            None => return false,
        };
        let idx = (tile_y * self.width_tiles + tile_x) as usize;
        self.tile_indices[idx] = palette_idx;
        true
    }

    // Total number of tiles in mask.
    pub fn total_tiles(&self) -> usize {
        (self.width_tiles as usize) * (self.height_tiles as usize)
    }
}

// Compiler translating screen contrast maps into coarse shading rate tile masks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileShadingRateMaskCompiler {
    // Screen-space tile granularity in pixels (e.g. 8 or 16).
    pub tile_size: u32,
    // Contrast analyzer for edge preservation.
    pub contrast_detector: ContrastEdgeDetector,
    // Active shading rate palette.
    pub palette: ShadingRatePalette,
    // Total compiled tile masks.
    pub compiled_masks_count: u64,
}

impl TileShadingRateMaskCompiler {
    // Creates a mask compiler with specified tile size and default detector.
    pub fn new(tile_size: u32) -> Self {
        Self {
            tile_size: if tile_size == 0 { 16 } else { tile_size },
            contrast_detector: ContrastEdgeDetector::new(),
            palette: ShadingRatePalette::new_standard(),
            compiled_masks_count: 0,
        }
    }

    // Compiles a tile mask from per-tile luminance sample buffers.
    pub fn compile_from_tile_samples(
        &mut self,
        screen_width: u32,
        screen_height: u32,
        tile_samples: &[&[u8]],
    ) -> TileShadingRateMask {
        let width_tiles = (screen_width + self.tile_size - 1) / self.tile_size;
        let height_tiles = (screen_height + self.tile_size - 1) / self.tile_size;
        let mut mask =
            TileShadingRateMask::new(width_tiles, height_tiles, self.tile_size, self.palette);

        let total = (width_tiles * height_tiles) as usize;
        let mut i = 0usize;
        while i < total && i < tile_samples.len() {
            let contrast = self.contrast_detector.compute_tile_contrast(tile_samples[i]);
            let rate = self.contrast_detector.classify_rate(contrast);
            let tx = (i as u32) % width_tiles;
            let ty = (i as u32) / width_tiles;
            mask.set_rate(tx, ty, rate);
            i += 1;
        }

        self.compiled_masks_count += 1;
        mask
    }

    // Compiles a uniform shading rate tile mask across the entire screen.
    pub fn compile_uniform(
        &mut self,
        screen_width: u32,
        screen_height: u32,
        rate: CoarseShadingRate,
    ) -> TileShadingRateMask {
        let width_tiles = (screen_width + self.tile_size - 1) / self.tile_size;
        let height_tiles = (screen_height + self.tile_size - 1) / self.tile_size;
        let mut mask =
            TileShadingRateMask::new(width_tiles, height_tiles, self.tile_size, self.palette);
        let palette_idx = self.palette.find_index(rate).unwrap_or(0);
        let mut i = 0usize;
        while i < mask.tile_indices.len() {
            mask.tile_indices[i] = palette_idx;
            i += 1;
        }
        self.compiled_masks_count += 1;
        mask
    }
}

// Configuration for handheld peripheral foveated shading rate generation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeripheralFoveationConfig {
    // Horizontal focal center coordinate in screen pixels.
    pub center_x: f32,
    // Vertical focal center coordinate in screen pixels.
    pub center_y: f32,
    // Inner foveal radius in pixels where full 1x1 detail is maintained.
    pub inner_radius: f32,
    // Mid transition radius in pixels beyond which coarse 4x4 shading applies.
    pub outer_radius: f32,
    // Whether to preserve high-contrast edges in peripheral regions.
    pub preserve_edges: bool,
}

impl Default for PeripheralFoveationConfig {
    fn default() -> Self {
        Self {
            center_x: 640.0,
            center_y: 400.0,
            inner_radius: 200.0,
            outer_radius: 400.0,
            preserve_edges: true,
        }
    }
}

// Handheld peripheral foveated shading generator reducing GPU load at display edges.
#[derive(Debug, Clone, PartialEq)]
pub struct PeripheralFoveatedShadingGenerator {
    // Active foveation configuration.
    pub config: PeripheralFoveationConfig,
    // Contrast detector for edge preservation.
    pub contrast_detector: ContrastEdgeDetector,
    // Total generated foveated masks.
    pub generated_count: u64,
}

impl PeripheralFoveatedShadingGenerator {
    // Creates a new peripheral foveated shading generator.
    pub fn new(config: PeripheralFoveationConfig) -> Self {
        Self {
            config,
            contrast_detector: ContrastEdgeDetector::new(),
            generated_count: 0,
        }
    }

    // Evaluates radial coarse shading rate at specified tile center.
    pub fn evaluate_tile_rate(&self, tile_center_x: f32, tile_center_y: f32) -> CoarseShadingRate {
        let dx = tile_center_x - self.config.center_x;
        let dy = tile_center_y - self.config.center_y;
        let dist_sq = dx * dx + dy * dy;
        let inner_sq = self.config.inner_radius * self.config.inner_radius;
        let outer_sq = self.config.outer_radius * self.config.outer_radius;

        if dist_sq <= inner_sq {
            CoarseShadingRate::Rate1x1
        } else if dist_sq <= outer_sq {
            CoarseShadingRate::Rate2x2
        } else {
            CoarseShadingRate::Rate4x4
        }
    }

    // Generates a foveated tile shading rate mask for given screen dimensions.
    pub fn generate_mask(
        &mut self,
        screen_width: u32,
        screen_height: u32,
        tile_size: u32,
        palette: ShadingRatePalette,
        tile_contrasts: Option<&[u8]>,
    ) -> TileShadingRateMask {
        let ts = if tile_size == 0 { 16 } else { tile_size };
        let width_tiles = (screen_width + ts - 1) / ts;
        let height_tiles = (screen_height + ts - 1) / ts;
        let mut mask = TileShadingRateMask::new(width_tiles, height_tiles, ts, palette);

        let half_tile = (ts as f32) * 0.5;
        let total_tiles = (width_tiles * height_tiles) as usize;

        let mut i = 0usize;
        while i < total_tiles {
            let tx = (i as u32) % width_tiles;
            let ty = (i as u32) / width_tiles;
            let px = (tx * ts) as f32 + half_tile;
            let py = (ty * ts) as f32 + half_tile;

            let mut rate = self.evaluate_tile_rate(px, py);

            // If edge preservation enabled, clamp rate on high contrast edges.
            if self.config.preserve_edges {
                if let Some(contrasts) = tile_contrasts {
                    if i < contrasts.len() {
                        let c = contrasts[i];
                        if c >= self.contrast_detector.thresholds.high_contrast_threshold {
                            rate = CoarseShadingRate::Rate1x1;
                        } else if c >= self.contrast_detector.thresholds.medium_contrast_threshold
                            && rate == CoarseShadingRate::Rate4x4
                        {
                            rate = CoarseShadingRate::Rate2x2;
                        }
                    }
                }
            }

            mask.set_rate(tx, ty, rate);
            i += 1;
        }

        self.generated_count += 1;
        mask
    }

    // Computes estimated fragment shading workload reduction percentage (0 to 100).
    pub fn compute_workload_reduction_percentage(mask: &TileShadingRateMask) -> u32 {
        let total_tiles = mask.total_tiles();
        if total_tiles == 0 {
            return 0;
        }
        let mut total_effective_invocations = 0u64;
        let tile_pixels = (mask.tile_size * mask.tile_size) as u64;
        let total_baseline_invocations = (total_tiles as u64) * tile_pixels;

        let mut i = 0usize;
        while i < mask.tile_indices.len() {
            let idx = mask.tile_indices[i];
            let rate = mask.palette.lookup(idx);
            let invocations = tile_pixels / (rate.pixel_area() as u64);
            total_effective_invocations += invocations;
            i += 1;
        }

        if total_baseline_invocations == 0 {
            return 0;
        }
        let saved = total_baseline_invocations.saturating_sub(total_effective_invocations);
        ((saved * 100) / total_baseline_invocations) as u32
    }
}

// Operations supported by D3D12 shading rate combiners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadingRateCombinerOp {
    Passthrough,
    Override,
    Min,
    Max,
    Sum,
}

// Clamps arbitrary width and height to closest valid D3D12 Tier 2 coarse rate.
pub const fn clamp_to_valid_rate(width: u32, height: u32) -> CoarseShadingRate {
    let mut w = if width < 1 {
        1
    } else if width > 4 {
        4
    } else {
        width
    };
    let mut h = if height < 1 {
        1
    } else if height > 4 {
        4
    } else {
        height
    };

    if w == 3 {
        w = 2;
    }
    if h == 3 {
        h = 2;
    }
    if w == 1 && h == 4 {
        h = 2;
    }
    if w == 4 && h == 1 {
        w = 2;
    }

    match (w, h) {
        (1, 1) => CoarseShadingRate::Rate1x1,
        (1, 2) => CoarseShadingRate::Rate1x2,
        (2, 1) => CoarseShadingRate::Rate2x1,
        (2, 2) => CoarseShadingRate::Rate2x2,
        (2, 4) => CoarseShadingRate::Rate2x4,
        (4, 2) => CoarseShadingRate::Rate4x2,
        _ => CoarseShadingRate::Rate4x4,
    }
}

// Evaluates a single combiner operation between source and modifier rates.
pub fn evaluate_coarse_combiner_op(
    source: CoarseShadingRate,
    modifier: CoarseShadingRate,
    op: ShadingRateCombinerOp,
) -> CoarseShadingRate {
    match op {
        ShadingRateCombinerOp::Passthrough => source,
        ShadingRateCombinerOp::Override => modifier,
        ShadingRateCombinerOp::Min => {
            let w = core::cmp::min(source.width(), modifier.width());
            let h = core::cmp::min(source.height(), modifier.height());
            clamp_to_valid_rate(w, h)
        }
        ShadingRateCombinerOp::Max => {
            let w = core::cmp::max(source.width(), modifier.width());
            let h = core::cmp::max(source.height(), modifier.height());
            clamp_to_valid_rate(w, h)
        }
        ShadingRateCombinerOp::Sum => {
            let w = source.width() + modifier.width() - 1;
            let h = source.height() + modifier.height() - 1;
            clamp_to_valid_rate(w, h)
        }
    }
}

// Command list shading rate state tracker managing dynamic bindings and combiner pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandListShadingRateState {
    // Current per-draw base coarse shading rate.
    pub base_shading_rate: CoarseShadingRate,
    // Combiner rule between primitive rate and image rate.
    pub combiner_primitive_image: ShadingRateCombinerOp,
    // Combiner rule between intermediate result and base rate.
    pub combiner_result_base: ShadingRateCombinerOp,
    // GPU virtual address of bound tile shading rate image, if any.
    pub bound_image_gpu_va: Option<u64>,
    // CPU-accessible tile mask for bound shading rate image, if provided.
    pub bound_mask: Option<TileShadingRateMask>,
    // Flag indicating state changes requiring hardware command packet update.
    pub is_dirty: bool,
    // Total RSSetShadingRate invocations recorded.
    pub total_set_shading_rate_calls: u64,
    // Total draws evaluated through combiner pipeline.
    pub total_draws_evaluated: u64,
}

impl Default for CommandListShadingRateState {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandListShadingRateState {
    // Creates a new command list shading rate state with default passthrough rules.
    pub fn new() -> Self {
        Self {
            base_shading_rate: CoarseShadingRate::Rate1x1,
            combiner_primitive_image: ShadingRateCombinerOp::Passthrough,
            combiner_result_base: ShadingRateCombinerOp::Passthrough,
            bound_image_gpu_va: None,
            bound_mask: None,
            is_dirty: false,
            total_set_shading_rate_calls: 0,
            total_draws_evaluated: 0,
        }
    }

    // Updates base shading rate and combiner rules (matches RSSetShadingRate).
    pub fn set_shading_rate(
        &mut self,
        base_rate: CoarseShadingRate,
        combiner_prim_img: ShadingRateCombinerOp,
        combiner_res_base: ShadingRateCombinerOp,
    ) {
        if self.base_shading_rate != base_rate
            || self.combiner_primitive_image != combiner_prim_img
            || self.combiner_result_base != combiner_res_base
        {
            self.base_shading_rate = base_rate;
            self.combiner_primitive_image = combiner_prim_img;
            self.combiner_result_base = combiner_res_base;
            self.is_dirty = true;
        }
        self.total_set_shading_rate_calls += 1;
    }

    // Binds a screen-space shading rate image (matches RSSetShadingRateImage).
    pub fn set_shading_rate_image(&mut self, gpu_va: u64, mask: Option<TileShadingRateMask>) {
        if self.bound_image_gpu_va != Some(gpu_va) {
            self.bound_image_gpu_va = Some(gpu_va);
            self.is_dirty = true;
        }
        self.bound_mask = mask;
    }

    // Unbinds currently attached shading rate image.
    pub fn clear_shading_rate_image(&mut self) {
        if self.bound_image_gpu_va.is_some() {
            self.bound_image_gpu_va = None;
            self.bound_mask = None;
            self.is_dirty = true;
        }
    }

    // Evaluates final coarse shading rate for a draw given primitive rate and tile position.
    pub fn evaluate_effective_rate(
        &mut self,
        primitive_rate: Option<CoarseShadingRate>,
        tile_coord: Option<(u32, u32)>,
    ) -> CoarseShadingRate {
        self.total_draws_evaluated += 1;

        let prim = primitive_rate.unwrap_or(CoarseShadingRate::Rate1x1);
        let img = match (tile_coord, &self.bound_mask) {
            (Some((tx, ty)), Some(mask)) => {
                mask.get_rate(tx, ty).unwrap_or(CoarseShadingRate::Rate1x1)
            }
            _ => CoarseShadingRate::Rate1x1,
        };

        // Stage 1: combine primitive rate with image rate.
        let intermediate = evaluate_coarse_combiner_op(prim, img, self.combiner_primitive_image);

        // Stage 2: combine intermediate result with per-draw base rate.
        evaluate_coarse_combiner_op(
            intermediate,
            self.base_shading_rate,
            self.combiner_result_base,
        )
    }

    // Clears dirty flag when state is flushed to command stream.
    pub fn commit_bindings(&mut self) -> bool {
        let was_dirty = self.is_dirty;
        self.is_dirty = false;
        was_dirty
    }
}

// Comprehensive verification self-test suite for Direct3D 12 Agility coarse pixel shading.
pub fn test_coarse_pixel_shading_subsystem() -> bool {
    // 1. Verify Tier 2 shading rate palette, encodings, and dimensions.
    let palette = ShadingRatePalette::new_standard();
    if palette.lookup(0) != CoarseShadingRate::Rate1x1
        || palette.lookup(3) != CoarseShadingRate::Rate2x2
        || palette.lookup(6) != CoarseShadingRate::Rate4x4
    {
        return false;
    }
    if palette.find_index(CoarseShadingRate::Rate2x4) != Some(4) {
        return false;
    }

    if CoarseShadingRate::Rate1x1.to_d3d12_encoding() != 0x0
        || CoarseShadingRate::Rate2x2.to_d3d12_encoding() != 0x5
        || CoarseShadingRate::Rate4x4.to_d3d12_encoding() != 0xa
        || CoarseShadingRate::from_d3d12_encoding(0x6) != Some(CoarseShadingRate::Rate2x4)
    {
        return false;
    }

    if CoarseShadingRate::Rate4x4.width() != 4
        || CoarseShadingRate::Rate4x4.height() != 4
        || CoarseShadingRate::Rate4x4.pixel_area() != 16
    {
        return false;
    }

    // 2. Verify screen-space tile mask compiler and edge-preserving contrast detector.
    let detector = ContrastEdgeDetector::new();
    let high_c = detector.compute_tile_contrast(&[10, 80]);
    let mid_c = detector.compute_tile_contrast(&[20, 50]);
    let low_c = detector.compute_tile_contrast(&[100, 105]);

    if detector.classify_rate(high_c) != CoarseShadingRate::Rate1x1
        || detector.classify_rate(mid_c) != CoarseShadingRate::Rate2x2
        || detector.classify_rate(low_c) != CoarseShadingRate::Rate4x4
    {
        return false;
    }

    let mut compiler = TileShadingRateMaskCompiler::new(16);
    let s_high: [u8; 2] = [10, 80];
    let s_mid: [u8; 2] = [20, 50];
    let s_low: [u8; 2] = [100, 105];
    let tile_samples: [&[u8]; 4] = [&s_high, &s_mid, &s_low, &s_high];

    let mask = compiler.compile_from_tile_samples(32, 32, &tile_samples);
    if mask.get_rate(0, 0) != Some(CoarseShadingRate::Rate1x1)
        || mask.get_rate(1, 0) != Some(CoarseShadingRate::Rate2x2)
        || mask.get_rate(0, 1) != Some(CoarseShadingRate::Rate4x4)
        || mask.get_rate(1, 1) != Some(CoarseShadingRate::Rate1x1)
    {
        return false;
    }

    // 3. Verify handheld peripheral foveated shading generator and workload reduction.
    let fov_cfg = PeripheralFoveationConfig {
        center_x: 320.0,
        center_y: 200.0,
        inner_radius: 100.0,
        outer_radius: 200.0,
        preserve_edges: true,
    };
    let mut fov_gen = PeripheralFoveatedShadingGenerator::new(fov_cfg);

    if fov_gen.evaluate_tile_rate(320.0, 200.0) != CoarseShadingRate::Rate1x1
        || fov_gen.evaluate_tile_rate(320.0, 320.0) != CoarseShadingRate::Rate2x2
        || fov_gen.evaluate_tile_rate(10.0, 10.0) != CoarseShadingRate::Rate4x4
    {
        return false;
    }

    let fov_mask = fov_gen.generate_mask(640, 400, 16, palette, None);
    let reduction =
        PeripheralFoveatedShadingGenerator::compute_workload_reduction_percentage(&fov_mask);
    if reduction < 30 {
        return false;
    }

    // 4. Verify command list dynamic shading rate binding and combiner evaluation.
    let mut cmd_state = CommandListShadingRateState::new();
    cmd_state.set_shading_rate(
        CoarseShadingRate::Rate2x2,
        ShadingRateCombinerOp::Passthrough,
        ShadingRateCombinerOp::Override,
    );
    if !cmd_state.is_dirty {
        return false;
    }
    let flushed = cmd_state.commit_bindings();
    if !flushed || cmd_state.is_dirty {
        return false;
    }

    let effective = cmd_state.evaluate_effective_rate(Some(CoarseShadingRate::Rate1x2), None);
    if effective != CoarseShadingRate::Rate2x2 {
        return false;
    }

    let min_rate = evaluate_coarse_combiner_op(
        CoarseShadingRate::Rate2x4,
        CoarseShadingRate::Rate4x2,
        ShadingRateCombinerOp::Min,
    );
    if min_rate != CoarseShadingRate::Rate2x2 {
        return false;
    }

    let max_rate = evaluate_coarse_combiner_op(
        CoarseShadingRate::Rate1x2,
        CoarseShadingRate::Rate2x1,
        ShadingRateCombinerOp::Max,
    );
    if max_rate != CoarseShadingRate::Rate2x2 {
        return false;
    }

    // 5. Emit deterministic boot sentinel.
    crate::serial_write_str(
        "[KNT-D3D12-COARSE-SHADING-PASS] Direct3D 12 agility coarse pixel shading verified\r\n",
    );
    true
}
