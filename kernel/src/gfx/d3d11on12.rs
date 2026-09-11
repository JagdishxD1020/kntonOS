//! Direct3D 11-on-12 (D3D11On12) Compatibility Translation Layer.
//! Wraps D3D12 devices and command queues to provide native D3D11 interfaces.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use alloc::vec::Vec;
use crate::gfx::resource_barrier::D3D12_RESOURCE_STATES;

// Errors produced during D3D11On12 translation operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum D3D11On12Error {
    InvalidDevice,
    InvalidQueue,
    InvalidResource,
    InvalidState,
    UnsupportedFeatureLevel,
    ResourceNotAcquired,
    ResourceAlreadyAcquired,
    FenceSyncError,
    BufferTooSmall,
}

// Direct3D 11 hardware feature levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D11_FEATURE_LEVEL {
    LEVEL_9_1 = 0x9100,
    LEVEL_9_2 = 0x9200,
    LEVEL_9_3 = 0x9300,
    LEVEL_10_0 = 0xa000,
    LEVEL_10_1 = 0xa100,
    LEVEL_11_0 = 0xb000,
    LEVEL_11_1 = 0xb100,
    LEVEL_12_0 = 0xc000,
    LEVEL_12_1 = 0xc100,
}

// Direct3D 11 core device interface
#[derive(Debug, Clone)]
pub struct ID3D11Device {
    pub feature_level: D3D11_FEATURE_LEVEL,
    pub creation_flags: u32,
    pub d3d12_device: u64,
}

impl ID3D11Device {
    // Get negotiated feature level
    pub fn get_feature_level(&self) -> D3D11_FEATURE_LEVEL {
        self.feature_level
    }

    // Get underlying D3D12 device GPU pointer
    pub fn get_d3d12_device(&self) -> u64 {
        self.d3d12_device
    }
}

// Direct3D 11 immediate device context interface
#[derive(Debug, Clone)]
pub struct ID3D11DeviceContext {
    pub d3d12_device: u64,
    pub default_command_queue: u64,
    pub bound_render_targets: Vec<u64>,
}

impl ID3D11DeviceContext {
    // Create new device context
    pub fn new(d3d12_device: u64, default_queue: u64) -> Self {
        Self {
            d3d12_device,
            default_command_queue: default_queue,
            bound_render_targets: Vec::new(),
        }
    }

    // Clear render target view simulation
    pub fn clear_render_target_view(&mut self, rtv_handle: u64, _color: [f32; 4]) {
        if !self.bound_render_targets.contains(&rtv_handle) {
            self.bound_render_targets.push(rtv_handle);
        }
    }
}


// Direct3D 11 resource binding capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct D3D11_BIND_FLAG(pub u32);

impl D3D11_BIND_FLAG {
    pub const VERTEX_BUFFER: Self = Self(0x1);
    pub const INDEX_BUFFER: Self = Self(0x2);
    pub const CONSTANT_BUFFER: Self = Self(0x4);
    pub const SHADER_RESOURCE: Self = Self(0x8);
    pub const STREAM_OUTPUT: Self = Self(0x10);
    pub const RENDER_TARGET: Self = Self(0x20);
    pub const DEPTH_STENCIL: Self = Self(0x40);
    pub const UNORDERED_ACCESS: Self = Self(0x80);

