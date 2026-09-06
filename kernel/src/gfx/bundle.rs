//! Direct3D 12 Bundle Recording & Fast Command List Pre-Compilation Engine.
//! Provides low-overhead pre-compiled GPU command streams for repetitive multi-draw execution.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use alloc::vec::Vec;
use crate::gfx::pso::DXGI_FORMAT;

// Command list type identifier constant for Bundles
pub const D3D12_COMMAND_LIST_TYPE_BUNDLE: u32 = 1;

// Maximum vertex buffer slots supported by the input assembler
pub const D3D12_IA_VERTEX_INPUT_RESOURCE_SLOT_COUNT: usize = 32;

// Vertex buffer binding view
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_VERTEX_BUFFER_VIEW {
    pub buffer_location: u64,
    pub size_in_bytes: u32,
    pub stride_in_bytes: u32,
}

// Index buffer binding view
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_INDEX_BUFFER_VIEW {
    pub buffer_location: u64,
    pub size_in_bytes: u32,
    pub format: DXGI_FORMAT,
}

impl Default for D3D12_INDEX_BUFFER_VIEW {
    fn default() -> Self {
        Self {
            buffer_location: 0,
            size_in_bytes: 0,
            format: DXGI_FORMAT::Unknown,
        }
    }
}

// Recorded GPU commands legal inside a Direct3D 12 bundle
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleCommand {
    DrawIndexedInstanced {
        index_count_per_instance: u32,
        instance_count: u32,
        start_index_location: u32,
        base_vertex_location: i32,
        start_instance_location: u32,
    },
    Dispatch {
        thread_group_count_x: u32,
        thread_group_count_y: u32,
        thread_group_count_z: u32,
    },
    SetGraphicsRootDescriptorTable {
        root_parameter_index: u32,
        base_descriptor_gpu_address: u64,
    },
    SetGraphicsRoot32BitConstants {
        root_parameter_index: u32,
        num_32bit_values_to_set: u32,
        dest_offset_in_32bit_values: u32,
        values: Vec<u32>,
    },
    IASetVertexBuffers {
        start_slot: u32,
        views: Vec<D3D12_VERTEX_BUFFER_VIEW>,
    },
    IASetIndexBuffer {
        view: D3D12_INDEX_BUFFER_VIEW,
    },
    SetPipelineState {
        pso_id: u64,
    },
    SetGraphicsRootSignature {
        root_signature_id: u64,
    },
}

// Memory allocator backing pre-compiled bundle command stream allocations
pub struct BundleAllocator {
    capacity_bytes: usize,
    allocated_bytes: usize,
    command_count: usize,
}

impl BundleAllocator {
    // Construct bundle allocator with specific memory ceiling
    pub fn new(capacity_bytes: usize) -> Self {
        Self {
            capacity_bytes,
            allocated_bytes: 0,
            command_count: 0,
        }
    }

    // Allocate memory footprint for recorded command
    pub fn track_allocation(&mut self, size_bytes: usize) -> Result<(), BundleError> {
        if self.allocated_bytes + size_bytes > self.capacity_bytes {
            return Err(BundleError::OutOfMemory);
        }
        self.allocated_bytes += size_bytes;
        self.command_count += 1;
        Ok(())
    }

    // Reset allocator memory pool for bundle re-recording
    pub fn reset(&mut self) {
        self.allocated_bytes = 0;
        self.command_count = 0;
    }

    // Query active memory usage metrics
    pub fn memory_usage(&self) -> (usize, usize, usize) {
        (self.allocated_bytes, self.capacity_bytes, self.command_count)
    }
}

// Errors occurring during bundle compilation, inheritance, and execution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleError {
    BundleNotClosed,
    BundleAlreadyClosed,
    ForbiddenCommandInBundle,
    MissingParentPipelineState,
    MissingParentRootSignature,
    IncompatibleRootSignature,
    InvalidSlotIndex,
    OutOfMemory,
    ExecutionFailed,
}

// Direct3D 12 bundle creation descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_BUNDLE_DESC {
    pub node_mask: u32,
    pub initial_pipeline_state: Option<u64>,
}
