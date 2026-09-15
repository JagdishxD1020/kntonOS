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
