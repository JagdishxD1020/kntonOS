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

/// Token stream opcodes for predicated command buffer recording.
pub const D3D12_TOKEN_SET_PREDICATION: u32 = 0x5052_4544; // 'PRED'
pub const D3D12_TOKEN_DRAW_INSTANCED: u32 = 0x4452_574E; // 'DRWN'
pub const D3D12_TOKEN_DRAW_INDEXED_INSTANCED: u32 = 0x4452_5749; // 'DRWI'
pub const D3D12_TOKEN_DISPATCH: u32 = 0x4453_5054; // 'DSPT'

/// Concrete predicated packet recorded into GPU command stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PredicatedCommandPacket {
    SetPredication {
        buffer_gpu_va: u64,
        aligned_buffer_offset: u64,
        operation: D3D12_PREDICATION_OP,
    },
    DrawInstanced {
        vertex_count_per_instance: u32,
        instance_count: u32,
        start_vertex_location: u32,
        start_instance_location: u32,
        predicate: D3D12_PREDICATION_DESC,
    },
    DrawIndexedInstanced {
        index_count_per_instance: u32,
        instance_count: u32,
        start_index_location: u32,
        base_vertex_location: i32,
        start_instance_location: u32,
        predicate: D3D12_PREDICATION_DESC,
    },
    Dispatch {
        thread_group_count_x: u32,
        thread_group_count_y: u32,
        thread_group_count_z: u32,
        predicate: D3D12_PREDICATION_DESC,
    },
}

/// Command recorder token stream encoder for predicated draw and dispatch commands.
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct D3D12_PREDICATION_COMMAND_RECORDER {
    pub current_predication: D3D12_PREDICATION_DESC,
    pub total_commands_recorded: u32,
    pub packets: alloc::vec::Vec<PredicatedCommandPacket>,
}

impl D3D12_PREDICATION_COMMAND_RECORDER {
    /// Creates a new command recorder with predication initially disabled.
    pub fn new() -> Self {
        Self {
            current_predication: D3D12_PREDICATION_DESC::disabled(),
            total_commands_recorded: 0,
            packets: alloc::vec::Vec::new(),
        }
    }

    /// Sets or clears predication on the command list.
    pub fn set_predication(
        &mut self,
        buffer_gpu_va: u64,
        aligned_buffer_offset: u64,
        operation: D3D12_PREDICATION_OP,
        buffer_size: Option<u64>,
    ) -> Result<(), PredicationError> {
        let desc = D3D12_PREDICATION_VALIDATOR::validate_predication(
            buffer_gpu_va,
            aligned_buffer_offset,
            operation,
            buffer_size,
        )?;

        self.current_predication = desc;
        self.packets.push(PredicatedCommandPacket::SetPredication {
            buffer_gpu_va,
            aligned_buffer_offset,
            operation,
        });
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
        Ok(())
    }

