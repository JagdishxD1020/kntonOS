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

// Pre-compiled GPU command list bundle for high-frequency multi-draw execution
pub struct D3D12GraphicsCommandListBundle {
    pub allocator: BundleAllocator,
    pub commands: Vec<BundleCommand>,
    pub is_closed: bool,
    pub active_pso_id: Option<u64>,
    pub active_root_signature_id: Option<u64>,
    pub vsv_slots: [Option<D3D12_VERTEX_BUFFER_VIEW>; D3D12_IA_VERTEX_INPUT_RESOURCE_SLOT_COUNT],
    pub ibv: Option<D3D12_INDEX_BUFFER_VIEW>,
    pub recorded_draw_count: usize,
    pub recorded_dispatch_count: usize,
}

impl D3D12GraphicsCommandListBundle {
    // Construct empty bundle command list with memory allocation quota
    pub fn new(allocator_capacity: usize, initial_pso: Option<u64>) -> Self {
        Self {
            allocator: BundleAllocator::new(allocator_capacity),
            commands: Vec::new(),
            is_closed: false,
            active_pso_id: initial_pso,
            active_root_signature_id: None,
            vsv_slots: [None; D3D12_IA_VERTEX_INPUT_RESOURCE_SLOT_COUNT],
            ibv: None,
            recorded_draw_count: 0,
            recorded_dispatch_count: 0,
        }
    }

    // Reset bundle recorder state and re-open command stream for writing
    pub fn reset(&mut self, initial_pso: Option<u64>) {
        self.allocator.reset();
        self.commands.clear();
        self.is_closed = false;
        self.active_pso_id = initial_pso;
        self.active_root_signature_id = None;
        self.vsv_slots = [None; D3D12_IA_VERTEX_INPUT_RESOURCE_SLOT_COUNT];
        self.ibv = None;
        self.recorded_draw_count = 0;
        self.recorded_dispatch_count = 0;
    }

    // Record DrawIndexedInstanced draw call into the bundle stream
    pub fn draw_indexed_instanced(
        &mut self,
        index_count_per_instance: u32,
        instance_count: u32,
        start_index_location: u32,
        base_vertex_location: i32,
        start_instance_location: u32,
    ) -> Result<(), BundleError> {
        if self.is_closed {
            return Err(BundleError::BundleAlreadyClosed);
        }
        self.allocator.track_allocation(32)?;
        self.commands.push(BundleCommand::DrawIndexedInstanced {
            index_count_per_instance,
            instance_count,
            start_index_location,
            base_vertex_location,
            start_instance_location,
        });
        self.recorded_draw_count += 1;
        Ok(())
    }

    // Record compute dispatch call into the bundle stream
    pub fn dispatch(
        &mut self,
        thread_group_count_x: u32,
        thread_group_count_y: u32,
        thread_group_count_z: u32,
    ) -> Result<(), BundleError> {
        if self.is_closed {
            return Err(BundleError::BundleAlreadyClosed);
        }
        self.allocator.track_allocation(24)?;
        self.commands.push(BundleCommand::Dispatch {
            thread_group_count_x,
            thread_group_count_y,
            thread_group_count_z,
        });
        self.recorded_dispatch_count += 1;
        Ok(())
    }

    // Bind descriptor table to graphics root signature parameter slot
    pub fn set_graphics_root_descriptor_table(
        &mut self,
        root_parameter_index: u32,
        base_descriptor_gpu_address: u64,
    ) -> Result<(), BundleError> {
        if self.is_closed {
            return Err(BundleError::BundleAlreadyClosed);
        }
        self.allocator.track_allocation(16)?;
        self.commands.push(BundleCommand::SetGraphicsRootDescriptorTable {
            root_parameter_index,
            base_descriptor_gpu_address,
        });
        Ok(())
    }

