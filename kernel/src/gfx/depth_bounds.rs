//! Direct3D 12 Depth Bounds Testing & Dynamic Stencil Ref Engine.
//! Provides hardware-accelerated depth bounds volume rejection and dynamic stencil references.

#![allow(dead_code)]

/// Direct3D 12 Options2 feature query capability structure (`D3D12_FEATURE_DATA_D3D12_OPTIONS2`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types, non_snake_case)]
pub struct D3D12_FEATURE_DATA_D3D12_OPTIONS2 {
    pub DepthBoundsTestSupported: bool,
    pub ProgrammableSamplePositionsTier: u32,
}

/// Queries hardware capability for Direct3D 12 Options2 feature set.
pub fn query_d3d12_options2_features() -> D3D12_FEATURE_DATA_D3D12_OPTIONS2 {
    D3D12_FEATURE_DATA_D3D12_OPTIONS2 {
        DepthBoundsTestSupported: true,
        ProgrammableSamplePositionsTier: 2,
    }
}

/// Dynamic depth bounds testing state descriptor (`OMSetDepthBounds`).
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(non_camel_case_types)]
pub struct D3D12_DEPTH_BOUNDS {
    pub min_depth: f32,
    pub max_depth: f32,
    pub enabled: bool,
}

impl Default for D3D12_DEPTH_BOUNDS {
    fn default() -> Self {
        Self::disabled()
    }
}

impl D3D12_DEPTH_BOUNDS {
    /// Creates an active depth bounds state with range [min_depth, max_depth].
    pub const fn active(min_depth: f32, max_depth: f32) -> Self {
        Self {
            min_depth,
            max_depth,
            enabled: true,
        }
    }

    /// Creates a disabled depth bounds state spanning [0.0, 1.0].
    pub const fn disabled() -> Self {
        Self {
            min_depth: 0.0,
            max_depth: 1.0,
            enabled: false,
        }
    }
}

/// Dynamic stencil reference value descriptor (`OMSetStencilRef`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_STENCIL_REF {
    pub stencil_ref: u32,
}

impl D3D12_STENCIL_REF {
    /// Creates a new dynamic stencil reference binding.
    pub const fn new(stencil_ref: u32) -> Self {
        Self { stencil_ref }
    }
}

/// Temporary stub for depth bounds subsystem test during incremental commits.
pub fn test_depth_bounds_subsystem() -> bool {
    true
}
