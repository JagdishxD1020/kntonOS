//! Direct3D 12 Conservative Rasterization & Pixel Coverage Engine.
//! Provides hardware tiers, sub-pixel snapping, and inner/outer coverage evaluation.

#![allow(dead_code)]

extern crate alloc;
#[allow(unused_imports)]
use alloc::vec::Vec;

// Direct3D 12 Conservative Rasterization Hardware Tiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum D3D12_CONSERVATIVE_RASTERIZATION_TIER {
    #[default]
    D3D12_CONSERVATIVE_RASTERIZATION_TIER_NOT_SUPPORTED = 0,
    D3D12_CONSERVATIVE_RASTERIZATION_TIER_1 = 1,
    D3D12_CONSERVATIVE_RASTERIZATION_TIER_2 = 2,
    D3D12_CONSERVATIVE_RASTERIZATION_TIER_3 = 3,
}

// Conservative rasterization pipeline mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum D3D12_CONSERVATIVE_RASTERIZATION_MODE {
    #[default]
    D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF = 0,
    D3D12_CONSERVATIVE_RASTERIZATION_MODE_ON = 1,
}

// Sub-pixel snap precision bits according to conservative rasterization tier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum D3D12_SUBPIXEL_SNAP_PRECISION {
    Bits4 = 4,   // 1/16th sub-pixel snap (standard rasterizer / Tier 1)
    Bits8 = 8,   // 1/256th sub-pixel snap (Tier 2 and Tier 3)
}

// Conservative rasterizer state descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_CONSERVATIVE_RASTERIZER_DESC {
    pub mode: D3D12_CONSERVATIVE_RASTERIZATION_MODE,
    pub tier: D3D12_CONSERVATIVE_RASTERIZATION_TIER,
    pub subpixel_snap_bits: D3D12_SUBPIXEL_SNAP_PRECISION,
    pub preserve_degenerate_primitives: bool,
    pub forced_sample_count: u32,
}

impl Default for D3D12_CONSERVATIVE_RASTERIZER_DESC {
    fn default() -> Self {
        Self {
            mode: D3D12_CONSERVATIVE_RASTERIZATION_MODE::D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
            tier: D3D12_CONSERVATIVE_RASTERIZATION_TIER::D3D12_CONSERVATIVE_RASTERIZATION_TIER_NOT_SUPPORTED,
            subpixel_snap_bits: D3D12_SUBPIXEL_SNAP_PRECISION::Bits4,
            preserve_degenerate_primitives: false,
            forced_sample_count: 0,
        }
    }
}

impl D3D12_CONSERVATIVE_RASTERIZER_DESC {
    // Construct active conservative rasterizer descriptor for a specific tier
    pub fn new(tier: D3D12_CONSERVATIVE_RASTERIZATION_TIER) -> Self {
        let snap_bits = match tier {
            D3D12_CONSERVATIVE_RASTERIZATION_TIER::D3D12_CONSERVATIVE_RASTERIZATION_TIER_2
            | D3D12_CONSERVATIVE_RASTERIZATION_TIER::D3D12_CONSERVATIVE_RASTERIZATION_TIER_3 => {
                D3D12_SUBPIXEL_SNAP_PRECISION::Bits8
            }
            _ => D3D12_SUBPIXEL_SNAP_PRECISION::Bits4,
        };
        let preserve_degenerates = tier >= D3D12_CONSERVATIVE_RASTERIZATION_TIER::D3D12_CONSERVATIVE_RASTERIZATION_TIER_2;

        Self {
            mode: D3D12_CONSERVATIVE_RASTERIZATION_MODE::D3D12_CONSERVATIVE_RASTERIZATION_MODE_ON,
            tier,
            subpixel_snap_bits: snap_bits,
            preserve_degenerate_primitives: preserve_degenerates,
            forced_sample_count: 0,
        }
    }

    // Check if inner input coverage is supported for this tier
    #[inline]
    pub fn supports_inner_coverage(&self) -> bool {
        self.mode == D3D12_CONSERVATIVE_RASTERIZATION_MODE::D3D12_CONSERVATIVE_RASTERIZATION_MODE_ON
            && self.tier >= D3D12_CONSERVATIVE_RASTERIZATION_TIER::D3D12_CONSERVATIVE_RASTERIZATION_TIER_2
    }