    // Push inline 32-bit constants into graphics root signature parameter slot
    pub fn set_graphics_root_32bit_constants(
        &mut self,
        root_parameter_index: u32,
        num_32bit_values_to_set: u32,
        dest_offset_in_32bit_values: u32,
        values: &[u32],
    ) -> Result<(), BundleError> {
        if self.is_closed {
            return Err(BundleError::BundleAlreadyClosed);
        }
        let payload_size = 16 + (values.len() * 4);
        self.allocator.track_allocation(payload_size)?;
        self.commands.push(BundleCommand::SetGraphicsRoot32BitConstants {
            root_parameter_index,
            num_32bit_values_to_set,
            dest_offset_in_32bit_values,
            values: values.to_vec(),
        });
        Ok(())
    }

    // Bind array of vertex buffer views to input assembler slots
    pub fn ia_set_vertex_buffers(
        &mut self,
        start_slot: u32,
        views: &[D3D12_VERTEX_BUFFER_VIEW],
    ) -> Result<(), BundleError> {
        if self.is_closed {
            return Err(BundleError::BundleAlreadyClosed);
        }
        let end_slot = start_slot as usize + views.len();
        if end_slot > D3D12_IA_VERTEX_INPUT_RESOURCE_SLOT_COUNT {
            return Err(BundleError::InvalidSlotIndex);
        }

        let alloc_size = 8 + (views.len() * 16);
        self.allocator.track_allocation(alloc_size)?;

        for (i, view) in views.iter().enumerate() {
            self.vsv_slots[start_slot as usize + i] = Some(*view);
        }

        self.commands.push(BundleCommand::IASetVertexBuffers {
            start_slot,
            views: views.to_vec(),
        });
        Ok(())
    }

    // Bind index buffer view to input assembler
    pub fn ia_set_index_buffer(&mut self, view: &D3D12_INDEX_BUFFER_VIEW) -> Result<(), BundleError> {
        if self.is_closed {
            return Err(BundleError::BundleAlreadyClosed);
        }
        self.allocator.track_allocation(16)?;
        self.ibv = Some(*view);
        self.commands.push(BundleCommand::IASetIndexBuffer { view: *view });
        Ok(())
    }

    // Set pipeline state object within the bundle
    pub fn set_pipeline_state(&mut self, pso_id: u64) -> Result<(), BundleError> {
        if self.is_closed {
            return Err(BundleError::BundleAlreadyClosed);
        }
        self.allocator.track_allocation(16)?;
        self.active_pso_id = Some(pso_id);
        self.commands.push(BundleCommand::SetPipelineState { pso_id });
        Ok(())
    }

    // Set graphics root signature within the bundle
    pub fn set_graphics_root_signature(&mut self, root_signature_id: u64) -> Result<(), BundleError> {
        if self.is_closed {
            return Err(BundleError::BundleAlreadyClosed);
        }
        self.allocator.track_allocation(16)?;
        self.active_root_signature_id = Some(root_signature_id);
        self.commands.push(BundleCommand::SetGraphicsRootSignature { root_signature_id });
        Ok(())
    }

    // Finalize bundle recording and lock stream against further command writes
    pub fn close(&mut self) -> Result<(), BundleError> {
        if self.is_closed {
            return Err(BundleError::BundleAlreadyClosed);
        }
        self.is_closed = true;
        Ok(())
    }

    // Attempting to record ClearRenderTargetView in bundle must be rejected
    pub fn record_clear_render_target_view(&mut self) -> Result<(), BundleError> {
        Err(BundleError::ForbiddenCommandInBundle)
    }

    // Attempting to record ResourceBarrier in bundle must be rejected
    pub fn record_resource_barrier(&mut self) -> Result<(), BundleError> {
        Err(BundleError::ForbiddenCommandInBundle)
    }

    // Attempting to record CopyResource in bundle must be rejected
    pub fn record_copy_resource(&mut self) -> Result<(), BundleError> {
        Err(BundleError::ForbiddenCommandInBundle)
    }

    // Attempting to nest ExecuteBundle within a bundle must be rejected
    pub fn record_execute_bundle(&mut self) -> Result<(), BundleError> {
        Err(BundleError::ForbiddenCommandInBundle)
    }

