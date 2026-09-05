//! Direct3D 12 Resource Barrier State Tracker & Implicit State Transition Engine.
//! Provides resource state bitmasks, queue validation, and synchronization stage mapping.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use alloc::vec::Vec;
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

// Barrier classification category
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_RESOURCE_BARRIER_TYPE {
    Transition = 0,
    Aliasing = 1,
    Uav = 2,
}

// Split barrier synchronization execution flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct D3D12_RESOURCE_BARRIER_FLAGS(pub u32);

impl D3D12_RESOURCE_BARRIER_FLAGS {
    pub const NONE: Self = Self(0);
    pub const BEGIN_ONLY: Self = Self(1);
    pub const END_ONLY: Self = Self(2);

    // Check if barrier is starting an asynchronous split transition
    pub const fn is_begin_only(&self) -> bool {
        (self.0 & Self::BEGIN_ONLY.0) != 0
    }

    // Check if barrier is concluding an asynchronous split transition
    pub const fn is_end_only(&self) -> bool {
        (self.0 & Self::END_ONLY.0) != 0
    }
}

// Transition barrier payload between GPU resource states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct D3D12_RESOURCE_TRANSITION_BARRIER {
    pub resource_id: u64,
    pub subresource: u32,
    pub state_before: D3D12_RESOURCE_STATES,
    pub state_after: D3D12_RESOURCE_STATES,
}

// Aliasing barrier payload for overlapping placed resources in a heap
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct D3D12_RESOURCE_ALIASING_BARRIER {
    pub resource_before_id: u64,
    pub resource_after_id: u64,
}

// UAV barrier payload ensuring all unordered access operations have finished
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct D3D12_RESOURCE_UAV_BARRIER {
    pub resource_id: u64,
}

// Complete Direct3D 12 resource barrier descriptor
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct D3D12_RESOURCE_BARRIER {
    pub barrier_type: D3D12_RESOURCE_BARRIER_TYPE,
    pub flags: D3D12_RESOURCE_BARRIER_FLAGS,
    pub transition: Option<D3D12_RESOURCE_TRANSITION_BARRIER>,
    pub aliasing: Option<D3D12_RESOURCE_ALIASING_BARRIER>,
    pub uav: Option<D3D12_RESOURCE_UAV_BARRIER>,
}

impl D3D12_RESOURCE_BARRIER {
    // Construct transition barrier between two resource states
    pub fn transition(
        resource_id: u64,
        subresource: u32,
        state_before: D3D12_RESOURCE_STATES,
        state_after: D3D12_RESOURCE_STATES,
    ) -> Self {
        Self {
            barrier_type: D3D12_RESOURCE_BARRIER_TYPE::Transition,
            flags: D3D12_RESOURCE_BARRIER_FLAGS::NONE,
            transition: Some(D3D12_RESOURCE_TRANSITION_BARRIER {
                resource_id,
                subresource,
                state_before,
                state_after,
            }),
            aliasing: None,
            uav: None,
        }
    }

    // Construct aliasing barrier between two placed resources
    pub fn aliasing(resource_before_id: u64, resource_after_id: u64) -> Self {
        Self {
            barrier_type: D3D12_RESOURCE_BARRIER_TYPE::Aliasing,
            flags: D3D12_RESOURCE_BARRIER_FLAGS::NONE,
            transition: None,
            aliasing: Some(D3D12_RESOURCE_ALIASING_BARRIER {
                resource_before_id,
                resource_after_id,
            }),
            uav: None,
        }
    }

    // Construct UAV barrier for unordered access synchronization
    pub fn uav(resource_id: u64) -> Self {
        Self {
            barrier_type: D3D12_RESOURCE_BARRIER_TYPE::Uav,
            flags: D3D12_RESOURCE_BARRIER_FLAGS::NONE,
            transition: None,
            aliasing: None,
            uav: Some(D3D12_RESOURCE_UAV_BARRIER { resource_id }),
        }
    }

    // Apply split barrier synchronization execution flag
    pub fn with_flags(mut self, flags: D3D12_RESOURCE_BARRIER_FLAGS) -> Self {
        self.flags = flags;
        self
    }

