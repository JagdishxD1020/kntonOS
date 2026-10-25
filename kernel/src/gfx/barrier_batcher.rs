//! Direct3D 12 Agility Command Queue Barrier Batching & Barrier Insertion Elimination Governor.
//! Provides split-barrier generation into BeginBarrier and EndBarrier pairs, redundant transition
//! elimination, and enhanced barrier type batching with pipeline stage flags.

#![allow(dead_code, non_camel_case_types)]

extern crate alloc;
use alloc::vec::Vec;

// Hardware pipeline stage flags for barrier synchronization scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BarrierPipelineStageFlags(pub u32);

impl BarrierPipelineStageFlags {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(1 << 0);
    pub const DRAW: Self = Self(1 << 1);
    pub const INDEX_INPUT: Self = Self(1 << 2);
    pub const VERTEX_SHADING: Self = Self(1 << 3);
    pub const PIXEL_SHADING: Self = Self(1 << 4);
    pub const DEPTH_STENCIL: Self = Self(1 << 5);
    pub const RENDER_TARGET: Self = Self(1 << 6);
    pub const COMPUTE: Self = Self(1 << 7);
    pub const RAYTRACING: Self = Self(1 << 8);
    pub const COPY: Self = Self(1 << 9);
    pub const RESOLVE: Self = Self(1 << 10);
    pub const EXECUTE_INDIRECT: Self = Self(1 << 11);
    pub const ALL_SHADING: Self = Self(1 << 12);

    #[inline(always)]
    pub const fn bits(&self) -> u32 {
        self.0
    }

    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline(always)]
    pub const fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

// Memory access cache flush and invalidation flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BarrierMemoryAccessFlags(pub u32);

impl BarrierMemoryAccessFlags {
    pub const COMMON: Self = Self(0);
    pub const VERTEX_BUFFER: Self = Self(1 << 0);
    pub const CONSTANT_BUFFER: Self = Self(1 << 1);
    pub const INDEX_BUFFER: Self = Self(1 << 2);
    pub const RENDER_TARGET: Self = Self(1 << 3);
    pub const UNORDERED_ACCESS: Self = Self(1 << 4);
    pub const DEPTH_STENCIL_WRITE: Self = Self(1 << 5);
    pub const DEPTH_STENCIL_READ: Self = Self(1 << 6);
    pub const SHADER_RESOURCE: Self = Self(1 << 7);
    pub const COPY_SOURCE: Self = Self(1 << 8);
    pub const COPY_DEST: Self = Self(1 << 9);
    pub const RESOLVE_SOURCE: Self = Self(1 << 10);
    pub const RESOLVE_DEST: Self = Self(1 << 11);
    pub const NO_ACCESS: Self = Self(1 << 12);

    #[inline(always)]
    pub const fn bits(&self) -> u32 {
        self.0
    }

    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline(always)]
    pub const fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

// Texture layout state representation.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BatchBarrierLayout {
    #[default]
    Undefined = 0,
    Common = 1,
    Present = 2,
    GenericRead = 3,
    RenderTarget = 4,
    UnorderedAccess = 5,
    DepthStencilWrite = 6,
    DepthStencilRead = 7,
    ShaderResource = 8,
    CopySource = 9,
    CopyDest = 10,
    ResolveSource = 11,
    ResolveDest = 12,
}

// Subresource range identifier for batch barrier operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchSubresourceRange {
    pub first_mip: u32,
    pub num_mips: u32,
    pub first_slice: u32,
    pub num_slices: u32,
    pub first_plane: u32,
    pub num_planes: u32,
}

impl BatchSubresourceRange {
    pub const ALL_SUBRESOURCES: u32 = 0xFFFF_FFFF;

    pub const fn all() -> Self {
        Self {
            first_mip: Self::ALL_SUBRESOURCES,
            num_mips: 0,
            first_slice: 0,
            num_slices: 0,
            first_plane: 0,
            num_planes: 0,
        }
    }

    pub const fn single_mip(mip: u32) -> Self {
        Self {
            first_mip: mip,
            num_mips: 1,
            first_slice: 0,
            num_slices: 1,
            first_plane: 0,
            num_planes: 1,
        }
    }

    #[inline(always)]
    pub const fn is_all(&self) -> bool {
        self.first_mip == Self::ALL_SUBRESOURCES
    }
}

// Barrier execution split phase.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitBarrierPhase {
    Immediate = 0,
    Begin = 1,
    End = 2,
}