    /// Records an instanced draw call subject to current predication state.
    pub fn draw_instanced(
        &mut self,
        vertex_count_per_instance: u32,
        instance_count: u32,
        start_vertex_location: u32,
        start_instance_location: u32,
    ) {
        self.packets.push(PredicatedCommandPacket::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex_location,
            start_instance_location,
            predicate: self.current_predication,
        });
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
    }

    /// Records an indexed instanced draw call subject to current predication state.
    pub fn draw_indexed_instanced(
        &mut self,
        index_count_per_instance: u32,
        instance_count: u32,
        start_index_location: u32,
        base_vertex_location: i32,
        start_instance_location: u32,
    ) {
        self.packets.push(PredicatedCommandPacket::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index_location,
            base_vertex_location,
            start_instance_location,
            predicate: self.current_predication,
        });
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
    }

    /// Records a compute dispatch command subject to current predication state.
    pub fn dispatch(&mut self, thread_group_count_x: u32, thread_group_count_y: u32, thread_group_count_z: u32) {
        self.packets.push(PredicatedCommandPacket::Dispatch {
            thread_group_count_x,
            thread_group_count_y,
            thread_group_count_z,
            predicate: self.current_predication,
        });
        self.total_commands_recorded = self.total_commands_recorded.saturating_add(1);
    }

    /// Encodes all recorded packets into a binary token stream.
    pub fn encode_token_stream(&self, stream: &mut alloc::vec::Vec<u8>) {
        for packet in &self.packets {
            match packet {
                PredicatedCommandPacket::SetPredication {
                    buffer_gpu_va,
                    aligned_buffer_offset,
                    operation,
                } => {
                    stream.extend_from_slice(&D3D12_TOKEN_SET_PREDICATION.to_le_bytes());
                    stream.extend_from_slice(&buffer_gpu_va.to_le_bytes());
                    stream.extend_from_slice(&aligned_buffer_offset.to_le_bytes());
                    stream.extend_from_slice(&(*operation as u32).to_le_bytes());
                }
                PredicatedCommandPacket::DrawInstanced {
                    vertex_count_per_instance,
                    instance_count,
                    start_vertex_location,
                    start_instance_location,
                    ..
                } => {
                    stream.extend_from_slice(&D3D12_TOKEN_DRAW_INSTANCED.to_le_bytes());
                    stream.extend_from_slice(&vertex_count_per_instance.to_le_bytes());
                    stream.extend_from_slice(&instance_count.to_le_bytes());
                    stream.extend_from_slice(&start_vertex_location.to_le_bytes());
                    stream.extend_from_slice(&start_instance_location.to_le_bytes());
                }
                PredicatedCommandPacket::DrawIndexedInstanced {
                    index_count_per_instance,
                    instance_count,
                    start_index_location,
                    base_vertex_location,
                    start_instance_location,
                    ..
                } => {
                    stream.extend_from_slice(&D3D12_TOKEN_DRAW_INDEXED_INSTANCED.to_le_bytes());
                    stream.extend_from_slice(&index_count_per_instance.to_le_bytes());
                    stream.extend_from_slice(&instance_count.to_le_bytes());
                    stream.extend_from_slice(&start_index_location.to_le_bytes());
                    stream.extend_from_slice(&base_vertex_location.to_le_bytes());
                    stream.extend_from_slice(&start_instance_location.to_le_bytes());
                }
                PredicatedCommandPacket::Dispatch {
                    thread_group_count_x,
                    thread_group_count_y,
                    thread_group_count_z,
                    ..
                } => {
                    stream.extend_from_slice(&D3D12_TOKEN_DISPATCH.to_le_bytes());
                    stream.extend_from_slice(&thread_group_count_x.to_le_bytes());
                    stream.extend_from_slice(&thread_group_count_y.to_le_bytes());
                    stream.extend_from_slice(&thread_group_count_z.to_le_bytes());
                }
            }
        }
    }

    /// Replays recorded packets through an evaluator simulating zero-readback GPU occlusion skipping.
    pub fn replay_packets<F>(&self, evaluator: &mut D3D12_PREDICATION_EVALUATOR, read_predicate_val: F) -> (u32, u32)
    where
        F: Fn(u64, u64) -> u64,
    {
        let mut executed = 0u32;
        let mut skipped = 0u32;

        for packet in &self.packets {
            match packet {
                PredicatedCommandPacket::SetPredication {
                    buffer_gpu_va,
                    aligned_buffer_offset,
                    operation,
                } => {
                    let desc = if *buffer_gpu_va == 0 {
                        D3D12_PREDICATION_DESC::disabled()
                    } else {
                        D3D12_PREDICATION_DESC::active(*buffer_gpu_va, *aligned_buffer_offset, *operation)
                    };
                    evaluator.set_predication(desc);
                }
                PredicatedCommandPacket::DrawInstanced { predicate, .. }
                | PredicatedCommandPacket::DrawIndexedInstanced { predicate, .. } => {
                    let val = if predicate.is_active {
                        Some(read_predicate_val(predicate.buffer_gpu_va, predicate.aligned_buffer_offset))
                    } else {
                        None
                    };
                    if evaluator.evaluate_draw(val) {
                        executed = executed.saturating_add(1);
                    } else {
                        skipped = skipped.saturating_add(1);
                    }
                }
                PredicatedCommandPacket::Dispatch { predicate, .. } => {
                    let val = if predicate.is_active {
                        Some(read_predicate_val(predicate.buffer_gpu_va, predicate.aligned_buffer_offset))
                    } else {
                        None
                    };
                    if evaluator.evaluate_dispatch(val) {
                        executed = executed.saturating_add(1);
                    } else {
                        skipped = skipped.saturating_add(1);
                    }
                }
            }
        }

        (executed, skipped)
    }
}

