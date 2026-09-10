#![allow(non_camel_case_types, non_snake_case, dead_code)]
//! Direct3D 12 Mesh Shader & Amplification Shader Primitive Pipeline Engine.
// Replaces legacy fixed-function Input Assembler and VS/HS/DS/GS geometry stages

use alloc::vec::Vec;
use crate::gfx::pso::D3D12_SHADER_BYTECODE;

// Pipeline state subobject type for Amplification Shader (AS)
pub const D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_AS: u32 = 24;

// Pipeline state subobject type for Mesh Shader (MS)
pub const D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MS: u32 = 25;

// Mesh shader primitive output topology (triangles or lines)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_MESH_SHADER_PRIMITIVE_TOPOLOGY {
    UNDEFINED = 0,
    LINES = 1,
    TRIANGLES = 2,
}

// Complete D3D12 mesh and amplification shader pipeline state stream descriptor
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3D12_MESH_SHADER_PIPELINE_STATE_DESC {
    pub root_signature_gpu_va: u64,
    pub amplification_shader: Option<D3D12_SHADER_BYTECODE>,
    pub mesh_shader: D3D12_SHADER_BYTECODE,
    pub primitive_topology: D3D12_MESH_SHADER_PRIMITIVE_TOPOLOGY,
    pub payload_size_bytes: u32,
}

impl D3D12_MESH_SHADER_PIPELINE_STATE_DESC {
    // Validate pipeline subobjects and topology
    pub fn validate(&self) -> Result<(), MeshShaderError> {
        if self.mesh_shader.is_empty() {
            return Err(MeshShaderError::InvalidPipelineConfig);
        }
        if self.primitive_topology == D3D12_MESH_SHADER_PRIMITIVE_TOPOLOGY::UNDEFINED {
            return Err(MeshShaderError::InvalidPipelineConfig);
        }
        if let Some(ref as_bc) = self.amplification_shader {
            if as_bc.is_empty() {
                return Err(MeshShaderError::InvalidPipelineConfig);
            }
        }
        if self.payload_size_bytes > 16384 {
            return Err(MeshShaderError::ExceededMaxPayload(self.payload_size_bytes as usize));
        }
        Ok(())
    }
}


// Direct3D 12 maximum allowable meshlet output vertices (hardware limit = 256)
pub const D3D12_MESH_SHADER_MAX_TOTAL_VERTICES: u32 = 256;

// Direct3D 12 maximum allowable meshlet output primitives (hardware limit = 512)
pub const D3D12_MESH_SHADER_MAX_TOTAL_PRIMITIVES: u32 = 512;

// Meshlet triangle primitive local index layout (three local vertex indices)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct D3D12_MESHLET_TRIANGLE {
    pub i0: u8,
    pub i1: u8,
    pub i2: u8,
}

impl D3D12_MESHLET_TRIANGLE {
    // Create new triangle primitive
    pub const fn new(i0: u8, i1: u8, i2: u8) -> Self {
        Self { i0, i1, i2 }
    }

    // Encode into packed 32-bit uint (10 bits per index: [9:0] i0, [19:10] i1, [29:20] i2)
    pub const fn to_packed_u32(&self) -> u32 {
        (self.i0 as u32) | ((self.i1 as u32) << 10) | ((self.i2 as u32) << 20)
    }

    // Decode from packed 32-bit uint
    pub const fn from_packed_u32(val: u32) -> Self {
        Self {
            i0: (val & 0x3FF) as u8,
            i1: ((val >> 10) & 0x3FF) as u8,
            i2: ((val >> 20) & 0x3FF) as u8,
        }
    }
}

// D3D12 meshlet geometry chunk descriptor referencing vertex and index buffer offsets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct D3D12_MESHLET {
    pub VertOffset: u32,
    pub VertCount: u32,
    pub PrimOffset: u32,
    pub PrimCount: u32,
}

impl D3D12_MESHLET {
    // Construct new meshlet descriptor
    pub const fn new(vert_offset: u32, vert_count: u32, prim_offset: u32, prim_count: u32) -> Self {
        Self {
            VertOffset: vert_offset,
            VertCount: vert_count,
            PrimOffset: prim_offset,
            PrimCount: prim_count,
        }
    }

    // Validate meshlet vertex count (<= 256) and primitive count (<= 512)
    pub fn validate(&self) -> Result<(), MeshShaderError> {
        if self.VertCount == 0 || self.VertCount > D3D12_MESH_SHADER_MAX_TOTAL_VERTICES {
            return Err(MeshShaderError::ExceededMaxVertices(self.VertCount));
        }
        if self.PrimCount == 0 || self.PrimCount > D3D12_MESH_SHADER_MAX_TOTAL_PRIMITIVES {
            return Err(MeshShaderError::ExceededMaxPrimitives(self.PrimCount));
        }
        Ok(())
    }
}


// Direct3D 12 maximum allowable task payload size shared from AS to MS (16 KiB)
pub const D3D12_AMPLIFICATION_MAX_PAYLOAD_SIZE: usize = 16384;

// Amplification-to-Mesh Shader task payload container for inter-stage data sharing
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct D3D12_AMPLIFICATION_TASK_PAYLOAD {
    pub payload_data: Vec<u8>,
    pub meshlet_indices: Vec<u32>,
    pub meshlet_count: u32,
}

impl D3D12_AMPLIFICATION_TASK_PAYLOAD {
    // Construct an empty task payload
    pub fn new() -> Self {
        Self::default()
    }

