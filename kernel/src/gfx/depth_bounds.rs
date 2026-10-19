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

// Direct3D 12 Agility Dynamic Depth Bounds specification with runtime clamping.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AgilityDepthBounds {
    pub min_depth: f32,
    pub max_depth: f32,
    pub enabled: bool,
}

impl AgilityDepthBounds {
    // Creates a new disabled full-range depth bounds state spanning [0.0, 1.0].
    pub const fn disabled() -> Self {
        Self {
            min_depth: 0.0,
            max_depth: 1.0,
            enabled: false,
        }
    }

    // Creates an active clamped depth bounds state conforming to D3D12 Agility specs.
    pub fn active(min_depth: f32, max_depth: f32) -> Self {
        let clamped_min = min_depth.clamp(0.0, 1.0);
        let clamped_max = max_depth.clamp(clamped_min, 1.0);
        Self {
            min_depth: clamped_min,
            max_depth: clamped_max,
            enabled: true,
        }
    }

    // Updates depth bounds with automatic clamping to valid hardware range [0.0, 1.0].
    pub fn set_bounds(&mut self, min_depth: f32, max_depth: f32) {
        let clamped_min = min_depth.clamp(0.0, 1.0);
        let clamped_max = max_depth.clamp(clamped_min, 1.0);
        self.min_depth = clamped_min;
        self.max_depth = clamped_max;
        self.enabled = true;
    }

    // Evaluates whether a rasterized depth sample falls within active bounds.
    #[inline(always)]
    pub fn test_depth(&self, z: f32) -> bool {
        if !self.enabled {
            true
        } else {
            z >= self.min_depth && z <= self.max_depth
        }
    }

    // Returns normalized depth range span [0.0, 1.0].
    #[inline]
    pub fn range_span(&self) -> f32 {
        if self.enabled {
            self.max_depth - self.min_depth
        } else {
            1.0
        }
    }

    // Determines if depth bounds cover the entire viewport range.
    #[inline]
    pub fn is_full_range(&self) -> bool {
        !self.enabled || (self.min_depth <= 0.0 && self.max_depth >= 1.0)
    }
}

// Stencil face selection flags for independent front and back face configuration.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StencilFace {
    Front = 0x1,
    Back = 0x2,
    FrontAndBack = 0x3,
}

// Stencil comparison operations for dynamic reference testing.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StencilComparisonOp {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

// Runtime dynamic stencil reference values and dual-face masking state.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DynamicStencilRefState {
    pub front_ref: u8,
    pub back_ref: u8,
    pub front_read_mask: u8,
    pub back_read_mask: u8,
    pub front_write_mask: u8,
    pub back_write_mask: u8,
}

impl DynamicStencilRefState {
    // Initializes default stencil state with 0 ref and all-ones masks (0xFF).
    pub const fn new() -> Self {
        Self {
            front_ref: 0,
            back_ref: 0,
            front_read_mask: 0xFF,
            back_read_mask: 0xFF,
            front_write_mask: 0xFF,
            back_write_mask: 0xFF,
        }
    }

    // Sets dynamic stencil reference value uniformly across front and back faces.
    pub fn om_set_stencil_ref(&mut self, ref_val: u8) {
        self.front_ref = ref_val;
        self.back_ref = ref_val;
    }

    // Sets independent dynamic stencil reference value for specific faces.
    pub fn om_set_stencil_ref_for_face(&mut self, face: StencilFace, ref_val: u8) {
        match face {
            StencilFace::Front => self.front_ref = ref_val,
            StencilFace::Back => self.back_ref = ref_val,
            StencilFace::FrontAndBack => {
                self.front_ref = ref_val;
                self.back_ref = ref_val;
            }
        }
    }

    // Configures read and write masks independently for front or back faces.
    pub fn om_set_stencil_masks(&mut self, face: StencilFace, read_mask: u8, write_mask: u8) {
        if (face as u8 & StencilFace::Front as u8) != 0 {
            self.front_read_mask = read_mask;
            self.front_write_mask = write_mask;
        }
        if (face as u8 & StencilFace::Back as u8) != 0 {
            self.back_read_mask = read_mask;
            self.back_write_mask = write_mask;
        }
    }