    // Validate structural consistency of barrier configuration
    pub fn validate(&self) -> Result<(), ResourceBarrierError> {
        match self.barrier_type {
            D3D12_RESOURCE_BARRIER_TYPE::Transition => {
                let t = self.transition.as_ref().ok_or(ResourceBarrierError::InvalidBarrierType)?;
                if !t.state_before.is_valid_combination() || !t.state_after.is_valid_combination() {
                    return Err(ResourceBarrierError::IncompatibleStates);
                }
                if t.state_before == t.state_after {
                    return Err(ResourceBarrierError::RedundantTransition);
                }
            }
            D3D12_RESOURCE_BARRIER_TYPE::Aliasing => {
                if self.aliasing.is_none() {
                    return Err(ResourceBarrierError::InvalidBarrierType);
                }
            }
            D3D12_RESOURCE_BARRIER_TYPE::Uav => {
                if self.uav.is_none() {
                    return Err(ResourceBarrierError::InvalidBarrierType);
                }
            }
        }
        Ok(())
    }
}

// Errors occurring during barrier validation and state tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceBarrierError {
    InvalidBarrierType,
    IncompatibleStates,
    MissingSubresource,
    InvalidQueueOperation,
    InvalidSplitBarrierPair,
    RedundantTransition,
    ResourceNotFound,
}

// Computes flat subresource index according to standard D3D12 slice arithmetic
pub const fn calc_subresource(
    mip_slice: u32,
    array_slice: u32,
    plane_slice: u32,
    mip_levels: u32,
    array_size: u32,
) -> u32 {
    mip_slice + (array_slice * mip_levels) + (plane_slice * mip_levels * array_size)
}

// Decomposes flat subresource index into mip, array, and plane coordinates
pub const fn decompose_subresource(
    subresource: u32,
    mip_levels: u32,
    array_size: u32,
) -> (u32, u32, u32) {
    if mip_levels == 0 {
        return (0, 0, 0);
    }
    let plane_stride = mip_levels * array_size;
    let plane_slice = if plane_stride > 0 { subresource / plane_stride } else { 0 };
    let remainder = if plane_stride > 0 { subresource % plane_stride } else { subresource };
    let array_slice = remainder / mip_levels;
    let mip_slice = remainder % mip_levels;
    (mip_slice, array_slice, plane_slice)
}

// Per-resource subresource state and queue affinity tracking metadata
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceStateTracking {
    pub resource_id: u64,
    pub is_buffer: bool,
    pub has_simultaneous_access: bool,
    pub mip_levels: u32,
    pub array_size: u32,
    pub plane_count: u32,
    pub subresource_states: Vec<D3D12_RESOURCE_STATES>,
    pub current_queue: D3D12_COMMAND_LIST_TYPE,
    pub is_promoted: bool,
}

impl ResourceStateTracking {
    // Construct new tracking descriptor initialized with uniform state
    pub fn new(
        resource_id: u64,
        is_buffer: bool,
        mip_levels: u32,
        array_size: u32,
        plane_count: u32,
        initial_state: D3D12_RESOURCE_STATES,
        initial_queue: D3D12_COMMAND_LIST_TYPE,
    ) -> Self {
        let total_subresources = if is_buffer {
            1
        } else {
            (mip_levels.max(1) * array_size.max(1) * plane_count.max(1)) as usize
        };
        let mut subresource_states = Vec::with_capacity(total_subresources);
        for _ in 0..total_subresources {
            subresource_states.push(initial_state);
        }
        Self {
            resource_id,
            is_buffer,
            has_simultaneous_access: false,
            mip_levels: mip_levels.max(1),
            array_size: array_size.max(1),
            plane_count: plane_count.max(1),
            subresource_states,
            current_queue: initial_queue,
            is_promoted: false,
        }
    }

    // Number of trackable subresources for this resource
    pub fn subresource_count(&self) -> usize {
        self.subresource_states.len()
    }
}

// Global resource barrier state registry tracking GPU resource states
pub struct ResourceStateRegistry {
    resources: Vec<ResourceStateTracking>,
}