    // Set list of surviving meshlet IDs to dispatch to the mesh shader stage
    pub fn set_meshlets(&mut self, indices: &[u32]) -> Result<(), MeshShaderError> {
        let count = indices.len() as u32;
        self.meshlet_indices = indices.to_vec();
        self.meshlet_count = count;
        Ok(())
    }

    // Write arbitrary user payload data into the task payload buffer up to 16 KiB
    pub fn write_payload_bytes(&mut self, offset: usize, bytes: &[u8]) -> Result<(), MeshShaderError> {
        let total = offset + bytes.len();
        if total > D3D12_AMPLIFICATION_MAX_PAYLOAD_SIZE {
            return Err(MeshShaderError::ExceededMaxPayload(total));
        }
        if self.payload_data.len() < total {
            self.payload_data.resize(total, 0);
        }
        self.payload_data[offset..total].copy_from_slice(bytes);
        Ok(())
    }

    // Read payload bytes from the task payload buffer
    pub fn read_payload_bytes(&self, offset: usize, length: usize) -> Result<&[u8], MeshShaderError> {
        if offset + length > self.payload_data.len() {
            return Err(MeshShaderError::BufferOverflow);
        }
        Ok(&self.payload_data[offset..offset + length])
    }
}

// Amplification shader threadgroup synchronization and memory barrier tracker
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_TASK_PAYLOAD_BARRIER {
    pub is_synchronized: bool,
    pub barrier_generation: u32,
}

impl D3D12_TASK_PAYLOAD_BARRIER {
    // Construct new unsynchronized threadgroup barrier
    pub fn new() -> Self {
        Self::default()
    }

    // Execute threadgroup payload memory barrier ensuring all AS writes are visible to MS
    pub fn sync_threadgroup(&mut self) {
        self.is_synchronized = true;
        self.barrier_generation = self.barrier_generation.wrapping_add(1);
    }

    // Invalidate synchronization state prior to next AS write phase
    pub fn invalidate(&mut self) {
        self.is_synchronized = false;
    }
}


// Direct3D 12 3D threadgroup dispatch arguments for mesh and amplification shaders
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct D3D12_DISPATCH_MESH_ARGUMENTS {
    pub ThreadGroupCountX: u32,
    pub ThreadGroupCountY: u32,
    pub ThreadGroupCountZ: u32,
}

impl D3D12_DISPATCH_MESH_ARGUMENTS {
    // Create new dispatch dimensions descriptor
    pub const fn new(x: u32, y: u32, z: u32) -> Self {
        Self {
            ThreadGroupCountX: x,
            ThreadGroupCountY: y,
            ThreadGroupCountZ: z,
        }
    }

    // Total threadgroups dispatched across all three dimensions
    pub fn total_threadgroups(&self) -> u64 {
        (self.ThreadGroupCountX as u64)
            * (self.ThreadGroupCountY as u64)
            * (self.ThreadGroupCountZ as u64)
    }

    // Validate dispatch bounds (each dimension in 1..=65535)
    pub fn validate(&self) -> Result<(), MeshShaderError> {
        if self.ThreadGroupCountX == 0 || self.ThreadGroupCountX > 65535 {
            return Err(MeshShaderError::InvalidThreadgroupDimension);
        }
        if self.ThreadGroupCountY == 0 || self.ThreadGroupCountY > 65535 {
            return Err(MeshShaderError::InvalidThreadgroupDimension);
        }
        if self.ThreadGroupCountZ == 0 || self.ThreadGroupCountZ > 65535 {
            return Err(MeshShaderError::InvalidThreadgroupDimension);
        }
        Ok(())
    }
}

// Recorded DispatchMesh command
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3D12_DISPATCH_MESH_COMMAND {
    pub arguments: D3D12_DISPATCH_MESH_ARGUMENTS,
    pub payload: Option<D3D12_AMPLIFICATION_TASK_PAYLOAD>,
}

// Mesh and Amplification shader command list recorder
#[derive(Debug, Default)]
pub struct D3D12MeshShaderCommandRecorder {
    pub current_pipeline: Option<D3D12_MESH_SHADER_PIPELINE_STATE_DESC>,
    pub recorded_dispatches: Vec<D3D12_DISPATCH_MESH_COMMAND>,
    pub payload_barrier: D3D12_TASK_PAYLOAD_BARRIER,
}

impl D3D12MeshShaderCommandRecorder {
    // Create new mesh shader command recorder
    pub fn new() -> Self {
        Self::default()
    }

    // Set active mesh shader pipeline state
    pub fn set_pipeline_state(&mut self, pso: D3D12_MESH_SHADER_PIPELINE_STATE_DESC) -> Result<(), MeshShaderError> {
        pso.validate()?;
        self.current_pipeline = Some(pso);
        Ok(())
    }

    // Record DispatchMesh invocation with validated dimensions
    pub fn dispatch_mesh(
        &mut self,
        args: D3D12_DISPATCH_MESH_ARGUMENTS,
        payload: Option<D3D12_AMPLIFICATION_TASK_PAYLOAD>,
    ) -> Result<(), MeshShaderError> {
        if self.current_pipeline.is_none() {
            return Err(MeshShaderError::InvalidPipelineConfig);
        }
        args.validate()?;
        self.recorded_dispatches.push(D3D12_DISPATCH_MESH_COMMAND {
            arguments: args,
            payload,
        });
        self.payload_barrier.sync_threadgroup();
        Ok(())
    }

    // Calculate total threadgroups dispatched across all recorded commands
    pub fn total_dispatched_threadgroups(&self) -> u64 {
        self.recorded_dispatches.iter().map(|c| c.arguments.total_threadgroups()).sum()
    }
}

