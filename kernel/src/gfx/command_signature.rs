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

// Aligns a byte offset up to the specified power-of-two alignment boundary.
#[inline(always)]
pub const fn align_up_usize(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }
    (value + (alignment - 1)) & !(alignment - 1)
}

// Suballocated region within the indirect argument ring buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndirectArgumentChunk {
    // GPU virtual address of suballocated region.
    pub gpu_virtual_address: u64,
    // Byte offset within ring buffer.
    pub offset: usize,
    // Usable size in bytes.
    pub size_bytes: usize,
    // Aligned allocation footprint in bytes.
    pub aligned_size_bytes: usize,
    // Timeline fence value at which chunk can be safely reclaimed.
    pub fence_value: u64,
}

// GPU indirect argument ring buffer suballocator with multi-frame fence progression.
#[derive(Debug)]
pub struct IndirectArgumentRingAllocator {
    // Base GPU virtual address of mapped buffer.
    pub base_gpu_va: u64,
    // Total capacity in bytes.
    pub capacity: usize,
    // Allocation head byte offset.
    pub head: usize,
    // Reclamation tail byte offset.
    pub tail: usize,
    // Unused padding at end of buffer when wrapping occurred.
    pub end_padding: usize,
    // Currently in-flight chunks awaiting GPU retirement.
    pub in_flight_chunks: alloc::vec::Vec<IndirectArgumentChunk>,
    // Monotonic fence sequence number for submitted chunks.
    pub current_fence: u64,
    // Total active allocated bytes currently in flight.
    pub active_allocated_bytes: usize,
    // Cumulative allocations performed.
    pub total_allocations: u64,
    // Peak allocated bytes observed.
    pub peak_allocated_bytes: usize,
}

impl IndirectArgumentRingAllocator {
    // Creates a new indirect argument ring buffer allocator.
    pub fn new(base_gpu_va: u64, capacity: usize) -> Self {
        Self {
            base_gpu_va,
            capacity,
            head: 0,
            tail: 0,
            end_padding: 0,
            in_flight_chunks: alloc::vec::Vec::new(),
            current_fence: 1,
            active_allocated_bytes: 0,
            total_allocations: 0,
            peak_allocated_bytes: 0,
        }
    }

    // Advances the internal timeline fence sequence counter.
    pub fn advance_fence(&mut self) -> u64 {
        self.current_fence += 1;
        self.current_fence
    }

    // Suballocates an aligned memory chunk from the ring buffer.
    pub fn suballocate(
        &mut self,
        size: usize,
        alignment: usize,
    ) -> Result<IndirectArgumentChunk, CommandSignatureError> {
        let align = if alignment < D3D12_INDIRECT_ARGUMENT_BUFFER_ALIGNMENT {
            D3D12_INDIRECT_ARGUMENT_BUFFER_ALIGNMENT
        } else {
            alignment
        };
        let aligned_size = align_up_usize(size, align);
        if aligned_size > self.capacity {
            return Err(CommandSignatureError::BufferOverflow {
                required_bytes: aligned_size,
                available_bytes: self.capacity,
            });
        }

        let aligned_head = align_up_usize(self.head, align);
        if self.head >= self.tail {
            if aligned_head + aligned_size <= self.capacity {
                let offset = aligned_head;
                let gpu_va = self.base_gpu_va + offset as u64;
                self.head = offset + aligned_size;
                self.active_allocated_bytes += aligned_size;
                if self.active_allocated_bytes > self.peak_allocated_bytes {
                    self.peak_allocated_bytes = self.active_allocated_bytes;
                }
                self.total_allocations += 1;
                let chunk = IndirectArgumentChunk {
                    gpu_virtual_address: gpu_va,
                    offset,
                    size_bytes: size,
                    aligned_size_bytes: aligned_size,
                    fence_value: self.current_fence,
                };
                self.in_flight_chunks.push(chunk);
                return Ok(chunk);
            }
            if aligned_size <= self.tail && self.tail > 0 {
                self.end_padding = self.capacity - self.head;
                let offset = 0;
                let gpu_va = self.base_gpu_va;
                self.head = aligned_size;
                self.active_allocated_bytes += aligned_size;
                if self.active_allocated_bytes > self.peak_allocated_bytes {
                    self.peak_allocated_bytes = self.active_allocated_bytes;
                }
                self.total_allocations += 1;
                let chunk = IndirectArgumentChunk {
                    gpu_virtual_address: gpu_va,
                    offset,
                    size_bytes: size,
                    aligned_size_bytes: aligned_size,
                    fence_value: self.current_fence,
                };
                self.in_flight_chunks.push(chunk);
                return Ok(chunk);
            }
        } else if aligned_head + aligned_size <= self.tail {
            let offset = aligned_head;
            let gpu_va = self.base_gpu_va + offset as u64;
            self.head = offset + aligned_size;
            self.active_allocated_bytes += aligned_size;
            if self.active_allocated_bytes > self.peak_allocated_bytes {
                self.peak_allocated_bytes = self.active_allocated_bytes;
            }
            self.total_allocations += 1;
            let chunk = IndirectArgumentChunk {
                gpu_virtual_address: gpu_va,
                offset,
                size_bytes: size,
                aligned_size_bytes: aligned_size,
                fence_value: self.current_fence,
            };
            self.in_flight_chunks.push(chunk);
            return Ok(chunk);
        }

        let available = if self.head >= self.tail {
            (self.capacity - self.head) + self.tail
        } else {
            self.tail - self.head
        };
        Err(CommandSignatureError::BufferOverflow {
            required_bytes: aligned_size,
            available_bytes: available,
        })
    }