    // Validate and resolve inherited pipeline state and root signature from parent
    pub fn validate_and_resolve_inheritance(
        &self,
        parent_pso: Option<u64>,
        parent_root_sig: Option<u64>,
    ) -> Result<BundleInheritanceContext, BundleError> {
        let effective_pso_id = match (self.active_pso_id, parent_pso) {
            (Some(id), _) => id,
            (None, Some(id)) => id,
            (None, None) => return Err(BundleError::MissingParentPipelineState),
        };

        let effective_root_sig_id = match (self.active_root_signature_id, parent_root_sig) {
            (Some(bundle_sig), Some(parent_sig)) => {
                if bundle_sig != parent_sig {
                    return Err(BundleError::IncompatibleRootSignature);
                }
                bundle_sig
            }
            (Some(bundle_sig), None) => bundle_sig,
            (None, Some(parent_sig)) => parent_sig,
            (None, None) => return Err(BundleError::MissingParentRootSignature),
        };

        Ok(BundleInheritanceContext {
            effective_pso_id,
            effective_root_signature_id: effective_root_sig_id,
            inherited_pso: self.active_pso_id.is_none(),
            inherited_root_signature: self.active_root_signature_id.is_none(),
        })
    }
}

// Resolved bundle execution state after inheritance validation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundleInheritanceContext {
    pub effective_pso_id: u64,
    pub effective_root_signature_id: u64,
    pub inherited_pso: bool,
    pub inherited_root_signature: bool,
}

// Forbidden command list operations inside bundles
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForbiddenBundleOperation {
    ClearRenderTargetView,
    ClearDepthStencilView,
    ResourceBarrier,
    CopyResource,
    ResolveSubresource,
    ExecuteBundle,
}

// Helper validating whether a command is forbidden in D3D12 bundle context
pub fn validate_bundle_operation_legality(op: ForbiddenBundleOperation) -> Result<(), BundleError> {
    let _ = op;
    Err(BundleError::ForbiddenCommandInBundle)
}

// Low-overhead pre-compiled GPU DMA hardware packet representation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecompiledGpuPacket {
    pub opcode: u16,
    pub payload: Vec<u8>,
}

// Pre-compiled GPU command packet stream ready for instant multi-draw replay
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecompiledBundle {
    pub bundle_id: u64,
    pub packets: Vec<PrecompiledGpuPacket>,
    pub total_dwords: usize,
    pub draw_count: usize,
    pub dispatch_count: usize,
    pub required_pso: Option<u64>,
    pub required_root_signature: Option<u64>,
}

// Pre-compilation engine baking recorded high-level bundle commands into hardware DMA packets
pub struct BundlePrecompiler;