// Global execution and memory barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchedGlobalBarrier {
    pub sync_before: BarrierPipelineStageFlags,
    pub sync_after: BarrierPipelineStageFlags,
    pub access_before: BarrierMemoryAccessFlags,
    pub access_after: BarrierMemoryAccessFlags,
}

// Buffer range transition barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchedBufferBarrier {
    pub buffer_id: u32,
    pub sync_before: BarrierPipelineStageFlags,
    pub sync_after: BarrierPipelineStageFlags,
    pub access_before: BarrierMemoryAccessFlags,
    pub access_after: BarrierMemoryAccessFlags,
    pub offset: u64,
    pub size: u64,
}

// Texture layout and subresource transition barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchedTextureBarrier {
    pub texture_id: u32,
    pub sync_before: BarrierPipelineStageFlags,
    pub sync_after: BarrierPipelineStageFlags,
    pub access_before: BarrierMemoryAccessFlags,
    pub access_after: BarrierMemoryAccessFlags,
    pub layout_before: BatchBarrierLayout,
    pub layout_after: BatchBarrierLayout,
    pub subresources: BatchSubresourceRange,
    pub discard_contents: bool,
}

// Batch container holding heterogeneous barrier items.
#[derive(Debug, Clone, Default)]
pub struct BarrierBatch {
    pub global_barriers: Vec<BatchedGlobalBarrier>,
    pub buffer_barriers: Vec<BatchedBufferBarrier>,
    pub texture_barriers: Vec<BatchedTextureBarrier>,
}

impl BarrierBatch {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.global_barriers.is_empty() && self.buffer_barriers.is_empty() && self.texture_barriers.is_empty()
    }

    #[inline(always)]
    pub fn count(&self) -> usize {
        self.global_barriers.len() + self.buffer_barriers.len() + self.texture_barriers.len()
    }
}

// Split barrier generator pairing BeginBarrier and EndBarrier dispatches.
#[derive(Debug)]
pub struct SplitBarrierPair {
    pub token: u64,
    pub begin_batch: BarrierBatch,
    pub end_batch: BarrierBatch,
    pub is_retired: bool,
}

// Dynamic split-barrier generator hiding GPU pipeline stall latency.
#[derive(Debug)]
pub struct DynamicSplitBarrierGenerator {
    pub next_token: u64,
    pub active_pairs: Vec<SplitBarrierPair>,
    pub total_split_pairs_generated: u64,
}

impl DynamicSplitBarrierGenerator {
    pub fn new() -> Self {
        Self {
            next_token: 1,
            active_pairs: Vec::new(),
            total_split_pairs_generated: 0,
        }
    }

    // Splits an immediate batch into asynchronous BeginBarrier and EndBarrier batches.
    pub fn create_split_pair(&mut self, batch: BarrierBatch) -> u64 {
        let token = self.next_token;
        self.next_token += 1;

        let mut begin_batch = BarrierBatch::new();
        let mut end_batch = BarrierBatch::new();

        // Global barriers: Begin performs flush, End unblocks waiting stages.
        for g in batch.global_barriers {
            begin_batch.global_barriers.push(BatchedGlobalBarrier {
                sync_before: g.sync_before,
                sync_after: BarrierPipelineStageFlags::NONE,
                access_before: g.access_before,
                access_after: BarrierMemoryAccessFlags::COMMON,
            });
            end_batch.global_barriers.push(BatchedGlobalBarrier {
                sync_before: BarrierPipelineStageFlags::NONE,
                sync_after: g.sync_after,
                access_before: BarrierMemoryAccessFlags::COMMON,
                access_after: g.access_after,
            });
        }

        // Buffer barriers: split cache flush from consumer stage unblocking.
        for b in batch.buffer_barriers {
            begin_batch.buffer_barriers.push(BatchedBufferBarrier {
                buffer_id: b.buffer_id,
                sync_before: b.sync_before,
                sync_after: BarrierPipelineStageFlags::NONE,
                access_before: b.access_before,
                access_after: BarrierMemoryAccessFlags::COMMON,
                offset: b.offset,
                size: b.size,
            });
            end_batch.buffer_barriers.push(BatchedBufferBarrier {
                buffer_id: b.buffer_id,
                sync_before: BarrierPipelineStageFlags::NONE,
                sync_after: b.sync_after,
                access_before: BarrierMemoryAccessFlags::COMMON,
                access_after: b.access_after,
                offset: b.offset,
                size: b.size,
            });
        }

        // Texture barriers: Begin initiates decompressive transitions, End finalizes layout.
        for t in batch.texture_barriers {
            begin_batch.texture_barriers.push(BatchedTextureBarrier {
                texture_id: t.texture_id,
                sync_before: t.sync_before,
                sync_after: BarrierPipelineStageFlags::NONE,
                access_before: t.access_before,
                access_after: BarrierMemoryAccessFlags::COMMON,
                layout_before: t.layout_before,
                layout_after: t.layout_after,
                subresources: t.subresources,
                discard_contents: t.discard_contents,
            });
            end_batch.texture_barriers.push(BatchedTextureBarrier {
                texture_id: t.texture_id,
                sync_before: BarrierPipelineStageFlags::NONE,
                sync_after: t.sync_after,
                access_before: BarrierMemoryAccessFlags::COMMON,
                access_after: t.access_after,
                layout_before: t.layout_before,
                layout_after: t.layout_after,
                subresources: t.subresources,
                discard_contents: t.discard_contents,
            });
        }

        self.active_pairs.push(SplitBarrierPair {
            token,
            begin_batch,
            end_batch,
            is_retired: false,
        });
        self.total_split_pairs_generated += 1;
        token
    }