    // Uncertainty region half-width in sub-pixel fraction (e.g. 0.5 for Tier 1, 0.0039 for Tier 2)
    #[inline]
    pub fn uncertainty_region(&self) -> f32 {
        match self.tier {
            D3D12_CONSERVATIVE_RASTERIZATION_TIER::D3D12_CONSERVATIVE_RASTERIZATION_TIER_NOT_SUPPORTED => 0.0,
            D3D12_CONSERVATIVE_RASTERIZATION_TIER::D3D12_CONSERVATIVE_RASTERIZATION_TIER_1 => 0.5,
            D3D12_CONSERVATIVE_RASTERIZATION_TIER::D3D12_CONSERVATIVE_RASTERIZATION_TIER_2
            | D3D12_CONSERVATIVE_RASTERIZATION_TIER::D3D12_CONSERVATIVE_RASTERIZATION_TIER_3 => 1.0 / 256.0,
        }
    }
}

// Sub-pixel snapping engine quantizing floating-point vertices onto fixed-point grids
#[allow(non_camel_case_types)]
pub struct D3D12_SUBPIXEL_SNAPPER;

impl D3D12_SUBPIXEL_SNAPPER {
    // Snap a 2D vertex coordinate to the specified sub-pixel precision grid
    #[inline]
    pub fn snap_vertex(coord: [f32; 2], precision: D3D12_SUBPIXEL_SNAP_PRECISION) -> [f32; 2] {
        let grid_scale = match precision {
            D3D12_SUBPIXEL_SNAP_PRECISION::Bits4 => 16.0f32,
            D3D12_SUBPIXEL_SNAP_PRECISION::Bits8 => 256.0f32,
        };

        let snap_axis = |val: f32| -> f32 {
            let scaled = val * grid_scale;
            let rounded = if scaled >= 0.0 {
                (scaled + 0.5) as i32 as f32
            } else {
                (scaled - 0.5) as i32 as f32
            };
            rounded / grid_scale
        };

        [snap_axis(coord[0]), snap_axis(coord[1])]
    }
}

// Geometric degenerate state classification of post-snap primitives
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum D3D12_DEGENERATE_STATUS {
    NonDegenerate,
    PointDegenerate,
    CollinearDegenerate,
}

// Conservative rasterization triangle representation with sub-pixel snap and AABB dilation
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(non_camel_case_types)]
pub struct D3D12_CONSERVATIVE_TRIANGLE {
    pub v0: [f32; 2],
    pub v1: [f32; 2],
    pub v2: [f32; 2],
    pub signed_area: f32,
    pub degenerate_status: D3D12_DEGENERATE_STATUS,
    pub should_cull: bool,
    pub dilated_aabb: [f32; 4], // [min_x, min_y, max_x, max_y]
}

impl D3D12_CONSERVATIVE_TRIANGLE {
    // Construct and snap triangle vertices according to conservative rasterizer state
    pub fn new(
        raw_v0: [f32; 2],
        raw_v1: [f32; 2],
        raw_v2: [f32; 2],
        desc: &D3D12_CONSERVATIVE_RASTERIZER_DESC,
    ) -> Self {
        let v0 = D3D12_SUBPIXEL_SNAPPER::snap_vertex(raw_v0, desc.subpixel_snap_bits);
        let v1 = D3D12_SUBPIXEL_SNAPPER::snap_vertex(raw_v1, desc.subpixel_snap_bits);
        let v2 = D3D12_SUBPIXEL_SNAPPER::snap_vertex(raw_v2, desc.subpixel_snap_bits);

        // 2D cross product for signed area: 0.5 * ((v1.x - v0.x)*(v2.y - v0.y) - (v2.x - v0.x)*(v1.y - v0.y))
        let signed_area = 0.5 * ((v1[0] - v0[0]) * (v2[1] - v0[1]) - (v2[0] - v0[0]) * (v1[1] - v0[1]));

        let degenerate_status = if (v0[0] == v1[0] && v0[1] == v1[1]) && (v1[0] == v2[0] && v1[1] == v2[1]) {
            D3D12_DEGENERATE_STATUS::PointDegenerate
        } else if signed_area.abs() <= 1e-6 {
            D3D12_DEGENERATE_STATUS::CollinearDegenerate
        } else {
            D3D12_DEGENERATE_STATUS::NonDegenerate
        };

        // Determine if degenerate primitive is preserved (Tier 2/3) or culled (Tier 1 default)
        let is_degenerate = degenerate_status != D3D12_DEGENERATE_STATUS::NonDegenerate;
        let should_cull = is_degenerate && !desc.preserve_degenerate_primitives;

        // Compute conservative bounding box dilated by uncertainty region
        let uncertainty = desc.uncertainty_region();
        let min_x = v0[0].min(v1[0]).min(v2[0]) - uncertainty;
        let min_y = v0[1].min(v1[1]).min(v2[1]) - uncertainty;
        let max_x = v0[0].max(v1[0]).max(v2[0]) + uncertainty;
        let max_y = v0[1].max(v1[1]).max(v2[1]) + uncertainty;

        Self {
            v0,
            v1,
            v2,
            signed_area,
            degenerate_status,
            should_cull,
            dilated_aabb: [min_x, min_y, max_x, max_y],
        }
    }
}

