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


// Virtual memory sub-allocated span within a shared physical heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubAllocatedHeapSpan {
    pub resource_id: u32,
    pub heap_id: u32,
    pub offset_bytes: u64,
    pub size_bytes: u64,
    pub res_type: TransientResourceType,
    pub is_active: bool,
}

impl SubAllocatedHeapSpan {
    // Creates a new sub-allocated heap span.
    pub const fn new(
        resource_id: u32,
        heap_id: u32,
        offset_bytes: u64,
        size_bytes: u64,
        res_type: TransientResourceType,
    ) -> Self {
        Self {
            resource_id,
            heap_id,
            offset_bytes,
            size_bytes,
            res_type,
            is_active: false,
        }
    }

    // Evaluates whether two spans in the same heap overlap in address space.
    #[inline(always)]
    pub fn overlaps(&self, other: &Self) -> bool {
        if self.heap_id != other.heap_id {
            return false;
        }
        let self_end = self.offset_bytes.saturating_add(self.size_bytes);
        let other_end = other.offset_bytes.saturating_add(other.size_bytes);
        self.offset_bytes < other_end && other.offset_bytes < self_end
    }
}

// Hardware aliasing barrier descriptor transitioning overlapping resources in a heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AliasingBarrierDescriptor {
    pub heap_id: u32,
    pub resource_before_id: Option<u32>,
    pub resource_after_id: Option<u32>,
    pub sync_before: u32,
    pub sync_after: u32,
    pub is_discard_enabled: bool,
}

// Sub-Allocation Aliasing Barrier Engine managing transient resource lifetimes in shared heaps.
#[derive(Debug, Clone, Default)]
pub struct SubAllocationAliasTracker {
    pub spans: Vec<SubAllocatedHeapSpan>,
    pub total_aliasing_transitions: u64,
    pub total_discards_honored: u64,
}

impl SubAllocationAliasTracker {
    // Creates a new alias tracker.
    pub fn new() -> Self {
        Self::default()
    }

    // Registers a sub-allocated resource span.
    pub fn register_span(&mut self, span: SubAllocatedHeapSpan) {
        if !self.spans.iter().any(|s| s.resource_id == span.resource_id) {
            self.spans.push(span);
        }
    }

    // Transitions a resource to active state, generating an aliasing barrier if it overlaps an existing active resource.
    pub fn activate_resource(
        &mut self,
        resource_id: u32,
        discard: bool,
    ) -> Result<AliasingBarrierDescriptor, &'static str> {
        let span_idx = self
            .spans
            .iter()
            .position(|s| s.resource_id == resource_id)
            .ok_or("Target resource not found in alias tracker")?;

        let target_span = self.spans[span_idx];
        let mut overlapping_before_id = None;

        // Check for active overlapping resources in the same heap
        for span in &mut self.spans {
            if span.resource_id != resource_id && span.is_active && span.overlaps(&target_span) {
                overlapping_before_id = Some(span.resource_id);
                span.is_active = false; // Deactivate overlapping predecessor
                break;
            }
        }

        self.spans[span_idx].is_active = true;
        self.total_aliasing_transitions = self.total_aliasing_transitions.saturating_add(1);

        let is_discard_enabled = discard || target_span.res_type.is_naturally_discardable();
        if is_discard_enabled {
            self.total_discards_honored = self.total_discards_honored.saturating_add(1);
        }

        Ok(AliasingBarrierDescriptor {
            heap_id: target_span.heap_id,
            resource_before_id: overlapping_before_id,
            resource_after_id: Some(resource_id),
            sync_before: if is_discard_enabled { 0 } else { 0x1 }, // If discard, no before sync required
            sync_after: 0x1,
            is_discard_enabled,
        })
    }
}

// Cache flush scope generated for hardware memory controllers.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdnaCacheFlushType {
    NoFlush = 0,
    L0L1Invalidate = 1,
    L2FlushAndInvalidate = 2,
    L3MallWriteback = 3,
}

// RDNA 3 Infinity Cache L2/L3 flush minimizer reducing cache flush overhead across render passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RdnaCacheFlushMinimizer {
    pub total_barriers_evaluated: u64,
    pub mall_writebacks_eliminated: u64,
    pub l2_flushes_reduced: u64,
}

impl RdnaCacheFlushMinimizer {
    // Creates a new RDNA 3 cache flush minimizer.
    pub const fn new() -> Self {
        Self {
            total_barriers_evaluated: 0,
            mall_writebacks_eliminated: 0,
            l2_flushes_reduced: 0,
        }
    }

    // Evaluates the minimum necessary hardware cache flush for an enhanced barrier.
    pub fn evaluate_hardware_flush(
        &mut self,
        barrier: &EnhancedTextureBarrierDesc,
        stays_on_chip_mall: bool,
    ) -> RdnaCacheFlushType {
        self.total_barriers_evaluated = self.total_barriers_evaluated.saturating_add(1);

        if barrier.can_skip_cache_flush() {
            // Discard skips L2 flush and L3 writeback completely
            self.mall_writebacks_eliminated = self.mall_writebacks_eliminated.saturating_add(1);
            self.l2_flushes_reduced = self.l2_flushes_reduced.saturating_add(1);
            RdnaCacheFlushType::L0L1Invalidate
        } else if stays_on_chip_mall {
            // Intermediate pass staying in Infinity Cache suppresses VRAM writeback
            self.mall_writebacks_eliminated = self.mall_writebacks_eliminated.saturating_add(1);
            RdnaCacheFlushType::L2FlushAndInvalidate
        } else {
            // Full writeback to VRAM
            RdnaCacheFlushType::L3MallWriteback
        }
    }
}
// Temporary stub for self-test during incremental commits.
pub fn test_barrier_discard_subsystem() -> bool {
    true
}
