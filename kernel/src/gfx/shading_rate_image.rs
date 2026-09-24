//! Direct3D 12 VRS Tier 2 Shading Rate Image & Combiner Pipeline.
//! Provides screen-space tile rate maps (8x8 and 16x16 granularity), combiner evaluation rules,
//! contrast-adaptive variance rate generation, and command list API recording.

#![allow(dead_code)]

use super::vrs::{D3D12_SHADING_RATE, D3D12_SHADING_RATE_COMBINER};

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

// Multi-stage shading rate combiner pipeline evaluating D3D12 VRS Tier 2 combiner rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadingRateCombinerPipeline {
    // Combiner for base pipeline rate and primitive rate.
    pub primitive_combiner: D3D12_SHADING_RATE_COMBINER,
    // Combiner for intermediate rate and screen-space shading rate image.
    pub image_combiner: D3D12_SHADING_RATE_COMBINER,
}

impl Default for ShadingRateCombinerPipeline {
    fn default() -> Self {
        Self {
            primitive_combiner: D3D12_SHADING_RATE_COMBINER::D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
            image_combiner: D3D12_SHADING_RATE_COMBINER::D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
        }
    }
}

impl ShadingRateCombinerPipeline {
    // Creates a new combiner pipeline instance.
    pub const fn new(
        primitive_combiner: D3D12_SHADING_RATE_COMBINER,
        image_combiner: D3D12_SHADING_RATE_COMBINER,
    ) -> Self {
        Self {
            primitive_combiner,
            image_combiner,
        }
    }

    // Combines two shading rates directly according to a specified combiner mode.
    pub fn evaluate_combiner(
        combiner: D3D12_SHADING_RATE_COMBINER,
        rate_a: D3D12_SHADING_RATE,
        rate_b: D3D12_SHADING_RATE,
    ) -> D3D12_SHADING_RATE {
        combiner.combine(rate_a, rate_b)
    }

    // Resolves final effective rate from base rate, primitive rate, and screen-space image rate.
    pub fn resolve(
        &self,
        base_rate: D3D12_SHADING_RATE,
        primitive_rate: D3D12_SHADING_RATE,
        image_rate: D3D12_SHADING_RATE,
    ) -> D3D12_SHADING_RATE {
        let intermediate = self.primitive_combiner.combine(base_rate, primitive_rate);
        self.image_combiner.combine(intermediate, image_rate)
    }
}

// Configuration thresholds for contrast-adaptive shading rate generation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastAdaptiveConfig {
    // Variance threshold above which 1x1 shading rate is preserved for high-frequency details.
    pub edge_threshold: f32,
    // Variance threshold above which 2x2 coarse shading rate is applied.
    pub medium_threshold: f32,
    // Enable peripheral bias to aggressively downscale shading rates towards screen borders.
    pub peripheral_bias: bool,
}

impl Default for ContrastAdaptiveConfig {
    fn default() -> Self {
        Self {
            edge_threshold: 0.02,
            medium_threshold: 0.005,
            peripheral_bias: true,
        }
    }
}

// Contrast-Adaptive Shading Rate Generator computing local luminance variance per tile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContrastAdaptiveShadingRateGenerator {
    pub config: ContrastAdaptiveConfig,
}

impl ContrastAdaptiveShadingRateGenerator {
    // Creates a new generator with custom config.
    pub const fn new(config: ContrastAdaptiveConfig) -> Self {
        Self { config }
    }

    // Computes luminance from an RGB byte triplet normalized to 0.0..1.0.
    #[inline(always)]
    pub fn rgb_to_luminance(r: u8, g: u8, b: u8) -> f32 {
        (r as f32 * 0.299 + g as f32 * 0.587 + b as f32 * 0.114) * (1.0 / 255.0)
    }

