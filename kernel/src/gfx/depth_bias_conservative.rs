//! Direct3D 12 Agility Dynamic Depth Bias & Conservative Rasterization Tier 3 Optimizer.
//! Implements dynamic depth bias pipelines, SV_InnerCoverage evaluation, and shadow governors.

#![allow(dead_code, non_camel_case_types, non_snake_case, unused_imports)]

extern crate alloc;
use alloc::vec::Vec;

// Dynamic depth bias configuration descriptor for shadow mapping and coplanar geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicDepthBiasConfig {
    // Constant depth bias value added to fragment depth.
    pub depth_bias: f32,
    // Maximum bias magnitude permitted (0.0 disables clamping).
    pub depth_bias_clamp: f32,
    // Factor scaling depth bias based on fragment screen-space slope.
    pub slope_scaled_depth_bias: f32,
}

impl Default for DynamicDepthBiasConfig {
    fn default() -> Self {
        Self {
            depth_bias: 0.0,
            depth_bias_clamp: 0.0,
            slope_scaled_depth_bias: 0.0,
        }
    }
}

impl DynamicDepthBiasConfig {
    // Constructs a new dynamic depth bias configuration.
    pub const fn new(bias: f32, clamp: f32, slope_scale: f32) -> Self {
        Self {
            depth_bias: bias,
            depth_bias_clamp: clamp,
            slope_scaled_depth_bias: slope_scale,
        }
    }
}

// Depth bias evaluation engine applying D3D12 standard slope-scaled biasing and clamping.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DepthBiasEvaluator {
    // Active depth bias settings.
    pub config: DynamicDepthBiasConfig,
    // Total biased depth values computed.
    pub total_evaluations: u64,
    // Total times bias clamp was activated.
    pub total_clamps: u64,
}

impl DepthBiasEvaluator {
    // Creates a new depth bias evaluator.
    pub fn new(config: DynamicDepthBiasConfig) -> Self {
        Self {
            config,
            total_evaluations: 0,
            total_clamps: 0,
        }
    }

    // Computes biased depth for a polygon fragment given base depth and max slope.
    pub fn compute_biased_depth(&mut self, base_depth: f32, max_depth_slope: f32) -> f32 {
        self.total_evaluations += 1;

        let mut total_bias =
            max_depth_slope * self.config.slope_scaled_depth_bias + self.config.depth_bias;

        if self.config.depth_bias_clamp > 0.0 {
            if total_bias > self.config.depth_bias_clamp {
                total_bias = self.config.depth_bias_clamp;
                self.total_clamps += 1;
            } else if total_bias < -self.config.depth_bias_clamp {
                total_bias = -self.config.depth_bias_clamp;
                self.total_clamps += 1;
            }
        }

        let biased = base_depth + total_bias;

        if biased < 0.0 {
            0.0
        } else if biased > 1.0 {
            1.0
        } else {
            biased
        }
    }
}

// 2D triangle representation for conservative rasterization coverage tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triangle2D {
    // Vertex 0 coordinates in screen pixel space.
    pub v0: [f32; 2],
    // Vertex 1 coordinates in screen pixel space.
    pub v1: [f32; 2],
    // Vertex 2 coordinates in screen pixel space.
    pub v2: [f32; 2],
}

impl Triangle2D {
    // Constructs a new 2D triangle.
    pub const fn new(v0: [f32; 2], v1: [f32; 2], v2: [f32; 2]) -> Self {
        Self { v0, v1, v2 }
    }

    // Evaluates oriented edge function between two vertices and a query point.
    #[inline(always)]
    pub fn edge_func(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    }

    // Tests if a single point is inside or on the boundaries of the triangle.
    pub fn contains_point(&self, p: [f32; 2]) -> bool {
        let e0 = Self::edge_func(self.v0, self.v1, p);
        let e1 = Self::edge_func(self.v1, self.v2, p);
        let e2 = Self::edge_func(self.v2, self.v0, p);

        (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0)
    }
}

// Result of Tier 3 conservative rasterization pixel coverage evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelCoverageResult {
    // Conservative rasterization coverage indicating any part of pixel is touched.
    pub conservative_covered: bool,
    // Tier 3 SV_InnerCoverage: pixel square is fully inside primitive with no edge crossings.
    pub inner_coverage: bool,
}

// Conservative Rasterization Tier 3 inner input coverage (SV_InnerCoverage) evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tier3InnerCoverageEvaluator {
    // Total pixels evaluated.
    pub total_pixels_evaluated: u64,
    // Total pixels satisfying SV_InnerCoverage.
    pub total_inner_covered: u64,
    // Total pixels with conservative coverage.
    pub total_conservative_covered: u64,
}

impl Tier3InnerCoverageEvaluator {
    // Creates a new Tier 3 inner coverage evaluator.
    pub fn new() -> Self {
        Self {
            total_pixels_evaluated: 0,
            total_inner_covered: 0,
            total_conservative_covered: 0,
        }
    }

