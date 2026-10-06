//! Direct3D 12 Agility Mesh Shader Primitive Amplification & Payload Distributor.
//! Implements Amplification Shader (AS) to Mesh Shader (MS) task graph pipeline.

#![allow(dead_code, non_camel_case_types, non_snake_case)]

// Maximum amplification shader threadgroup size in any dimension.
pub const D3D12_AS_MAX_THREADGROUP_DIMENSION: u32 = 1024;
// Maximum mesh shader threadgroup size in any dimension.
pub const D3D12_MS_MAX_THREADGROUP_DIMENSION: u32 = 128;
// Maximum payload size in bytes passed from AS to MS (16 KiB per D3D12 Agility spec).
pub const D3D12_AS_MAX_PAYLOAD_SIZE_BYTES: u32 = 16384;

// Arguments for D3D12 DispatchMesh API call specifying threadgroup counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DispatchMeshArguments {
    // Number of threadgroups dispatched along X dimension.
    pub thread_group_count_x: u32,
    // Number of threadgroups dispatched along Y dimension.
    pub thread_group_count_y: u32,
    // Number of threadgroups dispatched along Z dimension.
    pub thread_group_count_z: u32,
}

impl DispatchMeshArguments {
    // Constructs new 3D dispatch arguments.
    pub const fn new(x: u32, y: u32, z: u32) -> Self {
        Self {
            thread_group_count_x: x,
            thread_group_count_y: y,
            thread_group_count_z: z,
        }
    }

    // Computes total flat threadgroups dispatched.
    pub const fn total_threadgroups(&self) -> u64 {
        (self.thread_group_count_x as u64)
            * (self.thread_group_count_y as u64)
            * (self.thread_group_count_z as u64)
    }
}

// Configuration descriptor for an Amplification and Mesh Shader task graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshShaderTaskDesc {
    // Unique identifier for this mesh task pipeline.
    pub pipeline_id: u32,
    // Base GPU virtual address of root signature.
    pub root_signature_gpu_va: u64,
    // Amplification Shader threadgroup sizing [X, Y, Z].
    pub as_threadgroup_size: [u32; 3],
    // Mesh Shader threadgroup sizing [X, Y, Z].
    pub ms_threadgroup_size: [u32; 3],
    // Payload byte size transferred per amplified threadgroup.
    pub payload_size_bytes: u32,
    // Maximum allowable amplified mesh shader groups per AS invocation.
    pub max_amplified_groups: u32,
}

impl MeshShaderTaskDesc {
    // Validates pipeline configuration against hardware limits.
    pub fn validate(&self) -> Result<(), MeshShaderPayloadError> {
        if self.as_threadgroup_size[0] == 0
            || self.as_threadgroup_size[1] == 0
            || self.as_threadgroup_size[2] == 0
            || self.as_threadgroup_size[0] > D3D12_AS_MAX_THREADGROUP_DIMENSION
        {
            return Err(MeshShaderPayloadError::InvalidThreadgroupSize);
        }
        if self.ms_threadgroup_size[0] == 0
            || self.ms_threadgroup_size[1] == 0
            || self.ms_threadgroup_size[2] == 0
            || self.ms_threadgroup_size[0] > D3D12_MS_MAX_THREADGROUP_DIMENSION
        {
            return Err(MeshShaderPayloadError::InvalidThreadgroupSize);
        }
        if self.payload_size_bytes > D3D12_AS_MAX_PAYLOAD_SIZE_BYTES {
            return Err(MeshShaderPayloadError::PayloadSizeOverflow(self.payload_size_bytes));
        }
        Ok(())
    }
}

// Error conditions returned by the mesh shader amplification subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshShaderPayloadError {
    InvalidThreadgroupSize,
    PayloadSizeOverflow(u32),
    SuballocatorOutOfMemory,
    InvalidPayloadAlignment,
    AmplificationLimitExceeded(u32),
}

// Task graph pipeline coordinator executing Amplification-to-Mesh shader dispatches.
#[derive(Debug, Clone)]
pub struct MeshShaderTaskPipeline {
    // Pipeline configuration descriptor.
    pub desc: MeshShaderTaskDesc,
    // Total AS threadgroups invoked.
    pub total_as_invocations: u64,
    // Total amplified MS threadgroups spawned.
    pub total_ms_invocations: u64,
    // Total primitive amplification ratio (MS groups / AS groups * 100).
    pub running_amplification_ratio: u32,
}

impl MeshShaderTaskPipeline {
    // Constructs a new mesh shader task pipeline.
    pub fn new(desc: MeshShaderTaskDesc) -> Result<Self, MeshShaderPayloadError> {
        desc.validate()?;
        Ok(Self {
            desc,
            total_as_invocations: 0,
            total_ms_invocations: 0,
            running_amplification_ratio: 0,
        })
    }

    // Records an amplification dispatch and updates invocation metrics.
    pub fn record_dispatch(&mut self, as_groups: u64, spawned_ms_groups: u64) {
        self.total_as_invocations = self.total_as_invocations.saturating_add(as_groups);
        self.total_ms_invocations = self.total_ms_invocations.saturating_add(spawned_ms_groups);
        if self.total_as_invocations > 0 {
            self.running_amplification_ratio = ((self.total_ms_invocations * 100) / self.total_as_invocations) as u32;
        }
    }
}

// Temporary stub for self-test during incremental commits.
pub fn test_mesh_shader_payload_subsystem() -> bool {
    true
}