/// Direct3D 12 maximum allowable output vertices per meshlet (D3D12_MESH_SHADER_MAX_VERTICES).
pub const MAX_MESHLET_VERTICES: usize = 64;

/// Direct3D 12 maximum allowable output primitives per meshlet (D3D12_MESH_SHADER_MAX_PRIMITIVES).
pub const MAX_MESHLET_PRIMITIVES: usize = 126;

/// Direct3D 12 maximum allowable amplification shader payload size in bytes (16 KiB limit).
pub const MAX_PAYLOAD_SIZE_BYTES: usize = 16384;

/// Errors returned by the Mesh Shader primitive pipeline engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshShaderError {
    /// Meshlet output vertex count exceeds the maximum limit (64).
    ExceededMaxVertices(u32),
    /// Meshlet output primitive count exceeds the maximum limit (126).
    ExceededMaxPrimitives(u32),
    /// Amplification shader payload size exceeds the 16 KiB limit.
    ExceededMaxPayload(usize),
    /// Threadgroup dimension X, Y, or Z is zero or exceeds limits.
    InvalidThreadgroupDimension,
    /// Pipeline state configuration is invalid.
    InvalidPipelineConfig,
    /// Output buffer overflow during vertex or primitive streaming.
    BufferOverflow,
}

/// 3D vector for geometry and culling calculations.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
            z: self.z - other.z,
        }
    }

    #[inline]
    pub fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }
}

/// Bounding sphere enclosing all vertices of a meshlet for frustum culling.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct MeshletBoundingSphere {
    /// Center of the bounding sphere in object / world space.
    pub center: Vec3,
    /// Radius of the bounding sphere.
    pub radius: f32,
}

impl MeshletBoundingSphere {
    pub const fn new(center: Vec3, radius: f32) -> Self {
        Self { center, radius }
    }

    /// Evaluates if the sphere is outside a plane defined by normal `(nx, ny, nz)` and distance `d`.
    /// Returns true if completely outside (culled).
    pub fn is_outside_plane(&self, normal: Vec3, d: f32) -> bool {
        let dist = self.center.dot(normal) + d;
        dist < -self.radius
    }
}

/// Bounding normal cone enclosing all face normals of a meshlet for early backface culling.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct MeshletBoundingCone {
    /// Apex origin of the normal cone in object / world space.
    pub apex: Vec3,
    /// Normalized axis vector of the cone (average surface normal direction).
    pub axis: Vec3,
    /// Cone cutoff cosine (cos(theta + pi/2), or cos of angle).
    /// If the camera vector to the apex has dot product < cutoff, all triangles are backfacing.
    pub cutoff: f32,
}

impl MeshletBoundingCone {
    pub const fn new(apex: Vec3, axis: Vec3, cutoff: f32) -> Self {
        Self { apex, axis, cutoff }
    }

    /// Tests if the entire meshlet is backfacing relative to a camera/view position.
    /// If true, every triangle in the meshlet faces away from the camera and can be
    pub fn is_backfacing(&self, camera_pos: Vec3) -> bool {
        let view_dir = self.apex.sub(camera_pos);
        let len_sq = view_dir.length_squared();
        if len_sq <= 0.000001 {
            return false;
        }
        // Approximate normalization using reciprocal square root in software
        let inv_len = 1.0 / sqrt_approx(len_sq);
        let normalized_view = Vec3::new(view_dir.x * inv_len, view_dir.y * inv_len, view_dir.z * inv_len);

        // If the view vector dot product with the cone axis is >= cutoff,
        // it means the camera is completely behind all normal vectors of this cluster.
        normalized_view.dot(self.axis) >= self.cutoff
    }
}

/// Fast software square root approximation for no_std environments.
#[inline]
pub fn sqrt_approx(val: f32) -> f32 {
    if val <= 0.0 {
        return 0.0;
    }
    // Newton-Raphson approximation
    let mut x = val;
    for _ in 0..6 {
        x = 0.5 * (x + val / x);
    }
    x
}

/// Meshlet descriptor defining a localized cluster of up to 64 vertices and 126 primitives.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct MeshletDescriptor {
    /// Byte or index offset into the global vertex index buffer.
    pub vertex_offset: u32,
    /// Number of unique vertices in this meshlet (must be <= 64).
    pub vertex_count: u32,
    /// Byte or index offset into the global packed primitive index buffer.
    pub primitive_offset: u32,
    /// Number of primitive triangles in this meshlet (must be <= 126).
    pub primitive_count: u32,
    /// Bounding sphere for frustum culling.
    pub bounds_sphere: MeshletBoundingSphere,
    /// Bounding cone for cluster backface culling.
    pub bounds_cone: MeshletBoundingCone,
}

impl MeshletDescriptor {
    /// Validates that the meshlet does not exceed D3D12 hardware limits.
    pub fn validate(&self) -> Result<(), MeshShaderError> {
        if self.vertex_count > MAX_MESHLET_VERTICES as u32 {
            return Err(MeshShaderError::ExceededMaxVertices(self.vertex_count));
        }
        if self.primitive_count > MAX_MESHLET_PRIMITIVES as u32 {
            return Err(MeshShaderError::ExceededMaxPrimitives(self.primitive_count));
        }
        Ok(())
    }
}

/// Output primitive topology generated by the Mesh Shader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MeshShaderPrimitiveTopology {
    /// Triangle list output (up to 126 triangles).
    Triangles = 0,
    /// Line list output.
    Lines = 1,
    /// Point list output.
    Points = 2,
}

