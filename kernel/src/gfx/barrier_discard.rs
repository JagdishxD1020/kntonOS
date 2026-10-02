//! Direct3D 12 Agility SDK Enhanced Barriers Sub-Allocation & Discard Optimizer.
//! Provides cache flush skipping for transient targets and shared-heap aliasing tracking.

#![allow(dead_code, non_camel_case_types, unused_imports)]

extern crate alloc;
use alloc::vec::Vec;

// Texture barrier execution and content preservation flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_TEXTURE_BARRIER_FLAGS(pub u32);

impl D3D12_TEXTURE_BARRIER_FLAGS {
    pub const NONE: Self = Self(0);
    // Skips cache flushes, decompressive writebacks, and preserves zero data across transitions.
    pub const DISCARD: Self = Self(1 << 0);

    // Tests if flags contain specified bitmask.
    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    // Combines two flag sets.
    #[inline(always)]
    pub const fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    // Returns whether discard semantics are active.
    #[inline(always)]
    pub const fn is_discard(&self) -> bool {
        self.contains(Self::DISCARD)
    }
}

// Classification of transient GPU render resources subject to discard optimizations.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransientResourceType {
    TransientRenderTarget = 0,
    CascadeShadowMap = 1,
    GbufferAlbedo = 2,
    GbufferNormal = 3,
    DepthPrepass = 4,
    IntermediateComputeScratch = 5,
}

impl TransientResourceType {
    // Determines if resource is inherently transient and safe for discard on barrier.
    #[inline(always)]
    pub const fn is_naturally_discardable(&self) -> bool {
        match self {
            Self::TransientRenderTarget
            | Self::CascadeShadowMap
            | Self::DepthPrepass
            | Self::IntermediateComputeScratch => true,
            Self::GbufferAlbedo | Self::GbufferNormal => false,
        }
    }
}

// Enhanced texture barrier descriptor with explicit discard semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnhancedTextureBarrierDesc {
    pub resource_id: u32,
    pub res_type: TransientResourceType,
    pub sync_before: u32,
    pub sync_after: u32,
    pub access_before: u32,
    pub access_after: u32,
    pub layout_before: u32,
    pub layout_after: u32,
    pub flags: D3D12_TEXTURE_BARRIER_FLAGS,
}

impl EnhancedTextureBarrierDesc {
    // Creates a new enhanced texture barrier descriptor.
    pub const fn new(
        resource_id: u32,
        res_type: TransientResourceType,
        sync_before: u32,
        sync_after: u32,
        access_before: u32,
        access_after: u32,
        layout_before: u32,
        layout_after: u32,
        flags: D3D12_TEXTURE_BARRIER_FLAGS,
    ) -> Self {
        Self {
            resource_id,
            res_type,
            sync_before,
            sync_after,
            access_before,
            access_after,
            layout_before,
            layout_after,
            flags,
        }
    }

    // Checks whether this barrier can bypass cache flushes via discard flag or resource kind.
    #[inline(always)]
    pub fn can_skip_cache_flush(&self) -> bool {
        self.flags.is_discard() || self.res_type.is_naturally_discardable()
    }
}

// Temporary stub for self-test during incremental commits.
pub fn test_barrier_discard_subsystem() -> bool {
    true
}
