//! Direct3D 12 Raytracing (DXR) Acceleration Structure & DispatchRays Engine.
//! Provides hardware BVH construction, instance descriptors, SBT management, and ray dispatch.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use alloc::vec::Vec;
use crate::gfx::dispatch_rays::{
    D3D12_DISPATCH_RAYS_DESC, D3D12_GPU_VIRTUAL_ADDRESS_RANGE, D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE,
};
use crate::gfx::dxr_as::D3D12_RAYTRACING_INSTANCE_DESC;

// Acceleration structure memory alignment requirement in GPU virtual memory
pub const D3D12_RAYTRACING_ACCELERATION_STRUCTURE_ALIGNMENT: u64 = 64;

// Instance descriptor buffer memory alignment
pub const D3D12_RAYTRACING_INSTANCE_DESC_ALIGNMENT: u64 = 16;

// Acceleration structure hierarchy category
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE {
    TopLevel = 0,
    BottomLevel = 1,
}

// Raytracing acceleration structure build optimization and update flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAGS(pub u32);

impl D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAGS {
    pub const NONE: Self = Self(0);
    pub const ALLOW_UPDATE: Self = Self(1);
    pub const ALLOW_COMPACTION: Self = Self(2);
    pub const PREFER_FAST_TRACE: Self = Self(4);
    pub const PREFER_FAST_BUILD: Self = Self(8);
    pub const MINIMIZE_MEMORY: Self = Self(16);

    // Combine build flags
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    // Check if flag is set
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

// Geometry primitive type for Bottom-Level Acceleration Structure (BLAS)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_RAYTRACING_GEOMETRY_TYPE {
    Triangles = 0,
    Aabbs = 1,
}

// Geometry transparency and any-hit invocation flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct D3D12_RAYTRACING_GEOMETRY_FLAGS(pub u32);

impl D3D12_RAYTRACING_GEOMETRY_FLAGS {
    pub const NONE: Self = Self(0);
    pub const OPAQUE: Self = Self(1);
    pub const NO_DUPLICATE_ANYHIT_INVOCATION: Self = Self(2);
}

// Triangle mesh geometry description for BLAS leaf generation
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
    pub transform3x4_gpu_va: u64,
    pub index_format: u32,
    pub vertex_format: u32,
    pub index_count: u32,
    pub vertex_count: u32,
    pub index_buffer_gpu_va: u64,
    pub vertex_buffer_gpu_va: u64,
    pub vertex_stride_bytes: u64,
}

impl D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC {
    // Construct triangle mesh descriptor
    pub const fn new(
        vertex_buffer_gpu_va: u64,
        vertex_count: u32,
        vertex_stride_bytes: u64,
        index_buffer_gpu_va: u64,
        index_count: u32,
    ) -> Self {
        Self {
            transform3x4_gpu_va: 0,
            index_format: 42, // DXGI_FORMAT_R32_UINT
            vertex_format: 6, // DXGI_FORMAT_R32G32B32_FLOAT
            index_count,
            vertex_count,
            index_buffer_gpu_va,
            vertex_buffer_gpu_va,
            vertex_stride_bytes,
        }
    }

    // Number of primitive triangles
    pub const fn primitive_count(&self) -> u32 {
        if self.index_count > 0 {
            self.index_count / 3
        } else {
            self.vertex_count / 3
        }
    }
}

// Axis-aligned bounding box definition
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct D3D12_RAYTRACING_AABB {
    pub min_x: f32,
    pub min_y: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_y: f32,
    pub max_z: f32,
}

// Procedural AABB geometry description for custom intersection shaders
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_RAYTRACING_GEOMETRY_AABBS_DESC {
    pub aabb_count: u64,
    pub aabb_gpu_va: u64,
    pub stride_in_bytes: u64,
}

// BLAS geometric primitive input descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct D3D12_RAYTRACING_GEOMETRY_DESC {
    pub geometry_type: D3D12_RAYTRACING_GEOMETRY_TYPE,
    pub flags: D3D12_RAYTRACING_GEOMETRY_FLAGS,
    pub triangles: Option<D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC>,
    pub aabbs: Option<D3D12_RAYTRACING_GEOMETRY_AABBS_DESC>,
}