/// Shader bytecode descriptor for Amplification or Mesh Shader stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshShaderBytecode {
    /// GPU virtual address of the compiled shader microcode.
    pub gpu_virtual_address: u64,
    /// Size of the shader microcode in bytes.
    pub size_in_bytes: usize,
    /// Threadgroup dimension X.
    pub threadgroup_size_x: u32,
    /// Threadgroup dimension Y.
    pub threadgroup_size_y: u32,
    /// Threadgroup dimension Z.
    pub threadgroup_size_z: u32,
}

impl MeshShaderBytecode {
    pub const fn new(
        gpu_va: u64,
        size: usize,
        threads_x: u32,
        threads_y: u32,
        threads_z: u32,
    ) -> Self {
        Self {
            gpu_virtual_address: gpu_va,
            size_in_bytes: size,
            threadgroup_size_x: threads_x,
            threadgroup_size_y: threads_y,
            threadgroup_size_z: threads_z,
        }
    }

    /// Total threads in a single threadgroup.
    #[inline]
    pub const fn total_threads(&self) -> u32 {
        self.threadgroup_size_x * self.threadgroup_size_y * self.threadgroup_size_z
    }
}

/// Direct3D 12 Mesh Shader Pipeline State Object (PSO) descriptor.
#[derive(Debug, Clone)]
pub struct MeshShaderPipelineState {
    /// Optional Amplification (Task) shader stage descriptor.
    pub amplification_shader: Option<MeshShaderBytecode>,
    /// Required Mesh shader stage descriptor.
    pub mesh_shader: MeshShaderBytecode,
    /// Optional Pixel shader stage descriptor.
    pub pixel_shader: Option<MeshShaderBytecode>,
    /// Output primitive topology.
    pub primitive_topology: MeshShaderPrimitiveTopology,
    /// Maximum output vertices per mesh shader invocation (<= 64).
    pub max_output_vertices: u32,
    /// Maximum output primitives per mesh shader invocation (<= 126).
    pub max_output_primitives: u32,
    /// Size of amplification-to-mesh payload struct in bytes (<= 16 KiB).
    pub payload_size_bytes: usize,
}

impl MeshShaderPipelineState {
    /// Validates the PSO configuration according to D3D12 hardware rules.
    pub fn validate(&self) -> Result<(), MeshShaderError> {
        if self.max_output_vertices == 0 || self.max_output_vertices > MAX_MESHLET_VERTICES as u32 {
            return Err(MeshShaderError::ExceededMaxVertices(self.max_output_vertices));
        }
        if self.max_output_primitives == 0 || self.max_output_primitives > MAX_MESHLET_PRIMITIVES as u32 {
            return Err(MeshShaderError::ExceededMaxPrimitives(self.max_output_primitives));
        }
        if self.payload_size_bytes > MAX_PAYLOAD_SIZE_BYTES {
            return Err(MeshShaderError::ExceededMaxPayload(self.payload_size_bytes));
        }
        if self.mesh_shader.total_threads() == 0 || self.mesh_shader.total_threads() > 1024 {
            return Err(MeshShaderError::InvalidThreadgroupDimension);
        }
        if let Some(ref as_stage) = self.amplification_shader {
            if as_stage.total_threads() == 0 || as_stage.total_threads() > 1024 {
                return Err(MeshShaderError::InvalidThreadgroupDimension);
            }
        }
        Ok(())
    }
}

/// Maximum number of surviving meshlet IDs that can be passed in a single Amplification Shader payload.
pub const MAX_PAYLOAD_MESHLETS: usize = 32;

/// Direct3D 12 Amplification Shader to Mesh Shader payload structure.
/// Passed via `DispatchMesh` from the amplification stage down to child mesh shader threadgroups.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct AmplificationPayload {
    /// Array of surviving meshlet indices allocated to child threadgroups.
    pub meshlet_indices: [u32; MAX_PAYLOAD_MESHLETS],
    /// Total number of valid surviving meshlets in this payload.
    pub meshlet_count: u32,
    /// Evaluated Level-of-Detail (0 = high, 1 = medium, 2 = coarse).
    pub lod_level: u8,
    /// Instance ID for multi-draw instanced rendering.
    pub instance_id: u32,
    /// User payload custom scratch storage.
    pub custom_data: [u8; 64],
}

impl Default for AmplificationPayload {
    fn default() -> Self {
        Self {
            meshlet_indices: [0u32; MAX_PAYLOAD_MESHLETS],
            meshlet_count: 0,
            lod_level: 0,
            instance_id: 0,
            custom_data: [0u8; 64],
        }
    }
}

/// A 3D plane in Hessian normal form `(nx * x + ny * y + nz * z + d = 0)`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct FrustumPlane {
    /// Normalized surface normal pointing towards the inside of the frustum.
    pub normal: Vec3,
    /// Signed distance to the coordinate origin.
    pub d: f32,
}

impl FrustumPlane {
    pub const fn new(normal: Vec3, d: f32) -> Self {
        Self { normal, d }
    }
}

/// Camera view frustum consisting of 6 clipping planes (Near, Far, Left, Right, Top, Bottom).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Frustum {
    /// 6 frustum clipping planes.
    pub planes: [FrustumPlane; 6],
}

impl Frustum {
    /// Creates a Frustum from 6 planes.
    pub const fn new(planes: [FrustumPlane; 6]) -> Self {
        Self { planes }
    }