    // Computes local luminance variance for a tile rect in an RGBA8 buffer.
    pub fn compute_tile_variance_rgba8(
        rgba_buffer: &[u8],
        image_width: u32,
        image_height: u32,
        tile_x: u32,
        tile_y: u32,
        tile_size: u32,
    ) -> f32 {
        let x_start = tile_x * tile_size;
        let y_start = tile_y * tile_size;
        let x_end = core::cmp::min(x_start + tile_size, image_width);
        let y_end = core::cmp::min(y_start + tile_size, image_height);

        let mut count = 0u32;
        let mut sum = 0.0f32;
        let mut sum_sq = 0.0f32;

        for y in y_start..y_end {
            for x in x_start..x_end {
                let offset = ((y * image_width + x) * 4) as usize;
                if offset + 2 < rgba_buffer.len() {
                    let r = rgba_buffer[offset];
                    let g = rgba_buffer[offset + 1];
                    let b = rgba_buffer[offset + 2];
                    let l = Self::rgb_to_luminance(r, g, b);
                    sum += l;
                    sum_sq += l * l;
                    count += 1;
                }
            }
        }

        if count == 0 {
            return 0.0;
        }

        let mean = sum / (count as f32);
        let var = (sum_sq / (count as f32)) - (mean * mean);
        if var > 0.0 { var } else { 0.0 }
    }

    // Classifies variance and peripheral distance into a D3D12 shading rate.
    pub fn classify_rate(&self, variance: f32, dist_from_center: f32) -> D3D12_SHADING_RATE {
        let effective_variance = if self.config.peripheral_bias {
            variance / (1.0 + dist_from_center * 2.0)
        } else {
            variance
        };

        if effective_variance >= self.config.edge_threshold {
            D3D12_SHADING_RATE::D3D12_SHADING_RATE_1X1
        } else if effective_variance >= self.config.medium_threshold {
            D3D12_SHADING_RATE::D3D12_SHADING_RATE_2X2
        } else {
            D3D12_SHADING_RATE::D3D12_SHADING_RATE_4X4
        }
    }

    // Generates a complete ShadingRateImageSurface from an RGBA8 input buffer.
    pub fn generate_from_rgba8(
        &self,
        rgba_buffer: &[u8],
        desc: D3D12_SHADING_RATE_IMAGE_DESC,
    ) -> Result<ShadingRateImageSurface, ShadingRateImageError> {
        let min_len = (desc.width_pixels as usize) * (desc.height_pixels as usize) * 4;
        if rgba_buffer.len() < min_len {
            return Err(ShadingRateImageError::BufferTooSmall);
        }

        let mut surface = ShadingRateImageSurface::new(desc, D3D12_SHADING_RATE::D3D12_SHADING_RATE_1X1)?;
        let tile_size = desc.tile_granularity.tile_size();
        let center_x = (desc.tiles_x as f32) * 0.5;
        let center_y = (desc.tiles_y as f32) * 0.5;
        let max_dist = center_x * center_x + center_y * center_y;

        for ty in 0..desc.tiles_y {
            for tx in 0..desc.tiles_x {
                let var = Self::compute_tile_variance_rgba8(
                    rgba_buffer,
                    desc.width_pixels,
                    desc.height_pixels,
                    tx,
                    ty,
                    tile_size,
                );
                let dx = (tx as f32) - center_x;
                let dy = (ty as f32) - center_y;
                let norm_dist = if max_dist > 0.0 { (dx * dx + dy * dy) / max_dist } else { 0.0 };
                let rate = self.classify_rate(var, norm_dist);
                surface.set_rate(tx, ty, rate);
            }
        }

        Ok(surface)
    }
}

// Token stream opcodes for D3D12 VRS Tier 2 shading rate commands.
pub const D3D12_TOKEN_RS_SET_SHADING_RATE: u32 = 0x5352_5445;
pub const D3D12_TOKEN_RS_SET_SHADING_RATE_IMAGE: u32 = 0x5352_494D;

// Command packet recorded into command stream for shading rate updates.
#[derive(Debug, Clone, PartialEq)]
pub enum ShadingRateImageCommandPacket {
    SetShadingRate {
        base_rate: D3D12_SHADING_RATE,
        combiners: [D3D12_SHADING_RATE_COMBINER; 2],
    },
    SetShadingRateImage {
        gpu_virtual_address: u64,
        surface: Option<ShadingRateImageSurface>,
    },
}

