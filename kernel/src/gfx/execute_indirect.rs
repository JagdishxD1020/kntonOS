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

// Maximum permitted byte stride for a D3D12 command signature (2048 bytes)
pub const D3D12_MAX_COMMAND_SIGNATURE_BYTE_STRIDE: u32 = 2048;

// Error conditions encountered during command signature compilation or indirect dispatch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndirectError {
    EmptyArgumentList,
    InvalidByteStride { stride: u32, min_stride: u32 },
    StrideNotMultipleOfFour(u32),
    StrideExceedsMaximum(u32),
    MultipleDrawOrDispatch,
    DrawOrDispatchNotFinalArgument,
    MissingRootSignature,
    InvalidRootParameter { index: u32, available: u32 },
    BufferTooSmall { required: usize, provided: usize },
    InvalidBufferOffset(u64),
    ArgumentUnpackError,
}

// Compiled Direct3D 12 Command Signature object ready for GPU indirect execution
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_COMMAND_SIGNATURE {
    pub desc: D3D12_COMMAND_SIGNATURE_DESC,
    pub byte_stride: u32,
    pub requires_root_signature: bool,
    pub is_draw: bool,
    pub is_draw_indexed: bool,
    pub is_dispatch: bool,
    pub is_dispatch_mesh: bool,
}

impl D3D12_COMMAND_SIGNATURE {
    // Compile and validate a command signature descriptor
    pub fn create(
        desc: D3D12_COMMAND_SIGNATURE_DESC,
        root_sig_param_count: Option<u32>,
    ) -> Result<Self, IndirectError> {
        if desc.argument_descs.is_empty() {
            return Err(IndirectError::EmptyArgumentList);
        }

        if desc.byte_stride % 4 != 0 {
            return Err(IndirectError::StrideNotMultipleOfFour(desc.byte_stride));
        }

        if desc.byte_stride > D3D12_MAX_COMMAND_SIGNATURE_BYTE_STRIDE {
            return Err(IndirectError::StrideExceedsMaximum(desc.byte_stride));
        }

        let min_stride = desc.minimum_byte_stride();
        if desc.byte_stride < min_stride {
            return Err(IndirectError::InvalidByteStride {
                stride: desc.byte_stride,
                min_stride,
            });
        }

        let mut draw_or_dispatch_count = 0u32;
        let mut requires_root_signature = false;
        let num_args = desc.argument_descs.len();

        for (idx, arg) in desc.argument_descs.iter().enumerate() {
            match arg.arg_type {
                D3D12_INDIRECT_ARGUMENT_TYPE::Draw
                | D3D12_INDIRECT_ARGUMENT_TYPE::DrawIndexed
                | D3D12_INDIRECT_ARGUMENT_TYPE::Dispatch
                | D3D12_INDIRECT_ARGUMENT_TYPE::DispatchMesh
                | D3D12_INDIRECT_ARGUMENT_TYPE::DispatchRays => {
                    draw_or_dispatch_count += 1;
                    if draw_or_dispatch_count > 1 {
                        return Err(IndirectError::MultipleDrawOrDispatch);
                    }
                    if idx != num_args - 1 {
                        return Err(IndirectError::DrawOrDispatchNotFinalArgument);
                    }
                }
                D3D12_INDIRECT_ARGUMENT_TYPE::Constant
                | D3D12_INDIRECT_ARGUMENT_TYPE::ConstantBufferView
                | D3D12_INDIRECT_ARGUMENT_TYPE::ShaderResourceView
                | D3D12_INDIRECT_ARGUMENT_TYPE::UnorderedAccessView => {
                    requires_root_signature = true;
                    match root_sig_param_count {
                        None => return Err(IndirectError::MissingRootSignature),
                        Some(count) => {
                            if arg.root_parameter_index >= count {
                                return Err(IndirectError::InvalidRootParameter {
                                    index: arg.root_parameter_index,
                                    available: count,
                                });
                            }
                        }
                    }
                }
                D3D12_INDIRECT_ARGUMENT_TYPE::VertexBufferView
                | D3D12_INDIRECT_ARGUMENT_TYPE::IndexBufferView => {}
            }
        }

        let last_arg = desc.argument_descs.last().unwrap();
        let is_draw = last_arg.arg_type == D3D12_INDIRECT_ARGUMENT_TYPE::Draw;
        let is_draw_indexed = last_arg.arg_type == D3D12_INDIRECT_ARGUMENT_TYPE::DrawIndexed;
        let is_dispatch = last_arg.arg_type == D3D12_INDIRECT_ARGUMENT_TYPE::Dispatch;
        let is_dispatch_mesh = last_arg.arg_type == D3D12_INDIRECT_ARGUMENT_TYPE::DispatchMesh;

        Ok(Self {
            byte_stride: desc.byte_stride,
            desc,
            requires_root_signature,
            is_draw,
            is_draw_indexed,
            is_dispatch,
            is_dispatch_mesh,
        })
    }

