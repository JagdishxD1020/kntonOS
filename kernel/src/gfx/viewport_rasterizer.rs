//! Direct3D 12 Agility Dynamic Viewport & Scissor Rect Caching & Sub-Pixel Precision Rasterization Governor.
//! Implements multi-viewport state caching, conservative rasterization Tier 3 coverage mask generation,
//! and Variable Rate Shading (VRS) Tier 2 tile combiners with viewport-level coarse overrides.

#![allow(dead_code, non_camel_case_types)]

extern crate alloc;

// Maximum viewports and scissor rects supported simultaneously in Direct3D 12.
pub const D3D12_VIEWPORT_AND_SCISSORRECT_MAX_INDEX: usize = 16;

// Viewport descriptor matching D3D12_VIEWPORT.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct D3d12DynamicViewport {
    pub top_left_x: f32,
    pub top_left_y: f32,
    pub width: f32,
    pub height: f32,
    pub min_depth: f32,
    pub max_depth: f32,
}

impl Default for D3d12DynamicViewport {
    fn default() -> Self {
        Self {
            top_left_x: 0.0,
            top_left_y: 0.0,
            width: 0.0,
            height: 0.0,
            min_depth: 0.0,
            max_depth: 1.0,
        }
    }
}

// Scissor rectangle descriptor matching D3D12_RECT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3d12DynamicScissorRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

// Multi-viewport and scissor rect dynamic state cache.
#[derive(Debug, Clone)]
pub struct DynamicViewportStateCache {
    pub cached_viewports: [D3d12DynamicViewport; D3D12_VIEWPORT_AND_SCISSORRECT_MAX_INDEX],
    pub cached_scissors: [D3d12DynamicScissorRect; D3D12_VIEWPORT_AND_SCISSORRECT_MAX_INDEX],
    pub active_viewport_count: usize,
    pub active_scissor_count: usize,
    pub total_viewport_sets: u64,
    pub redundant_viewport_flushes_avoided: u64,
    pub total_scissor_sets: u64,
    pub redundant_scissor_flushes_avoided: u64,
}

impl DynamicViewportStateCache {
    pub fn new() -> Self {
        Self {
            cached_viewports: [D3d12DynamicViewport::default(); D3D12_VIEWPORT_AND_SCISSORRECT_MAX_INDEX],
            cached_scissors: [D3d12DynamicScissorRect::default(); D3D12_VIEWPORT_AND_SCISSORRECT_MAX_INDEX],
            active_viewport_count: 0,
            active_scissor_count: 0,
            total_viewport_sets: 0,
            redundant_viewport_flushes_avoided: 0,
            total_scissor_sets: 0,
            redundant_scissor_flushes_avoided: 0,
        }
    }

    // Updates viewport array, returning true if state changed (hardware flush needed).
    pub fn set_viewports(&mut self, viewports: &[D3d12DynamicViewport]) -> bool {
        self.total_viewport_sets += 1;
        let count = viewports.len().min(D3D12_VIEWPORT_AND_SCISSORRECT_MAX_INDEX);

        if self.active_viewport_count == count && self.cached_viewports[..count] == viewports[..count] {
            self.redundant_viewport_flushes_avoided += 1;
            return false;
        }

        self.cached_viewports[..count].copy_from_slice(&viewports[..count]);
        self.active_viewport_count = count;
        true
    }

    // Updates scissor array, returning true if state changed (hardware flush needed).
    pub fn set_scissor_rects(&mut self, scissors: &[D3d12DynamicScissorRect]) -> bool {
        self.total_scissor_sets += 1;
        let count = scissors.len().min(D3D12_VIEWPORT_AND_SCISSORRECT_MAX_INDEX);

        if self.active_scissor_count == count && self.cached_scissors[..count] == scissors[..count] {
            self.redundant_scissor_flushes_avoided += 1;
            return false;
        }

        self.cached_scissors[..count].copy_from_slice(&scissors[..count]);
        self.active_scissor_count = count;
        true
    }
}


// Sub-pixel precision rasterization grid (256x256 sub-pixel positions per pixel).
pub const D3D12_SUBPIXEL_GRID_RESOLUTION: u32 = 256;

// Sub-pixel point coordinate within a 256x256 unit pixel cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubpixelPoint {
    pub x: u8,
    pub y: u8,
}

// 2D Triangle bounding box and sub-pixel edge equations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriangleRasterDesc {
    pub v0: [f32; 2],
    pub v1: [f32; 2],
    pub v2: [f32; 2],
}

impl TriangleRasterDesc {
    pub const fn new(v0: [f32; 2], v1: [f32; 2], v2: [f32; 2]) -> Self {
        Self { v0, v1, v2 }
    }

    // Evaluates 2D signed distance for edge equation.
    #[inline(always)]
    pub fn edge_func(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
        (c[0] - a[0]) * (b[1] - a[1]) - (c[1] - a[1]) * (b[0] - a[0])
    }
}

// Conservative rasterization Tier 3 governor with sub-pixel coverage mask generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConservativeRasterTier3Governor {
    pub conservative_enabled: bool,
    pub total_pixels_rasterized: u64,
    pub conservative_expanded_pixels: u64,
    pub fully_covered_inner_pixels: u64,
}