impl D3D12_RAYTRACING_GEOMETRY_DESC {
    // Construct triangle mesh geometry
    pub const fn triangles(
        flags: D3D12_RAYTRACING_GEOMETRY_FLAGS,
        triangles: D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC,
    ) -> Self {
        Self {
            geometry_type: D3D12_RAYTRACING_GEOMETRY_TYPE::Triangles,
            flags,
            triangles: Some(triangles),
            aabbs: None,
        }
    }

    // Construct AABB bounding geometry
    pub const fn aabbs(
        flags: D3D12_RAYTRACING_GEOMETRY_FLAGS,
        aabbs: D3D12_RAYTRACING_GEOMETRY_AABBS_DESC,
    ) -> Self {
        Self {
            geometry_type: D3D12_RAYTRACING_GEOMETRY_TYPE::Aabbs,
            flags,
            triangles: None,
            aabbs: Some(aabbs),
        }
    }
}

// D3D12_RAYTRACING_INSTANCE_DESC imported from crate::gfx::dxr_as

// Build inputs describing acceleration structure geometry or instance list
#[derive(Debug, Clone, PartialEq)]
pub struct D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
    pub as_type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE,
    pub flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAGS,
    pub num_descs: u32,
    pub geometries: Vec<D3D12_RAYTRACING_GEOMETRY_DESC>,
    pub instance_descs_gpu_va: u64,
    pub instances: Vec<D3D12_RAYTRACING_INSTANCE_DESC>,
}

impl D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS {
    // Construct BLAS inputs from geometry descriptions
    pub fn blas(
        flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAGS,
        geometries: Vec<D3D12_RAYTRACING_GEOMETRY_DESC>,
    ) -> Self {
        let num_descs = geometries.len() as u32;
        Self {
            as_type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE::BottomLevel,
            flags,
            num_descs,
            geometries,
            instance_descs_gpu_va: 0,
            instances: Vec::new(),
        }
    }

    // Construct TLAS inputs from instance descriptions
    pub fn tlas(
        flags: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAGS,
        instances: Vec<D3D12_RAYTRACING_INSTANCE_DESC>,
        instance_descs_gpu_va: u64,
    ) -> Self {
        let num_descs = instances.len() as u32;
        Self {
            as_type: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE::TopLevel,
            flags,
            num_descs,
            geometries: Vec::new(),
            instance_descs_gpu_va,
            instances,
        }
    }
}

// Acceleration structure build command descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
    pub dest_acceleration_structure_data: u64,
    pub inputs: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS,
    pub source_acceleration_structure_data: u64,
    pub scratch_acceleration_structure_data: u64,
}

// Errors produced during DXR acceleration structure building and dispatch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxrError {
    InvalidAlignment,
    InvalidGeometryType,
    InvalidPrimitiveCount,
    InvalidTransform,
    BufferTooSmall,
    InvalidShaderBindingTable,
    InvalidDispatchDimensions,
    MissingRayGenRecord,
}

// Prebuild sizing information for acceleration structure allocation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO {
    pub result_data_max_size_in_bytes: u64,
    pub scratch_data_size_in_bytes: u64,
    pub update_scratch_data_size_in_bytes: u64,
}

