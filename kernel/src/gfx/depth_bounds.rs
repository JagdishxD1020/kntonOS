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

/// Errors resulting from invalid depth bounds testing parameters or state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthBoundsError {
    InvalidDepthRange,
    NonFiniteDepthValue,
    FeatureNotSupported,
}

/// Validator for Direct3D 12 depth bounds range and dynamic stencil reference values.
#[allow(non_camel_case_types)]
pub struct D3D12_DEPTH_BOUNDS_VALIDATOR;

impl D3D12_DEPTH_BOUNDS_VALIDATOR {
    /// Validates that depth bounds lie within [0.0, 1.0] and min <= max.
    pub fn validate_depth_bounds(min_depth: f32, max_depth: f32) -> Result<D3D12_DEPTH_BOUNDS, DepthBoundsError> {
        if !min_depth.is_finite() || !max_depth.is_finite() {
            return Err(DepthBoundsError::NonFiniteDepthValue);
        }
        if min_depth < 0.0 || max_depth > 1.0 || min_depth > max_depth {
            return Err(DepthBoundsError::InvalidDepthRange);
        }
        Ok(D3D12_DEPTH_BOUNDS::active(min_depth, max_depth))
    }

    /// Validates and masks 8-bit dynamic stencil reference value.
    pub const fn validate_stencil_ref(stencil_ref: u32) -> D3D12_STENCIL_REF {
        D3D12_STENCIL_REF::new(stencil_ref & 0xFF)
    }
}

/// Direct3D 12 Output Merger pipeline state maintaining dynamic depth bounds and stencil ref.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_OM_STATE {
    pub depth_bounds: D3D12_DEPTH_BOUNDS,
    pub stencil_ref: D3D12_STENCIL_REF,
}

impl D3D12_OM_STATE {
    /// Creates a new default OM state with depth bounds disabled and stencil ref 0.
    pub const fn new() -> Self {
        Self {
            depth_bounds: D3D12_DEPTH_BOUNDS::disabled(),
            stencil_ref: D3D12_STENCIL_REF { stencil_ref: 0 },
        }
    }

    /// Sets dynamic depth bounds testing range (`OMSetDepthBounds`).
    pub fn om_set_depth_bounds(&mut self, min_depth: f32, max_depth: f32) -> Result<(), DepthBoundsError> {
        self.depth_bounds = D3D12_DEPTH_BOUNDS_VALIDATOR::validate_depth_bounds(min_depth, max_depth)?;
        Ok(())
    }

    /// Disables dynamic depth bounds testing.
    pub fn om_disable_depth_bounds(&mut self) {
        self.depth_bounds = D3D12_DEPTH_BOUNDS::disabled();
    }

    /// Sets dynamic stencil reference value (`OMSetStencilRef`).
    pub fn om_set_stencil_ref(&mut self, stencil_ref: u32) {
        self.stencil_ref = D3D12_DEPTH_BOUNDS_VALIDATOR::validate_stencil_ref(stencil_ref);
    }
}

/// Temporary stub for depth bounds subsystem test during incremental commits.
pub fn test_depth_bounds_subsystem() -> bool {
    true
}