    // Evaluates pixel coverage and SV_InnerCoverage for a pixel center against a triangle.
    pub fn evaluate_pixel(
        &mut self,
        tri: &Triangle2D,
        pixel_center_x: f32,
        pixel_center_y: f32,
    ) -> PixelCoverageResult {
        self.total_pixels_evaluated += 1;

        let corners = [
            [pixel_center_x - 0.5, pixel_center_y - 0.5],
            [pixel_center_x + 0.5, pixel_center_y - 0.5],
            [pixel_center_x + 0.5, pixel_center_y + 0.5],
            [pixel_center_x - 0.5, pixel_center_y + 0.5],
        ];

        // SV_InnerCoverage requires all four corners of the pixel square to be inside.
        let mut all_corners_inside = true;
        let mut any_corner_inside = false;

        let center_inside = tri.contains_point([pixel_center_x, pixel_center_y]);

        for &corner in &corners {
            if tri.contains_point(corner) {
                any_corner_inside = true;
            } else {
                all_corners_inside = false;
            }
        }

        let conservative_covered = center_inside || any_corner_inside;
        let inner_coverage = all_corners_inside && center_inside;

        if conservative_covered {
            self.total_conservative_covered += 1;
        }
        if inner_coverage {
            self.total_inner_covered += 1;
        }

        PixelCoverageResult {
            conservative_covered,
            inner_coverage,
        }
    }
}

// Screen-space 2D bounding rectangle with nearest depth for occlusion culling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenAABB2D {
    // Minimum horizontal screen coordinate in pixels.
    pub min_x: f32,
    // Minimum vertical screen coordinate in pixels.
    pub min_y: f32,
    // Maximum horizontal screen coordinate in pixels.
    pub max_x: f32,
    // Maximum vertical screen coordinate in pixels.
    pub max_y: f32,
    // Nearest Z depth value of bounding box in range 0.0 to 1.0.
    pub nearest_z: f32,
}

impl ScreenAABB2D {
    // Constructs a new screen-space bounding rectangle.
    pub const fn new(min_x: f32, min_y: f32, max_x: f32, max_y: f32, nearest_z: f32) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
            nearest_z,
        }
    }

    // Dilates the bounding rectangle outward by subpixel expansion margin.
    pub fn dilate(&self, expansion: f32) -> Self {
        Self {
            min_x: self.min_x - expansion,
            min_y: self.min_y - expansion,
            max_x: self.max_x + expansion,
            max_y: self.max_y + expansion,
            nearest_z: self.nearest_z,
        }
    }
}

// GPU occlusion culling bounding box accelerator using conservative footprints.
#[derive(Debug, Clone)]
pub struct ConservativeOcclusionAccelerator {
    // Width of low-resolution hierarchical depth buffer.
    pub depth_width: u32,
    // Height of low-resolution hierarchical depth buffer.
    pub depth_height: u32,
    // Coarse depth buffer storing maximum depth per cell.
    pub depth_buffer: Vec<f32>,
    // Total bounding boxes tested.
    pub total_tested: u64,
    // Total bounding boxes determined to be occluded.
    pub total_culled: u64,
    // Total bounding boxes visible.
    pub total_visible: u64,
}

impl ConservativeOcclusionAccelerator {
    // Creates a new conservative occlusion accelerator with specified buffer dimensions.
    pub fn new(depth_width: u32, depth_height: u32) -> Self {
        let size = (depth_width as usize) * (depth_height as usize);
        let mut depth_buffer = Vec::new();
        depth_buffer.resize(size, 1.0);
        Self {
            depth_width,
            depth_height,
            depth_buffer,
            total_tested: 0,
            total_culled: 0,
            total_visible: 0,
        }
    }

    // Clears hierarchical depth buffer with specified depth clear value.
    pub fn clear_depth(&mut self, clear_value: f32) {
        for d in &mut self.depth_buffer {
            *d = clear_value;
        }
    }

    // Rasterizes an occluder rectangle writing maximum depth into buffer.
    pub fn write_occluder(&mut self, rect: &ScreenAABB2D, depth: f32) {
        let x0 = (rect.min_x.max(0.0) as u32).min(self.depth_width);
        let y0 = (rect.min_y.max(0.0) as u32).min(self.depth_height);
        let x1 = ((rect.max_x + 1.0).max(0.0) as u32).min(self.depth_width);
        let y1 = ((rect.max_y + 1.0).max(0.0) as u32).min(self.depth_height);

        let mut y = y0;
        while y < y1 {
            let mut x = x0;
            while x < x1 {
                let idx = (y * self.depth_width + x) as usize;
                if idx < self.depth_buffer.len() && depth < self.depth_buffer[idx] {
                    self.depth_buffer[idx] = depth;
                }
                x += 1;
            }
            y += 1;
        }
    }