    // Check if a flag is active in bitmask
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

// Resource dimensionality in D3D11
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D11_RESOURCE_DIMENSION {
    UNKNOWN = 0,
    BUFFER = 1,
    TEXTURE1D = 2,
    TEXTURE2D = 3,
    TEXTURE3D = 4,
}

// Description of D3D11 binding and access properties for a wrapped D3D12 resource
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D11_RESOURCE_FLAGS {
    pub BindFlags: u32,
    pub MiscFlags: u32,
    pub CPUAccessFlags: u32,
    pub StructureByteStride: u32,
}

// Interop descriptor binding a D3D12 resource handle to D3D11 usage
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3D11WrappedResource {
    pub resource_id: u64,
    pub d3d12_resource: u64,
    pub flags11: D3D11_RESOURCE_FLAGS,
    pub dimension: D3D11_RESOURCE_DIMENSION,
    pub in_state: D3D12_RESOURCE_STATES,
    pub out_state: D3D12_RESOURCE_STATES,
    pub current_state: D3D12_RESOURCE_STATES,
    pub is_acquired: bool,
}


// Record of an explicit or implicit D3D12 resource barrier transition generated during interop
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D11On12TransitionBarrier {
    pub resource_id: u64,
    pub d3d12_resource: u64,
    pub state_before: D3D12_RESOURCE_STATES,
    pub state_after: D3D12_RESOURCE_STATES,
    pub subresource: u32,
}

impl D3D11On12TransitionBarrier {
    // Construct new interop transition barrier
    pub const fn new(
        resource_id: u64,
        d3d12_resource: u64,
        state_before: D3D12_RESOURCE_STATES,
        state_after: D3D12_RESOURCE_STATES,
    ) -> Self {
        Self {
            resource_id,
            d3d12_resource,
            state_before,
            state_after,
            subresource: 0xFFFF_FFFF,
        }
    }
}

// Interop state tracker recording transitions and maintaining barrier history
#[derive(Debug, Clone, Default)]
pub struct D3D11On12StateTracker {
    pub pending_barriers: Vec<D3D11On12TransitionBarrier>,
    pub total_transitions_count: u64,
}

impl D3D11On12StateTracker {
    // Construct new state tracker
    pub fn new() -> Self {
        Self::default()
    }

    // Transition a wrapped resource to target state and record barrier if state changed
    pub fn transition_resource(
        &mut self,
        resource: &mut D3D11WrappedResource,
        target_state: D3D12_RESOURCE_STATES,
    ) -> Option<D3D11On12TransitionBarrier> {
        if resource.current_state == target_state {
            return None;
        }

        let barrier = D3D11On12TransitionBarrier::new(
            resource.resource_id,
            resource.d3d12_resource,
            resource.current_state,
            target_state,
        );
        resource.current_state = target_state;
        self.pending_barriers.push(barrier);
        self.total_transitions_count += 1;
        Some(barrier)
    }

    // Drain and return all pending transition barriers
    pub fn drain_pending_barriers(&mut self) -> Vec<D3D11On12TransitionBarrier> {
        core::mem::take(&mut self.pending_barriers)
    }
}


// Timeline fence synchronization tracker for CPU/GPU interop between D3D11 and D3D12
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D11On12FenceSync {
    pub fence_handle: u64,
    pub current_value: u64,
    pub completed_value: u64,
}

impl D3D11On12FenceSync {
    // Construct new fence synchronizer
    pub fn new(fence_handle: u64) -> Self {
        Self {
            fence_handle,
            current_value: 0,
            completed_value: 0,
        }
    }

    // Advance and signal fence returning the new monotonic value
    pub fn signal(&mut self) -> u64 {
        self.current_value += 1;
        self.current_value
    }

    // Simulate completion of GPU work up to value
    pub fn mark_completed(&mut self, value: u64) {
        if value > self.completed_value {
            self.completed_value = value;
        }
    }

    // Check if fence value has been completed by GPU
    pub fn is_completed(&self, value: u64) -> bool {
        self.completed_value >= value
    }
}

// D3D11-on-12 interop device interface
#[derive(Debug, Clone)]
pub struct ID3D11On12Device {
    pub d3d12_device: u64,
    pub command_queues: Vec<u64>,
    pub next_resource_id: u64,
    pub wrapped_resources: Vec<D3D11WrappedResource>,
    pub state_tracker: D3D11On12StateTracker,
    pub fence_sync: D3D11On12FenceSync,
}

impl ID3D11On12Device {
    // Construct new D3D11On12 interop device
    pub fn new(d3d12_device: u64, queues: &[u64]) -> Self {
        Self {
            d3d12_device,
            command_queues: queues.to_vec(),
            next_resource_id: 1,
            wrapped_resources: Vec::new(),
            state_tracker: D3D11On12StateTracker::new(),
            fence_sync: D3D11On12FenceSync::new(0x5000),
        }
    }

    // Allocate unique resource identifier
    pub fn allocate_resource_id(&mut self) -> u64 {
        let id = self.next_resource_id;
        self.next_resource_id += 1;
        id
    }