impl BundlePrecompiler {
    // Pre-compile closed bundle into hardware-ready packet cache
    pub fn compile(bundle: &D3D12GraphicsCommandListBundle, bundle_id: u64) -> Result<PrecompiledBundle, BundleError> {
        if !bundle.is_closed {
            return Err(BundleError::BundleNotClosed);
        }

        let mut packets = Vec::with_capacity(bundle.commands.len());
        let mut total_dwords = 0;

        for cmd in &bundle.commands {
            match cmd {
                BundleCommand::DrawIndexedInstanced {
                    index_count_per_instance,
                    instance_count,
                    start_index_location,
                    base_vertex_location,
                    start_instance_location,
                } => {
                    let mut payload = Vec::with_capacity(20);
                    payload.extend_from_slice(&index_count_per_instance.to_le_bytes());
                    payload.extend_from_slice(&instance_count.to_le_bytes());
                    payload.extend_from_slice(&start_index_location.to_le_bytes());
                    payload.extend_from_slice(&base_vertex_location.to_le_bytes());
                    payload.extend_from_slice(&start_instance_location.to_le_bytes());
                    total_dwords += 5;
                    packets.push(PrecompiledGpuPacket { opcode: 1, payload });
                }
                BundleCommand::Dispatch {
                    thread_group_count_x,
                    thread_group_count_y,
                    thread_group_count_z,
                } => {
                    let mut payload = Vec::with_capacity(12);
                    payload.extend_from_slice(&thread_group_count_x.to_le_bytes());
                    payload.extend_from_slice(&thread_group_count_y.to_le_bytes());
                    payload.extend_from_slice(&thread_group_count_z.to_le_bytes());
                    total_dwords += 3;
                    packets.push(PrecompiledGpuPacket { opcode: 12, payload });
                }
                BundleCommand::SetGraphicsRootDescriptorTable {
                    root_parameter_index,
                    base_descriptor_gpu_address,
                } => {
                    let mut payload = Vec::with_capacity(12);
                    payload.extend_from_slice(&root_parameter_index.to_le_bytes());
                    payload.extend_from_slice(&base_descriptor_gpu_address.to_le_bytes());
                    total_dwords += 3;
                    packets.push(PrecompiledGpuPacket { opcode: 11, payload });
                }
                BundleCommand::SetGraphicsRoot32BitConstants {
                    root_parameter_index,
                    num_32bit_values_to_set,
                    dest_offset_in_32bit_values,
                    values,
                } => {
                    let mut payload = Vec::with_capacity(12 + values.len() * 4);
                    payload.extend_from_slice(&root_parameter_index.to_le_bytes());
                    payload.extend_from_slice(&num_32bit_values_to_set.to_le_bytes());
                    payload.extend_from_slice(&dest_offset_in_32bit_values.to_le_bytes());
                    for v in values {
                        payload.extend_from_slice(&v.to_le_bytes());
                    }
                    total_dwords += 3 + values.len();
                    packets.push(PrecompiledGpuPacket { opcode: 13, payload });
                }
                BundleCommand::IASetVertexBuffers { start_slot, views } => {
                    let mut payload = Vec::with_capacity(8 + views.len() * 16);
                    payload.extend_from_slice(&start_slot.to_le_bytes());
                    payload.extend_from_slice(&(views.len() as u32).to_le_bytes());
                    for v in views {
                        payload.extend_from_slice(&v.buffer_location.to_le_bytes());
                        payload.extend_from_slice(&v.size_in_bytes.to_le_bytes());
                        payload.extend_from_slice(&v.stride_in_bytes.to_le_bytes());
                    }
                    total_dwords += 2 + (views.len() * 4);
                    packets.push(PrecompiledGpuPacket { opcode: 8, payload });
                }
                BundleCommand::IASetIndexBuffer { view } => {
                    let mut payload = Vec::with_capacity(16);
                    payload.extend_from_slice(&view.buffer_location.to_le_bytes());
                    payload.extend_from_slice(&view.size_in_bytes.to_le_bytes());
                    payload.extend_from_slice(&(view.format as u32).to_le_bytes());
                    total_dwords += 4;
                    packets.push(PrecompiledGpuPacket { opcode: 7, payload });
                }
                BundleCommand::SetPipelineState { pso_id } => {
                    let mut payload = Vec::with_capacity(8);
                    payload.extend_from_slice(&pso_id.to_le_bytes());
                    total_dwords += 2;
                    packets.push(PrecompiledGpuPacket { opcode: 2, payload });
                }
                BundleCommand::SetGraphicsRootSignature { root_signature_id } => {
                    let mut payload = Vec::with_capacity(8);
                    payload.extend_from_slice(&root_signature_id.to_le_bytes());
                    total_dwords += 2;
                    packets.push(PrecompiledGpuPacket { opcode: 14, payload });
                }
            }
        }

        Ok(PrecompiledBundle {
            bundle_id,
            packets,
            total_dwords,
            draw_count: bundle.recorded_draw_count,
            dispatch_count: bundle.recorded_dispatch_count,
            required_pso: bundle.active_pso_id,
            required_root_signature: bundle.active_root_signature_id,
        })
    }
}

// Parent direct command list simulator executing bundles via zero-translation stream splice
pub struct DirectCommandListExecutor {
    pub active_pso_id: Option<u64>,
    pub active_root_signature_id: Option<u64>,
    pub executed_bundles_count: usize,
    pub total_replayed_draws: usize,
    pub dispatched_packets: Vec<PrecompiledGpuPacket>,
}

