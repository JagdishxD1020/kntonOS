//! Direct3D 12 Resource Barrier State Tracker & Implicit State Transition Engine.
//! Provides resource state bitmasks, queue validation, and synchronization stage mapping.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not};

// Direct3D 12 standard subresource index sentinel for all subresources
pub const D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES: u32 = 0xFFFF_FFFF;

// Direct3D 12 resource state bitmask flags representing GPU access patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct D3D12_RESOURCE_STATES(pub u32);

impl D3D12_RESOURCE_STATES {
    pub const COMMON: Self = Self(0);
    pub const VERTEX_AND_CONSTANT_BUFFER: Self = Self(0x1);
    pub const INDEX_BUFFER: Self = Self(0x2);
    pub const RENDER_TARGET: Self = Self(0x4);
    pub const UNORDERED_ACCESS: Self = Self(0x8);
    pub const DEPTH_WRITE: Self = Self(0x10);
    pub const DEPTH_READ: Self = Self(0x20);
    pub const NON_PIXEL_SHADER_RESOURCE: Self = Self(0x40);
    pub const PIXEL_SHADER_RESOURCE: Self = Self(0x80);
    pub const STREAM_OUT: Self = Self(0x100);
    pub const INDIRECT_ARGUMENT: Self = Self(0x200);
    pub const COPY_DEST: Self = Self(0x400);
    pub const COPY_SOURCE: Self = Self(0x800);
    pub const RESOLVE_DEST: Self = Self(0x1000);
    pub const RESOLVE_SOURCE: Self = Self(0x2000);
    pub const RAYTRACING_ACCELERATION_STRUCTURE: Self = Self(0x40_0000);
    pub const SHADING_RATE_SOURCE: Self = Self(0x100_0000);
    pub const PRESENT: Self = Self(0);

    // Combined multi-stage shader read bitmask
    pub const ALL_SHADER_RESOURCE: Self = Self(0x40 | 0x80);

    // Generic read state combination bitmask across all graphics read stages
    pub const GENERIC_READ: Self = Self(0x1 | 0x2 | 0x40 | 0x80 | 0x200 | 0x800);

    // Extract underlying primitive bitmask value
    pub const fn bits(&self) -> u32 {
        self.0
    }

    // Check if bitmask contains specific resource state flag
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    // Determine if state is exclusively read-only without modifying memory
    pub const fn is_read_only(&self) -> bool {
        let write_mask = Self::RENDER_TARGET.0
            | Self::UNORDERED_ACCESS.0
            | Self::DEPTH_WRITE.0
            | Self::STREAM_OUT.0
            | Self::COPY_DEST.0
            | Self::RESOLVE_DEST.0;
        (self.0 & write_mask) == 0 && self.0 != 0
    }

    // Determine if state performs GPU writes requiring hazard synchronization
    pub const fn is_write_state(&self) -> bool {
        let write_mask = Self::RENDER_TARGET.0
            | Self::UNORDERED_ACCESS.0
            | Self::DEPTH_WRITE.0
            | Self::STREAM_OUT.0
            | Self::COPY_DEST.0
            | Self::RESOLVE_DEST.0;
        (self.0 & write_mask) != 0
    }

    // Validate that write states are mutually exclusive and combinations are legal
    pub fn is_valid_combination(&self) -> bool {
        if self.0 == 0 {
            return true;
        }
        let write_bits = (self.0 & Self::RENDER_TARGET.0 != 0) as u32
            + (self.0 & Self::UNORDERED_ACCESS.0 != 0) as u32
            + (self.0 & Self::DEPTH_WRITE.0 != 0) as u32
            + (self.0 & Self::COPY_DEST.0 != 0) as u32
            + (self.0 & Self::RESOLVE_DEST.0 != 0) as u32;

        if write_bits > 1 {
            return false;
        }
        if self.contains(Self::DEPTH_WRITE) && self.contains(Self::DEPTH_READ) {
            return false;
        }
        true
    }
}

impl BitOr for D3D12_RESOURCE_STATES {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for D3D12_RESOURCE_STATES {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for D3D12_RESOURCE_STATES {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for D3D12_RESOURCE_STATES {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl Not for D3D12_RESOURCE_STATES {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

// Command queue execution engine type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_COMMAND_LIST_TYPE {
    Direct = 0,
    Bundle = 1,
    Compute = 2,
    Copy = 3,
    VideoDecode = 4,
    VideoProcess = 5,
}

// Pipeline synchronization stage bitflags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct D3D12_PIPELINE_STAGE_FLAGS(pub u32);

impl D3D12_PIPELINE_STAGE_FLAGS {
    pub const TOP_OF_PIPE: Self = Self(1 << 0);
    pub const DRAW_INDIRECT: Self = Self(1 << 1);
    pub const VERTEX_INPUT: Self = Self(1 << 2);
    pub const VERTEX_SHADER: Self = Self(1 << 3);
    pub const PIXEL_SHADER: Self = Self(1 << 4);
    pub const RENDER_TARGET: Self = Self(1 << 5);
    pub const DEPTH_STENCIL: Self = Self(1 << 6);
    pub const COMPUTE_SHADER: Self = Self(1 << 7);
    pub const TRANSFER_COPY: Self = Self(1 << 8);
    pub const RESOLVE: Self = Self(1 << 9);
    pub const RAYTRACING: Self = Self(1 << 10);
    pub const BOTTOM_OF_PIPE: Self = Self(1 << 11);
    pub const ALL_COMMANDS: Self = Self(0xFFFF);