    // Create a wrapped D3D11 resource from an underlying D3D12 resource handle
    pub fn CreateWrappedResource(
        &mut self,
        d3d12_resource: u64,
        flags11: &D3D11_RESOURCE_FLAGS,
        in_state: D3D12_RESOURCE_STATES,
        out_state: D3D12_RESOURCE_STATES,
    ) -> Result<u64, D3D11On12Error> {
        if d3d12_resource == 0 {
            return Err(D3D11On12Error::InvalidResource);
        }

        let dimension = if (flags11.BindFlags & D3D11_BIND_FLAG::RENDER_TARGET.0) != 0
            || (flags11.BindFlags & D3D11_BIND_FLAG::DEPTH_STENCIL.0) != 0
        {
            D3D11_RESOURCE_DIMENSION::TEXTURE2D
        } else if (flags11.BindFlags & (D3D11_BIND_FLAG::VERTEX_BUFFER.0 | D3D11_BIND_FLAG::INDEX_BUFFER.0 | D3D11_BIND_FLAG::CONSTANT_BUFFER.0)) != 0 {
            D3D11_RESOURCE_DIMENSION::BUFFER
        } else {
            D3D11_RESOURCE_DIMENSION::TEXTURE2D
        };

        let resource_id = self.allocate_resource_id();
        let wrapped = D3D11WrappedResource {
            resource_id,
            d3d12_resource,
            flags11: *flags11,
            dimension,
            in_state,
            out_state,
            current_state: out_state,
            is_acquired: false,
        };
        self.wrapped_resources.push(wrapped);
        Ok(resource_id)
    }

    // Lookup wrapped resource by resource identifier
    pub fn get_wrapped_resource(&self, resource_id: u64) -> Result<&D3D11WrappedResource, D3D11On12Error> {
        self.wrapped_resources
            .iter()
            .find(|r| r.resource_id == resource_id)
            .ok_or(D3D11On12Error::InvalidResource)
    }

    // Lookup wrapped resource mutably by resource identifier
    pub fn get_wrapped_resource_mut(&mut self, resource_id: u64) -> Result<&mut D3D11WrappedResource, D3D11On12Error> {
        self.wrapped_resources
            .iter_mut()
            .find(|r| r.resource_id == resource_id)
            .ok_or(D3D11On12Error::InvalidResource)
    }

    // Transition a wrapped resource to target state and record transition barrier
    pub fn transition_wrapped_resource(
        &mut self,
        resource_id: u64,
        target_state: D3D12_RESOURCE_STATES,
    ) -> Result<Option<D3D11On12TransitionBarrier>, D3D11On12Error> {
        let res = self
            .wrapped_resources
            .iter_mut()
            .find(|r| r.resource_id == resource_id)
            .ok_or(D3D11On12Error::InvalidResource)?;

        if !res.is_acquired {
            return Err(D3D11On12Error::ResourceNotAcquired);
        }

        Ok(self.state_tracker.transition_resource(res, target_state))
    }

    // Acquire wrapped resources transitioning them to in_state for D3D11 use
    pub fn AcquireWrappedResources(&mut self, resource_ids: &[u64]) -> Result<(), D3D11On12Error> {
        if resource_ids.is_empty() {
            return Ok(());
        }

        for &id in resource_ids {
            let res = self
                .wrapped_resources
                .iter()
                .find(|r| r.resource_id == id)
                .ok_or(D3D11On12Error::InvalidResource)?;
            if res.is_acquired {
                return Err(D3D11On12Error::ResourceAlreadyAcquired);
            }
        }

        for &id in resource_ids {
            let res = self
                .wrapped_resources
                .iter_mut()
                .find(|r| r.resource_id == id)
                .ok_or(D3D11On12Error::InvalidResource)?;
            let target_in = res.in_state;
            self.state_tracker.transition_resource(res, target_in);
            res.is_acquired = true;
        }

        Ok(())
    }