impl ResourceStateRegistry {
    // Construct empty resource state registry
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
        }
    }

    // Register a new GPU resource into the barrier tracking registry
    pub fn register_resource(
        &mut self,
        id: u64,
        is_buffer: bool,
        mip_levels: u32,
        array_size: u32,
        plane_count: u32,
        initial_state: D3D12_RESOURCE_STATES,
        initial_queue: D3D12_COMMAND_LIST_TYPE,
    ) -> Result<(), ResourceBarrierError> {
        if self.resources.iter().any(|r| r.resource_id == id) {
            return Err(ResourceBarrierError::IncompatibleStates);
        }
        self.resources.push(ResourceStateTracking::new(
            id,
            is_buffer,
            mip_levels,
            array_size,
            plane_count,
            initial_state,
            initial_queue,
        ));
        Ok(())
    }

    // Unregister GPU resource from barrier tracker on destruction
    pub fn unregister_resource(&mut self, id: u64) {
        self.resources.retain(|r| r.resource_id != id);
    }

    // Query state of an individual subresource
    pub fn get_subresource_state(
        &self,
        id: u64,
        subresource: u32,
    ) -> Result<D3D12_RESOURCE_STATES, ResourceBarrierError> {
        let res = self
            .resources
            .iter()
            .find(|r| r.resource_id == id)
            .ok_or(ResourceBarrierError::ResourceNotFound)?;

        if subresource == D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES {
            if let Some(first) = res.subresource_states.first() {
                if res.subresource_states.iter().all(|s| s == first) {
                    return Ok(*first);
                }
            }
            return Err(ResourceBarrierError::MissingSubresource);
        }

        let idx = subresource as usize;
        if idx >= res.subresource_states.len() {
            return Err(ResourceBarrierError::MissingSubresource);
        }
        Ok(res.subresource_states[idx])
    }

    // Transition an individual subresource or all subresources
    pub fn transition(
        &mut self,
        id: u64,
        subresource: u32,
        expected_before: D3D12_RESOURCE_STATES,
        state_after: D3D12_RESOURCE_STATES,
        queue: D3D12_COMMAND_LIST_TYPE,
    ) -> Result<(), ResourceBarrierError> {
        if !is_state_supported_on_queue(state_after, queue) {
            return Err(ResourceBarrierError::InvalidQueueOperation);
        }
        if expected_before == state_after {
            return Err(ResourceBarrierError::RedundantTransition);
        }

        let res = self
            .resources
            .iter_mut()
            .find(|r| r.resource_id == id)
            .ok_or(ResourceBarrierError::ResourceNotFound)?;

        res.current_queue = queue;

        if subresource == D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES {
            for state in &mut res.subresource_states {
                if *state != expected_before {
                    return Err(ResourceBarrierError::IncompatibleStates);
                }
                *state = state_after;
            }
        } else {
            let idx = subresource as usize;
            if idx >= res.subresource_states.len() {
                return Err(ResourceBarrierError::MissingSubresource);
            }
            if res.subresource_states[idx] != expected_before {
                return Err(ResourceBarrierError::IncompatibleStates);
            }
            res.subresource_states[idx] = state_after;
        }

        res.is_promoted = false;
        Ok(())
    }

    // Apply explicit barrier directly to tracked state
    pub fn apply_barrier(
        &mut self,
        barrier: &D3D12_RESOURCE_BARRIER,
        queue: D3D12_COMMAND_LIST_TYPE,
    ) -> Result<(), ResourceBarrierError> {
        barrier.validate()?;

        if let Some(t) = &barrier.transition {
            self.transition(
                t.resource_id,
                t.subresource,
                t.state_before,
                t.state_after,
                queue,
            )?;
        }
        Ok(())
    }

    // Total count of registered resources in tracker
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    // Set simultaneous access flag on texture resource
    pub fn set_simultaneous_access(&mut self, id: u64, enabled: bool) -> Result<(), ResourceBarrierError> {
        let res = self
            .resources
            .iter_mut()
            .find(|r| r.resource_id == id)
            .ok_or(ResourceBarrierError::ResourceNotFound)?;
        res.has_simultaneous_access = enabled;
        Ok(())
    }

    // Check if implicit state promotion is permitted from COMMON state
    pub fn can_implicitly_promote(
        is_buffer: bool,
        has_simultaneous_access: bool,
        target_state: D3D12_RESOURCE_STATES,
        queue: D3D12_COMMAND_LIST_TYPE,
    ) -> bool {
        if !is_state_supported_on_queue(target_state, queue) {
            return false;
        }
        if is_buffer {
            return true;
        }

        let standard_promo = D3D12_RESOURCE_STATES::COPY_DEST
            | D3D12_RESOURCE_STATES::COPY_SOURCE
            | D3D12_RESOURCE_STATES::RESOLVE_DEST
            | D3D12_RESOURCE_STATES::RESOLVE_SOURCE;

        if (target_state & !standard_promo) == D3D12_RESOURCE_STATES::COMMON {
            return true;
        }

        if has_simultaneous_access && target_state.is_read_only() {
            return true;
        }

        false
    }

    // Attempt implicit state promotion on resource first access
    pub fn try_implicit_promotion(
        &mut self,
        id: u64,
        subresource: u32,
        target_state: D3D12_RESOURCE_STATES,
        queue: D3D12_COMMAND_LIST_TYPE,
    ) -> Result<bool, ResourceBarrierError> {
        let res = self
            .resources
            .iter_mut()
            .find(|r| r.resource_id == id)
            .ok_or(ResourceBarrierError::ResourceNotFound)?;

        let current_state = if subresource == D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES {
            if let Some(first) = res.subresource_states.first() {
                if res.subresource_states.iter().all(|s| s == first) {
                    *first
                } else {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        } else {
            let idx = subresource as usize;
            if idx >= res.subresource_states.len() {
                return Err(ResourceBarrierError::MissingSubresource);
            }
            res.subresource_states[idx]
        };

        if current_state != D3D12_RESOURCE_STATES::COMMON {
            return Ok(false);
        }

        if !Self::can_implicitly_promote(res.is_buffer, res.has_simultaneous_access, target_state, queue) {
            return Ok(false);
        }

        res.current_queue = queue;
        res.is_promoted = true;

        if subresource == D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES {
            for state in &mut res.subresource_states {
                *state = target_state;
            }
        } else {
            res.subresource_states[subresource as usize] = target_state;
        }

        Ok(true)
    }

    // Executes D3D12 command list completion state decay back to COMMON
    pub fn execute_command_list_decay(&mut self, queue: D3D12_COMMAND_LIST_TYPE) {
        for res in &mut self.resources {
            if res.current_queue != queue {
                continue;
            }
            if res.is_buffer || res.is_promoted || res.has_simultaneous_access {
                for state in &mut res.subresource_states {
                    *state = D3D12_RESOURCE_STATES::COMMON;
                }
                res.is_promoted = false;
            }
        }
    }

    // Ensures resource is in required state, promoting implicitly or returning barrier
    pub fn ensure_state(
        &mut self,
        id: u64,
        subresource: u32,
        required_state: D3D12_RESOURCE_STATES,
        queue: D3D12_COMMAND_LIST_TYPE,
    ) -> Result<Option<D3D12_RESOURCE_BARRIER>, ResourceBarrierError> {
        let current_state = self.get_subresource_state(id, subresource)?;
        if current_state == required_state {
            return Ok(None);
        }

        if current_state == D3D12_RESOURCE_STATES::COMMON {
            if self.try_implicit_promotion(id, subresource, required_state, queue)? {
                return Ok(None);
            }
        }

        let barrier = D3D12_RESOURCE_BARRIER::transition(id, subresource, current_state, required_state);
        self.transition(id, subresource, current_state, required_state, queue)?;
        Ok(Some(barrier))
    }
}

// Active pending split barrier record
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingSplitBarrier {
    pub resource_id: u64,
    pub subresource: u32,
    pub state_before: D3D12_RESOURCE_STATES,
    pub state_after: D3D12_RESOURCE_STATES,
}

// Split barrier pairing validator tracking BeginBarrier and EndBarrier cycles
pub struct SplitBarrierTracker {
    pending_splits: Vec<PendingSplitBarrier>,
}

impl SplitBarrierTracker {
    // Construct new empty split barrier tracker
    pub fn new() -> Self {
        Self {
            pending_splits: Vec::new(),
        }
    }

    // Register a BeginBarrier transition
    pub fn begin_split_barrier(&mut self, barrier: &D3D12_RESOURCE_BARRIER) -> Result<(), ResourceBarrierError> {
        let t = barrier.transition.as_ref().ok_or(ResourceBarrierError::InvalidBarrierType)?;
        if !barrier.flags.is_begin_only() {
            return Err(ResourceBarrierError::InvalidSplitBarrierPair);
        }

        if self.pending_splits.iter().any(|p| p.resource_id == t.resource_id && p.subresource == t.subresource) {
            return Err(ResourceBarrierError::InvalidSplitBarrierPair);
        }

        self.pending_splits.push(PendingSplitBarrier {
            resource_id: t.resource_id,
            subresource: t.subresource,
            state_before: t.state_before,
            state_after: t.state_after,
        });
        Ok(())
    }

    // Match and complete an EndBarrier transition
    pub fn end_split_barrier(&mut self, barrier: &D3D12_RESOURCE_BARRIER) -> Result<(), ResourceBarrierError> {
        let t = barrier.transition.as_ref().ok_or(ResourceBarrierError::InvalidBarrierType)?;
        if !barrier.flags.is_end_only() {
            return Err(ResourceBarrierError::InvalidSplitBarrierPair);
        }

        let idx = self
            .pending_splits
            .iter()
            .position(|p| {
                p.resource_id == t.resource_id
                    && p.subresource == t.subresource
                    && p.state_before == t.state_before
                    && p.state_after == t.state_after
            })
            .ok_or(ResourceBarrierError::InvalidSplitBarrierPair)?;

        self.pending_splits.swap_remove(idx);
        Ok(())
    }

    // Check if subresource currently has an active pending split transition
    pub fn has_pending_split(&self, resource_id: u64, subresource: u32) -> bool {
        self.pending_splits.iter().any(|p| p.resource_id == resource_id && p.subresource == subresource)
    }

    // Number of uncompleted split barriers
    pub fn pending_count(&self) -> usize {
        self.pending_splits.len()
    }
}

// Redundant barrier removal and transition coalescing optimizer
pub struct BarrierBatchOptimizer;

impl BarrierBatchOptimizer {
    // Coalesce sequential transitions and strip no-op barriers
    pub fn coalesce_barriers(barriers: &[D3D12_RESOURCE_BARRIER]) -> Vec<D3D12_RESOURCE_BARRIER> {
        let mut coalesced: Vec<D3D12_RESOURCE_BARRIER> = Vec::new();

        for barrier in barriers {
            if barrier.barrier_type != D3D12_RESOURCE_BARRIER_TYPE::Transition {
                coalesced.push(barrier.clone());
                continue;
            }

            let t = match &barrier.transition {
                Some(t) => t,
                None => continue,
            };

            // Eliminate no-op transitions
            if t.state_before == t.state_after {
                continue;
            }

            // Look for existing transition in the current batch on same resource and subresource
            if let Some(pos) = coalesced.iter().position(|b| {
                if let Some(prev) = &b.transition {
                    b.flags == barrier.flags
                        && prev.resource_id == t.resource_id
                        && prev.subresource == t.subresource
                } else {
                    false
                }
            }) {
                let prev_t = coalesced[pos].transition.as_ref().unwrap();
                if prev_t.state_after == t.state_before {
                    let initial_before = prev_t.state_before;
                    let final_after = t.state_after;

                    if initial_before == final_after {
                        // Cycles back to initial state -> completely cancel out
                        coalesced.remove(pos);
                    } else {
                        // Coalesce into initial_before -> final_after
                        coalesced[pos] = D3D12_RESOURCE_BARRIER::transition(
                            t.resource_id,
                            t.subresource,
                            initial_before,
                            final_after,
                        ).with_flags(barrier.flags);
                    }
                } else {
                    coalesced.push(barrier.clone());
                }
            } else {
                coalesced.push(barrier.clone());
            }
        }

        coalesced
    }
}

// Comprehensive verification self-test suite for D3D12 Resource Barrier State Tracker
pub fn test_resource_barrier_subsystem() -> bool {
    crate::serial_write_str("[KNT-GFX] Testing Direct3D 12 Resource Barrier State Tracker & Implicit Transitions...\r\n");

    // 1. Verify resource state bitmasks, access classifications, and stage mapping
    let rt_state = D3D12_RESOURCE_STATES::RENDER_TARGET;
    let srv_state = D3D12_RESOURCE_STATES::PIXEL_SHADER_RESOURCE;
    let mixed_valid = rt_state | D3D12_RESOURCE_STATES::COPY_SOURCE;
    let mixed_invalid = rt_state | D3D12_RESOURCE_STATES::DEPTH_WRITE;

    if !rt_state.is_write_state() || rt_state.is_read_only() {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Render target must be classified as write state\r\n");
        return false;
    }
    if srv_state.is_write_state() || !srv_state.is_read_only() {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Pixel shader resource must be classified as read-only\r\n");
        return false;
    }
    if !mixed_valid.is_valid_combination() || mixed_invalid.is_valid_combination() {
        crate::serial_write_str("[KNT-BARRIER-FAIL] State combination validation logic mismatch\r\n");
        return false;
    }

    let stages = get_pipeline_stages_for_state(rt_state);
    if !stages.contains(D3D12_PIPELINE_STAGE_FLAGS::RENDER_TARGET) {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Pipeline stage mapping mismatch\r\n");
        return false;
    }
    if !is_state_supported_on_queue(D3D12_RESOURCE_STATES::COPY_DEST, D3D12_COMMAND_LIST_TYPE::Copy)
        || is_state_supported_on_queue(D3D12_RESOURCE_STATES::RENDER_TARGET, D3D12_COMMAND_LIST_TYPE::Copy)
    {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Queue capability check mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Resource state bitmask operations and pipeline stage mapping verified\r\n");

    // 2. Verify subresource slice indexing and reverse decomposition
    let flat_idx = calc_subresource(2, 3, 0, 4, 6);
    let (mip, array, plane) = decompose_subresource(flat_idx, 4, 6);
    if mip != 2 || array != 3 || plane != 0 || flat_idx != (2 + 3 * 4) {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Subresource slice arithmetic mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Subresource coordinate arithmetic and slice decomposition verified\r\n");

    // 3. Verify registry tracking and explicit state transitions
    let mut registry = ResourceStateRegistry::new();
    let buffer_id = 101;
    let texture_id = 202;

    if registry.register_resource(buffer_id, true, 1, 1, 1, D3D12_RESOURCE_STATES::COMMON, D3D12_COMMAND_LIST_TYPE::Direct).is_err() {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Failed to register buffer resource\r\n");
        return false;
    }
    if registry.register_resource(texture_id, false, 4, 1, 1, D3D12_RESOURCE_STATES::COMMON, D3D12_COMMAND_LIST_TYPE::Direct).is_err() {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Failed to register texture resource\r\n");
        return false;
    }

    let b_barrier = D3D12_RESOURCE_BARRIER::transition(
        texture_id,
        D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
        D3D12_RESOURCE_STATES::COMMON,
        D3D12_RESOURCE_STATES::RENDER_TARGET,
    );
    if registry.apply_barrier(&b_barrier, D3D12_COMMAND_LIST_TYPE::Direct).is_err() {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Failed to apply ALL_SUBRESOURCES transition\r\n");
        return false;
    }
    if registry.get_subresource_state(texture_id, 2) != Ok(D3D12_RESOURCE_STATES::RENDER_TARGET) {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Transition state mismatch on subresource 2\r\n");
        return false;
    }
    crate::serial_write_str("  * Per-subresource state registry and explicit transitions verified\r\n");

    // 4. Verify implicit state promotion rules and decay lifecycle
    let dyn_buffer_id = 303;
    let _ = registry.register_resource(dyn_buffer_id, true, 1, 1, 1, D3D12_RESOURCE_STATES::COMMON, D3D12_COMMAND_LIST_TYPE::Direct);

    // Buffer in COMMON should implicitly promote to VERTEX_AND_CONSTANT_BUFFER without barrier
    let promo_result = match registry.ensure_state(dyn_buffer_id, 0, D3D12_RESOURCE_STATES::VERTEX_AND_CONSTANT_BUFFER, D3D12_COMMAND_LIST_TYPE::Direct) {
        Ok(res) => res,
        Err(_) => return false,
    };
    if promo_result.is_some() {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Buffer promotion must not emit explicit barrier\r\n");
        return false;
    }
    if registry.get_subresource_state(dyn_buffer_id, 0) != Ok(D3D12_RESOURCE_STATES::VERTEX_AND_CONSTANT_BUFFER) {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Promoted buffer state mismatch\r\n");
        return false;
    }

    // ExecuteCommandLists completion should decay promoted buffer back to COMMON
    registry.execute_command_list_decay(D3D12_COMMAND_LIST_TYPE::Direct);
    if registry.get_subresource_state(dyn_buffer_id, 0) != Ok(D3D12_RESOURCE_STATES::COMMON) {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Buffer must decay to COMMON on command list completion\r\n");
        return false;
    }
    // Explicitly transitioned texture without simultaneous access must retain its state
    if registry.get_subresource_state(texture_id, 0) != Ok(D3D12_RESOURCE_STATES::RENDER_TARGET) {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Explicitly transitioned texture must retain state\r\n");
        return false;
    }
    crate::serial_write_str("  * Implicit state promotion and ExecuteCommandLists decay lifecycle verified\r\n");

    // 5. Verify SplitBarrierTracker pairing and BarrierBatchOptimizer coalescing
    let mut split_tracker = SplitBarrierTracker::new();
    let split_begin = D3D12_RESOURCE_BARRIER::transition(
        texture_id,
        0,
        D3D12_RESOURCE_STATES::RENDER_TARGET,
        D3D12_RESOURCE_STATES::PIXEL_SHADER_RESOURCE,
    ).with_flags(D3D12_RESOURCE_BARRIER_FLAGS::BEGIN_ONLY);

    let split_end = D3D12_RESOURCE_BARRIER::transition(
        texture_id,
        0,
        D3D12_RESOURCE_STATES::RENDER_TARGET,
        D3D12_RESOURCE_STATES::PIXEL_SHADER_RESOURCE,
    ).with_flags(D3D12_RESOURCE_BARRIER_FLAGS::END_ONLY);

    if split_tracker.begin_split_barrier(&split_begin).is_err() {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Failed to register BeginBarrier\r\n");
        return false;
    }
    if !split_tracker.has_pending_split(texture_id, 0) {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Pending split barrier status mismatch\r\n");
        return false;
    }
    if split_tracker.begin_split_barrier(&split_begin).is_ok() {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Duplicate BeginBarrier must be rejected\r\n");
        return false;
    }
    if split_tracker.end_split_barrier(&split_end).is_err() {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Failed to complete matching EndBarrier\r\n");
        return false;
    }
    if split_tracker.pending_count() != 0 {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Split tracker must have zero pending barriers\r\n");
        return false;
    }

    // Coalescing test: COMMON -> COPY_DEST then COPY_DEST -> PIXEL_SHADER_RESOURCE should coalesce to COMMON -> PIXEL_SHADER_RESOURCE
    let unoptimized = [
        D3D12_RESOURCE_BARRIER::transition(404, 0, D3D12_RESOURCE_STATES::COMMON, D3D12_RESOURCE_STATES::COPY_DEST),
        D3D12_RESOURCE_BARRIER::transition(404, 0, D3D12_RESOURCE_STATES::COPY_DEST, D3D12_RESOURCE_STATES::PIXEL_SHADER_RESOURCE),
        D3D12_RESOURCE_BARRIER::transition(505, 0, D3D12_RESOURCE_STATES::RENDER_TARGET, D3D12_RESOURCE_STATES::RENDER_TARGET),
    ];
    let optimized = BarrierBatchOptimizer::coalesce_barriers(&unoptimized);
    if optimized.len() != 1 {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Coalesced barrier count mismatch\r\n");
        return false;
    }
    let opt_t = optimized[0].transition.as_ref().unwrap();
    if opt_t.state_before != D3D12_RESOURCE_STATES::COMMON || opt_t.state_after != D3D12_RESOURCE_STATES::PIXEL_SHADER_RESOURCE {
        crate::serial_write_str("[KNT-BARRIER-FAIL] Coalesced state endpoints mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Split barrier pairing and redundant transition coalescing verified\r\n");

    crate::serial_write_str("[KNT-BARRIER-TRACK-PASS] Direct3D 12 resource barrier state tracker and implicit transition engine verified\r\n");
    true
}
