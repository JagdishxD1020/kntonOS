//! Direct3D 12 Command Signature & GPU ExecuteIndirect Engine.
//! Provides indirect argument descriptors, command signatures, and GPU-driven draw execution.

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

// Classification of indirect argument types supported in D3D12 command signatures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum D3D12_INDIRECT_ARGUMENT_TYPE {
    Draw = 0,
    DrawIndexed = 1,
    Dispatch = 2,
    VertexBufferView = 3,
    IndexBufferView = 4,
    Constant = 5,
    ConstantBufferView = 6,
    ShaderResourceView = 7,
    UnorderedAccessView = 8,
    DispatchRays = 9,
    DispatchMesh = 10,
}

// 32-bit draw arguments matching D3D12_DRAW_ARGUMENTS layout
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_DRAW_ARGUMENTS {
    pub vertex_count_per_instance: u32,
    pub instance_count: u32,
    pub start_vertex_location: u32,
    pub start_instance_location: u32,
}

// 32-bit indexed draw arguments matching D3D12_DRAW_INDEXED_ARGUMENTS layout
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_DRAW_INDEXED_ARGUMENTS {
    pub index_count_per_instance: u32,
    pub instance_count: u32,
    pub start_index_location: u32,
    pub base_vertex_location: i32,
    pub start_instance_location: u32,
}

// 32-bit dispatch thread group count arguments matching D3D12_DISPATCH_ARGUMENTS layout
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_DISPATCH_ARGUMENTS {
    pub thread_group_count_x: u32,
    pub thread_group_count_y: u32,
    pub thread_group_count_z: u32,
}

use crate::gfx::mesh_shader::D3D12_DISPATCH_MESH_ARGUMENTS;

// Vertex buffer view indirect argument layout (GPU virtual address, size, and stride)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_VERTEX_BUFFER_VIEW_ARGUMENT {
    pub buffer_location: u64,
    pub size_in_bytes: u32,
    pub stride_in_bytes: u32,
}

// Index buffer view indirect argument layout (GPU virtual address, size, and format)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_INDEX_BUFFER_VIEW_ARGUMENT {
    pub buffer_location: u64,
    pub size_in_bytes: u32,
    pub format: u32,
}

// Indirect argument descriptor configuring parameter type and root binding details
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_INDIRECT_ARGUMENT_DESC {
    pub arg_type: D3D12_INDIRECT_ARGUMENT_TYPE,
    pub root_parameter_index: u32,
    pub dest_offset_in_32bit_values: u32,
    pub num_32bit_values_to_set: u32,
    pub vertex_buffer_slot: u32,
}

impl D3D12_INDIRECT_ARGUMENT_DESC {
    // Construct a Draw indirect argument descriptor
    pub const fn make_draw() -> Self {
        Self {
            arg_type: D3D12_INDIRECT_ARGUMENT_TYPE::Draw,
            root_parameter_index: 0,
            dest_offset_in_32bit_values: 0,
            num_32bit_values_to_set: 0,
            vertex_buffer_slot: 0,
        }
    }

    // Construct a DrawIndexed indirect argument descriptor
    pub const fn make_draw_indexed() -> Self {
        Self {
            arg_type: D3D12_INDIRECT_ARGUMENT_TYPE::DrawIndexed,
            root_parameter_index: 0,
            dest_offset_in_32bit_values: 0,
            num_32bit_values_to_set: 0,
            vertex_buffer_slot: 0,
        }
    }

    // Construct a Dispatch indirect argument descriptor
    pub const fn make_dispatch() -> Self {
        Self {
            arg_type: D3D12_INDIRECT_ARGUMENT_TYPE::Dispatch,
            root_parameter_index: 0,
            dest_offset_in_32bit_values: 0,
            num_32bit_values_to_set: 0,
            vertex_buffer_slot: 0,
        }
    }

    // Construct a VertexBufferView indirect argument descriptor for a given slot
    pub const fn make_vertex_buffer(slot: u32) -> Self {
        Self {
            arg_type: D3D12_INDIRECT_ARGUMENT_TYPE::VertexBufferView,
            root_parameter_index: 0,
            dest_offset_in_32bit_values: 0,
            num_32bit_values_to_set: 0,
            vertex_buffer_slot: slot,
        }
    }

    // Construct an IndexBufferView indirect argument descriptor
    pub const fn make_index_buffer() -> Self {
        Self {
            arg_type: D3D12_INDIRECT_ARGUMENT_TYPE::IndexBufferView,
            root_parameter_index: 0,
            dest_offset_in_32bit_values: 0,
            num_32bit_values_to_set: 0,
            vertex_buffer_slot: 0,
        }
    }

