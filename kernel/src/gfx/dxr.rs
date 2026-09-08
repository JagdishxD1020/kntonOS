//! Direct3D 12 Raytracing (DXR) Acceleration Structure & DispatchRays Engine.
//! Provides hardware BVH construction, instance descriptors, SBT management, and ray dispatch.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use alloc::vec::Vec;
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