// Evaluator for conservative pixel intersection and SV_Coverage sample mask computation
#[allow(non_camel_case_types)]
pub struct D3D12_CONSERVATIVE_COVERAGE_EVALUATOR;

impl D3D12_CONSERVATIVE_COVERAGE_EVALUATOR {
    // Test if a 2D integer pixel box [px, px+1] x [py, py+1] conservatives intersects a triangle
    pub fn pixel_intersects_triangle(
        triangle: &D3D12_CONSERVATIVE_TRIANGLE,
        px: i32,
        py: i32,
        uncertainty: f32,
    ) -> bool {
        if triangle.should_cull {
            return false;
        }

        let p_min_x = px as f32 - uncertainty;
        let p_min_y = py as f32 - uncertainty;
        let p_max_x = px as f32 + 1.0 + uncertainty;
        let p_max_y = py as f32 + 1.0 + uncertainty;

        // 1. AABB overlap test
        if triangle.dilated_aabb[2] < p_min_x
            || triangle.dilated_aabb[0] > p_max_x
            || triangle.dilated_aabb[3] < p_min_y
            || triangle.dilated_aabb[1] > p_max_y
        {
            return false;
        }

        // Handle degenerate point or collinear line
        if triangle.degenerate_status == D3D12_DEGENERATE_STATUS::PointDegenerate {
            return triangle.v0[0] >= p_min_x
                && triangle.v0[0] <= p_max_x
                && triangle.v0[1] >= p_min_y
                && triangle.v0[1] <= p_max_y;
        }

        // 2. Separating Axis Theorem (SAT) using triangle edge normals
        let box_center = [px as f32 + 0.5, py as f32 + 0.5];
        let box_half = [0.5 + uncertainty, 0.5 + uncertainty];

        let verts = [triangle.v0, triangle.v1, triangle.v2];
        for i in 0..3 {
            let v_curr = verts[i];
            let v_next = verts[(i + 1) % 3];

            // Edge vector and outward normal
            let edge = [v_next[0] - v_curr[0], v_next[1] - v_curr[1]];
            let normal = if triangle.signed_area >= 0.0 {
                [-edge[1], edge[0]]
            } else {
                [edge[1], -edge[0]]
            };

            // Distance from box center to edge
            let dist = (box_center[0] - v_curr[0]) * normal[0] + (box_center[1] - v_curr[1]) * normal[1];
            // Maximum projected extent of box along normal
            let radius = box_half[0] * normal[0].abs() + box_half[1] * normal[1].abs();

            if dist > radius {
                return false; // Box is entirely on the outside of this edge
            }
        }

        true
    }

    // Compute SV_Coverage bitmask for a pixel (supporting 1x and 4x MSAA patterns)
    pub fn compute_sv_coverage(
        triangle: &D3D12_CONSERVATIVE_TRIANGLE,
        px: i32,
        py: i32,
        sample_count: u32,
        uncertainty: f32,
    ) -> u32 {
        let touched = Self::pixel_intersects_triangle(triangle, px, py, uncertainty);
        if !touched {
            return 0;
        }

        if sample_count <= 1 {
            return 1;
        }

        // 4x MSAA standard sample pattern offsets relative to pixel top-left
        let msaa_4x_offsets: [[f32; 2]; 4] = [
            [0.375, 0.125],
            [0.875, 0.375],
            [0.125, 0.625],
            [0.625, 0.875],
        ];

        let mut mask = 0u32;
        let verts = [triangle.v0, triangle.v1, triangle.v2];

        for (i, offset) in msaa_4x_offsets.iter().enumerate() {
            let sx = px as f32 + offset[0];
            let sy = py as f32 + offset[1];

            let mut inside = true;
            for j in 0..3 {
                let v_curr = verts[j];
                let v_next = verts[(j + 1) % 3];
                let edge = [v_next[0] - v_curr[0], v_next[1] - v_curr[1]];
                let cross = (sx - v_curr[0]) * edge[1] - (sy - v_curr[1]) * edge[0];
                if (triangle.signed_area >= 0.0 && cross < -1e-5)
                    || (triangle.signed_area < 0.0 && cross > 1e-5)
                {
                    inside = false;
                    break;
                }
            }
            if inside {
                mask |= 1 << i;
            }
        }

        // In conservative rasterization, a touched pixel must have at least one sample covered
        if mask == 0 {
            mask = 1;
        }
        mask
    }
}