    // Construct a Constant (root constant 32-bit values) indirect argument descriptor
    pub const fn make_constant(root_parameter_index: u32, dest_offset: u32, num_values: u32) -> Self {
        Self {
            arg_type: D3D12_INDIRECT_ARGUMENT_TYPE::Constant,
            root_parameter_index,
            dest_offset_in_32bit_values: dest_offset,
            num_32bit_values_to_set: num_values,
            vertex_buffer_slot: 0,
        }
    }

    // Construct a ConstantBufferView indirect argument descriptor
    pub const fn make_cbv(root_parameter_index: u32) -> Self {
        Self {
            arg_type: D3D12_INDIRECT_ARGUMENT_TYPE::ConstantBufferView,
            root_parameter_index,
            dest_offset_in_32bit_values: 0,
            num_32bit_values_to_set: 0,
            vertex_buffer_slot: 0,
        }
    }

    // Construct a ShaderResourceView indirect argument descriptor
    pub const fn make_srv(root_parameter_index: u32) -> Self {
        Self {
            arg_type: D3D12_INDIRECT_ARGUMENT_TYPE::ShaderResourceView,
            root_parameter_index,
            dest_offset_in_32bit_values: 0,
            num_32bit_values_to_set: 0,
            vertex_buffer_slot: 0,
        }
    }

    // Construct an UnorderedAccessView indirect argument descriptor
    pub const fn make_uav(root_parameter_index: u32) -> Self {
        Self {
            arg_type: D3D12_INDIRECT_ARGUMENT_TYPE::UnorderedAccessView,
            root_parameter_index,
            dest_offset_in_32bit_values: 0,
            num_32bit_values_to_set: 0,
            vertex_buffer_slot: 0,
        }
    }

    // Size in bytes occupied by this argument in an indirect argument buffer
    pub const fn byte_size(&self) -> u32 {
        match self.arg_type {
            D3D12_INDIRECT_ARGUMENT_TYPE::Draw => core::mem::size_of::<D3D12_DRAW_ARGUMENTS>() as u32,
            D3D12_INDIRECT_ARGUMENT_TYPE::DrawIndexed => {
                core::mem::size_of::<D3D12_DRAW_INDEXED_ARGUMENTS>() as u32
            }
            D3D12_INDIRECT_ARGUMENT_TYPE::Dispatch => {
                core::mem::size_of::<D3D12_DISPATCH_ARGUMENTS>() as u32
            }
            D3D12_INDIRECT_ARGUMENT_TYPE::DispatchMesh => {
                core::mem::size_of::<D3D12_DISPATCH_MESH_ARGUMENTS>() as u32
            }
            D3D12_INDIRECT_ARGUMENT_TYPE::VertexBufferView => {
                core::mem::size_of::<D3D12_VERTEX_BUFFER_VIEW_ARGUMENT>() as u32
            }
            D3D12_INDIRECT_ARGUMENT_TYPE::IndexBufferView => {
                core::mem::size_of::<D3D12_INDEX_BUFFER_VIEW_ARGUMENT>() as u32
            }
            D3D12_INDIRECT_ARGUMENT_TYPE::Constant => self.num_32bit_values_to_set * 4,
            D3D12_INDIRECT_ARGUMENT_TYPE::ConstantBufferView
            | D3D12_INDIRECT_ARGUMENT_TYPE::ShaderResourceView
            | D3D12_INDIRECT_ARGUMENT_TYPE::UnorderedAccessView => 8, // 64-bit GPU virtual address
            D3D12_INDIRECT_ARGUMENT_TYPE::DispatchRays => 32,
        }
    }
}

// Descriptor specifying byte stride, argument list, and node mask for a command signature
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_COMMAND_SIGNATURE_DESC {
    pub byte_stride: u32,
    pub argument_descs: Vec<D3D12_INDIRECT_ARGUMENT_DESC>,
    pub node_mask: u32,
}

impl D3D12_COMMAND_SIGNATURE_DESC {
    // Construct signature descriptor with explicit byte stride and argument sequence
    pub fn new(byte_stride: u32, argument_descs: Vec<D3D12_INDIRECT_ARGUMENT_DESC>, node_mask: u32) -> Self {
        Self {
            byte_stride,
            argument_descs,
            node_mask,
        }
    }

    // Minimum byte stride required to tightly pack all indirect argument descriptors
    pub fn minimum_byte_stride(&self) -> u32 {
        self.argument_descs.iter().map(|arg| arg.byte_size()).sum()
    }
}