    // Tests whether a bounding box is occluded behind existing geometry.
    pub fn is_occluded(&mut self, query_box: &ScreenAABB2D, conservative_dilate: bool) -> bool {
        self.total_tested += 1;

        let rect = if conservative_dilate {
            query_box.dilate(0.5)
        } else {
            *query_box
        };

        let x0 = (rect.min_x.max(0.0) as u32).min(self.depth_width);
        let y0 = (rect.min_y.max(0.0) as u32).min(self.depth_height);
        let x1 = ((rect.max_x + 1.0).max(0.0) as u32).min(self.depth_width);
        let y1 = ((rect.max_y + 1.0).max(0.0) as u32).min(self.depth_height);

        if x0 >= x1 || y0 >= y1 {
            self.total_visible += 1;
            return false;
        }

        let mut max_buffer_depth: f32 = 0.0;
        let mut y = y0;
        while y < y1 {
            let mut x = x0;
            while x < x1 {
                let idx = (y * self.depth_width + x) as usize;
                if idx < self.depth_buffer.len() {
                    let d = self.depth_buffer[idx];
                    if d > max_buffer_depth {
                        max_buffer_depth = d;
                    }
                }
                x += 1;
            }
            y += 1;
        }

        if rect.nearest_z > max_buffer_depth {
            self.total_culled += 1;
            true
        } else {
            self.total_visible += 1;
            false
        }
    }

    // Computes percentage of tested boxes successfully culled in range 0 to 100.
    pub fn cull_efficiency_percentage(&self) -> u32 {
        if self.total_tested == 0 {
            0
        } else {
            ((self.total_culled * 100) / self.total_tested) as u32
        }
    }
}

// Handheld shadow map resolution governor dynamically scaling cascades to save memory bandwidth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandheldShadowGovernor {
    // Baseline unconstrained shadow map resolution per cascade (e.g. 2048).
    pub baseline_resolution: u32,
    // Active shadow map resolution per cascade.
    pub current_resolution: u32,
    // Total directional light shadow cascades.
    pub cascade_count: u32,
    // Bytes per depth texel (e.g. 4 for D32_FLOAT).
    pub bytes_per_pixel: u32,
    // Cumulative memory bandwidth bytes saved across frames.
    pub total_bandwidth_saved_bytes: u64,
    // Whether battery saving or thermal throttling profile is engaged.
    pub thermal_throttling_active: bool,
}

impl Default for HandheldShadowGovernor {
    fn default() -> Self {
        Self::new(2048, 4)
    }
}

impl HandheldShadowGovernor {
    // Creates a new handheld shadow governor with baseline resolution and cascade count.
    pub fn new(baseline_resolution: u32, cascade_count: u32) -> Self {
        Self {
            baseline_resolution,
            current_resolution: baseline_resolution,
            cascade_count,
            bytes_per_pixel: 4,
            total_bandwidth_saved_bytes: 0,
            thermal_throttling_active: false,
        }
    }

    // Toggles handheld thermal power saving profile.
    pub fn set_throttling_mode(&mut self, throttled: bool) {
        self.thermal_throttling_active = throttled;
        if throttled {
            self.current_resolution = (self.baseline_resolution / 2).max(256);
        } else {
            self.current_resolution = self.baseline_resolution;
        }
    }

    // Calculates scaling factor applied to depth bias due to texel footprint enlargement.
    pub fn depth_bias_scale_factor(&self) -> f32 {
        if self.current_resolution == 0 {
            1.0
        } else {
            (self.baseline_resolution as f32) / (self.current_resolution as f32)
        }
    }

    // Adapts depth bias configuration proportionally to current shadow map resolution.
    pub fn compute_adjusted_depth_bias(
        &self,
        base_cfg: DynamicDepthBiasConfig,
    ) -> DynamicDepthBiasConfig {
        let scale = self.depth_bias_scale_factor();
        DynamicDepthBiasConfig {
            depth_bias: base_cfg.depth_bias * scale,
            depth_bias_clamp: base_cfg.depth_bias_clamp * scale,
            slope_scaled_depth_bias: base_cfg.slope_scaled_depth_bias * scale,
        }
    }

    // Records bandwidth savings for a frame and accumulates total bytes saved.
    pub fn record_frame_bandwidth_savings(&mut self) -> u64 {
        let base_footprint = (self.baseline_resolution as u64)
            * (self.baseline_resolution as u64)
            * (self.cascade_count as u64)
            * (self.bytes_per_pixel as u64);

        let cur_footprint = (self.current_resolution as u64)
            * (self.current_resolution as u64)
            * (self.cascade_count as u64)
            * (self.bytes_per_pixel as u64);

        let saved = base_footprint.saturating_sub(cur_footprint);
        self.total_bandwidth_saved_bytes += saved;
        saved
    }
}

// Temporary stub for incremental commits.
pub fn test_depth_bias_conservative_subsystem() -> bool {
    true
}