    /// Evaluates whether a meshlet bounding sphere intersects or is inside the frustum.
    /// If the sphere is completely behind any plane, it is culled.
    pub fn is_sphere_visible(&self, sphere: &MeshletBoundingSphere) -> bool {
        for plane in &self.planes {
            if sphere.is_outside_plane(plane.normal, plane.d) {
                return false;
            }
        }
        true
    }
}

/// Direct3D 12 `DispatchMesh` command parameters.
/// Generated either by CPU command submission or emitted dynamically from an Amplification Shader.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DispatchMeshDesc {
    /// Number of Mesh Shader threadgroups to spawn along X (one per surviving meshlet).
    pub threadgroup_count_x: u32,
    /// Number of Mesh Shader threadgroups along Y.
    pub threadgroup_count_y: u32,
    /// Number of Mesh Shader threadgroups along Z.
    pub threadgroup_count_z: u32,
    /// Amplification Shader payload passed to the spawned Mesh Shader threadgroups.
    pub payload: AmplificationPayload,
}

impl DispatchMeshDesc {
    /// Total Mesh Shader threadgroups dispatched.
    #[inline]
    pub const fn total_threadgroups(&self) -> u32 {
        self.threadgroup_count_x * self.threadgroup_count_y * self.threadgroup_count_z
    }
}

/// Telemetry metrics for Amplification Shader culling and LOD selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AmplificationStats {
    /// Total meshlets processed across all amplification threadgroups.
    pub total_meshlets_input: u32,
    /// Meshlets eliminated by frustum culling.
    pub meshlets_frustum_culled: u32,
    /// Meshlets eliminated by cluster normal cone backface culling.
    pub meshlets_backface_culled: u32,
    /// Meshlets surviving culling and dispatched to Mesh Shaders.
    pub meshlets_surviving_dispatched: u32,
}

impl AmplificationStats {
    /// Percentage of meshlets culled before reaching the Mesh Shader stage (0 to 100%).
    pub fn cull_rate_percentage(&self) -> u32 {
        if self.total_meshlets_input == 0 {
            return 0;
        }
        let culled = self.meshlets_frustum_culled + self.meshlets_backface_culled;
        (culled * 100) / self.total_meshlets_input
    }
}

/// Amplification Shader Execution Engine & Task Dispatcher.
/// Processes meshlet batches, performs parallel frustum and backface culling, selects
#[derive(Debug)]
pub struct AmplificationShaderDispatcher {
    /// Active camera view frustum.
    frustum: Frustum,
    /// Active camera world position.
    camera_pos: Vec3,
    /// Distance threshold for high detail (LOD 0).
    lod0_distance_sq: f32,
    /// Distance threshold for medium detail (LOD 1).
    lod1_distance_sq: f32,
    /// Profiling and culling telemetry.
    stats: AmplificationStats,
}

impl AmplificationShaderDispatcher {
    /// Creates a new AmplificationShaderDispatcher with the specified camera view.
    pub fn new(frustum: Frustum, camera_pos: Vec3) -> Self {
        Self {
            frustum,
            camera_pos,
            lod0_distance_sq: 2500.0, // 50 units squared
            lod1_distance_sq: 10000.0, // 100 units squared
            stats: AmplificationStats::default(),
        }
    }

    /// Updates the camera frustum and position.
    pub fn update_camera(&mut self, frustum: Frustum, camera_pos: Vec3) {
        self.frustum = frustum;
        self.camera_pos = camera_pos;
    }

    /// Configures LOD distance thresholds (in world units squared).
    pub fn set_lod_thresholds(&mut self, lod0_sq: f32, lod1_sq: f32) {
        self.lod0_distance_sq = lod0_sq;
        self.lod1_distance_sq = lod1_sq;
    }

    /// Current culling telemetry statistics.
    #[inline]
    pub fn stats(&self) -> &AmplificationStats {
        &self.stats
    }

    /// Resets culling telemetry statistics.
    pub fn reset_stats(&mut self) {
        self.stats = AmplificationStats::default();
    }

    /// Executes the Amplification Shader stage across a slice of meshlets.
    /// Evaluates frustum visibility and cluster backfacing for each meshlet.
    pub fn dispatch_meshlets(
        &mut self,
        meshlets: &[MeshletDescriptor],
        base_instance_id: u32,
    ) -> Result<Vec<DispatchMeshDesc>, MeshShaderError> {
        let mut dispatches = Vec::new();

        // Process meshlets in batches up to MAX_PAYLOAD_MESHLETS
        for (chunk_idx, chunk) in meshlets.chunks(MAX_PAYLOAD_MESHLETS).enumerate() {
            let mut payload = AmplificationPayload::default();
            payload.instance_id = base_instance_id;

            let chunk_base_index = (chunk_idx * MAX_PAYLOAD_MESHLETS) as u32;

            for (local_idx, meshlet) in chunk.iter().enumerate() {
                self.stats.total_meshlets_input += 1;
                let meshlet_id = chunk_base_index + (local_idx as u32);

                // 1. Frustum culling
                if !self.frustum.is_sphere_visible(&meshlet.bounds_sphere) {
                    self.stats.meshlets_frustum_culled += 1;
                    continue;
                }

                // 2. Normal cone backface culling
                if meshlet.bounds_cone.is_backfacing(self.camera_pos) {
                    self.stats.meshlets_backface_culled += 1;
                    continue;
                }

                // 3. Dynamic LOD selection based on camera distance
                let dist_sq = meshlet.bounds_sphere.center.sub(self.camera_pos).length_squared();
                let lod = if dist_sq <= self.lod0_distance_sq {
                    0
                } else if dist_sq <= self.lod1_distance_sq {
                    1
                } else {
                    2
                };

                // Add to payload
                let count = payload.meshlet_count as usize;
                if count < MAX_PAYLOAD_MESHLETS {
                    payload.meshlet_indices[count] = meshlet_id;
                    payload.meshlet_count += 1;
                    payload.lod_level = core::cmp::max(payload.lod_level, lod);
                    self.stats.meshlets_surviving_dispatched += 1;
                }
            }

            // If any meshlet survived culling in this threadgroup, issue DispatchMesh
            if payload.meshlet_count > 0 {
                dispatches.push(DispatchMeshDesc {
                    threadgroup_count_x: payload.meshlet_count,
                    threadgroup_count_y: 1,
                    threadgroup_count_z: 1,
                    payload,
                });
            }
        }

        Ok(dispatches)
    }
}

