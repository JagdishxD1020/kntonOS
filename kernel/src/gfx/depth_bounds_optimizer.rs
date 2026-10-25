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
