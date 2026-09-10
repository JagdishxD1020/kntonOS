//! Direct3D 11-on-12 (D3D11On12) Compatibility Translation Layer.
//! Wraps D3D12 devices and command queues to provide native D3D11 interfaces.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use alloc::vec::Vec;

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

// D3D11-on-12 interop device interface
#[derive(Debug, Clone)]
pub struct ID3D11On12Device {
    pub d3d12_device: u64,
    pub command_queues: Vec<u64>,
    pub next_resource_id: u64,
}

impl ID3D11On12Device {
    // Construct new D3D11On12 interop device
    pub fn new(d3d12_device: u64, queues: &[u64]) -> Self {
        Self {
            d3d12_device,
            command_queues: queues.to_vec(),
            next_resource_id: 1,
        }
    }

    // Allocate unique resource identifier
    pub fn allocate_resource_id(&mut self) -> u64 {
        let id = self.next_resource_id;
        self.next_resource_id += 1;
        id
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