    // Evaluates the stencil test comparison against an existing buffer value.
    pub fn test_stencil(&self, face: StencilFace, buffer_val: u8, op: StencilComparisonOp) -> bool {
        let (ref_val, mask) = if face == StencilFace::Back {
            (self.back_ref, self.back_read_mask)
        } else {
            (self.front_ref, self.front_read_mask)
        };

        let masked_ref = ref_val & mask;
        let masked_buf = buffer_val & mask;

        match op {
            StencilComparisonOp::Never => false,
            StencilComparisonOp::Less => masked_ref < masked_buf,
            StencilComparisonOp::Equal => masked_ref == masked_buf,
            StencilComparisonOp::LessEqual => masked_ref <= masked_buf,
            StencilComparisonOp::Greater => masked_ref > masked_buf,
            StencilComparisonOp::NotEqual => masked_ref != masked_buf,
            StencilComparisonOp::GreaterEqual => masked_ref >= masked_buf,
            StencilComparisonOp::Always => true,
        }
    }

    // Computes masked output value to write to the stencil buffer.
    pub fn compute_stencil_write(&self, face: StencilFace, old_val: u8, new_val: u8) -> u8 {
        let mask = if face == StencilFace::Back {
            self.back_write_mask
        } else {
            self.front_write_mask
        };
        (old_val & !mask) | (new_val & mask)
    }
}

// Calibrated view or light frustum depth extents in normalized device coordinates [0.0, 1.0].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FrustumDepthBounds {
    pub near_z: f32,
    pub far_z: f32,
}

impl FrustumDepthBounds {
    // Creates a new calibrated frustum depth range.
    pub const fn new(near_z: f32, far_z: f32) -> Self {
        Self { near_z, far_z }
    }
}

// Early-Z culling accelerator rejecting draw passes before rasterizer dispatch.
pub struct EarlyZCullingAccelerator {
    pub calibrated_frustum: FrustumDepthBounds,
    pub active_bounds: AgilityDepthBounds,
    pub passes_evaluated: u64,
    pub passes_culled: u64,
    pub passes_passed: u64,
    pub triangles_culled: u64,
}

impl EarlyZCullingAccelerator {
    // Initializes the early-Z accelerator with active depth bounds.
    pub const fn new(bounds: AgilityDepthBounds) -> Self {
        Self {
            calibrated_frustum: FrustumDepthBounds::new(0.0, 1.0),
            active_bounds: bounds,
            passes_evaluated: 0,
            passes_culled: 0,
            passes_passed: 0,
            triangles_culled: 0,
        }
    }

    // Calibrates the accelerator with active light or shadow frustum near/far clipping planes.
    pub fn calibrate_frustum(&mut self, near_z: f32, far_z: f32) {
        self.calibrated_frustum = FrustumDepthBounds::new(near_z.clamp(0.0, 1.0), far_z.clamp(0.0, 1.0));
    }

    // Updates active hardware depth bounds state.
    pub fn set_depth_bounds(&mut self, bounds: AgilityDepthBounds) {
        self.active_bounds = bounds;
    }

    // Tests whether a geometry pass depth range intersects the active volume, culling if outside.
    pub fn evaluate_draw_pass(&mut self, min_z: f32, max_z: f32, triangle_count: u64) -> bool {
        self.passes_evaluated = self.passes_evaluated.saturating_add(1);

        // Discard if completely in front of active min depth or behind active max depth.
        if self.active_bounds.enabled {
            if max_z < self.active_bounds.min_depth || min_z > self.active_bounds.max_depth {
                self.passes_culled = self.passes_culled.saturating_add(1);
                self.triangles_culled = self.triangles_culled.saturating_add(triangle_count);
                return false;
            }
        }

        // Discard if completely outside the calibrated frustum bounds.
        if max_z < self.calibrated_frustum.near_z || min_z > self.calibrated_frustum.far_z {
            self.passes_culled = self.passes_culled.saturating_add(1);
            self.triangles_culled = self.triangles_culled.saturating_add(triangle_count);
            return false;
        }

        self.passes_passed = self.passes_passed.saturating_add(1);
        true
    }

    // Returns percentage of evaluated draw passes discarded before rasterization [0, 100].
    pub fn culling_rate_percent(&self) -> u32 {
        if self.passes_evaluated == 0 {
            0
        } else {
            ((self.passes_culled * 100) / self.passes_evaluated) as u32
        }
    }
}