// Calculate prebuild memory requirements for acceleration structure inputs
pub fn get_raytracing_acceleration_structure_prebuild_info(
    inputs: &D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS,
) -> Result<D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO, DxrError> {
    match inputs.as_type {
        D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE::BottomLevel => {
            let mut total_primitives: u64 = 0;
            for geom in &inputs.geometries {
                match geom.geometry_type {
                    D3D12_RAYTRACING_GEOMETRY_TYPE::Triangles => {
                        let tri = geom.triangles.as_ref().ok_or(DxrError::InvalidGeometryType)?;
                        total_primitives += tri.primitive_count() as u64;
                    }
                    D3D12_RAYTRACING_GEOMETRY_TYPE::Aabbs => {
                        let aabb = geom.aabbs.as_ref().ok_or(DxrError::InvalidGeometryType)?;
                        total_primitives += aabb.aabb_count;
                    }
                }
            }
            if total_primitives == 0 {
                return Err(DxrError::InvalidPrimitiveCount);
            }
            let node_size = 64u64;
            let leaf_size = 64u64;
            let raw_result = 256u64 + total_primitives * (node_size + leaf_size);
            let aligned_result = (raw_result + 63) & !63;
            let raw_scratch = total_primitives * 128u64 + 512;
            let aligned_scratch = (raw_scratch + 63) & !63;
            let raw_update = total_primitives * 64u64 + 256;
            let aligned_update = (raw_update + 63) & !63;

            Ok(D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO {
                result_data_max_size_in_bytes: aligned_result,
                scratch_data_size_in_bytes: aligned_scratch,
                update_scratch_data_size_in_bytes: aligned_update,
            })
        }
        D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE::TopLevel => {
            let instance_count = inputs.num_descs as u64;
            if instance_count == 0 {
                return Err(DxrError::InvalidPrimitiveCount);
            }
            let raw_result = 256u64 + instance_count * 128u64;
            let aligned_result = (raw_result + 63) & !63;
            let raw_scratch = instance_count * 128u64 + 256;
            let aligned_scratch = (raw_scratch + 63) & !63;
            let raw_update = instance_count * 64u64 + 128;
            let aligned_update = (raw_update + 63) & !63;

            Ok(D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO {
                result_data_max_size_in_bytes: aligned_result,
                scratch_data_size_in_bytes: aligned_scratch,
                update_scratch_data_size_in_bytes: aligned_update,
            })
        }
    }
}

// Validated build command ready for hardware submission or execution recording
#[derive(Debug, Clone, PartialEq)]
pub struct BuildRaytracingAccelerationStructureCommand {
    pub desc: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC,
    pub prebuild_info: D3D12_RAYTRACING_ACCELERATION_STRUCTURE_PREBUILD_INFO,
}

impl BuildRaytracingAccelerationStructureCommand {
    // Validate build descriptor and return executable build command
    pub fn new(
        desc: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC,
    ) -> Result<Self, DxrError> {
        if desc.dest_acceleration_structure_data == 0
            || (desc.dest_acceleration_structure_data % D3D12_RAYTRACING_ACCELERATION_STRUCTURE_ALIGNMENT) != 0
        {
            return Err(DxrError::InvalidAlignment);
        }
        if desc.scratch_acceleration_structure_data == 0
            || (desc.scratch_acceleration_structure_data % D3D12_RAYTRACING_ACCELERATION_STRUCTURE_ALIGNMENT) != 0
        {
            return Err(DxrError::InvalidAlignment);
        }
        if desc.source_acceleration_structure_data != 0
            && (desc.source_acceleration_structure_data % D3D12_RAYTRACING_ACCELERATION_STRUCTURE_ALIGNMENT) != 0
        {
            return Err(DxrError::InvalidAlignment);
        }
        if desc.inputs.as_type == D3D12_RAYTRACING_ACCELERATION_STRUCTURE_TYPE::TopLevel
            && desc.inputs.instance_descs_gpu_va != 0
            && (desc.inputs.instance_descs_gpu_va % D3D12_RAYTRACING_INSTANCE_DESC_ALIGNMENT) != 0
        {
            return Err(DxrError::InvalidAlignment);
        }

        let prebuild_info = get_raytracing_acceleration_structure_prebuild_info(&desc.inputs)?;
        Ok(Self {
            desc,
            prebuild_info,
        })
    }
}

// Shader record containing a 32-byte identifier and arbitrary local root arguments
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DxrShaderRecord {
    pub shader_id: [u8; 32],
    pub local_arguments: Vec<u8>,
}

impl DxrShaderRecord {
    // Create a new shader record
    pub fn new(shader_id: [u8; 32], local_arguments: &[u8]) -> Self {
        Self {
            shader_id,
            local_arguments: local_arguments.to_vec(),
        }
    }