    // Standard Draw signature constructor (16-byte stride)
    pub fn create_draw() -> Result<Self, IndirectError> {
        let desc = D3D12_COMMAND_SIGNATURE_DESC::new(
            core::mem::size_of::<D3D12_DRAW_ARGUMENTS>() as u32,
            alloc::vec![D3D12_INDIRECT_ARGUMENT_DESC::make_draw()],
            0,
        );
        Self::create(desc, None)
    }

    // Standard DrawIndexed signature constructor (20-byte stride)
    pub fn create_draw_indexed() -> Result<Self, IndirectError> {
        let desc = D3D12_COMMAND_SIGNATURE_DESC::new(
            core::mem::size_of::<D3D12_DRAW_INDEXED_ARGUMENTS>() as u32,
            alloc::vec![D3D12_INDIRECT_ARGUMENT_DESC::make_draw_indexed()],
            0,
        );
        Self::create(desc, None)
    }

    // Standard Dispatch signature constructor (12-byte stride)
    pub fn create_dispatch() -> Result<Self, IndirectError> {
        let desc = D3D12_COMMAND_SIGNATURE_DESC::new(
            core::mem::size_of::<D3D12_DISPATCH_ARGUMENTS>() as u32,
            alloc::vec![D3D12_INDIRECT_ARGUMENT_DESC::make_dispatch()],
            0,
        );
        Self::create(desc, None)
    }
}

// Parsed concrete value of an individual indirect argument
#[derive(Debug, Clone, PartialEq)]
#[allow(non_camel_case_types)]
pub enum D3D12_PARSED_INDIRECT_ARGUMENT {
    Draw(D3D12_DRAW_ARGUMENTS),
    DrawIndexed(D3D12_DRAW_INDEXED_ARGUMENTS),
    Dispatch(D3D12_DISPATCH_ARGUMENTS),
    DispatchMesh(D3D12_DISPATCH_MESH_ARGUMENTS),
    VertexBufferView {
        slot: u32,
        view: D3D12_VERTEX_BUFFER_VIEW_ARGUMENT,
    },
    IndexBufferView(D3D12_INDEX_BUFFER_VIEW_ARGUMENT),
    Constant {
        root_parameter_index: u32,
        offset: u32,
        values: Vec<u32>,
    },
    ConstantBufferView {
        root_parameter_index: u32,
        gpu_va: u64,
    },
    ShaderResourceView {
        root_parameter_index: u32,
        gpu_va: u64,
    },
    UnorderedAccessView {
        root_parameter_index: u32,
        gpu_va: u64,
    },
}

// Complete set of parsed arguments for a single indirect command invocation
#[derive(Debug, Clone, PartialEq)]
#[allow(non_camel_case_types)]
pub struct D3D12_PARSED_COMMAND {
    pub command_index: u32,
    pub arguments: Vec<D3D12_PARSED_INDIRECT_ARGUMENT>,
}

// Indirect argument buffer parser with GPU count buffer evaluation
#[allow(non_camel_case_types)]
pub struct D3D12_INDIRECT_ARGUMENT_PARSER;