/// Output vertex format emitted by the Mesh Shader directly into the primitive pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct MeshVertexOutput {
    /// Clip-space homogeneous position `(x, y, z, w)`.
    pub position: [f32; 4],
    /// Normal vector for lighting and shading.
    pub normal: Vec3,
    /// Texture coordinates `(u, v)`.
    pub uv: [f32; 2],
    /// Vertex color RGBA.
    pub color: [f32; 4],
}

impl Default for MeshVertexOutput {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0, 1.0],
            normal: Vec3::default(),
            uv: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// Meshlet local triangle indices (3 local vertex indices in range 0..63).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct MeshTriangleIndices {
    /// First vertex index in meshlet local vertex array (0..63).
    pub i0: u8,
    /// Second vertex index in meshlet local vertex array (0..63).
    pub i1: u8,
    /// Third vertex index in meshlet local vertex array (0..63).
    pub i2: u8,
}

impl MeshTriangleIndices {
    /// Creates a new triangle primitive from 3 local vertex indices.
    pub const fn new(i0: u8, i1: u8, i2: u8) -> Self {
        Self { i0, i1, i2 }
    }

    /// Returns the indices as a 3-element array.
    #[inline]
    pub const fn as_array(self) -> [u8; 3] {
        [self.i0, self.i1, self.i2]
    }
}

/// Output primitive streamer buffer for a single meshlet threadgroup invocation.
#[derive(Debug, Clone)]
pub struct MeshletStreamOutput {
    /// Output vertices (up to 64).
    pub vertices: [MeshVertexOutput; MAX_MESHLET_VERTICES],
    /// Output triangle primitive indices (up to 126).
    pub primitives: [MeshTriangleIndices; MAX_MESHLET_PRIMITIVES],
    /// Valid vertex count emitted (`SetMeshOutputCounts`).
    pub vertex_count: u32,
    /// Valid primitive count emitted (`SetMeshOutputCounts`).
    pub primitive_count: u32,
}

impl Default for MeshletStreamOutput {
    fn default() -> Self {
        Self {
            vertices: [MeshVertexOutput::default(); MAX_MESHLET_VERTICES],
            primitives: [MeshTriangleIndices::default(); MAX_MESHLET_PRIMITIVES],
            vertex_count: 0,
            primitive_count: 0,
        }
    }
}

impl MeshletStreamOutput {
    /// Sets the dynamic output counts for the meshlet (`SetMeshOutputCounts`).
    pub fn set_mesh_output_counts(&mut self, vertex_count: u32, primitive_count: u32) -> Result<(), MeshShaderError> {
        if vertex_count > MAX_MESHLET_VERTICES as u32 {
            return Err(MeshShaderError::ExceededMaxVertices(vertex_count));
        }
        if primitive_count > MAX_MESHLET_PRIMITIVES as u32 {
            return Err(MeshShaderError::ExceededMaxPrimitives(primitive_count));
        }
        self.vertex_count = vertex_count;
        self.primitive_count = primitive_count;
        Ok(())
    }

    /// Writes an output vertex at local slot `index`.
    pub fn write_vertex(&mut self, index: usize, vertex: MeshVertexOutput) -> Result<(), MeshShaderError> {
        if index >= self.vertex_count as usize || index >= MAX_MESHLET_VERTICES {
            return Err(MeshShaderError::BufferOverflow);
        }
        self.vertices[index] = vertex;
        Ok(())
    }

    /// Writes an output triangle primitive at local slot `index`.
    pub fn write_primitive(&mut self, index: usize, tri: MeshTriangleIndices) -> Result<(), MeshShaderError> {
        if index >= self.primitive_count as usize || index >= MAX_MESHLET_PRIMITIVES {
            return Err(MeshShaderError::BufferOverflow);
        }
        if tri.i0 >= self.vertex_count as u8 || tri.i1 >= self.vertex_count as u8 || tri.i2 >= self.vertex_count as u8 {
            return Err(MeshShaderError::InvalidPipelineConfig);
        }
        self.primitives[index] = tri;
        Ok(())
    }
}

/// Telemetry metrics for Mesh Shader primitive streaming.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshShaderStats {
    /// Total meshlet threadgroups executed and expanded into primitives.
    pub meshlets_expanded: u32,
    /// Total vertices streamed to the primitive rasterizer.
    pub vertices_streamed: u64,
    /// Total primitive triangles streamed to the primitive rasterizer.
    pub primitives_streamed: u64,
    /// Peak vertices emitted by a single meshlet.
    pub peak_vertices_per_meshlet: u32,
    /// Peak primitives emitted by a single meshlet.
    pub peak_primitives_per_meshlet: u32,
}