    // Byte length of identifier plus local arguments
    pub fn raw_size(&self) -> usize {
        32 + self.local_arguments.len()
    }
}

// Shader Binding Table generator organizing RayGen, Miss, HitGroup, and Callable records
#[derive(Debug, Clone, Default)]
pub struct DxrShaderBindingTable {
    pub raygen: Option<DxrShaderRecord>,
    pub miss_records: Vec<DxrShaderRecord>,
    pub hit_group_records: Vec<DxrShaderRecord>,
    pub callable_records: Vec<DxrShaderRecord>,
}

impl DxrShaderBindingTable {
    // Create an empty shader binding table
    pub fn new() -> Self {
        Self::default()
    }

    // Set ray generation shader record
    pub fn set_raygen(&mut self, record: DxrShaderRecord) {
        self.raygen = Some(record);
    }

    // Add miss shader record
    pub fn add_miss(&mut self, record: DxrShaderRecord) {
        self.miss_records.push(record);
    }

    // Add hit group shader record
    pub fn add_hit_group(&mut self, record: DxrShaderRecord) {
        self.hit_group_records.push(record);
    }

    // Add callable shader record
    pub fn add_callable(&mut self, record: DxrShaderRecord) {
        self.callable_records.push(record);
    }

    // Calculate aligned stride for a list of shader records
    fn calculate_stride(records: &[DxrShaderRecord]) -> usize {
        let max_size = records.iter().map(|r| r.raw_size()).max().unwrap_or(32);
        (max_size + 63) & !63
    }

    // Build the packed SBT memory buffer and populate dispatch rays descriptor
    pub fn build(
        &self,
        base_gpu_va: u64,
        width: u32,
        height: u32,
        depth: u32,
    ) -> Result<(Vec<u8>, D3D12_DISPATCH_RAYS_DESC), DxrError> {
        if (base_gpu_va % 64) != 0 {
            return Err(DxrError::InvalidAlignment);
        }
        let raygen = self.raygen.as_ref().ok_or(DxrError::MissingRayGenRecord)?;

        let mut buffer = Vec::new();

        // 1. RayGen Record (must be aligned to 64 bytes)
        let raygen_va = base_gpu_va;
        let raygen_size = ((raygen.raw_size() + 63) & !63) as u64;
        buffer.extend_from_slice(&raygen.shader_id);
        buffer.extend_from_slice(&raygen.local_arguments);
        buffer.resize(raygen_size as usize, 0);

        // Helper to align buffer to 64 bytes
        let align_buf = |buf: &mut Vec<u8>| {
            let rem = buf.len() % 64;
            if rem != 0 {
                buf.resize(buf.len() + (64 - rem), 0);
            }
        };

        // 2. Miss Shader Table
        align_buf(&mut buffer);
        let miss_va = base_gpu_va + buffer.len() as u64;
        let miss_stride = if self.miss_records.is_empty() {
            0
        } else {
            Self::calculate_stride(&self.miss_records)
        };
        for record in &self.miss_records {
            let offset = buffer.len();
            buffer.extend_from_slice(&record.shader_id);
            buffer.extend_from_slice(&record.local_arguments);
            buffer.resize(offset + miss_stride, 0);
        }
        let miss_size = (self.miss_records.len() * miss_stride) as u64;

        // 3. Hit Group Table
        align_buf(&mut buffer);
        let hit_va = base_gpu_va + buffer.len() as u64;
        let hit_stride = if self.hit_group_records.is_empty() {
            0
        } else {
            Self::calculate_stride(&self.hit_group_records)
        };
        for record in &self.hit_group_records {
            let offset = buffer.len();
            buffer.extend_from_slice(&record.shader_id);
            buffer.extend_from_slice(&record.local_arguments);
            buffer.resize(offset + hit_stride, 0);
        }
        let hit_size = (self.hit_group_records.len() * hit_stride) as u64;

        // 4. Callable Table
        align_buf(&mut buffer);
        let call_va = base_gpu_va + buffer.len() as u64;
        let call_stride = if self.callable_records.is_empty() {
            0
        } else {
            Self::calculate_stride(&self.callable_records)
        };
        for record in &self.callable_records {
            let offset = buffer.len();
            buffer.extend_from_slice(&record.shader_id);
            buffer.extend_from_slice(&record.local_arguments);
            buffer.resize(offset + call_stride, 0);
        }
        let call_size = (self.callable_records.len() * call_stride) as u64;

        let desc = D3D12_DISPATCH_RAYS_DESC {
            ray_generation_shader_record: D3D12_GPU_VIRTUAL_ADDRESS_RANGE {
                start_address: raygen_va,
                size_in_bytes: raygen_size,
            },
            miss_shader_table: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
                start_address: if miss_size > 0 { miss_va } else { 0 },
                size_in_bytes: miss_size,
                stride_in_bytes: miss_stride as u64,
            },
            hit_group_table: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
                start_address: if hit_size > 0 { hit_va } else { 0 },
                size_in_bytes: hit_size,
                stride_in_bytes: hit_stride as u64,
            },
            callable_shader_table: D3D12_GPU_VIRTUAL_ADDRESS_RANGE_AND_STRIDE {
                start_address: if call_size > 0 { call_va } else { 0 },
                size_in_bytes: call_size,
                stride_in_bytes: call_stride as u64,
            },
            width,
            height,
            depth,
        };

        Ok((buffer, desc))
    }
}