impl D3D12_INDIRECT_ARGUMENT_PARSER {
    // Parse indirect argument buffer and resolve autonomous GPU count buffer clamping
    pub fn parse(
        signature: &D3D12_COMMAND_SIGNATURE,
        arg_buffer: &[u8],
        arg_buffer_offset: usize,
        max_command_count: u32,
        count_buffer_data: Option<(&[u8], usize)>,
    ) -> Result<Vec<D3D12_PARSED_COMMAND>, IndirectError> {
        if arg_buffer_offset % 4 != 0 {
            return Err(IndirectError::InvalidBufferOffset(arg_buffer_offset as u64));
        }

        // 1. Evaluate GPU count buffer with max_command_count clamping
        let effective_command_count = match count_buffer_data {
            Some((buf, offset)) => {
                if offset % 4 != 0 {
                    return Err(IndirectError::InvalidBufferOffset(offset as u64));
                }
                if buf.len() < offset + 4 {
                    return Err(IndirectError::BufferTooSmall {
                        required: offset + 4,
                        provided: buf.len(),
                    });
                }
                let count_bytes: [u8; 4] = buf[offset..offset + 4]
                    .try_into()
                    .map_err(|_| IndirectError::ArgumentUnpackError)?;
                let gpu_count = u32::from_le_bytes(count_bytes);
                core::cmp::min(gpu_count, max_command_count)
            }
            None => max_command_count,
        };

        if effective_command_count == 0 {
            return Ok(Vec::new());
        }

        // 2. Validate total argument buffer bounds
        let total_required_bytes = arg_buffer_offset
            + (effective_command_count as usize * signature.byte_stride as usize);
        if arg_buffer.len() < total_required_bytes {
            return Err(IndirectError::BufferTooSmall {
                required: total_required_bytes,
                provided: arg_buffer.len(),
            });
        }

        // 3. Unpack commands sequentially across the buffer stride
        let mut parsed_commands = Vec::with_capacity(effective_command_count as usize);

        for cmd_idx in 0..effective_command_count {
            let cmd_base = arg_buffer_offset + (cmd_idx as usize * signature.byte_stride as usize);
            let mut arg_cursor = cmd_base;
            let mut cmd_args = Vec::with_capacity(signature.desc.argument_descs.len());

            for arg_desc in &signature.desc.argument_descs {
                let parsed_arg = match arg_desc.arg_type {
                    D3D12_INDIRECT_ARGUMENT_TYPE::Draw => {
                        let b: [u8; 16] = arg_buffer[arg_cursor..arg_cursor + 16]
                            .try_into()
                            .map_err(|_| IndirectError::ArgumentUnpackError)?;
                        let draw_args = D3D12_DRAW_ARGUMENTS {
                            vertex_count_per_instance: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                            instance_count: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                            start_vertex_location: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
                            start_instance_location: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
                        };
                        arg_cursor += 16;
                        D3D12_PARSED_INDIRECT_ARGUMENT::Draw(draw_args)
                    }
                    D3D12_INDIRECT_ARGUMENT_TYPE::DrawIndexed => {
                        let b: [u8; 20] = arg_buffer[arg_cursor..arg_cursor + 20]
                            .try_into()
                            .map_err(|_| IndirectError::ArgumentUnpackError)?;
                        let draw_indexed_args = D3D12_DRAW_INDEXED_ARGUMENTS {
                            index_count_per_instance: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                            instance_count: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                            start_index_location: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
                            base_vertex_location: i32::from_le_bytes([b[12], b[13], b[14], b[15]]),
                            start_instance_location: u32::from_le_bytes([b[16], b[17], b[18], b[19]]),
                        };
                        arg_cursor += 20;
                        D3D12_PARSED_INDIRECT_ARGUMENT::DrawIndexed(draw_indexed_args)
                    }
                    D3D12_INDIRECT_ARGUMENT_TYPE::Dispatch => {
                        let b: [u8; 12] = arg_buffer[arg_cursor..arg_cursor + 12]
                            .try_into()
                            .map_err(|_| IndirectError::ArgumentUnpackError)?;
                        let dispatch_args = D3D12_DISPATCH_ARGUMENTS {
                            thread_group_count_x: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                            thread_group_count_y: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                            thread_group_count_z: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
                        };
                        arg_cursor += 12;
                        D3D12_PARSED_INDIRECT_ARGUMENT::Dispatch(dispatch_args)
                    }
                    D3D12_INDIRECT_ARGUMENT_TYPE::DispatchMesh => {
                        let b: [u8; 12] = arg_buffer[arg_cursor..arg_cursor + 12]
                            .try_into()
                            .map_err(|_| IndirectError::ArgumentUnpackError)?;
                        let mesh_args = D3D12_DISPATCH_MESH_ARGUMENTS::new(
                            u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                            u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                            u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
                        );
                        arg_cursor += 12;
                        D3D12_PARSED_INDIRECT_ARGUMENT::DispatchMesh(mesh_args)
                    }
                    D3D12_INDIRECT_ARGUMENT_TYPE::VertexBufferView => {
                        let b: [u8; 16] = arg_buffer[arg_cursor..arg_cursor + 16]
                            .try_into()
                            .map_err(|_| IndirectError::ArgumentUnpackError)?;
                        let view = D3D12_VERTEX_BUFFER_VIEW_ARGUMENT {
                            buffer_location: u64::from_le_bytes(b[0..8].try_into().unwrap()),
                            size_in_bytes: u32::from_le_bytes(b[8..12].try_into().unwrap()),
                            stride_in_bytes: u32::from_le_bytes(b[12..16].try_into().unwrap()),
                        };
                        arg_cursor += 16;
                        D3D12_PARSED_INDIRECT_ARGUMENT::VertexBufferView {
                            slot: arg_desc.vertex_buffer_slot,
                            view,
                        }
                    }
                    D3D12_INDIRECT_ARGUMENT_TYPE::IndexBufferView => {
                        let b: [u8; 16] = arg_buffer[arg_cursor..arg_cursor + 16]
                            .try_into()
                            .map_err(|_| IndirectError::ArgumentUnpackError)?;
                        let view = D3D12_INDEX_BUFFER_VIEW_ARGUMENT {
                            buffer_location: u64::from_le_bytes(b[0..8].try_into().unwrap()),
                            size_in_bytes: u32::from_le_bytes(b[8..12].try_into().unwrap()),
                            format: u32::from_le_bytes(b[12..16].try_into().unwrap()),
                        };
                        arg_cursor += 16;
                        D3D12_PARSED_INDIRECT_ARGUMENT::IndexBufferView(view)
                    }
                    D3D12_INDIRECT_ARGUMENT_TYPE::Constant => {
                        let num_vals = arg_desc.num_32bit_values_to_set as usize;
                        let mut vals = Vec::with_capacity(num_vals);
                        for _ in 0..num_vals {
                            let val_bytes: [u8; 4] = arg_buffer[arg_cursor..arg_cursor + 4]
                                .try_into()
                                .map_err(|_| IndirectError::ArgumentUnpackError)?;
                            vals.push(u32::from_le_bytes(val_bytes));
                            arg_cursor += 4;
                        }
                        D3D12_PARSED_INDIRECT_ARGUMENT::Constant {
                            root_parameter_index: arg_desc.root_parameter_index,
                            offset: arg_desc.dest_offset_in_32bit_values,
                            values: vals,
                        }
                    }
                    D3D12_INDIRECT_ARGUMENT_TYPE::ConstantBufferView => {
                        let b: [u8; 8] = arg_buffer[arg_cursor..arg_cursor + 8]
                            .try_into()
                            .map_err(|_| IndirectError::ArgumentUnpackError)?;
                        let gpu_va = u64::from_le_bytes(b);
                        arg_cursor += 8;
                        D3D12_PARSED_INDIRECT_ARGUMENT::ConstantBufferView {
                            root_parameter_index: arg_desc.root_parameter_index,
                            gpu_va,
                        }
                    }
                    D3D12_INDIRECT_ARGUMENT_TYPE::ShaderResourceView => {
                        let b: [u8; 8] = arg_buffer[arg_cursor..arg_cursor + 8]
                            .try_into()
                            .map_err(|_| IndirectError::ArgumentUnpackError)?;
                        let gpu_va = u64::from_le_bytes(b);
                        arg_cursor += 8;
                        D3D12_PARSED_INDIRECT_ARGUMENT::ShaderResourceView {
                            root_parameter_index: arg_desc.root_parameter_index,
                            gpu_va,
                        }
                    }
                    D3D12_INDIRECT_ARGUMENT_TYPE::UnorderedAccessView => {
                        let b: [u8; 8] = arg_buffer[arg_cursor..arg_cursor + 8]
                            .try_into()
                            .map_err(|_| IndirectError::ArgumentUnpackError)?;
                        let gpu_va = u64::from_le_bytes(b);
                        arg_cursor += 8;
                        D3D12_PARSED_INDIRECT_ARGUMENT::UnorderedAccessView {
                            root_parameter_index: arg_desc.root_parameter_index,
                            gpu_va,
                        }
                    }
                    D3D12_INDIRECT_ARGUMENT_TYPE::DispatchRays => {
                        arg_cursor += 32;
                        continue;
                    }
                };
                cmd_args.push(parsed_arg);
            }

            parsed_commands.push(D3D12_PARSED_COMMAND {
                command_index: cmd_idx,
                arguments: cmd_args,
            });
        }

        Ok(parsed_commands)
    }
}