/// Mesh Shader Primitive Pipeline Execution Engine.
/// Executes Mesh Shader threadgroups dispatched by the Amplification Shader or CPU,
#[derive(Debug)]
pub struct MeshShaderEngine {
    /// Active Mesh Shader PSO.
    pipeline_state: MeshShaderPipelineState,
    /// Profiling and primitive streaming telemetry.
    stats: MeshShaderStats,
}

impl MeshShaderEngine {
    /// Creates a new MeshShaderEngine with the specified PSO.
    pub fn new(pipeline_state: MeshShaderPipelineState) -> Result<Self, MeshShaderError> {
        pipeline_state.validate()?;
        Ok(Self {
            pipeline_state,
            stats: MeshShaderStats::default(),
        })
    }

    /// Active pipeline state object.
    #[inline]
    pub fn pipeline_state(&self) -> &MeshShaderPipelineState {
        &self.pipeline_state
    }

    /// Current primitive streaming telemetry.
    #[inline]
    pub fn stats(&self) -> &MeshShaderStats {
        &self.stats
    }

    /// Resets streaming telemetry statistics.
    pub fn reset_stats(&mut self) {
        self.stats = MeshShaderStats::default();
    }

    /// Executes a single `DispatchMeshDesc` batch against a meshlet database.
    pub fn execute_dispatch(
        &mut self,
        dispatch: &DispatchMeshDesc,
        meshlet_database: &[MeshletDescriptor],
    ) -> Result<Vec<MeshletStreamOutput>, MeshShaderError> {
        let mut outputs = Vec::with_capacity(dispatch.payload.meshlet_count as usize);

        for &meshlet_id in &dispatch.payload.meshlet_indices[..dispatch.payload.meshlet_count as usize] {
            let meshlet = meshlet_database
                .get(meshlet_id as usize)
                .ok_or(MeshShaderError::InvalidPipelineConfig)?;

            meshlet.validate()?;

            let mut stream = MeshletStreamOutput::default();
            stream.set_mesh_output_counts(meshlet.vertex_count, meshlet.primitive_count)?;

            // Stream vertices: synthesize positions and attributes for this meshlet
            for v in 0..meshlet.vertex_count {
                let local_v = v as f32;
                let world_x = meshlet.bounds_sphere.center.x + (local_v * 0.1);
                let world_y = meshlet.bounds_sphere.center.y + (local_v * 0.1);
                let world_z = meshlet.bounds_sphere.center.z;

                let vertex = MeshVertexOutput {
                    position: [world_x, world_y, world_z, 1.0],
                    normal: meshlet.bounds_cone.axis,
                    uv: [local_v / (meshlet.vertex_count as f32), 0.5],
                    color: [1.0, 1.0, 1.0, 1.0],
                };
                stream.write_vertex(v as usize, vertex)?;
            }

            // Stream primitive triangle topology: generate triangle strips/fans across local vertices
            for p in 0..meshlet.primitive_count {
                let v0 = 0u8;
                let v1 = core::cmp::min((p + 1) as u8, (meshlet.vertex_count - 1) as u8);
                let v2 = core::cmp::min((p + 2) as u8, (meshlet.vertex_count - 1) as u8);

                let tri = MeshTriangleIndices::new(v0, v1, v2);
                stream.write_primitive(p as usize, tri)?;
            }

            // Update stats
            self.stats.meshlets_expanded += 1;
            self.stats.vertices_streamed += meshlet.vertex_count as u64;
            self.stats.primitives_streamed += meshlet.primitive_count as u64;
            self.stats.peak_vertices_per_meshlet =
                core::cmp::max(self.stats.peak_vertices_per_meshlet, meshlet.vertex_count);
            self.stats.peak_primitives_per_meshlet =
                core::cmp::max(self.stats.peak_primitives_per_meshlet, meshlet.primitive_count);

            outputs.push(stream);
        }

        Ok(outputs)
    }

    /// Executes multiple `DispatchMeshDesc` batches against a meshlet database.
    pub fn execute_dispatches(
        &mut self,
        dispatches: &[DispatchMeshDesc],
        meshlet_database: &[MeshletDescriptor],
    ) -> Result<Vec<MeshletStreamOutput>, MeshShaderError> {
        let mut all_outputs = Vec::new();
        for dispatch in dispatches {
            let mut out = self.execute_dispatch(dispatch, meshlet_database)?;
            all_outputs.append(&mut out);
        }
        Ok(all_outputs)
    }
}

