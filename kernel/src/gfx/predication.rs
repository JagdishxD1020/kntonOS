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

/// Errors resulting from invalid predication parameters or buffer bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredicationError {
    UnalignedBufferOffset(u64),
    BufferOutOfBounds { offset: u64, buffer_size: u64 },
    NullBufferWithActiveOperation,
    InvalidOperation(u32),
}

/// Hardware predicate alignment constant (D3D12 predicates require 8-byte alignment).
pub const D3D12_PREDICATION_BUFFER_ALIGNMENT: u64 = 8;

/// Validator for Direct3D 12 predication buffer bindings and offset alignment.
#[allow(non_camel_case_types)]
pub struct D3D12_PREDICATION_VALIDATOR;

impl D3D12_PREDICATION_VALIDATOR {
    /// Validates predication buffer parameters and ensures 8-byte alignment.
    pub fn validate_predication(
        buffer_gpu_va: u64,
        aligned_buffer_offset: u64,
        operation: D3D12_PREDICATION_OP,
        buffer_size: Option<u64>,
    ) -> Result<D3D12_PREDICATION_DESC, PredicationError> {
        // Disabling predication: null buffer GPU virtual address
        if buffer_gpu_va == 0 {
            return Ok(D3D12_PREDICATION_DESC::disabled());
        }

        // Direct3D 12 mandates 8-byte aligned offsets for 64-bit predicate values
        if aligned_buffer_offset % D3D12_PREDICATION_BUFFER_ALIGNMENT != 0 {
            return Err(PredicationError::UnalignedBufferOffset(aligned_buffer_offset));
        }

        // Bounds check against buffer size if known
        if let Some(size) = buffer_size {
            if aligned_buffer_offset.saturating_add(8) > size {
                return Err(PredicationError::BufferOutOfBounds {
                    offset: aligned_buffer_offset,
                    buffer_size: size,
                });
            }
        }

        Ok(D3D12_PREDICATION_DESC::active(buffer_gpu_va, aligned_buffer_offset, operation))
    }

    /// Verifies that an occlusion query heap result offset is properly aligned for predication.
    pub fn validate_occlusion_query_offset(query_index: u32, max_queries: u32) -> Result<u64, PredicationError> {
        if query_index >= max_queries {
            return Err(PredicationError::BufferOutOfBounds {
                offset: (query_index as u64) * 8,
                buffer_size: (max_queries as u64) * 8,
            });
        }
        Ok((query_index as u64) * 8)
    }
}

/// GPU-side predication evaluation engine simulating zero-readback occlusion skipping.
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct D3D12_PREDICATION_EVALUATOR {
    pub current_desc: D3D12_PREDICATION_DESC,
    pub draws_evaluated: u64,
    pub draws_executed: u64,
    pub draws_skipped: u64,
    pub dispatches_evaluated: u64,
    pub dispatches_executed: u64,
    pub dispatches_skipped: u64,
    pub zero_readback_stalls_avoided: u64,
    pub bytes_bandwidth_saved: u64,
}

impl D3D12_PREDICATION_EVALUATOR {
    /// Creates a new predication evaluator with predication initially disabled.
    pub const fn new() -> Self {
        Self {
            current_desc: D3D12_PREDICATION_DESC::disabled(),
            draws_evaluated: 0,
            draws_executed: 0,
            draws_skipped: 0,
            dispatches_evaluated: 0,
            dispatches_executed: 0,
            dispatches_skipped: 0,
            zero_readback_stalls_avoided: 0,
            bytes_bandwidth_saved: 0,
        }
    }

    /// Sets or updates active predication state.
    pub fn set_predication(&mut self, desc: D3D12_PREDICATION_DESC) {
        self.current_desc = desc;
    }

    /// Clears and disables current predication.
    pub fn clear_predication(&mut self) {
        self.current_desc = D3D12_PREDICATION_DESC::disabled();
    }

    /// Evaluates if commands should execute given the 64-bit predicate value.
    pub fn should_execute(&self, predicate_value: u64) -> bool {
        if !self.current_desc.is_active {
            return true;
        }

        match self.current_desc.operation {
            D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_EQUAL_ZERO => predicate_value == 0,
            D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_NOT_EQUAL_ZERO => predicate_value != 0,
        }
    }

    /// Evaluates a draw command and updates metrics.
    pub fn evaluate_draw(&mut self, predicate_value: Option<u64>) -> bool {
        self.draws_evaluated = self.draws_evaluated.saturating_add(1);

        let execute = match predicate_value {
            Some(val) => self.should_execute(val),
            None => !self.current_desc.is_active,
        };

        if execute {
            self.draws_executed = self.draws_executed.saturating_add(1);
        } else {
            self.draws_skipped = self.draws_skipped.saturating_add(1);
            self.zero_readback_stalls_avoided = self.zero_readback_stalls_avoided.saturating_add(1);
            // Estimate GPU vertex fetch and rasterization bandwidth saved per skipped draw
            self.bytes_bandwidth_saved = self.bytes_bandwidth_saved.saturating_add(65536);
        }

        execute
    }

    /// Evaluates a compute dispatch command and updates metrics.
    pub fn evaluate_dispatch(&mut self, predicate_value: Option<u64>) -> bool {
        self.dispatches_evaluated = self.dispatches_evaluated.saturating_add(1);

        let execute = match predicate_value {
            Some(val) => self.should_execute(val),
            None => !self.current_desc.is_active,
        };

        if execute {
            self.dispatches_executed = self.dispatches_executed.saturating_add(1);
        } else {
            self.dispatches_skipped = self.dispatches_skipped.saturating_add(1);
            self.zero_readback_stalls_avoided = self.zero_readback_stalls_avoided.saturating_add(1);
            self.bytes_bandwidth_saved = self.bytes_bandwidth_saved.saturating_add(131072);
        }

        execute
    }
}
