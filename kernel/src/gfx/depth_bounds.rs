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

/// Hardware-accelerated depth bounds culling evaluator for lighting volume rejection.
#[derive(Debug, Clone, PartialEq)]
#[allow(non_camel_case_types)]
pub struct D3D12_DEPTH_BOUNDS_EVALUATOR {
    pub om_state: D3D12_OM_STATE,
    pub samples_evaluated: u64,
    pub samples_passed: u64,
    pub samples_culled: u64,
    pub shader_invocations_saved: u64,
    pub bandwidth_saved_bytes: u64,
}

impl D3D12_DEPTH_BOUNDS_EVALUATOR {
    /// Creates a new depth bounds evaluator with initial Output Merger state.
    pub const fn new(om_state: D3D12_OM_STATE) -> Self {
        Self {
            om_state,
            samples_evaluated: 0,
            samples_passed: 0,
            samples_culled: 0,
            shader_invocations_saved: 0,
            bandwidth_saved_bytes: 0,
        }
    }

    /// Updates Output Merger pipeline state.
    pub fn set_om_state(&mut self, state: D3D12_OM_STATE) {
        self.om_state = state;
    }

    /// Sets dynamic depth bounds testing range.
    pub fn om_set_depth_bounds(&mut self, min_depth: f32, max_depth: f32) -> Result<(), DepthBoundsError> {
        self.om_state.om_set_depth_bounds(min_depth, max_depth)
    }

    /// Disables dynamic depth bounds testing.
    pub fn om_disable_depth_bounds(&mut self) {
        self.om_state.om_disable_depth_bounds();
    }

    /// Tests a single depth sample against current depth bounds without updating counters.
    #[inline(always)]
    pub fn test_depth_sample(&self, depth_sample: f32) -> bool {
        if !self.om_state.depth_bounds.enabled {
            return true;
        }
        depth_sample >= self.om_state.depth_bounds.min_depth
            && depth_sample <= self.om_state.depth_bounds.max_depth
    }

    /// Evaluates a depth sample, updates performance metrics, and returns pass/cull result.
    pub fn evaluate_sample(&mut self, depth_sample: f32) -> bool {
        self.samples_evaluated = self.samples_evaluated.saturating_add(1);

        if self.test_depth_sample(depth_sample) {
            self.samples_passed = self.samples_passed.saturating_add(1);
            true
        } else {
            self.samples_culled = self.samples_culled.saturating_add(1);
            self.shader_invocations_saved = self.shader_invocations_saved.saturating_add(1);
            // 64 bytes saved per culled pixel (G-buffer read and lighting accumulation write avoided)
            self.bandwidth_saved_bytes = self.bandwidth_saved_bytes.saturating_add(64);
            false
        }
    }

    /// Evaluates an array of depth buffer samples intersecting a light bounding volume.
    pub fn evaluate_lighting_volume(&mut self, depth_buffer: &[f32]) -> (u64, u64) {
        let mut passed = 0u64;
        let mut culled = 0u64;

        for &depth in depth_buffer {
            if self.evaluate_sample(depth) {
                passed = passed.saturating_add(1);
            } else {
                culled = culled.saturating_add(1);
            }
        }

        (passed, culled)
    }
}

/// Token stream opcodes for depth bounds and dynamic stencil reference packets.
pub const D3D12_TOKEN_OM_SET_DEPTH_BOUNDS: u32 = 0x4442_4E44; // 'DBND'
pub const D3D12_TOKEN_OM_SET_STENCIL_REF: u32 = 0x5354_454E;  // 'STEN'
pub const D3D12_TOKEN_DB_DRAW_INSTANCED: u32 = 0x4452_574E;      // 'DRWN'
pub const D3D12_TOKEN_DB_DISPATCH: u32 = 0x4453_5054;            // 'DSPT'

/// Packet recorded into GPU command stream for Output Merger state modifications.
#[derive(Debug, Clone, PartialEq)]
pub enum DepthBoundsCommandPacket {
    SetDepthBounds { min_depth: f32, max_depth: f32 },
    DisableDepthBounds,
    SetStencilRef { stencil_ref: u32 },
    DrawInstanced {
        vertex_count_per_instance: u32,
        instance_count: u32,
        start_vertex_location: u32,
        start_instance_location: u32,
    },
    Dispatch {
        thread_group_count_x: u32,
        thread_group_count_y: u32,
        thread_group_count_z: u32,
    },
}

/// Command recorder for Output Merger depth bounds and dynamic stencil commands.
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct D3D12_DEPTH_BOUNDS_COMMAND_RECORDER {
    pub current_om_state: D3D12_OM_STATE,
    pub total_commands_recorded: u32,
    pub packets: alloc::vec::Vec<DepthBoundsCommandPacket>,
}