// Opcode token identifying an ExecuteIndirect command packet in the graphics recorder stream
pub const D3D12_TOKEN_EXECUTE_INDIRECT: u32 = 0x4558_4944; // 'EXID'

// Command recorder token packet for ExecuteIndirect dispatch
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_EXECUTE_INDIRECT_COMMAND_PACKET {
    pub opcode: u32,
    pub command_signature_id: u32,
    pub max_command_count: u32,
    pub argument_buffer_gpu_va: u64,
    pub argument_buffer_offset: u64,
    pub count_buffer_gpu_va: u64,
    pub count_buffer_offset: u64,
}

impl D3D12_EXECUTE_INDIRECT_COMMAND_PACKET {
    // Construct and validate indirect execution command token packet
    pub fn new(
        command_signature_id: u32,
        max_command_count: u32,
        arg_va: u64,
        arg_offset: u64,
        count_va: u64,
        count_offset: u64,
    ) -> Result<Self, IndirectError> {
        if arg_offset % 4 != 0 {
            return Err(IndirectError::InvalidBufferOffset(arg_offset));
        }
        if count_va != 0 && count_offset % 4 != 0 {
            return Err(IndirectError::InvalidBufferOffset(count_offset));
        }

        Ok(Self {
            opcode: D3D12_TOKEN_EXECUTE_INDIRECT,
            command_signature_id,
            max_command_count,
            argument_buffer_gpu_va: arg_va,
            argument_buffer_offset: arg_offset,
            count_buffer_gpu_va: count_va,
            count_buffer_offset: count_offset,
        })
    }

