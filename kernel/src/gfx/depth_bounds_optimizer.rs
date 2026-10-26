//! Direct3D 12 Agility Dynamic Depth/Stencil Invalidation & Depth Bounds Optimization Engine.
//! Implements subpass depth/stencil discard optimization preventing redundant memory clears,
//! dynamic depth bounds test calculation with camera view frustum near/far plane tightening,
//! and stencil reference value dynamic masking for early-Z depth reject optimization.

#![allow(dead_code, non_camel_case_types)]

extern crate alloc;

// Subpass depth and stencil buffer load actions.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubpassAttachmentLoadOp {
    #[default]
    Load = 0,
    Clear = 1,
    Discard = 2,
    NoAccess = 3,
}

// Subpass depth and stencil buffer store actions.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubpassAttachmentStoreOp {
    #[default]
    Store = 0,
    Discard = 1,
    Resolve = 2,
    NoAccess = 3,
}

// Subpass depth/stencil discard flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SubpassDiscardFlags(pub u32);

impl SubpassDiscardFlags {
    pub const NONE: Self = Self(0);
    pub const DISCARD_DEPTH: Self = Self(1 << 0);
    pub const DISCARD_STENCIL: Self = Self(1 << 1);
    pub const PRESERVE_METADATA_ONLY: Self = Self(1 << 2);

    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline(always)]
    pub const fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

// Subpass depth/stencil attachment descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubpassDepthStencilAttachment {
    pub texture_id: u32,
    pub depth_load_op: SubpassAttachmentLoadOp,
    pub depth_store_op: SubpassAttachmentStoreOp,
    pub stencil_load_op: SubpassAttachmentLoadOp,
    pub stencil_store_op: SubpassAttachmentStoreOp,
    pub discard_flags: SubpassDiscardFlags,
}

impl SubpassDepthStencilAttachment {
    pub const fn new(
        texture_id: u32,
        depth_load_op: SubpassAttachmentLoadOp,
        depth_store_op: SubpassAttachmentStoreOp,
        stencil_load_op: SubpassAttachmentLoadOp,
        stencil_store_op: SubpassAttachmentStoreOp,
        discard_flags: SubpassDiscardFlags,
    ) -> Self {
        Self {
            texture_id,
            depth_load_op,
            depth_store_op,
            stencil_load_op,
            stencil_store_op,
            discard_flags,
        }
    }

    #[inline(always)]
    pub const fn can_skip_depth_clear(&self) -> bool {
        matches!(self.depth_load_op, SubpassAttachmentLoadOp::Discard | SubpassAttachmentLoadOp::NoAccess)
    }

    #[inline(always)]
    pub const fn can_skip_depth_store(&self) -> bool {
        matches!(self.depth_store_op, SubpassAttachmentStoreOp::Discard | SubpassAttachmentStoreOp::NoAccess)
            || self.discard_flags.contains(SubpassDiscardFlags::DISCARD_DEPTH)
    }

    #[inline(always)]
    pub const fn can_skip_stencil_store(&self) -> bool {
        matches!(self.stencil_store_op, SubpassAttachmentStoreOp::Discard | SubpassAttachmentStoreOp::NoAccess)
            || self.discard_flags.contains(SubpassDiscardFlags::DISCARD_STENCIL)
    }
}

// Subpass depth/stencil discard optimization governor.
#[derive(Debug)]
pub struct DepthStencilDiscardOptimizer {
    pub attachments_evaluated: u64,
    pub depth_clears_skipped: u64,
    pub depth_stores_skipped: u64,
    pub stencil_stores_skipped: u64,
    pub memory_bandwidth_saved_bytes: u64,
}

impl DepthStencilDiscardOptimizer {
    pub fn new() -> Self {
        Self {
            attachments_evaluated: 0,
            depth_clears_skipped: 0,
            depth_stores_skipped: 0,
            stencil_stores_skipped: 0,
            memory_bandwidth_saved_bytes: 0,
        }
    }

    // Evaluates a subpass attachment and applies discard / clear skip optimizations.
    pub fn evaluate_attachment(&mut self, attachment: &SubpassDepthStencilAttachment, surface_bytes: u64) {
        self.attachments_evaluated += 1;

        if attachment.can_skip_depth_clear() {
            self.depth_clears_skipped += 1;
            self.memory_bandwidth_saved_bytes += surface_bytes;
        }

        if attachment.can_skip_depth_store() {
            self.depth_stores_skipped += 1;
            self.memory_bandwidth_saved_bytes += surface_bytes;
        }

        if attachment.can_skip_stencil_store() {
            self.stencil_stores_skipped += 1;
            self.memory_bandwidth_saved_bytes += surface_bytes / 4;
        }
    }
}


// Frustum near and far plane bounds representation for projection matrices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewFrustumDepthRange {
    pub near_z: f32,
    pub far_z: f32,
}

impl ViewFrustumDepthRange {
    pub const fn new(near_z: f32, far_z: f32) -> Self {
        Self { near_z, far_z }
    }
}

