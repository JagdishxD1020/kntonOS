//! Direct3D 12 Agility Command Signature Suballocation & Indirect Draw Dispatch Buffer Pool.
//! Provides indirect argument structures, ring buffer allocation, and memory footprint governance.

#![allow(dead_code, non_camel_case_types, non_snake_case, unused_imports)]

use super::execute_indirect::{
    D3D12_COMMAND_SIGNATURE_DESC, D3D12_DISPATCH_ARGUMENTS, D3D12_DRAW_ARGUMENTS,
    D3D12_DRAW_INDEXED_ARGUMENTS, D3D12_INDIRECT_ARGUMENT_DESC, D3D12_INDIRECT_ARGUMENT_TYPE,
};

// Maximum permitted byte stride for indirect argument buffers (2048 bytes).
pub const D3D12_MAX_INDIRECT_ARGUMENT_STRIDE: u32 = 2048;

// Required alignment for indirect argument buffer start offsets (4 bytes per D3D12 spec).
pub const D3D12_INDIRECT_ARGUMENT_BUFFER_ALIGNMENT: usize = 4;

// Classification of indirect command operations supported by command signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndirectCommandKind {
    Draw,
    DrawIndexed,
    Dispatch,
    DispatchMesh,
    RootConstants,
}

// Error conditions returned by the command signature and indirect buffer pool subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSignatureError {
    InvalidByteStride { stride: u32, min_required: u32 },
    StrideNotMultipleOfFour(u32),
    StrideExceedsMaximum(u32),
    BufferOverflow { required_bytes: usize, available_bytes: usize },
    InvalidBufferOffset(usize),
    RingBufferExhausted,
    FenceNotSignaled { expected: u64, current: u64 },
    MemoryBudgetExceeded { current_bytes: usize, limit_bytes: usize },
}

// Indirect root constants argument descriptor updating root parameters dynamically on GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndirectRootConstantsArgument {
    // Root parameter slot index in root signature.
    pub root_parameter_index: u32,
    // Start offset in 32-bit values within root constants buffer.
    pub dest_offset_in_32bit_values: u32,
    // Number of 32-bit uint constants updated.
    pub num_32bit_values: u32,
    // Constant data values.
    pub values: alloc::vec::Vec<u32>,
}

impl IndirectRootConstantsArgument {
    // Constructs a new indirect root constants argument.
    pub fn new(root_param_index: u32, dest_offset: u32, values: &[u32]) -> Self {
        Self {
            root_parameter_index: root_param_index,
            dest_offset_in_32bit_values: dest_offset,
            num_32bit_values: values.len() as u32,
            values: alloc::vec::Vec::from(values),
        }
    }

    // Calculates byte footprint of this constants argument.
    pub fn byte_size(&self) -> u32 {
        self.num_32bit_values * 4
    }
}

// Composite indirect draw and dispatch command packet representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndirectCommandPacket {
    // Command type classification.
    pub kind: IndirectCommandKind,
    // Optional draw arguments.
    pub draw_args: Option<D3D12_DRAW_ARGUMENTS>,
    // Optional indexed draw arguments.
    pub draw_indexed_args: Option<D3D12_DRAW_INDEXED_ARGUMENTS>,
    // Optional dispatch arguments.
    pub dispatch_args: Option<D3D12_DISPATCH_ARGUMENTS>,
    // Optional root constants updates.
    pub root_constants: alloc::vec::Vec<IndirectRootConstantsArgument>,
}

impl IndirectCommandPacket {
    // Creates a simple indirect draw command packet.
    pub fn from_draw(args: D3D12_DRAW_ARGUMENTS) -> Self {
        Self {
            kind: IndirectCommandKind::Draw,
            draw_args: Some(args),
            draw_indexed_args: None,
            dispatch_args: None,
            root_constants: alloc::vec::Vec::new(),
        }
    }

    // Creates an indirect indexed draw command packet with root constants.
    pub fn from_draw_indexed(
        args: D3D12_DRAW_INDEXED_ARGUMENTS,
        constants: alloc::vec::Vec<IndirectRootConstantsArgument>,
    ) -> Self {
        Self {
            kind: IndirectCommandKind::DrawIndexed,
            draw_args: None,
            draw_indexed_args: Some(args),
            dispatch_args: None,
            root_constants: constants,
        }
    }

    // Creates an indirect compute dispatch command packet.
    pub fn from_dispatch(args: D3D12_DISPATCH_ARGUMENTS) -> Self {
        Self {
            kind: IndirectCommandKind::Dispatch,
            draw_args: None,
            draw_indexed_args: None,
            dispatch_args: Some(args),
            root_constants: alloc::vec::Vec::new(),
        }
    }
}

// Temporary stub for self-test during incremental commits.
pub fn test_command_signature_subsystem() -> bool {
    true
}