// Comprehensive verification test suite for Direct3D 12 Mesh Shader & Amplification Shader Pipeline
pub fn test_d3d12_mesh_shader_pipeline() -> bool {
    crate::serial_write_str("[KNT-MESH] Initializing Direct3D 12 mesh and amplification shader self-test...\r\n");

    // 1. Pipeline subobject descriptors and topology validation
    if D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_AS != 24 || D3D12_PIPELINE_STATE_SUBOBJECT_TYPE_MS != 25 {
        crate::serial_write_str("[KNT-MESH-FAIL] Pipeline subobject type enum values mismatch\r\n");
        return false;
    }

    let as_bc = D3D12_SHADER_BYTECODE::new(&[0xAA; 128]);
    let ms_bc = D3D12_SHADER_BYTECODE::new(&[0xBB; 256]);
    let valid_pso = D3D12_MESH_SHADER_PIPELINE_STATE_DESC {
        root_signature_gpu_va: 0x1000_0000,
        amplification_shader: Some(as_bc),
        mesh_shader: ms_bc,
        primitive_topology: D3D12_MESH_SHADER_PRIMITIVE_TOPOLOGY::TRIANGLES,
        payload_size_bytes: 2048,
    };
    if valid_pso.validate().is_err() {
        crate::serial_write_str("[KNT-MESH-FAIL] Valid D3D12 mesh shader PSO failed validation\r\n");
        return false;
    }

    let mut invalid_pso = valid_pso.clone();
    invalid_pso.primitive_topology = D3D12_MESH_SHADER_PRIMITIVE_TOPOLOGY::UNDEFINED;
    if invalid_pso.validate().is_ok() {
        crate::serial_write_str("[KNT-MESH-FAIL] Undefined primitive topology was not rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * D3D12 mesh and amplification shader pipeline subobjects and topology verified\r\n");

    // 2. Meshlet geometry limits (<=256 vertices, <=512 primitives) and packed index layout
    let valid_meshlet = D3D12_MESHLET::new(0, 256, 0, 512);
    if valid_meshlet.validate().is_err() {
        crate::serial_write_str("[KNT-MESH-FAIL] 256-vertex / 512-primitive meshlet failed validation\r\n");
        return false;
    }
    let overflow_meshlet_v = D3D12_MESHLET::new(0, 257, 0, 100);
    if overflow_meshlet_v.validate() != Err(MeshShaderError::ExceededMaxVertices(257)) {
        crate::serial_write_str("[KNT-MESH-FAIL] Vertex count exceeding 256 was not rejected\r\n");
        return false;
    }
    let overflow_meshlet_p = D3D12_MESHLET::new(0, 100, 0, 513);
    if overflow_meshlet_p.validate() != Err(MeshShaderError::ExceededMaxPrimitives(513)) {
        crate::serial_write_str("[KNT-MESH-FAIL] Primitive count exceeding 512 was not rejected\r\n");
        return false;
    }

    let tri = D3D12_MESHLET_TRIANGLE::new(5, 12, 250);
    let packed = tri.to_packed_u32();
    let unpacked = D3D12_MESHLET_TRIANGLE::from_packed_u32(packed);
    if unpacked != tri {
        crate::serial_write_str("[KNT-MESH-FAIL] Packed 10-bit meshlet triangle index round-trip mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Meshlet 256-vertex / 512-primitive limits and packed triangle indices verified\r\n");

    // 3. Task payload sharing and threadgroup barrier synchronization
    let mut payload = D3D12_AMPLIFICATION_TASK_PAYLOAD::new();
    let meshlets = [0, 4, 12, 19];
    if payload.set_meshlets(&meshlets).is_err() || payload.meshlet_count != 4 {
        crate::serial_write_str("[KNT-MESH-FAIL] Task payload meshlet list setting failed\r\n");
        return false;
    }
    let sample_bytes = [1u8, 2, 3, 4, 5, 6, 7, 8];
    if payload.write_payload_bytes(16, &sample_bytes).is_err() {
        crate::serial_write_str("[KNT-MESH-FAIL] Task payload byte write failed\r\n");
        return false;
    }
    let read_back = match payload.read_payload_bytes(16, 8) {
        Ok(b) => b,
        Err(_) => {
            crate::serial_write_str("[KNT-MESH-FAIL] Task payload byte read failed\r\n");
            return false;
        }
    };
    if read_back != sample_bytes {
        crate::serial_write_str("[KNT-MESH-FAIL] Task payload data mismatch\r\n");
        return false;
    }
    let oversized = [0u8; 16385];
    if payload.write_payload_bytes(0, &oversized) != Err(MeshShaderError::ExceededMaxPayload(16385)) {
        crate::serial_write_str("[KNT-MESH-FAIL] Task payload exceeding 16 KiB was not rejected\r\n");
        return false;
    }

    let mut barrier = D3D12_TASK_PAYLOAD_BARRIER::new();
    if barrier.is_synchronized {
        crate::serial_write_str("[KNT-MESH-FAIL] New payload barrier must be unsynchronized\r\n");
        return false;
    }
    barrier.sync_threadgroup();
    if !barrier.is_synchronized || barrier.barrier_generation != 1 {
        crate::serial_write_str("[KNT-MESH-FAIL] Payload barrier sync generation mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Amplification-to-Mesh task payload sharing and threadgroup barrier verified\r\n");

    // 4. Command list DispatchMesh recording and parameter validation
    let mut recorder = D3D12MeshShaderCommandRecorder::new();
    if recorder.set_pipeline_state(valid_pso).is_err() {
        crate::serial_write_str("[KNT-MESH-FAIL] Command recorder failed to set mesh shader PSO\r\n");
        return false;
    }
    let dispatch_args = D3D12_DISPATCH_MESH_ARGUMENTS::new(4, 2, 1);
    if recorder.dispatch_mesh(dispatch_args, Some(payload)).is_err() {
        crate::serial_write_str("[KNT-MESH-FAIL] Failed to record DispatchMesh command\r\n");
        return false;
    }
    if recorder.total_dispatched_threadgroups() != 8 {
        crate::serial_write_str("[KNT-MESH-FAIL] Total dispatched threadgroups count mismatch\r\n");
        return false;
    }
    let invalid_args = D3D12_DISPATCH_MESH_ARGUMENTS::new(0, 10, 1);
    if recorder.dispatch_mesh(invalid_args, None) != Err(MeshShaderError::InvalidThreadgroupDimension) {
        crate::serial_write_str("[KNT-MESH-FAIL] Zero threadgroup count was not rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * DispatchMesh command recording and 3D threadgroup validation verified\r\n");

    crate::serial_write_str("[KNT-MESH-SHADER-PASS] Direct3D 12 mesh and amplification shader pipeline verified\r\n");
    true
}