impl ConservativeRasterTier3Governor {
    pub fn new(conservative_enabled: bool) -> Self {
        Self {
            conservative_enabled,
            total_pixels_rasterized: 0,
            conservative_expanded_pixels: 0,
            fully_covered_inner_pixels: 0,
        }
    }

    // Evaluates conservative coverage and generates 16-sample sub-pixel bitmask for a pixel cell.
    pub fn evaluate_pixel_coverage(&mut self, tri: &TriangleRasterDesc, pixel_x: f32, pixel_y: f32) -> (bool, u16) {
        self.total_pixels_rasterized += 1;
        let corners = [
            [pixel_x, pixel_y],
            [pixel_x + 1.0, pixel_y],
            [pixel_x, pixel_y + 1.0],
            [pixel_x + 1.0, pixel_y + 1.0],
        ];

        let mut sample_mask: u16 = 0;
        // 4x4 sub-pixel sample points within pixel cell
        for sy in 0..4 {
            for sx in 0..4 {
                let px = pixel_x + (sx as f32 + 0.5) / 4.0;
                let py = pixel_y + (sy as f32 + 0.5) / 4.0;
                let p = [px, py];

                let w0 = TriangleRasterDesc::edge_func(tri.v1, tri.v2, p);
                let w1 = TriangleRasterDesc::edge_func(tri.v2, tri.v0, p);
                let w2 = TriangleRasterDesc::edge_func(tri.v0, tri.v1, p);

                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let bit = sy * 4 + sx;
                    sample_mask |= 1 << bit;
                }
            }
        }

        let standard_hit = sample_mask != 0;
        if sample_mask == 0xFFFF {
            self.fully_covered_inner_pixels += 1;
        }

        if !self.conservative_enabled {
            return (standard_hit, sample_mask);
        }

        // Conservative Tier 3: pixel is covered if triangle touches pixel AABB or any corner is inside
        let mut touches_any = standard_hit;
        if !touches_any {
            for c in &corners {
                let w0 = TriangleRasterDesc::edge_func(tri.v1, tri.v2, *c);
                let w1 = TriangleRasterDesc::edge_func(tri.v2, tri.v0, *c);
                let w2 = TriangleRasterDesc::edge_func(tri.v0, tri.v1, *c);
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    touches_any = true;
                    break;
                }
            }
        }

        if touches_any && !standard_hit {
            self.conservative_expanded_pixels += 1;
        }

        (touches_any, sample_mask)
    }
}

// Variable Rate Shading coarse pixel dimensions.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewportShadingRate {
    #[default]
    Rate1x1 = 0,
    Rate1x2 = 1,
    Rate2x1 = 2,
    Rate2x2 = 3,
    Rate2x4 = 4,
    Rate4x2 = 5,
    Rate4x4 = 6,
}

impl ViewportShadingRate {
    #[inline(always)]
    pub const fn pixel_area(&self) -> u32 {
        match self {
            Self::Rate1x1 => 1,
            Self::Rate1x2 | Self::Rate2x1 => 2,
            Self::Rate2x2 => 4,
            Self::Rate2x4 | Self::Rate4x2 => 8,
            Self::Rate4x4 => 16,
        }
    }
}

// VRS shading rate combiners matching D3D12_SHADING_RATE_COMBINER.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewportRateCombiner {
    #[default]
    Passthrough = 0,
    Override = 1,
    Min = 2,
    Max = 3,
    Sum = 4,
}

// Variable Rate Shading (VRS) Tier 2 tile combiner with viewport-level overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewportVrsCombiner {
    pub viewport_override_rate: ViewportShadingRate,
    pub combiner: ViewportRateCombiner,
    pub total_tiles_evaluated: u64,
    pub total_shading_rate_reductions: u64,
}

impl ViewportVrsCombiner {
    pub fn new(rate: ViewportShadingRate, combiner: ViewportRateCombiner) -> Self {
        Self {
            viewport_override_rate: rate,
            combiner,
            total_tiles_evaluated: 0,
            total_shading_rate_reductions: 0,
        }
    }

    // Combines tile-level shading rate image sample with viewport override rate.
    pub fn combine_shading_rate(&mut self, tile_rate: ViewportShadingRate) -> ViewportShadingRate {
        self.total_tiles_evaluated += 1;
        let ov = self.viewport_override_rate;

        let resolved = match self.combiner {
            ViewportRateCombiner::Passthrough => tile_rate,
            ViewportRateCombiner::Override => ov,
            ViewportRateCombiner::Min => {
                if ov.pixel_area() < tile_rate.pixel_area() { ov } else { tile_rate }
            }
            ViewportRateCombiner::Max => {
                if ov.pixel_area() > tile_rate.pixel_area() { ov } else { tile_rate }
            }
            ViewportRateCombiner::Sum => {
                let combined_area = ov.pixel_area() * tile_rate.pixel_area();
                if combined_area >= 16 {
                    ViewportShadingRate::Rate4x4
                } else if combined_area >= 8 {
                    ViewportShadingRate::Rate4x2
                } else if combined_area >= 4 {
                    ViewportShadingRate::Rate2x2
                } else if combined_area >= 2 {
                    ViewportShadingRate::Rate2x1
                } else {
                    ViewportShadingRate::Rate1x1
                }
            }
        };

        if resolved.pixel_area() > 1 {
            self.total_shading_rate_reductions += 1;
        }

        resolved
    }
}