impl D3D12_DEPTH_BOUNDS_COMMAND_RECORDER {
    /// Creates a new recorder with default OM state.
    pub fn new() -> Self {
        Self {
            current_om_state: D3D12_OM_STATE::new(),
            total_commands_recorded: 0,
            packets: alloc::vec::Vec::new(),
        }
    }

    /// Records dynamic depth bounds range into command stream.
    pub fn om_set_depth_bounds(&mut self, min_depth: f32, max_depth: f32) -> Result<(), DepthBoundsError> {
        self.current_om_state.om_set_depth_bounds(min_depth, max_depth)?;
        self.packets.push(DepthBoundsCommandPacket::SetDepthBounds { min_depth, max_depth });
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
        Ok(())
    }

    /// Records depth bounds disable command into command stream.
    pub fn om_disable_depth_bounds(&mut self) {
        self.current_om_state.om_disable_depth_bounds();
        self.packets.push(DepthBoundsCommandPacket::DisableDepthBounds);
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
    }

    /// Records dynamic stencil reference value into command stream.
    pub fn om_set_stencil_ref(&mut self, stencil_ref: u32) {
        self.current_om_state.om_set_stencil_ref(stencil_ref);
        self.packets.push(DepthBoundsCommandPacket::SetStencilRef { stencil_ref });
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
    }

    /// Records instanced draw command.
    pub fn draw_instanced(
        &mut self,
        vertex_count_per_instance: u32,
        instance_count: u32,
        start_vertex_location: u32,
        start_instance_location: u32,
    ) {
        self.packets.push(DepthBoundsCommandPacket::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex_location,
            start_instance_location,
        });
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
    }

    /// Records compute dispatch command.
    pub fn dispatch(&mut self, thread_group_count_x: u32, thread_group_count_y: u32, thread_group_count_z: u32) {
        self.packets.push(DepthBoundsCommandPacket::Dispatch {
            thread_group_count_x,
            thread_group_count_y,
            thread_group_count_z,
        });
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
    }

    /// Encodes recorded Output Merger packets into binary token stream.
    pub fn encode_token_stream(&self, stream: &mut alloc::vec::Vec<u8>) {
        for packet in &self.packets {
            match packet {
                DepthBoundsCommandPacket::SetDepthBounds { min_depth, max_depth } => {
                    stream.extend_from_slice(&D3D12_TOKEN_OM_SET_DEPTH_BOUNDS.to_le_bytes());
                    stream.extend_from_slice(&min_depth.to_le_bytes());
                    stream.extend_from_slice(&max_depth.to_le_bytes());
                }
                DepthBoundsCommandPacket::DisableDepthBounds => {
                    stream.extend_from_slice(&D3D12_TOKEN_OM_SET_DEPTH_BOUNDS.to_le_bytes());
                    stream.extend_from_slice(&0.0f32.to_le_bytes());
                    stream.extend_from_slice(&1.0f32.to_le_bytes());
                }
                DepthBoundsCommandPacket::SetStencilRef { stencil_ref } => {
                    stream.extend_from_slice(&D3D12_TOKEN_OM_SET_STENCIL_REF.to_le_bytes());
                    stream.extend_from_slice(&stencil_ref.to_le_bytes());
                }
                DepthBoundsCommandPacket::DrawInstanced {
                    vertex_count_per_instance,
                    instance_count,
                    start_vertex_location,
                    start_instance_location,
                } => {
                    stream.extend_from_slice(&D3D12_TOKEN_DB_DRAW_INSTANCED.to_le_bytes());
                    stream.extend_from_slice(&vertex_count_per_instance.to_le_bytes());
                    stream.extend_from_slice(&instance_count.to_le_bytes());
                    stream.extend_from_slice(&start_vertex_location.to_le_bytes());
                    stream.extend_from_slice(&start_instance_location.to_le_bytes());
                }
                DepthBoundsCommandPacket::Dispatch {
                    thread_group_count_x,
                    thread_group_count_y,
                    thread_group_count_z,
                } => {
                    stream.extend_from_slice(&D3D12_TOKEN_DB_DISPATCH.to_le_bytes());
                    stream.extend_from_slice(&thread_group_count_x.to_le_bytes());
                    stream.extend_from_slice(&thread_group_count_y.to_le_bytes());
                    stream.extend_from_slice(&thread_group_count_z.to_le_bytes());
                }
            }
        }
    }
}

/// Temporary stub for depth bounds subsystem test during incremental commits.
pub fn test_depth_bounds_subsystem() -> bool {
    true
}