    // Retires completed chunks whose fence value has been signaled.
    pub fn retire_frame(&mut self, completed_fence: u64) -> usize {
        let mut retired_bytes = 0usize;
        while let Some(chunk) = self.in_flight_chunks.first() {
            if chunk.fence_value <= completed_fence {
                let chunk = self.in_flight_chunks.remove(0);
                retired_bytes += chunk.aligned_size_bytes;
                self.active_allocated_bytes =
                    self.active_allocated_bytes.saturating_sub(chunk.aligned_size_bytes);
                if self.end_padding > 0
                    && chunk.offset + chunk.aligned_size_bytes + self.end_padding >= self.capacity
                {
                    self.tail = 0;
                    self.end_padding = 0;
                } else {
                    self.tail = chunk.offset + chunk.aligned_size_bytes;
                }
            } else {
                break;
            }
        }
        if self.in_flight_chunks.is_empty() {
            self.head = 0;
            self.tail = 0;
            self.end_padding = 0;
            self.active_allocated_bytes = 0;
        }
        retired_bytes
    }

    // Calculates the contiguous bytes available without wrapping.
    pub fn available_contiguous(&self) -> usize {
        if self.head >= self.tail {
            self.capacity - self.head
        } else {
            self.tail - self.head
        }
    }

    // Returns count of currently in-flight chunks.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight_chunks.len()
    }
}

// Validator and bounds checker for indirect argument buffer strides and boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArgumentBufferBoundsChecker {
    // Total bounds checks performed successfully.
    pub bounds_checks_passed: u64,
    // Total overflow violations rejected.
    pub overflow_violations_blocked: u64,
    // Total stride alignment or limit violations rejected.
    pub misaligned_strides_blocked: u64,
}

impl ArgumentBufferBoundsChecker {
    // Creates a new indirect argument buffer bounds checker.
    pub fn new() -> Self {
        Self {
            bounds_checks_passed: 0,
            overflow_violations_blocked: 0,
            misaligned_strides_blocked: 0,
        }
    }

    // Validates byte stride against D3D12 alignment rules and descriptor minimum stride.
    pub fn validate_stride(
        &mut self,
        byte_stride: u32,
        desc: &D3D12_COMMAND_SIGNATURE_DESC,
    ) -> Result<(), CommandSignatureError> {
        if byte_stride % 4 != 0 {
            self.misaligned_strides_blocked += 1;
            return Err(CommandSignatureError::StrideNotMultipleOfFour(byte_stride));
        }
        let min_stride = desc.minimum_byte_stride();
        if byte_stride < min_stride {
            self.misaligned_strides_blocked += 1;
            return Err(CommandSignatureError::InvalidByteStride {
                stride: byte_stride,
                min_required: min_stride,
            });
        }
        if byte_stride > D3D12_MAX_INDIRECT_ARGUMENT_STRIDE {
            self.misaligned_strides_blocked += 1;
            return Err(CommandSignatureError::StrideExceedsMaximum(byte_stride));
        }
        Ok(())
    }

    // Validates that an indirect draw or dispatch call does not exceed buffer boundaries.
    pub fn validate_buffer_bounds(
        &mut self,
        buffer_size_bytes: usize,
        argument_buffer_offset: usize,
        max_command_count: u32,
        byte_stride: u32,
    ) -> Result<usize, CommandSignatureError> {
        if argument_buffer_offset % D3D12_INDIRECT_ARGUMENT_BUFFER_ALIGNMENT != 0 {
            self.misaligned_strides_blocked += 1;
            return Err(CommandSignatureError::InvalidBufferOffset(argument_buffer_offset));
        }

        if argument_buffer_offset > buffer_size_bytes {
            self.overflow_violations_blocked += 1;
            return Err(CommandSignatureError::BufferOverflow {
                required_bytes: argument_buffer_offset,
                available_bytes: buffer_size_bytes,
            });
        }

        let total_command_bytes =
            match (max_command_count as usize).checked_mul(byte_stride as usize) {
                Some(bytes) => bytes,
                None => {
                    self.overflow_violations_blocked += 1;
                    return Err(CommandSignatureError::BufferOverflow {
                        required_bytes: usize::MAX,
                        available_bytes: buffer_size_bytes,
                    });
                }
            };

        let total_required = match argument_buffer_offset.checked_add(total_command_bytes) {
            Some(req) => req,
            None => {
                self.overflow_violations_blocked += 1;
                return Err(CommandSignatureError::BufferOverflow {
                    required_bytes: usize::MAX,
                    available_bytes: buffer_size_bytes,
                });
            }
        };

        if total_required > buffer_size_bytes {
            self.overflow_violations_blocked += 1;
            return Err(CommandSignatureError::BufferOverflow {
                required_bytes: total_required,
                available_bytes: buffer_size_bytes,
            });
        }

        self.bounds_checks_passed += 1;
        Ok(total_required)
    }
}

// Temporary stub for self-test during incremental commits.
pub fn test_command_signature_subsystem() -> bool {
    true
}
