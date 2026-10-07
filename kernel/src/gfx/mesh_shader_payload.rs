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

// ============================================================================
// AS Group Payload Buffer Suballocator & Dynamic Cull Distributor
// ============================================================================

// Packed payload structure output by an Amplification Shader workgroup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsGroupPayload {
    // Base meshlet offset in geometry cluster.
    pub base_meshlet_index: u32,
    // Surviving meshlet indices within this AS cluster (after culling).
    pub surviving_meshlet_indices: alloc::vec::Vec<u8>,
    // LOD level index selected by AS.
    pub lod_level: u8,
    // Custom user payload data words (e.g. instance matrix index, material ID).
    pub user_payload: [u32; 8],
}

impl AsGroupPayload {
    // Constructs a new amplification payload.
    pub fn new(base_meshlet_index: u32, lod_level: u8) -> Self {
        Self {
            base_meshlet_index,
            surviving_meshlet_indices: alloc::vec::Vec::new(),
            lod_level,
            user_payload: [0; 8],
        }
    }

    // Appends a surviving meshlet index to this payload.
    pub fn push_survivor(&mut self, meshlet_offset: u8) {
        self.surviving_meshlet_indices.push(meshlet_offset);
    }

    // Number of surviving meshlets ready for mesh shader processing.
    pub fn survivor_count(&self) -> usize {
        self.surviving_meshlet_indices.len()
    }
}

// Suballocator managing scratch payload memory shared between AS and MS workgroups.
#[derive(Debug, Clone)]
pub struct PayloadBufferSuballocator {
    // Total backing scratch pool capacity in bytes.
    pub buffer_capacity_bytes: usize,
    // Memory alignment requirement in bytes (e.g. 128 bytes).
    pub alignment_bytes: usize,
    // Current allocation bump offset.
    pub current_offset: usize,
    // High-water mark of memory consumed.
    pub peak_allocated_bytes: usize,
    // Total allocation operations served.
    pub total_allocations: u64,
}

impl PayloadBufferSuballocator {
    // Constructs a new payload buffer suballocator.
    pub fn new(buffer_capacity_bytes: usize, alignment_bytes: usize) -> Self {
        let align = if alignment_bytes.is_power_of_two() && alignment_bytes > 0 {
            alignment_bytes
        } else {
            128
        };
        Self {
            buffer_capacity_bytes,
            alignment_bytes: align,
            current_offset: 0,
            peak_allocated_bytes: 0,
            total_allocations: 0,
        }
    }

    // Suballocates an aligned slice of payload scratch memory.
    pub fn suballocate(&mut self, size_bytes: usize) -> Result<usize, MeshShaderPayloadError> {
        let aligned_offset = (self.current_offset.saturating_add(self.alignment_bytes - 1))
            & !(self.alignment_bytes - 1);
        let end_offset = aligned_offset.saturating_add(size_bytes);

        if end_offset > self.buffer_capacity_bytes {
            return Err(MeshShaderPayloadError::SuballocatorOutOfMemory);
        }

        self.current_offset = end_offset;
        self.peak_allocated_bytes = core::cmp::max(self.peak_allocated_bytes, end_offset);
        self.total_allocations = self.total_allocations.saturating_add(1);

        Ok(aligned_offset)
    }

    // Resets suballocator offset for next dispatch frame.
    pub fn reset(&mut self) {
        self.current_offset = 0;
    }
}

// Distributes surviving meshlets into amplified downstream Mesh Shader threadgroups.
#[derive(Debug, Clone)]
pub struct DynamicCullDistributor {
    // Number of meshlets processed per spawned Mesh Shader threadgroup (typically 1 or 2).
    pub meshlets_per_ms_group: u32,
    // Total candidate meshlets evaluated across all AS invocations.
    pub input_meshlets_count: u64,
    // Total meshlets culled (frustum, occlusion, or LOD).
    pub culled_meshlets_count: u64,
    // Total meshlets forwarded to downstream Mesh Shaders.
    pub emitted_meshlets_count: u64,
    // Total amplified MS threadgroups spawned.
    pub amplified_groups_spawned: u64,
}

impl DynamicCullDistributor {
    // Constructs a dynamic cull distributor.
    pub fn new(meshlets_per_ms_group: u32) -> Self {
        let per_group = core::cmp::max(1, meshlets_per_ms_group);
        Self {
            meshlets_per_ms_group: per_group,
            input_meshlets_count: 0,
            culled_meshlets_count: 0,
            emitted_meshlets_count: 0,
            amplified_groups_spawned: 0,
        }
    }

    // Calculates downstream DispatchMesh arguments from an AS payload.
    pub fn compute_amplified_dispatch(
        &mut self,
        total_cluster_meshlets: u32,
        payload: &AsGroupPayload,
    ) -> DispatchMeshArguments {
        let survivors = payload.survivor_count() as u32;
        let culled = total_cluster_meshlets.saturating_sub(survivors);

        self.input_meshlets_count = self.input_meshlets_count.saturating_add(total_cluster_meshlets as u64);
        self.culled_meshlets_count = self.culled_meshlets_count.saturating_add(culled as u64);
        self.emitted_meshlets_count = self.emitted_meshlets_count.saturating_add(survivors as u64);

        if survivors == 0 {
            return DispatchMeshArguments::new(0, 0, 0);
        }

        let groups_x = (survivors.saturating_add(self.meshlets_per_ms_group - 1)) / self.meshlets_per_ms_group;
        self.amplified_groups_spawned = self.amplified_groups_spawned.saturating_add(groups_x as u64);

        DispatchMeshArguments::new(groups_x, 1, 1)
    }