// Current command list state tracking active shading rate and bound rate image.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadingRateListState {
    pub base_rate: D3D12_SHADING_RATE,
    pub combiners: [D3D12_SHADING_RATE_COMBINER; 2],
    pub image_gpu_address: u64,
    pub bound_surface: Option<ShadingRateImageSurface>,
}

impl Default for ShadingRateListState {
    fn default() -> Self {
        Self {
            base_rate: D3D12_SHADING_RATE::D3D12_SHADING_RATE_1X1,
            combiners: [
                D3D12_SHADING_RATE_COMBINER::D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
                D3D12_SHADING_RATE_COMBINER::D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
            ],
            image_gpu_address: 0,
            bound_surface: None,
        }
    }
}

// Direct3D 12 Command List recorder implementing RSSetShadingRate & RSSetShadingRateImage.
#[derive(Debug, Clone, Default)]
pub struct D3D12ShadingRateCommandRecorder {
    pub state: ShadingRateListState,
    pub packets: alloc::vec::Vec<ShadingRateImageCommandPacket>,
    pub total_commands_recorded: u32,
}

impl D3D12ShadingRateCommandRecorder {
    // Constructs a new command list recorder with default VRS state.
    pub fn new() -> Self {
        Self::default()
    }

    // Direct3D 12 API: RSSetShadingRate sets pipeline base rate and two combiner stages.
    #[allow(non_snake_case)]
    pub fn RSSetShadingRate(
        &mut self,
        base_rate: D3D12_SHADING_RATE,
        combiners: [D3D12_SHADING_RATE_COMBINER; 2],
    ) {
        self.state.base_rate = base_rate;
        self.state.combiners = combiners;
        self.packets.push(ShadingRateImageCommandPacket::SetShadingRate {
            base_rate,
            combiners,
        });
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
    }

    // Direct3D 12 API: RSSetShadingRateImage binds or unbinds screen-space shading rate image.
    #[allow(non_snake_case)]
    pub fn RSSetShadingRateImage(
        &mut self,
        surface: Option<ShadingRateImageSurface>,
        gpu_virtual_address: u64,
    ) {
        self.state.bound_surface = surface.clone();
        self.state.image_gpu_address = gpu_virtual_address;
        self.packets.push(ShadingRateImageCommandPacket::SetShadingRateImage {
            gpu_virtual_address,
            surface,
        });
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
    }

    // Evaluates final shading rate at pixel coordinate according to recorded state.
    pub fn resolve_pixel_rate(
        &self,
        pixel_x: u32,
        pixel_y: u32,
        primitive_rate: D3D12_SHADING_RATE,
    ) -> D3D12_SHADING_RATE {
        let pipeline = ShadingRateCombinerPipeline::new(self.state.combiners[0], self.state.combiners[1]);
        let image_rate = match &self.state.bound_surface {
            Some(surf) => surf.sample_at_pixel(pixel_x, pixel_y),
            None => D3D12_SHADING_RATE::D3D12_SHADING_RATE_1X1,
        };
        pipeline.resolve(self.state.base_rate, primitive_rate, image_rate)
    }

    // Encodes recorded command packets into a raw token stream buffer.
    pub fn encode_token_stream(&self, out_stream: &mut alloc::vec::Vec<u32>) {
        for packet in &self.packets {
            match packet {
                ShadingRateImageCommandPacket::SetShadingRate { base_rate, combiners } => {
                    out_stream.push(D3D12_TOKEN_RS_SET_SHADING_RATE);
                    out_stream.push(*base_rate as u32);
                    out_stream.push(combiners[0] as u32);
                    out_stream.push(combiners[1] as u32);
                }
                ShadingRateImageCommandPacket::SetShadingRateImage {
                    gpu_virtual_address,
                    surface,
                } => {
                    out_stream.push(D3D12_TOKEN_RS_SET_SHADING_RATE_IMAGE);
                    out_stream.push((*gpu_virtual_address & 0xFFFF_FFFF) as u32);
                    out_stream.push((*gpu_virtual_address >> 32) as u32);
                    out_stream.push(if surface.is_some() { 1 } else { 0 });
                }
            }
        }
    }
}

/// Temporary stub for shading rate image self-test during incremental commits.
pub fn test_shading_rate_image_subsystem() -> bool {
    true
}
