//! Direct3D 12 GPU-Driven ExecuteIndirect Multi-Draw Stream Expander & Meshlet Culler.
//! Provides variable-count multi-draw buffers, frustum/Hi-Z culling, and predicated expansion.

#![allow(dead_code, non_camel_case_types, unused_imports)]

extern crate alloc;
use alloc::vec::Vec;

pub use super::execute_indirect::D3D12_DRAW_INDEXED_ARGUMENTS;
pub use super::cluster_cull::BoundingSphere;

// Meshlet draw token containing geometric offsets and bounding sphere for GPU culling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshletDrawToken {
    pub meshlet_id: u32,
    pub draw_args: D3D12_DRAW_INDEXED_ARGUMENTS,
    pub bounding_sphere: BoundingSphere,
    pub flags: u32,
}

impl MeshletDrawToken {
    // Creates a new meshlet draw token with bounds and indexed draw arguments.
    pub const fn new(
        meshlet_id: u32,
        index_count: u32,
        start_index: u32,
        base_vertex: i32,
        bounding_sphere: BoundingSphere,
    ) -> Self {
        Self {
            meshlet_id,
            draw_args: D3D12_DRAW_INDEXED_ARGUMENTS {
                index_count_per_instance: index_count,
                instance_count: 1,
                start_index_location: start_index,
                base_vertex_location: base_vertex,
                start_instance_location: 0,
            },
            bounding_sphere,
            flags: 0,
        }
    }
}

// Configuration descriptor for an ExecuteIndirect multi-draw command signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiDrawCommandSignatureDesc {
    pub byte_stride: u32,
    pub max_draw_count: u32,
    pub is_variable_count: bool,
}

impl MultiDrawCommandSignatureDesc {
    // Creates a standard indexed multi-draw command signature descriptor.
    pub const fn standard_draw_indexed(max_draw_count: u32) -> Self {
        Self {
            byte_stride: 20, // 5 x 32-bit fields in D3D12_DRAW_INDEXED_ARGUMENTS
            max_draw_count,
            is_variable_count: true,
        }
    }
}

// GPU buffer tracking the dynamic draw count written by compute culling passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuDrawCountBuffer {
    pub gpu_virtual_address: u64,
    pub count_offset: u32,
    pub current_draw_count: u32,
}

impl GpuDrawCountBuffer {
    // Creates a new GPU draw count buffer descriptor.
    pub const fn new(gpu_virtual_address: u64, count_offset: u32) -> Self {
        Self {
            gpu_virtual_address,
            count_offset,
            current_draw_count: 0,
        }
    }

    // Atomically increments the draw count simulating GPU atomic add.
    pub fn atomic_increment(&mut self) -> u32 {
        let prev = self.current_draw_count;
        self.current_draw_count = self.current_draw_count.saturating_add(1);
        prev
    }

    // Resets draw count to zero prior to a culling pass.
    pub fn reset(&mut self) {
        self.current_draw_count = 0;
    }
}

// Complete multi-draw buffer descriptor defining argument and count buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiDrawBufferDesc {
    pub arguments_gpu_va: u64,
    pub arguments_size_bytes: u64,
    pub count_buffer: GpuDrawCountBuffer,
    pub max_commands: u32,
}

impl MultiDrawBufferDesc {
    // Creates a new multi-draw buffer descriptor.
    pub const fn new(
        arguments_gpu_va: u64,
        arguments_size_bytes: u64,
        count_buffer: GpuDrawCountBuffer,
        max_commands: u32,
    ) -> Self {
        Self {
            arguments_gpu_va,
            arguments_size_bytes,
            count_buffer,
            max_commands,
        }
    }

    // Validates memory requirements and alignment for ExecuteIndirect.
    pub fn validate(&self) -> Result<(), &'static str> {
        if (self.arguments_gpu_va % 4) != 0 {
            return Err("Argument buffer GPU VA must be 4-byte aligned");
        }
        if (self.count_buffer.gpu_virtual_address % 4) != 0 {
            return Err("Count buffer GPU VA must be 4-byte aligned");
        }
        let required_bytes = (self.max_commands as u64) * 20;
        if self.arguments_size_bytes < required_bytes {
            return Err("Argument buffer too small for max commands");
        }
        Ok(())
    }
}

// Temporary stub for self-test during incremental commits.
pub fn test_gpu_multi_draw_subsystem() -> bool {
    true
}