impl DirectCommandListExecutor {
    // Construct new direct command list executor
    pub fn new() -> Self {
        Self {
            active_pso_id: None,
            active_root_signature_id: None,
            executed_bundles_count: 0,
            total_replayed_draws: 0,
            dispatched_packets: Vec::new(),
        }
    }

    // Set active pipeline state on direct command list
    pub fn set_pipeline_state(&mut self, pso_id: u64) {
        self.active_pso_id = Some(pso_id);
    }

    // Set active root signature on direct command list
    pub fn set_graphics_root_signature(&mut self, root_sig_id: u64) {
        self.active_root_signature_id = Some(root_sig_id);
    }

    // Execute bundle via ExecuteBundle dispatching pre-compiled packets
    pub fn execute_bundle(
        &mut self,
        bundle: &D3D12GraphicsCommandListBundle,
        precompiled: &PrecompiledBundle,
    ) -> Result<BundleInheritanceContext, BundleError> {
        if !bundle.is_closed {
            return Err(BundleError::BundleNotClosed);
        }

        // Validate and resolve state inheritance
        let inheritance = bundle.validate_and_resolve_inheritance(
            self.active_pso_id,
            self.active_root_signature_id,
        )?;

        // Splice pre-compiled packets into parent execution stream
        for pkt in &precompiled.packets {
            self.dispatched_packets.push(pkt.clone());
        }

        self.executed_bundles_count += 1;
        self.total_replayed_draws += precompiled.draw_count;

        Ok(inheritance)
    }

    // Reset direct command list state
    pub fn reset(&mut self) {
        self.active_pso_id = None;
        self.active_root_signature_id = None;
        self.executed_bundles_count = 0;
        self.total_replayed_draws = 0;
        self.dispatched_packets.clear();
    }
}