    // Release wrapped resources transitioning them to out_state and signaling interop fence
    pub fn ReleaseWrappedResources(&mut self, resource_ids: &[u64]) -> Result<u64, D3D11On12Error> {
        if resource_ids.is_empty() {
            return Ok(self.fence_sync.current_value);
        }

        for &id in resource_ids {
            let res = self
                .wrapped_resources
                .iter()
                .find(|r| r.resource_id == id)
                .ok_or(D3D11On12Error::InvalidResource)?;
            if !res.is_acquired {
                return Err(D3D11On12Error::ResourceNotAcquired);
            }
        }

        for &id in resource_ids {
            let res = self
                .wrapped_resources
                .iter_mut()
                .find(|r| r.resource_id == id)
                .ok_or(D3D11On12Error::InvalidResource)?;
            let target_out = res.out_state;
            self.state_tracker.transition_resource(res, target_out);
            res.is_acquired = false;
        }

        let fence_val = self.fence_sync.signal();
        Ok(fence_val)
    }
}

// Factory function to create D3D11 device and context wrapping a D3D12 device
pub fn d3d11on12_create_device(
    d3d12_device: u64,
    flags: u32,
    feature_levels: &[D3D11_FEATURE_LEVEL],
    command_queues: &[u64],
) -> Result<(ID3D11Device, ID3D11DeviceContext, ID3D11On12Device), D3D11On12Error> {
    if d3d12_device == 0 {
        return Err(D3D11On12Error::InvalidDevice);
    }
    if command_queues.is_empty() || command_queues.iter().any(|&q| q == 0) {
        return Err(D3D11On12Error::InvalidQueue);
    }

    let selected_level = if feature_levels.is_empty() {
        D3D11_FEATURE_LEVEL::LEVEL_11_0
    } else {
        *feature_levels.iter().max().unwrap_or(&D3D11_FEATURE_LEVEL::LEVEL_11_0)
    };

    let device11 = ID3D11Device {
        feature_level: selected_level,
        creation_flags: flags,
        d3d12_device,
    };
    let context11 = ID3D11DeviceContext::new(d3d12_device, command_queues[0]);
    let on12_device = ID3D11On12Device::new(d3d12_device, command_queues);

    Ok((device11, context11, on12_device))
}

// Comprehensive verification test suite for Direct3D 11-on-12 compatibility translation layer
pub fn test_d3d11on12_subsystem() -> bool {
    crate::serial_write_str("[KNT-D3D11ON12] Initializing Direct3D 11-on-12 translation self-test...\r\n");

    // 1. Device and context creation
    let dummy_d3d12_device = 0x1000_0000u64;
    let dummy_queue = 0x2000_0000u64;
    let (device11, mut context11, mut on12_dev) = match d3d11on12_create_device(
        dummy_d3d12_device,
        0,
        &[D3D11_FEATURE_LEVEL::LEVEL_11_0],
        &[dummy_queue],
    ) {
        Ok(res) => res,
        Err(_) => {
            crate::serial_write_str("[KNT-D3D11ON12-FAIL] Device creation failed\r\n");
            return false;
        }
    };

    if device11.get_feature_level() != D3D11_FEATURE_LEVEL::LEVEL_11_0 {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Feature level mismatch\r\n");
        return false;
    }
    if d3d11on12_create_device(0, 0, &[], &[dummy_queue]).is_ok() {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Null device was not rejected\r\n");
        return false;
    }
    if d3d11on12_create_device(dummy_d3d12_device, 0, &[], &[]).is_ok() {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Empty queue slice was not rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * D3D11On12 device creation and feature level negotiation verified\r\n");

    // 2. Wrapped resource creation and binding flag management
    let rt_flags = D3D11_RESOURCE_FLAGS {
        BindFlags: D3D11_BIND_FLAG::RENDER_TARGET.0 | D3D11_BIND_FLAG::SHADER_RESOURCE.0,
        MiscFlags: 0,
        CPUAccessFlags: 0,
        StructureByteStride: 0,
    };
    let rt_res_id = match on12_dev.CreateWrappedResource(
        0x3000_0000,
        &rt_flags,
        D3D12_RESOURCE_STATES::RENDER_TARGET,
        D3D12_RESOURCE_STATES::PRESENT,
    ) {
        Ok(id) => id,
        Err(_) => {
            crate::serial_write_str("[KNT-D3D11ON12-FAIL] Failed to wrap render target resource\r\n");
            return false;
        }
    };

    let rt_res = match on12_dev.get_wrapped_resource(rt_res_id) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if rt_res.dimension != D3D11_RESOURCE_DIMENSION::TEXTURE2D
        || rt_res.current_state != D3D12_RESOURCE_STATES::PRESENT
        || rt_res.is_acquired
    {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Wrapped render target state mismatch\r\n");
        return false;
    }

    let cb_flags = D3D11_RESOURCE_FLAGS {
        BindFlags: D3D11_BIND_FLAG::CONSTANT_BUFFER.0,
        MiscFlags: 0,
        CPUAccessFlags: 0,
        StructureByteStride: 64,
    };
    let cb_res_id = match on12_dev.CreateWrappedResource(
        0x4000_0000,
        &cb_flags,
        D3D12_RESOURCE_STATES::VERTEX_AND_CONSTANT_BUFFER,
        D3D12_RESOURCE_STATES::COMMON,
    ) {
        Ok(id) => id,
        Err(_) => return false,
    };
    let cb_res = match on12_dev.get_wrapped_resource(cb_res_id) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if cb_res.dimension != D3D11_RESOURCE_DIMENSION::BUFFER {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Constant buffer dimension mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Wrapped resource creation and D3D11 binding flags verified\r\n");

    // 3. Resource acquisition and implicit state transition to in_state
    if on12_dev.AcquireWrappedResources(&[rt_res_id]).is_err() {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Failed to acquire wrapped render target\r\n");
        return false;
    }
    let acquired_rt = match on12_dev.get_wrapped_resource(rt_res_id) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if !acquired_rt.is_acquired || acquired_rt.current_state != D3D12_RESOURCE_STATES::RENDER_TARGET {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Resource state was not transitioned to in_state\r\n");
        return false;
    }
    if on12_dev.AcquireWrappedResources(&[rt_res_id]) != Err(D3D11On12Error::ResourceAlreadyAcquired) {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Re-acquiring active resource was not rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * AcquireWrappedResources and implicit in_state transition verified\r\n");

    // 4. Interop state transition while acquired in D3D11 context
    let barrier = match on12_dev.transition_wrapped_resource(rt_res_id, D3D12_RESOURCE_STATES::PIXEL_SHADER_RESOURCE) {
        Ok(Some(b)) => b,
        _ => {
            crate::serial_write_str("[KNT-D3D11ON12-FAIL] Transition to shader resource failed\r\n");
            return false;
        }
    };
    if barrier.state_before != D3D12_RESOURCE_STATES::RENDER_TARGET
        || barrier.state_after != D3D12_RESOURCE_STATES::PIXEL_SHADER_RESOURCE
    {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Transition barrier state mismatch\r\n");
        return false;
    }
    context11.clear_render_target_view(0x3000_0000, [0.0, 0.2, 0.4, 1.0]);
    crate::serial_write_str("  * Interop resource state transitions and barrier generation verified\r\n");

    // 5. Release wrapped resources, transition to out_state, and fence signaling
    let fence_val = match on12_dev.ReleaseWrappedResources(&[rt_res_id]) {
        Ok(v) => v,
        Err(_) => {
            crate::serial_write_str("[KNT-D3D11ON12-FAIL] Failed to release wrapped render target\r\n");
            return false;
        }
    };
    if fence_val != 1 {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Signaled fence value mismatch\r\n");
        return false;
    }
    let released_rt = match on12_dev.get_wrapped_resource(rt_res_id) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if released_rt.is_acquired || released_rt.current_state != D3D12_RESOURCE_STATES::PRESENT {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Released resource was not transitioned to out_state\r\n");
        return false;
    }
    if on12_dev.ReleaseWrappedResources(&[rt_res_id]) != Err(D3D11On12Error::ResourceNotAcquired) {
        crate::serial_write_str("[KNT-D3D11ON12-FAIL] Releasing unacquired resource was not rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * ReleaseWrappedResources, out_state transition, and timeline fence verified\r\n");

    crate::serial_write_str("[KNT-GFX-D3D11ON12-PASS] Direct3D 11-on-12 compatibility translation layer verified\r\n");
    true
}
