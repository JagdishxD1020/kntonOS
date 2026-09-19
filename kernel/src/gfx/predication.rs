//! Direct3D 12 Predication & GPU Occlusion Query Skipping Engine.
//! Provides conditional draw/dispatch command filtering based on 64-bit GPU predicate buffers.

#![allow(dead_code)]

/// Direct3D 12 predication operation modes.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum D3D12_PREDICATION_OP {
    D3D12_PREDICATION_OP_EQUAL_ZERO = 0,
    D3D12_PREDICATION_OP_NOT_EQUAL_ZERO = 1,
}

pub const D3D12_PREDICATION_OP_EQUAL_ZERO: D3D12_PREDICATION_OP =
    D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_EQUAL_ZERO;
pub const D3D12_PREDICATION_OP_NOT_EQUAL_ZERO: D3D12_PREDICATION_OP =
    D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_NOT_EQUAL_ZERO;

/// Predication descriptor maintaining active GPU predicate buffer address and offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_PREDICATION_DESC {
    pub buffer_gpu_va: u64,
    pub aligned_buffer_offset: u64,
    pub operation: D3D12_PREDICATION_OP,
    pub is_active: bool,
}

impl D3D12_PREDICATION_DESC {
    /// Creates an active predication descriptor for a GPU virtual address and aligned offset.
    pub const fn active(buffer_gpu_va: u64, aligned_buffer_offset: u64, operation: D3D12_PREDICATION_OP) -> Self {
        Self {
            buffer_gpu_va,
            aligned_buffer_offset,
            operation,
            is_active: true,
        }
    }

    /// Creates a disabled predication descriptor.
    pub const fn disabled() -> Self {
        Self {
            buffer_gpu_va: 0,
            aligned_buffer_offset: 0,
            operation: D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_EQUAL_ZERO,
            is_active: false,
        }
    }
}