// Dynamic depth bounds test calculation with camera view frustum tightening.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicDepthBoundsCalculator {
    pub min_depth: f32,
    pub max_depth: f32,
    pub active_draw_cluster_min_z: f32,
    pub active_draw_cluster_max_z: f32,
    pub bounds_tightened: bool,
}

impl DynamicDepthBoundsCalculator {
    pub fn new() -> Self {
        Self {
            min_depth: 0.0,
            max_depth: 1.0,
            active_draw_cluster_min_z: 0.0,
            active_draw_cluster_max_z: 1.0,
            bounds_tightened: false,
        }
    }

    // Projects a view-space cluster [cluster_near, cluster_far] into normalized device coordinates [0.0, 1.0].
    pub fn calculate_tightened_bounds(&mut self, frustum: ViewFrustumDepthRange, cluster_near: f32, cluster_far: f32) -> (f32, f32) {
        if frustum.far_z <= frustum.near_z || cluster_far < cluster_near {
            return (0.0, 1.0);
        }

        // Standard D3D12 linear projection mapping: z_ndc = (z_view - near) / (far - near)
        let depth_span = frustum.far_z - frustum.near_z;
        let clamped_near = if cluster_near < frustum.near_z { frustum.near_z } else { cluster_near };
        let clamped_far = if cluster_far > frustum.far_z { frustum.far_z } else { cluster_far };

        let norm_min = (clamped_near - frustum.near_z) / depth_span;
        let norm_max = (clamped_far - frustum.near_z) / depth_span;

        self.min_depth = if norm_min < 0.0 { 0.0 } else if norm_min > 1.0 { 1.0 } else { norm_min };
        self.max_depth = if norm_max < 0.0 { 0.0 } else if norm_max > 1.0 { 1.0 } else { norm_max };
        self.active_draw_cluster_min_z = cluster_near;
        self.active_draw_cluster_max_z = cluster_far;
        self.bounds_tightened = self.min_depth > 0.0 || self.max_depth < 1.0;

        (self.min_depth, self.max_depth)
    }

    // Evaluates whether a candidate depth falls within active tightened bounds.
    #[inline(always)]
    pub fn test_depth_value(&self, z_ndc: f32) -> bool {
        z_ndc >= self.min_depth && z_ndc <= self.max_depth
    }
}

// Stencil test comparison functions.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DynamicStencilComparison {
    #[default]
    Always = 0,
    Never = 1,
    Equal = 2,
    NotEqual = 3,
    Less = 4,
    LessEqual = 5,
    Greater = 6,
    GreaterEqual = 7,
}

// Stencil dynamic reference value and bitmask descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicStencilMaskState {
    pub reference_value: u8,
    pub read_mask: u8,
    pub write_mask: u8,
    pub comparison_func: DynamicStencilComparison,
    pub early_z_reject_enabled: bool,
}

impl Default for DynamicStencilMaskState {
    fn default() -> Self {
        Self {
            reference_value: 0,
            read_mask: 0xFF,
            write_mask: 0xFF,
            comparison_func: DynamicStencilComparison::Always,
            early_z_reject_enabled: false,
        }
    }
}

// Stencil reference value dynamic masking and early-Z reject optimizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StencilEarlyZRejectOptimizer {
    pub mask_state: DynamicStencilMaskState,
    pub total_tests: u64,
    pub rejected_fragments: u64,
}

impl StencilEarlyZRejectOptimizer {
    pub fn new(mask_state: DynamicStencilMaskState) -> Self {
        Self {
            mask_state,
            total_tests: 0,
            rejected_fragments: 0,
        }
    }

    // Configures active dynamic stencil reference and masking.
    pub fn set_stencil_ref(&mut self, reference: u8, read_mask: u8, comp: DynamicStencilComparison) {
        self.mask_state.reference_value = reference;
        self.mask_state.read_mask = read_mask;
        self.mask_state.comparison_func = comp;
        self.mask_state.early_z_reject_enabled = comp != DynamicStencilComparison::Always;
    }

    // Evaluates a fragment's stencil buffer value against the masked reference.
    pub fn test_fragment_stencil(&mut self, buffer_stencil: u8) -> bool {
        self.total_tests += 1;
        let ref_val = self.mask_state.reference_value & self.mask_state.read_mask;
        let buf_val = buffer_stencil & self.mask_state.read_mask;

        let pass = match self.mask_state.comparison_func {
            DynamicStencilComparison::Always => true,
            DynamicStencilComparison::Never => false,
            DynamicStencilComparison::Equal => ref_val == buf_val,
            DynamicStencilComparison::NotEqual => ref_val != buf_val,
            DynamicStencilComparison::Less => ref_val < buf_val,
            DynamicStencilComparison::LessEqual => ref_val <= buf_val,
            DynamicStencilComparison::Greater => ref_val > buf_val,
            DynamicStencilComparison::GreaterEqual => ref_val >= buf_val,
        };

        if !pass {
            self.rejected_fragments += 1;
        }
        pass
    }

    #[inline(always)]
    pub fn reject_percentage(&self) -> u32 {
        if self.total_tests == 0 {
            0
        } else {
            ((self.rejected_fragments * 100) / self.total_tests) as u32
        }
    }
}