    // Has valid GPU count buffer binding
    #[inline]
    pub fn has_count_buffer(&self) -> bool {
        self.count_buffer_gpu_va != 0
    }
}

// Command list recorder token stream encoder for indirect graphics & compute dispatches
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_INDIRECT_COMMAND_RECORDER {
    pub packets: Vec<D3D12_EXECUTE_INDIRECT_COMMAND_PACKET>,
    pub total_indirect_dispatches: u64,
    pub total_max_commands_scheduled: u64,
}

impl D3D12_INDIRECT_COMMAND_RECORDER {
    // Create new indirect command recorder
    pub fn new() -> Self {
        Self {
            packets: Vec::new(),
            total_indirect_dispatches: 0,
            total_max_commands_scheduled: 0,
        }
    }

    // Record an ExecuteIndirect token into the command list stream
    pub fn record_execute_indirect(
        &mut self,
        _signature: &D3D12_COMMAND_SIGNATURE,
        signature_id: u32,
        max_command_count: u32,
        arg_va: u64,
        arg_offset: u64,
        count_va: u64,
        count_offset: u64,
    ) -> Result<&D3D12_EXECUTE_INDIRECT_COMMAND_PACKET, IndirectError> {
        let packet = D3D12_EXECUTE_INDIRECT_COMMAND_PACKET::new(
            signature_id,
            max_command_count,
            arg_va,
            arg_offset,
            count_va,
            count_offset,
        )?;

        self.total_indirect_dispatches += 1;
        self.total_max_commands_scheduled += max_command_count as u64;
        self.packets.push(packet);
        Ok(self.packets.last().unwrap())
    }

    // Serialize all recorded indirect commands into a linear byte token stream
    pub fn serialize_to_token_stream(&self, stream: &mut Vec<u8>) {
        for packet in &self.packets {
            stream.extend_from_slice(&packet.opcode.to_le_bytes());
            stream.extend_from_slice(&packet.command_signature_id.to_le_bytes());
            stream.extend_from_slice(&packet.max_command_count.to_le_bytes());
            stream.extend_from_slice(&packet.argument_buffer_gpu_va.to_le_bytes());
            stream.extend_from_slice(&packet.argument_buffer_offset.to_le_bytes());
            stream.extend_from_slice(&packet.count_buffer_gpu_va.to_le_bytes());
            stream.extend_from_slice(&packet.count_buffer_offset.to_le_bytes());
        }
    }

    // Number of recorded packets
    #[inline]
    pub fn len(&self) -> usize {
        self.packets.len()
    }

    // Whether no packets have been recorded
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }
}