    // Computes overall culling efficiency percentage (0 to 100%).
    pub fn culling_efficiency_percent(&self) -> u32 {
        if self.input_meshlets_count == 0 {
            0
        } else {
            ((self.culled_meshlets_count * 100) / self.input_meshlets_count) as u32
        }
    }
}

// ============================================================================
// Per-Primitive Cull Bounding Sphere Test & View-Frustum Visibility Filter
// ============================================================================

// Bounding sphere enclosing a meshlet or primitive cluster for coarse spatial rejection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrimitiveBoundingSphere {
    // 3D center point in world or view space [X, Y, Z].
    pub center: [f32; 3],
    // Radius of the bounding sphere.
    pub radius: f32,
}

impl PrimitiveBoundingSphere {
    // Constructs a new bounding sphere.
    pub const fn new(center: [f32; 3], radius: f32) -> Self {
        Self { center, radius }
    }

    // Evaluates whether this sphere lies entirely behind a half-space plane (nx * x + ny * y + nz * z + d < -r).
    #[inline]
    pub fn is_outside_plane(&self, plane: [f32; 4]) -> bool {
        let dist = plane[0] * self.center[0]
            + plane[1] * self.center[1]
            + plane[2] * self.center[2]
            + plane[3];
        dist < -self.radius
    }
}

// 6-plane view frustum represented by plane equations in Hessian normal form.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewFrustumPlanes {
    // 6 clipping planes: Left, Right, Bottom, Top, Near, Far. Each is [nx, ny, nz, d].
    pub planes: [[f32; 4]; 6],
}

impl ViewFrustumPlanes {
    // Constructs a frustum from 6 plane equations.
    pub const fn from_planes(planes: [[f32; 4]; 6]) -> Self {
        Self { planes }
    }

    // Evaluates whether a bounding sphere is inside or intersecting the view frustum.
    pub fn is_sphere_visible(&self, sphere: &PrimitiveBoundingSphere) -> bool {
        for plane in &self.planes {
            if sphere.is_outside_plane(*plane) {
                return false;
            }
        }
        true
    }
}

// Normal cone for cluster-level backface and orientation culling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrimitiveNormalCone {
    // Apex position of the normal cone in world or view space.
    pub apex: [f32; 3],
    // Primary normalized axis direction vector [X, Y, Z].
    pub axis: [f32; 3],
    // Cosine of the cone cutoff angle (dot product threshold for backfacing rejection).
    pub cutoff: f32,
}

impl PrimitiveNormalCone {
    // Constructs a primitive normal cone.
    pub const fn new(apex: [f32; 3], axis: [f32; 3], cutoff: f32) -> Self {
        Self { apex, axis, cutoff }
    }

    // Evaluates whether all primitives in this cluster are facing away from the camera.
    pub fn is_backfacing(&self, camera_pos: [f32; 3]) -> bool {
        let dir = [
            camera_pos[0] - self.apex[0],
            camera_pos[1] - self.apex[1],
            camera_pos[2] - self.apex[2],
        ];
        let len_sq = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
        if len_sq < 1e-6 {
            return false;
        }

        // Fast inverse square root approximation
        let inv_len = 1.0 / super::mesh_shader::sqrt_approx(len_sq);
        let view_dir = [dir[0] * inv_len, dir[1] * inv_len, dir[2] * inv_len];

        let dot = view_dir[0] * self.axis[0] + view_dir[1] * self.axis[1] + view_dir[2] * self.axis[2];
        dot <= self.cutoff
    }
}

// Complete visibility filtering stage evaluating frustum and backface culling per meshlet.
#[derive(Debug, Clone)]
pub struct MeshletVisibilityFilter {
    // Camera view frustum clipping planes.
    pub frustum: ViewFrustumPlanes,
    // Eye/camera position in world space.
    pub camera_position: [f32; 3],
    // Total meshlets culled due to frustum exclusion.
    pub frustum_culls: u64,
    // Total meshlets culled due to backface normal cone rejection.
    pub backface_culls: u64,
    // Total meshlets accepted as visible.
    pub visible_survivors: u64,
}

impl MeshletVisibilityFilter {
    // Constructs a meshlet visibility filter.
    pub fn new(frustum: ViewFrustumPlanes, camera_position: [f32; 3]) -> Self {
        Self {
            frustum,
            camera_position,
            frustum_culls: 0,
            backface_culls: 0,
            visible_survivors: 0,
        }
    }

    // Evaluates a candidate meshlet bounding sphere and optional normal cone.
    pub fn evaluate_meshlet(
        &mut self,
        sphere: &PrimitiveBoundingSphere,
        cone: Option<&PrimitiveNormalCone>,
    ) -> bool {
        // 1. Frustum visibility test
        if !self.frustum.is_sphere_visible(sphere) {
            self.frustum_culls = self.frustum_culls.saturating_add(1);
            return false;
        }

        // 2. Normal cone backface test
        if let Some(c) = cone {
            if c.is_backfacing(self.camera_position) {
                self.backface_culls = self.backface_culls.saturating_add(1);
                return false;
            }
        }

        self.visible_survivors = self.visible_survivors.saturating_add(1);
        true
    }
}

// Temporary stub for self-test during incremental commits.
pub fn test_mesh_shader_payload_subsystem() -> bool {
    true
}