// Comprehensive verification self-test suite for D3D12 Bundle Pre-Compilation Engine
pub fn test_bundle_subsystem() -> bool {
    crate::serial_write_str("[KNT-GFX] Testing Direct3D 12 Bundle Recording & Pre-Compilation Engine...\r\n");

    // 1. Verify bundle creation, allocation tracking, and legal command recording
    let mut bundle = D3D12GraphicsCommandListBundle::new(4096, None);

    let vb_views = [
        D3D12_VERTEX_BUFFER_VIEW {
            buffer_location: 0x1000_0000,
            size_in_bytes: 1024,
            stride_in_bytes: 32,
        },
        D3D12_VERTEX_BUFFER_VIEW {
            buffer_location: 0x1000_1000,
            size_in_bytes: 512,
            stride_in_bytes: 16,
        },
    ];
    if bundle.ia_set_vertex_buffers(0, &vb_views).is_err() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Failed to record IASetVertexBuffers\r\n");
        return false;
    }

    let ib_view = D3D12_INDEX_BUFFER_VIEW {
        buffer_location: 0x2000_0000,
        size_in_bytes: 2048,
        format: DXGI_FORMAT::R32_FLOAT,
    };
    if bundle.ia_set_index_buffer(&ib_view).is_err() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Failed to record IASetIndexBuffer\r\n");
        return false;
    }

    let const_values = [0x11223344, 0x55667788, 0x99AABBCC, 0xDDEEFF00];
    if bundle.set_graphics_root_32bit_constants(0, 4, 0, &const_values).is_err() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Failed to record SetGraphicsRoot32BitConstants\r\n");
        return false;
    }

    if bundle.set_graphics_root_descriptor_table(1, 0x3000_0000).is_err() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Failed to record SetGraphicsRootDescriptorTable\r\n");
        return false;
    }

    if bundle.draw_indexed_instanced(36, 1, 0, 0, 0).is_err()
        || bundle.draw_indexed_instanced(36, 1, 36, 0, 0).is_err()
    {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Failed to record DrawIndexedInstanced\r\n");
        return false;
    }

    if bundle.recorded_draw_count != 2 {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Recorded draw call count mismatch\r\n");
        return false;
    }
    let (allocated, capacity, cmd_count) = bundle.allocator.memory_usage();
    if allocated == 0 || allocated > capacity || cmd_count != 6 {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Allocator memory usage metrics mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Bundle command recording and linear allocation metrics verified\r\n");

    // 2. Verify strict rejection of forbidden operations inside bundles
    if bundle.record_clear_render_target_view().is_ok()
        || bundle.record_resource_barrier().is_ok()
        || bundle.record_copy_resource().is_ok()
        || bundle.record_execute_bundle().is_ok()
    {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Forbidden commands must be strictly rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * Strict validation and rejection of forbidden bundle commands verified\r\n");

    // 3. Verify pre-compilation lock and packet compilation
    if BundlePrecompiler::compile(&bundle, 1001).is_ok() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Compiling unclosed bundle must fail\r\n");
        return false;
    }

    if bundle.close().is_err() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Failed to close bundle\r\n");
        return false;
    }

    if bundle.draw_indexed_instanced(12, 1, 0, 0, 0).is_ok() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Writing commands to closed bundle must fail\r\n");
        return false;
    }

    let precompiled = match BundlePrecompiler::compile(&bundle, 1001) {
        Ok(p) => p,
        Err(_) => {
            crate::serial_write_str("[KNT-BUNDLE-FAIL] Failed to pre-compile closed bundle\r\n");
            return false;
        }
    };
    if precompiled.packets.len() != 6 || precompiled.draw_count != 2 || precompiled.total_dwords == 0 {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Pre-compiled packet count mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Fast pre-compilation and GPU DMA packet baking verified\r\n");

    // 4. Verify state inheritance rules during ExecuteBundle dispatch
    let mut direct_executor = DirectCommandListExecutor::new();

    // Executing without parent PSO must fail because bundle does not set its own
    if direct_executor.execute_bundle(&bundle, &precompiled).is_ok() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] ExecuteBundle without PSO must be rejected\r\n");
        return false;
    }

    direct_executor.set_pipeline_state(500);
    // Executing without parent Root Signature must fail
    if direct_executor.execute_bundle(&bundle, &precompiled).is_ok() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] ExecuteBundle without Root Signature must be rejected\r\n");
        return false;
    }

    direct_executor.set_graphics_root_signature(600);
    let inheritance = match direct_executor.execute_bundle(&bundle, &precompiled) {
        Ok(inh) => inh,
        Err(_) => {
            crate::serial_write_str("[KNT-BUNDLE-FAIL] ExecuteBundle failed with valid inheritance\r\n");
            return false;
        }
    };

    if !inheritance.inherited_pso || !inheritance.inherited_root_signature {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Bundle inheritance resolution flag mismatch\r\n");
        return false;
    }
    if direct_executor.executed_bundles_count != 1 || direct_executor.total_replayed_draws != 2 {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Replayed draw count mismatch after first execution\r\n");
        return false;
    }

    // Low-overhead multi-draw replay of the same pre-compiled bundle
    if direct_executor.execute_bundle(&bundle, &precompiled).is_err() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Second bundle replay execution failed\r\n");
        return false;
    }
    if direct_executor.executed_bundles_count != 2 || direct_executor.total_replayed_draws != 4 {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Multi-draw replay metrics mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * ExecuteBundle dispatch, state inheritance, and multi-draw replay verified\r\n");

    // 5. Verify root signature incompatibility detection
    let mut incompatible_bundle = D3D12GraphicsCommandListBundle::new(1024, Some(500));
    let _ = incompatible_bundle.set_graphics_root_signature(777); // Mismatch with parent (600)
    let _ = incompatible_bundle.close();
    let incomp_precompiled = BundlePrecompiler::compile(&incompatible_bundle, 1002).unwrap();

    if direct_executor.execute_bundle(&incompatible_bundle, &incomp_precompiled).is_ok() {
        crate::serial_write_str("[KNT-BUNDLE-FAIL] Incompatible root signature must be rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * Root signature compatibility verification across bundle boundary verified\r\n");

    crate::serial_write_str("[KNT-BUNDLE-PASS] Direct3D 12 bundle pre-compilation engine verified\r\n");
    true
}
