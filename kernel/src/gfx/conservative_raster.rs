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