// Recorded DispatchRays command with validated shader table bindings
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DxrDispatchRaysCommand {
    pub desc: D3D12_DISPATCH_RAYS_DESC,
}

impl DxrDispatchRaysCommand {
    // Validate dispatch descriptor parameters and memory alignments
    pub fn new(desc: D3D12_DISPATCH_RAYS_DESC) -> Result<Self, DxrError> {
        if desc.width == 0 || desc.width > 16384 {
            return Err(DxrError::InvalidDispatchDimensions);
        }
        if desc.height == 0 || desc.height > 16384 {
            return Err(DxrError::InvalidDispatchDimensions);
        }
        if desc.depth == 0 || desc.depth > 16384 {
            return Err(DxrError::InvalidDispatchDimensions);
        }

        if desc.ray_generation_shader_record.start_address == 0
            || (desc.ray_generation_shader_record.start_address % 64) != 0
            || desc.ray_generation_shader_record.size_in_bytes < 32
        {
            return Err(DxrError::InvalidShaderBindingTable);
        }

        if desc.miss_shader_table.size_in_bytes > 0 {
            if (desc.miss_shader_table.start_address % 64) != 0
                || desc.miss_shader_table.stride_in_bytes < 32
                || (desc.miss_shader_table.stride_in_bytes % 32) != 0
            {
                return Err(DxrError::InvalidShaderBindingTable);
            }
        }

        if desc.hit_group_table.size_in_bytes > 0 {
            if (desc.hit_group_table.start_address % 64) != 0
                || desc.hit_group_table.stride_in_bytes < 32
                || (desc.hit_group_table.stride_in_bytes % 32) != 0
            {
                return Err(DxrError::InvalidShaderBindingTable);
            }
        }

        if desc.callable_shader_table.size_in_bytes > 0 {
            if (desc.callable_shader_table.start_address % 64) != 0
                || desc.callable_shader_table.stride_in_bytes < 32
                || (desc.callable_shader_table.stride_in_bytes % 32) != 0
            {
                return Err(DxrError::InvalidShaderBindingTable);
            }
        }

        Ok(Self { desc })
    }

    // Total rays dispatched
    pub fn total_rays(&self) -> u64 {
        (self.desc.width as u64) * (self.desc.height as u64) * (self.desc.depth as u64)
    }
}

// Raytracing command stream recorder
#[derive(Debug, Default)]
pub struct DxrCommandRecorder {
    pub build_commands: Vec<BuildRaytracingAccelerationStructureCommand>,
    pub dispatch_commands: Vec<DxrDispatchRaysCommand>,
    pub pending_uav_barrier: bool,
}

impl DxrCommandRecorder {
    // Create new DXR command recorder
    pub fn new() -> Self {
        Self::default()
    }