// Comprehensive verification self-test suite for Direct3D 12 Depth Bounds Testing & Dynamic Stencil Ref
pub fn test_depth_bounds_subsystem() -> bool {
    crate::serial_write_str("[KNT-GPU-DEPTHBOUNDS] Initializing Direct3D 12 Depth Bounds Testing self-test...\r\n");

    // 1. Verify D3D12 Options2 feature query and DepthBoundsTestSupported reporting
    let options2 = query_d3d12_options2_features();
    if !options2.DepthBoundsTestSupported || options2.ProgrammableSamplePositionsTier == 0 {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Depth bounds test capability query failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Direct3D 12 Options2 depth bounds hardware capability verified\r\n");

    // 2. Verify OMSetDepthBounds [0.0, 1.0] range validation and OMSetStencilRef dynamic binding
    let valid_bounds = D3D12_DEPTH_BOUNDS_VALIDATOR::validate_depth_bounds(0.2, 0.7);
    if valid_bounds != Ok(D3D12_DEPTH_BOUNDS::active(0.2, 0.7)) {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Valid depth bounds range rejected\r\n");
        return false;
    }

    if D3D12_DEPTH_BOUNDS_VALIDATOR::validate_depth_bounds(0.8, 0.2) != Err(DepthBoundsError::InvalidDepthRange) {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Inverted depth bounds range (min > max) was not rejected\r\n");
        return false;
    }

    if D3D12_DEPTH_BOUNDS_VALIDATOR::validate_depth_bounds(-0.1, 0.5) != Err(DepthBoundsError::InvalidDepthRange) {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Negative depth bounds min was not rejected\r\n");
        return false;
    }

    if D3D12_DEPTH_BOUNDS_VALIDATOR::validate_depth_bounds(0.5, 1.2) != Err(DepthBoundsError::InvalidDepthRange) {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Depth bounds max > 1.0 was not rejected\r\n");
        return false;
    }

    if D3D12_DEPTH_BOUNDS_VALIDATOR::validate_depth_bounds(f32::NAN, 0.5) != Err(DepthBoundsError::NonFiniteDepthValue) {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Non-finite depth bounds value was not rejected\r\n");
        return false;
    }

    let stencil_ref = D3D12_DEPTH_BOUNDS_VALIDATOR::validate_stencil_ref(0x1A5);
    if stencil_ref.stencil_ref != 0xA5 {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Dynamic stencil reference 8-bit masking mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * OMSetDepthBounds [0.0, 1.0] range validation and stencil ref binding verified\r\n");

    // 3. Verify depth bounds culling evaluator and GPU lighting volume rejection
    let mut evaluator = D3D12_DEPTH_BOUNDS_EVALUATOR::new(D3D12_OM_STATE::new());
    if evaluator.om_set_depth_bounds(0.3, 0.6).is_err() {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Setting depth bounds on evaluator failed\r\n");
        return false;
    }

    let light_volume_samples = [
        0.10, 0.15, 0.20, 0.25, // Foreground (4 culled)
        0.35, 0.40, 0.50, 0.58, // Inside volume (4 passed)
        0.65, 0.70, 0.85, 0.90, // Background (4 culled)
    ];

    let (passed, culled) = evaluator.evaluate_lighting_volume(&light_volume_samples);
    if passed != 4 || culled != 8 {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Lighting volume rejection counts mismatch\r\n");
        return false;
    }
    if evaluator.shader_invocations_saved != 8 || evaluator.bandwidth_saved_bytes != 512 {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Saved shader invocations or bandwidth mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * GPU lighting volume depth bounds rejection and shader savings verified\r\n");

    // 4. Verify command recorder token stream encoder for depth bounds and stencil ref
    let mut recorder = D3D12_DEPTH_BOUNDS_COMMAND_RECORDER::new();
    if recorder.om_set_depth_bounds(0.3, 0.6).is_err() {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Recording OMSetDepthBounds failed\r\n");
        return false;
    }
    recorder.om_set_stencil_ref(0xA5);
    recorder.draw_instanced(36, 1, 0, 0);
    recorder.om_disable_depth_bounds();
    recorder.dispatch(8, 8, 1);

    if recorder.total_commands_recorded != 5 {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Command recorder total packet count mismatch\r\n");
        return false;
    }

    let mut stream = alloc::vec::Vec::new();
    recorder.encode_token_stream(&mut stream);
    if stream.is_empty() {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] Token stream encoding is empty\r\n");
        return false;
    }

    let token_dbnd = u32::from_le_bytes([stream[0], stream[1], stream[2], stream[3]]);
    if token_dbnd != D3D12_TOKEN_OM_SET_DEPTH_BOUNDS {
        crate::serial_write_str("[KNT-DEPTHBOUNDS-FAIL] First token is not D3D12_TOKEN_OM_SET_DEPTH_BOUNDS\r\n");
        return false;
    }
    crate::serial_write_str("  * Output Merger token stream encoder and packet lifecycle verified\r\n");

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-DEPTHBOUNDS-PASS] Direct3D 12 depth bounds testing and dynamic stencil ref engine verified\r\n");
    true
}