    // Retires an active split barrier pair when EndBarrier is submitted.
    pub fn retire_pair(&mut self, token: u64) -> bool {
        for pair in &mut self.active_pairs {
            if pair.token == token && !pair.is_retired {
                pair.is_retired = true;
                return true;
            }
        }
        false
    }
}


// Subresource layout tracking entry for barrier deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubresourceLayoutEntry {
    pub current_layout: BatchBarrierLayout,
    pub current_access: BarrierMemoryAccessFlags,
    pub current_sync: BarrierPipelineStageFlags,
}

// Subresource layout state tracker per registered texture.
#[derive(Debug, Clone)]
pub struct TextureLayoutTracker {
    pub texture_id: u32,
    pub mip_count: u32,
    pub slice_count: u32,
    pub subresources: Vec<SubresourceLayoutEntry>,
}

impl TextureLayoutTracker {
    pub fn new(texture_id: u32, mip_count: u32, slice_count: u32, initial_layout: BatchBarrierLayout) -> Self {
        let total = (mip_count * slice_count).max(1) as usize;
        let mut subresources = Vec::with_capacity(total);
        for _ in 0..total {
            subresources.push(SubresourceLayoutEntry {
                current_layout: initial_layout,
                current_access: BarrierMemoryAccessFlags::COMMON,
                current_sync: BarrierPipelineStageFlags::ALL,
            });
        }
        Self {
            texture_id,
            mip_count,
            slice_count,
            subresources,
        }
    }
}

// Redundant subresource layout transition deduplication and barrier elimination governor.
#[derive(Debug)]
pub struct BarrierEliminationGovernor {
    pub texture_trackers: Vec<TextureLayoutTracker>,
    pub redundant_barriers_eliminated: u64,
    pub total_barriers_processed: u64,
}

impl BarrierEliminationGovernor {
    pub fn new() -> Self {
        Self {
            texture_trackers: Vec::new(),
            redundant_barriers_eliminated: 0,
            total_barriers_processed: 0,
        }
    }

    // Registers a texture resource for layout tracking.
    pub fn register_texture(&mut self, texture_id: u32, mips: u32, slices: u32, initial_layout: BatchBarrierLayout) {
        self.texture_trackers.push(TextureLayoutTracker::new(texture_id, mips, slices, initial_layout));
    }

    // Filters redundant texture transitions from a barrier batch.
    pub fn filter_batch(&mut self, batch: BarrierBatch) -> BarrierBatch {
        let mut filtered = BarrierBatch::new();
        filtered.global_barriers = batch.global_barriers;
        filtered.buffer_barriers = batch.buffer_barriers;

        for barrier in batch.texture_barriers {
            self.total_barriers_processed += 1;
            let mut is_redundant = false;

            if let Some(tracker) = self.texture_trackers.iter_mut().find(|t| t.texture_id == barrier.texture_id) {
                // If layout is unchanged, access is compatible, and not discard, barrier can be eliminated.
                if barrier.layout_before == barrier.layout_after && !barrier.discard_contents {
                    let mut all_match = true;
                    if barrier.subresources.is_all() {
                        for sub in &tracker.subresources {
                            if sub.current_layout != barrier.layout_after {
                                all_match = false;
                                break;
                            }
                        }
                    } else {
                        let idx = barrier.subresources.first_mip as usize;
                        if let Some(sub) = tracker.subresources.get(idx) {
                            if sub.current_layout != barrier.layout_after {
                                all_match = false;
                            }
                        }
                    }
                    if all_match {
                        is_redundant = true;
                    }
                }

                if !is_redundant {
                    // Update tracked layouts
                    if barrier.subresources.is_all() {
                        for sub in &mut tracker.subresources {
                            sub.current_layout = barrier.layout_after;
                            sub.current_access = barrier.access_after;
                            sub.current_sync = barrier.sync_after;
                        }
                    } else {
                        let idx = barrier.subresources.first_mip as usize;
                        if let Some(sub) = tracker.subresources.get_mut(idx) {
                            sub.current_layout = barrier.layout_after;
                            sub.current_access = barrier.access_after;
                            sub.current_sync = barrier.sync_after;
                        }
                    }
                }
            }

            if is_redundant {
                self.redundant_barriers_eliminated += 1;
            } else {
                filtered.texture_barriers.push(barrier);
            }
        }

        filtered
    }