    // Record acceleration structure build
    pub fn record_build(
        &mut self,
        desc: D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC,
    ) -> Result<(), DxrError> {
        let cmd = BuildRaytracingAccelerationStructureCommand::new(desc)?;
        self.build_commands.push(cmd);
        self.pending_uav_barrier = true;
        Ok(())
    }

    // Insert BVH UAV barrier prior to ray traversal
    pub fn insert_uav_barrier(&mut self) {
        self.pending_uav_barrier = false;
    }

    // Record ray dispatch operation
    pub fn record_dispatch(
        &mut self,
        desc: D3D12_DISPATCH_RAYS_DESC,
    ) -> Result<(), DxrError> {
        let cmd = DxrDispatchRaysCommand::new(desc)?;
        self.dispatch_commands.push(cmd);
        Ok(())
    }

    // Calculate total rays recorded across all dispatches
    pub fn total_dispatched_rays(&self) -> u64 {
        self.dispatch_commands.iter().map(|c| c.total_rays()).sum()
    }
}

// Comprehensive verification test suite for DXR acceleration structure & DispatchRays subsystem
pub fn test_dxr_subsystem() -> bool {
    crate::serial_write_str("[KNT-DXR] Initializing Direct3D 12 raytracing acceleration structure self-test...\r\n");

    // 1. BLAS geometry inputs and prebuild sizing info calculation
    let tri = D3D12_RAYTRACING_GEOMETRY_TRIANGLES_DESC::new(0x1000_0000, 300, 32, 0x1000_4000, 300);
    let geom = D3D12_RAYTRACING_GEOMETRY_DESC::triangles(D3D12_RAYTRACING_GEOMETRY_FLAGS::OPAQUE, tri);
    let blas_inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS::blas(
        D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAGS::PREFER_FAST_TRACE,
        alloc::vec![geom],
    );
    let blas_prebuild = match get_raytracing_acceleration_structure_prebuild_info(&blas_inputs) {
        Ok(info) => info,
        Err(_) => {
            crate::serial_write_str("[KNT-DXR-FAIL] BLAS prebuild info calculation failed\r\n");
            return false;
        }
    };
    if blas_prebuild.result_data_max_size_in_bytes == 0 || (blas_prebuild.result_data_max_size_in_bytes % 64) != 0 {
        crate::serial_write_str("[KNT-DXR-FAIL] Invalid BLAS prebuild result buffer size\r\n");
        return false;
    }
    crate::serial_write_str("  * BLAS geometry description and memory prebuild sizing verified\r\n");

    // 2. Build command validation with GPU virtual address alignments
    let valid_blas_desc = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_DESC {
        dest_acceleration_structure_data: 0x2000_0000,
        inputs: blas_inputs,
        source_acceleration_structure_data: 0,
        scratch_acceleration_structure_data: 0x2001_0000,
    };
    if BuildRaytracingAccelerationStructureCommand::new(valid_blas_desc.clone()).is_err() {
        crate::serial_write_str("[KNT-DXR-FAIL] Valid BLAS build command failed validation\r\n");
        return false;
    }

    let mut unaligned_desc = valid_blas_desc.clone();
    unaligned_desc.dest_acceleration_structure_data = 0x2000_0003;
    if BuildRaytracingAccelerationStructureCommand::new(unaligned_desc) != Err(DxrError::InvalidAlignment) {
        crate::serial_write_str("[KNT-DXR-FAIL] Unaligned dest address was not rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * BLAS build command recording and 64-byte alignment validation verified\r\n");

    // 3. TLAS instance descriptors and prebuild sizing
    let inst = D3D12_RAYTRACING_INSTANCE_DESC::identity(1, 0, 0x2000_0000);
    let tlas_inputs = D3D12_BUILD_RAYTRACING_ACCELERATION_STRUCTURE_INPUTS::tlas(
        D3D12_RAYTRACING_ACCELERATION_STRUCTURE_BUILD_FLAGS::NONE,
        alloc::vec![inst],
        0x3000_0000,
    );
    let tlas_prebuild = match get_raytracing_acceleration_structure_prebuild_info(&tlas_inputs) {
        Ok(info) => info,
        Err(_) => {
            crate::serial_write_str("[KNT-DXR-FAIL] TLAS prebuild info calculation failed\r\n");
            return false;
        }
    };
    if tlas_prebuild.result_data_max_size_in_bytes == 0 || (tlas_prebuild.result_data_max_size_in_bytes % 64) != 0 {
        crate::serial_write_str("[KNT-DXR-FAIL] Invalid TLAS prebuild result buffer size\r\n");
        return false;
    }
    crate::serial_write_str("  * TLAS instance descriptors and hierarchy sizing verified\r\n");

    // 4. Shader Binding Table generation and record packing
    let mut sbt = DxrShaderBindingTable::new();
    let raygen_record = DxrShaderRecord::new([0xAA; 32], &[1, 2, 3, 4, 5, 6, 7, 8]);
    let miss_record = DxrShaderRecord::new([0xBB; 32], &[]);
    let hit_record = DxrShaderRecord::new([0xCC; 32], &[9, 10, 11, 12]);
    sbt.set_raygen(raygen_record);
    sbt.add_miss(miss_record);
    sbt.add_hit_group(hit_record);

    let (sbt_buffer, dispatch_desc) = match sbt.build(0x4000_0000, 1920, 1080, 1) {
        Ok(res) => res,
        Err(_) => {
            crate::serial_write_str("[KNT-DXR-FAIL] Shader binding table generation failed\r\n");
            return false;
        }
    };
    if sbt_buffer.is_empty() || (sbt_buffer.len() % 64) != 0 {
        crate::serial_write_str("[KNT-DXR-FAIL] SBT buffer alignment mismatch\r\n");
        return false;
    }
    if dispatch_desc.ray_generation_shader_record.start_address != 0x4000_0000 {
        crate::serial_write_str("[KNT-DXR-FAIL] RayGen table base address mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Shader Binding Table 64-byte record packing verified\r\n");

    // 5. DispatchRays command parameter validation
    let dispatch_cmd = match DxrDispatchRaysCommand::new(dispatch_desc) {
        Ok(cmd) => cmd,
        Err(_) => {
            crate::serial_write_str("[KNT-DXR-FAIL] DispatchRays command validation failed\r\n");
            return false;
        }
    };
    if dispatch_cmd.total_rays() != 1920 * 1080 {
        crate::serial_write_str("[KNT-DXR-FAIL] Dispatched ray count mismatch\r\n");
        return false;
    }

    let mut invalid_desc = dispatch_desc;
    invalid_desc.width = 20000;
    if DxrDispatchRaysCommand::new(invalid_desc) != Err(DxrError::InvalidDispatchDimensions) {
        crate::serial_write_str("[KNT-DXR-FAIL] Out-of-bounds dispatch dimensions were not rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * DispatchRays descriptor validation and dimension limits verified\r\n");

    // 6. Command recorder execution flow and UAV barrier tracking
    let mut recorder = DxrCommandRecorder::new();
    if recorder.record_build(valid_blas_desc).is_err() {
        crate::serial_write_str("[KNT-DXR-FAIL] Recorder failed to record BLAS build\r\n");
        return false;
    }
    if !recorder.pending_uav_barrier {
        crate::serial_write_str("[KNT-DXR-FAIL] Expected pending UAV barrier after build\r\n");
        return false;
    }
    recorder.insert_uav_barrier();
    if recorder.pending_uav_barrier {
        crate::serial_write_str("[KNT-DXR-FAIL] UAV barrier was not cleared\r\n");
        return false;
    }
    if recorder.record_dispatch(dispatch_desc).is_err() {
        crate::serial_write_str("[KNT-DXR-FAIL] Recorder failed to record dispatch rays\r\n");
        return false;
    }
    if recorder.total_dispatched_rays() != 1920 * 1080 {
        crate::serial_write_str("[KNT-DXR-FAIL] Recorder total dispatched rays mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * DXR command stream recording and BVH execution barrier verified\r\n");

    crate::serial_write_str("[KNT-DXR-PASS] Direct3D 12 raytracing acceleration structure engine verified\r\n");
    true
}
