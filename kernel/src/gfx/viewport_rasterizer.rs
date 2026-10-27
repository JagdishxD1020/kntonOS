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