    #[inline(always)]
    pub fn elimination_percentage(&self) -> u32 {
        if self.total_barriers_processed == 0 {
            0
        } else {
            ((self.redundant_barriers_eliminated * 100) / self.total_barriers_processed) as u32
        }
    }
}

// Classification of enhanced barrier types.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnhancedBarrierKind {
    Global = 0,
    Buffer = 1,
    Texture = 2,
}

// Unified barrier descriptor tagged with enhanced barrier type and hardware pipeline flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnifiedBarrierDesc {
    pub kind: EnhancedBarrierKind,
    pub sync_before: BarrierPipelineStageFlags,
    pub sync_after: BarrierPipelineStageFlags,
    pub access_before: BarrierMemoryAccessFlags,
    pub access_after: BarrierMemoryAccessFlags,
    pub resource_id: u64,
    pub layout_before: BatchBarrierLayout,
    pub layout_after: BatchBarrierLayout,
}

impl UnifiedBarrierDesc {
    pub fn from_global(g: &BatchedGlobalBarrier) -> Self {
        Self {
            kind: EnhancedBarrierKind::Global,
            sync_before: g.sync_before,
            sync_after: g.sync_after,
            access_before: g.access_before,
            access_after: g.access_after,
            resource_id: 0,
            layout_before: BatchBarrierLayout::Undefined,
            layout_after: BatchBarrierLayout::Undefined,
        }
    }

    pub fn from_buffer(b: &BatchedBufferBarrier) -> Self {
        Self {
            kind: EnhancedBarrierKind::Buffer,
            sync_before: b.sync_before,
            sync_after: b.sync_after,
            access_before: b.access_before,
            access_after: b.access_after,
            resource_id: b.buffer_id as u64,
            layout_before: BatchBarrierLayout::Undefined,
            layout_after: BatchBarrierLayout::Undefined,
        }
    }

    pub fn from_texture(t: &BatchedTextureBarrier) -> Self {
        Self {
            kind: EnhancedBarrierKind::Texture,
            sync_before: t.sync_before,
            sync_after: t.sync_after,
            access_before: t.access_before,
            access_after: t.access_after,
            resource_id: t.texture_id as u64,
            layout_before: t.layout_before,
            layout_after: t.layout_after,
        }
    }
}

// Enhanced barrier type mapper compiling heterogeneous batches to hardware barrier command packets.
#[derive(Debug)]
pub struct EnhancedBarrierMapper {
    pub mapped_barriers: Vec<UnifiedBarrierDesc>,
    pub global_count: usize,
    pub buffer_count: usize,
    pub texture_count: usize,
}

impl EnhancedBarrierMapper {
    pub fn new() -> Self {
        Self {
            mapped_barriers: Vec::new(),
            global_count: 0,
            buffer_count: 0,
            texture_count: 0,
        }
    }

    // Maps a high-level barrier batch to unified hardware pipeline descriptors.
    pub fn map_batch(&mut self, batch: &BarrierBatch) -> usize {
        let initial_count = self.mapped_barriers.len();

        for g in &batch.global_barriers {
            self.mapped_barriers.push(UnifiedBarrierDesc::from_global(g));
            self.global_count += 1;
        }
        for b in &batch.buffer_barriers {
            self.mapped_barriers.push(UnifiedBarrierDesc::from_buffer(b));
            self.buffer_count += 1;
        }
        for t in &batch.texture_barriers {
            self.mapped_barriers.push(UnifiedBarrierDesc::from_texture(t));
            self.texture_count += 1;
        }

        self.mapped_barriers.len() - initial_count
    }

    #[inline(always)]
    pub fn total_mapped(&self) -> usize {
        self.mapped_barriers.len()
    }
}
