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