// Comprehensive verification self-test suite for Direct3D 12 Predication & Occlusion Query Skipping
pub fn test_predication_subsystem() -> bool {
    crate::serial_write_str("[KNT-GPU-PREDICATION] Initializing Direct3D 12 Predication self-test...\r\n");

    // 1. Verify buffer offset validation and 8-byte alignment verification
    let unaligned = D3D12_PREDICATION_VALIDATOR::validate_predication(
        0x5000,
        4,
        D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_NOT_EQUAL_ZERO,
        Some(1024),
    );
    if unaligned != Err(PredicationError::UnalignedBufferOffset(4)) {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] Unaligned buffer offset was not rejected\r\n");
        return false;
    }

    let out_of_bounds = D3D12_PREDICATION_VALIDATOR::validate_predication(
        0x5000,
        64,
        D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_EQUAL_ZERO,
        Some(64),
    );
    if out_of_bounds != Err(PredicationError::BufferOutOfBounds { offset: 64, buffer_size: 64 }) {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] Out of bounds buffer offset was not rejected\r\n");
        return false;
    }

    let disabled_pred = D3D12_PREDICATION_VALIDATOR::validate_predication(
        0,
        0,
        D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_EQUAL_ZERO,
        None,
    );
    if disabled_pred != Ok(D3D12_PREDICATION_DESC::disabled()) {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] Disabling predication with null VA failed\r\n");
        return false;
    }

    let query_offset = D3D12_PREDICATION_VALIDATOR::validate_occlusion_query_offset(5, 16);
    if query_offset != Ok(40) {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] Occlusion query offset calculation mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Predication buffer offset validation and 8-byte alignment verified\r\n");

    // 2. Verify predication evaluation engine and condition checks
    let mut evaluator = D3D12_PREDICATION_EVALUATOR::new();

    // NOT_EQUAL_ZERO condition: passes when visible (pixels > 0), skips when occluded (pixels == 0)
    let not_equal_desc = D3D12_PREDICATION_DESC::active(
        0x6000,
        0,
        D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_NOT_EQUAL_ZERO,
    );
    evaluator.set_predication(not_equal_desc);

    if evaluator.should_execute(0) {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] NOT_EQUAL_ZERO condition should skip when predicate is 0\r\n");
        return false;
    }
    if !evaluator.should_execute(512) {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] NOT_EQUAL_ZERO condition should execute when predicate > 0\r\n");
        return false;
    }

    // EQUAL_ZERO condition: passes when occluded (0), skips when visible (!= 0)
    let equal_desc = D3D12_PREDICATION_DESC::active(
        0x6000,
        8,
        D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_EQUAL_ZERO,
    );
    evaluator.set_predication(equal_desc);

    if !evaluator.should_execute(0) {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] EQUAL_ZERO condition should execute when predicate is 0\r\n");
        return false;
    }
    if evaluator.should_execute(1024) {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] EQUAL_ZERO condition should skip when predicate != 0\r\n");
        return false;
    }

    evaluator.clear_predication();
    if !evaluator.should_execute(0) || !evaluator.should_execute(999) {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] Disabled predication should execute unconditionally\r\n");
        return false;
    }
    crate::serial_write_str("  * Predication evaluation engine and GPU zero-readback condition checks verified\r\n");

    // 3. Verify command recorder token stream and occlusion query execution replay
    let mut recorder = D3D12_PREDICATION_COMMAND_RECORDER::new();

    // Object 1: Predicated on query 0 (offset 0)
    if recorder.set_predication(0x8000, 0, D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_NOT_EQUAL_ZERO, Some(256)).is_err() {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] SetPredication for object 1 failed\r\n");
        return false;
    }
    recorder.draw_instanced(3, 1, 0, 0);

    // Object 2: Predicated on query 1 (offset 8)
    if recorder.set_predication(0x8000, 8, D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_NOT_EQUAL_ZERO, Some(256)).is_err() {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] SetPredication for object 2 failed\r\n");
        return false;
    }
    recorder.draw_indexed_instanced(36, 1, 0, 0, 0);

    // Unconditional compute dispatch: Predication cleared
    if recorder.set_predication(0, 0, D3D12_PREDICATION_OP::D3D12_PREDICATION_OP_EQUAL_ZERO, None).is_err() {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] Clear predication failed\r\n");
        return false;
    }
    recorder.dispatch(8, 8, 1);

    if recorder.total_commands_recorded != 6 {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] Command recorder total packet count mismatch\r\n");
        return false;
    }

    // Binary token stream encoding
    let mut stream = alloc::vec::Vec::new();
    recorder.encode_token_stream(&mut stream);
    if stream.is_empty() {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] Token stream encoding is empty\r\n");
        return false;
    }
    let first_token = u32::from_le_bytes([stream[0], stream[1], stream[2], stream[3]]);
    if first_token != D3D12_TOKEN_SET_PREDICATION {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] First token opcode is not SET_PREDICATION\r\n");
        return false;
    }

    // Replay simulation: query 0 = 0 (occluded), query 1 = 1500 (visible)
    let mut replay_eval = D3D12_PREDICATION_EVALUATOR::new();
    let (exec_count, skip_count) = recorder.replay_packets(&mut replay_eval, |_buf, offset| {
        if offset == 0 {
            0 // Occluded
        } else if offset == 8 {
            1500 // Visible
        } else {
            0
        }
    });

    if exec_count != 2 || skip_count != 1 {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] Replay execution/skip counts mismatch\r\n");
        return false;
    }
    if replay_eval.zero_readback_stalls_avoided != 1 || replay_eval.bytes_bandwidth_saved == 0 {
        crate::serial_write_str("[KNT-PREDICATION-FAIL] Zero-readback stalls avoided or bandwidth savings not tracked\r\n");
        return false;
    }
    crate::serial_write_str("  * Command recorder token stream encoder and occlusion skipping replay verified\r\n");

    // 4. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-GPU-PREDICATION-PASS] Direct3D 12 predication and GPU occlusion query skipping engine verified\r\n");
    true
}