    // Combine stage flags
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    // Check if stage is present
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

// Checks if resource state is supported by hardware command queue type
pub fn is_state_supported_on_queue(
    state: D3D12_RESOURCE_STATES,
    queue_type: D3D12_COMMAND_LIST_TYPE,
) -> bool {
    if state == D3D12_RESOURCE_STATES::COMMON {
        return true;
    }

    match queue_type {
        D3D12_COMMAND_LIST_TYPE::Copy => {
            let supported = D3D12_RESOURCE_STATES::COPY_DEST | D3D12_RESOURCE_STATES::COPY_SOURCE;
            (state & !supported) == D3D12_RESOURCE_STATES::COMMON
        }
        D3D12_COMMAND_LIST_TYPE::Compute => {
            let supported = D3D12_RESOURCE_STATES::UNORDERED_ACCESS
                | D3D12_RESOURCE_STATES::NON_PIXEL_SHADER_RESOURCE
                | D3D12_RESOURCE_STATES::COPY_DEST
                | D3D12_RESOURCE_STATES::COPY_SOURCE
                | D3D12_RESOURCE_STATES::INDIRECT_ARGUMENT
                | D3D12_RESOURCE_STATES::RAYTRACING_ACCELERATION_STRUCTURE;
            (state & !supported) == D3D12_RESOURCE_STATES::COMMON
        }
        D3D12_COMMAND_LIST_TYPE::Direct | D3D12_COMMAND_LIST_TYPE::Bundle => state.is_valid_combination(),
        _ => false,
    }
}

// Maps D3D12 resource state bitmask to corresponding pipeline execution stages
pub fn get_pipeline_stages_for_state(state: D3D12_RESOURCE_STATES) -> D3D12_PIPELINE_STAGE_FLAGS {
    if state == D3D12_RESOURCE_STATES::COMMON {
        return D3D12_PIPELINE_STAGE_FLAGS::TOP_OF_PIPE;
    }

    let mut stages = D3D12_PIPELINE_STAGE_FLAGS(0);

    if state.contains(D3D12_RESOURCE_STATES::VERTEX_AND_CONSTANT_BUFFER) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::VERTEX_INPUT);
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::VERTEX_SHADER);
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::PIXEL_SHADER);
    }
    if state.contains(D3D12_RESOURCE_STATES::INDEX_BUFFER) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::VERTEX_INPUT);
    }
    if state.contains(D3D12_RESOURCE_STATES::RENDER_TARGET) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::RENDER_TARGET);
    }
    if state.contains(D3D12_RESOURCE_STATES::UNORDERED_ACCESS) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::COMPUTE_SHADER);
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::PIXEL_SHADER);
    }
    if state.contains(D3D12_RESOURCE_STATES::DEPTH_WRITE) || state.contains(D3D12_RESOURCE_STATES::DEPTH_READ) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::DEPTH_STENCIL);
    }
    if state.contains(D3D12_RESOURCE_STATES::NON_PIXEL_SHADER_RESOURCE) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::VERTEX_SHADER);
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::COMPUTE_SHADER);
    }
    if state.contains(D3D12_RESOURCE_STATES::PIXEL_SHADER_RESOURCE) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::PIXEL_SHADER);
    }
    if state.contains(D3D12_RESOURCE_STATES::INDIRECT_ARGUMENT) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::DRAW_INDIRECT);
    }
    if state.contains(D3D12_RESOURCE_STATES::COPY_DEST) || state.contains(D3D12_RESOURCE_STATES::COPY_SOURCE) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::TRANSFER_COPY);
    }
    if state.contains(D3D12_RESOURCE_STATES::RESOLVE_DEST) || state.contains(D3D12_RESOURCE_STATES::RESOLVE_SOURCE) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::RESOLVE);
    }
    if state.contains(D3D12_RESOURCE_STATES::RAYTRACING_ACCELERATION_STRUCTURE) {
        stages = stages.union(D3D12_PIPELINE_STAGE_FLAGS::RAYTRACING);
    }

    stages
}