// Complete conservative rasterization output descriptor for a single pixel
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_CONSERVATIVE_PIXEL_RESULT {
    pub pixel_x: i32,
    pub pixel_y: i32,
    pub is_conservatively_covered: bool,
    pub sv_coverage: u32,
    pub sv_inner_coverage: bool,
}

// Inner input coverage evaluator for Tier 2/3 high-precision shadow mapping and visibility buffers
#[allow(non_camel_case_types)]
pub struct D3D12_INNER_COVERAGE_EVALUATOR;

impl D3D12_INNER_COVERAGE_EVALUATOR {
    // Determine whether a pixel is completely and unconditionally interior to the triangle
    pub fn evaluates_inner_coverage(
        triangle: &D3D12_CONSERVATIVE_TRIANGLE,
        px: i32,
        py: i32,
        desc: &D3D12_CONSERVATIVE_RASTERIZER_DESC,
    ) -> bool {
        // Inner coverage requires hardware Tier 2 or Tier 3
        if !desc.supports_inner_coverage() || triangle.should_cull {
            return false;
        }

        if triangle.degenerate_status != D3D12_DEGENERATE_STATUS::NonDegenerate {
            return false;
        }

        let u = desc.uncertainty_region();
        // The 4 corners of the pixel expanded outwards by the uncertainty region
        let corners = [
            [px as f32 - u, py as f32 - u],
            [px as f32 + 1.0 + u, py as f32 - u],
            [px as f32 + 1.0 + u, py as f32 + 1.0 + u],
            [px as f32 - u, py as f32 + 1.0 + u],
        ];

        let verts = [triangle.v0, triangle.v1, triangle.v2];

        // All 4 corners must be strictly on the interior side of every triangle edge
        for i in 0..3 {
            let v_curr = verts[i];
            let v_next = verts[(i + 1) % 3];
            let edge = [v_next[0] - v_curr[0], v_next[1] - v_curr[1]];

            for &corner in &corners {
                let cross = (corner[0] - v_curr[0]) * edge[1] - (corner[1] - v_curr[1]) * edge[0];
                if triangle.signed_area >= 0.0 {
                    if cross <= 1e-5 {
                        return false;
                    }
                } else {
                    if cross >= -1e-5 {
                        return false;
                    }
                }
            }
        }

        true
    }
}

// Conservative rasterizer execution pipeline rasterizing triangles into pixel coverage buffers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_CONSERVATIVE_RASTER_PIPELINE {
    pub desc: D3D12_CONSERVATIVE_RASTERIZER_DESC,
}

impl D3D12_CONSERVATIVE_RASTER_PIPELINE {
    // Construct new conservative rasterizer pipeline with descriptor
    pub fn new(desc: D3D12_CONSERVATIVE_RASTERIZER_DESC) -> Self {
        Self { desc }
    }

    // Rasterize a conservative triangle and return all covered pixels with SV_Coverage and SV_InnerCoverage
    pub fn rasterize_triangle(
        &self,
        triangle: &D3D12_CONSERVATIVE_TRIANGLE,
        sample_count: u32,
    ) -> Vec<D3D12_CONSERVATIVE_PIXEL_RESULT> {
        let mut results = Vec::new();
        if triangle.should_cull {
            return results;
        }

        let uncertainty = self.desc.uncertainty_region();
        let floor_i = |v: f32| -> i32 {
            let i = v as i32;
            if v < (i as f32) { i - 1 } else { i }
        };
        let ceil_i = |v: f32| -> i32 {
            let i = v as i32;
            if v > (i as f32) { i + 1 } else { i }
        };
        let min_px = floor_i(triangle.dilated_aabb[0]);
        let min_py = floor_i(triangle.dilated_aabb[1]);
        let max_px = ceil_i(triangle.dilated_aabb[2]);
        let max_py = ceil_i(triangle.dilated_aabb[3]);

        for py in min_py..=max_py {
            for px in min_px..=max_px {
                let covered = D3D12_CONSERVATIVE_COVERAGE_EVALUATOR::pixel_intersects_triangle(
                    triangle,
                    px,
                    py,
                    uncertainty,
                );

                if covered {
                    let sv_coverage = D3D12_CONSERVATIVE_COVERAGE_EVALUATOR::compute_sv_coverage(
                        triangle,
                        px,
                        py,
                        sample_count,
                        uncertainty,
                    );
                    let inner = D3D12_INNER_COVERAGE_EVALUATOR::evaluates_inner_coverage(
                        triangle,
                        px,
                        py,
                        &self.desc,
                    );

                    results.push(D3D12_CONSERVATIVE_PIXEL_RESULT {
                        pixel_x: px,
                        pixel_y: py,
                        is_conservatively_covered: true,
                        sv_coverage,
                        sv_inner_coverage: inner,
                    });
                }
            }
        }

        results
    }
}
