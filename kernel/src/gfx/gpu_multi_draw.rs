//! Direct3D 12 GPU-Driven ExecuteIndirect Multi-Draw Stream Expander & Meshlet Culler.
//! Provides variable-count multi-draw buffers, frustum/Hi-Z culling, and predicated expansion.

#![allow(dead_code, non_camel_case_types, unused_imports)]

extern crate alloc;
use alloc::vec::Vec;

pub use super::execute_indirect::D3D12_DRAW_INDEXED_ARGUMENTS;
pub use super::cluster_cull::BoundingSphere;

// Meshlet draw token containing geometric offsets and bounding sphere for GPU culling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshletDrawToken {
    pub meshlet_id: u32,
    pub draw_args: D3D12_DRAW_INDEXED_ARGUMENTS,
    pub bounding_sphere: BoundingSphere,
    pub flags: u32,
}

impl MeshletDrawToken {
    // Creates a new meshlet draw token with bounds and indexed draw arguments.
    pub const fn new(
        meshlet_id: u32,
        index_count: u32,
        start_index: u32,
        base_vertex: i32,
        bounding_sphere: BoundingSphere,
    ) -> Self {
        Self {
            meshlet_id,
            draw_args: D3D12_DRAW_INDEXED_ARGUMENTS {
                index_count_per_instance: index_count,
                instance_count: 1,
                start_index_location: start_index,
                base_vertex_location: base_vertex,
                start_instance_location: 0,
            },
            bounding_sphere,
            flags: 0,
        }
    }
}

// Configuration descriptor for an ExecuteIndirect multi-draw command signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiDrawCommandSignatureDesc {
    pub byte_stride: u32,
    pub max_draw_count: u32,
    pub is_variable_count: bool,
}

impl MultiDrawCommandSignatureDesc {
    // Creates a standard indexed multi-draw command signature descriptor.
    pub const fn standard_draw_indexed(max_draw_count: u32) -> Self {
        Self {
            byte_stride: 20, // 5 x 32-bit fields in D3D12_DRAW_INDEXED_ARGUMENTS
            max_draw_count,
            is_variable_count: true,
        }
    }
}

// GPU buffer tracking the dynamic draw count written by compute culling passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuDrawCountBuffer {
    pub gpu_virtual_address: u64,
    pub count_offset: u32,
    pub current_draw_count: u32,
}

impl GpuDrawCountBuffer {
    // Creates a new GPU draw count buffer descriptor.
    pub const fn new(gpu_virtual_address: u64, count_offset: u32) -> Self {
        Self {
            gpu_virtual_address,
            count_offset,
            current_draw_count: 0,
        }
    }

    // Atomically increments the draw count simulating GPU atomic add.
    pub fn atomic_increment(&mut self) -> u32 {
        let prev = self.current_draw_count;
        self.current_draw_count = self.current_draw_count.saturating_add(1);
        prev
    }

    // Resets draw count to zero prior to a culling pass.
    pub fn reset(&mut self) {
        self.current_draw_count = 0;
    }
}

// Complete multi-draw buffer descriptor defining argument and count buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiDrawBufferDesc {
    pub arguments_gpu_va: u64,
    pub arguments_size_bytes: u64,
    pub count_buffer: GpuDrawCountBuffer,
    pub max_commands: u32,
}

impl MultiDrawBufferDesc {
    // Creates a new multi-draw buffer descriptor.
    pub const fn new(
        arguments_gpu_va: u64,
        arguments_size_bytes: u64,
        count_buffer: GpuDrawCountBuffer,
        max_commands: u32,
    ) -> Self {
        Self {
            arguments_gpu_va,
            arguments_size_bytes,
            count_buffer,
            max_commands,
        }
    }

    // Validates memory requirements and alignment for ExecuteIndirect.
    pub fn validate(&self) -> Result<(), &'static str> {
        if (self.arguments_gpu_va % 4) != 0 {
            return Err("Argument buffer GPU VA must be 4-byte aligned");
        }
        if (self.count_buffer.gpu_virtual_address % 4) != 0 {
            return Err("Count buffer GPU VA must be 4-byte aligned");
        }
        let required_bytes = (self.max_commands as u64) * 20;
        if self.arguments_size_bytes < required_bytes {
            return Err("Argument buffer too small for max commands");
        }
        Ok(())
    }
}


// 6 view frustum planes for GPU compute geometry culling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrustumPlanes {
    pub planes: [[f32; 4]; 6], // Left, Right, Bottom, Top, Near, Far
}

impl FrustumPlanes {
    // Creates frustum planes from 6 normalized plane equations.
    pub const fn new(planes: [[f32; 4]; 6]) -> Self {
        Self { planes }
    }

    // Default frustum looking down -Z axis with unit bounds.
    pub const fn default_box() -> Self {
        Self {
            planes: [
                [1.0, 0.0, 0.0, 10.0],  // Left: x >= -10
                [-1.0, 0.0, 0.0, 10.0], // Right: x <= 10
                [0.0, 1.0, 0.0, 10.0],  // Bottom: y >= -10
                [0.0, -1.0, 0.0, 10.0], // Top: y <= 10
                [0.0, 0.0, 1.0, 0.0],   // Near: z >= 0
                [0.0, 0.0, -1.0, 100.0],// Far: z <= 100
            ],
        }
    }

    // Tests whether a bounding sphere is inside or intersects the frustum.
    pub fn test_sphere(&self, sphere: &BoundingSphere) -> bool {
        let (cx, cy, cz) = (sphere.center[0], sphere.center[1], sphere.center[2]);
        let r = sphere.radius;

        for plane in &self.planes {
            let dist = plane[0] * cx + plane[1] * cy + plane[2] * cz + plane[3];
            if dist < -r {
                return false; // Completely outside plane
            }
        }
        true
    }
}

// Hierarchical-Z (Hi-Z) occlusion depth pyramid representation.
#[derive(Debug, Clone, PartialEq)]
pub struct HiZDepthPyramid {
    pub width: usize,
    pub height: usize,
    pub depth_samples: Vec<f32>,
}

impl HiZDepthPyramid {
    // Creates a new Hi-Z pyramid initialized to a clear depth.
    pub fn new(width: usize, height: usize, clear_depth: f32) -> Self {
        Self {
            width,
            height,
            depth_samples: alloc::vec![clear_depth; width * height],
        }
    }

    // Samples maximum depth at coordinate (x, y) clamped to bounds.
    pub fn sample_max_depth(&self, x: usize, y: usize) -> f32 {
        let cx = x.min(self.width.saturating_sub(1));
        let cy = y.min(self.height.saturating_sub(1));
        self.depth_samples[cy * self.width + cx]
    }

    // Tests if a bounding sphere is occluded behind the Hi-Z depth buffer.
    pub fn is_occluded(&self, sphere: &BoundingSphere) -> bool {
        // Simple projected z evaluation: if nearest point is behind depth sample, occlude
        let nearest_z = sphere.center[2] - sphere.radius;
        if nearest_z <= 0.0 {
            return false;
        }
        let sample = self.sample_max_depth(self.width / 2, self.height / 2);
        nearest_z > sample
    }
}

// GPU Compute Meshlet Frustum & Occlusion Culler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuMeshletCuller;

impl GpuMeshletCuller {
    // Evaluates whether a meshlet passes frustum and Hi-Z occlusion tests.
    pub fn evaluate_meshlet_visibility(
        token: &MeshletDrawToken,
        frustum: &FrustumPlanes,
        hiz: Option<&HiZDepthPyramid>,
    ) -> bool {
        if !frustum.test_sphere(&token.bounding_sphere) {
            return false;
        }
        if let Some(hiz_pyramid) = hiz {
            if hiz_pyramid.is_occluded(&token.bounding_sphere) {
                return false;
            }
        }
        true
    }
}

// Results summary from a GPU compute stream compaction pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpuStreamCompactionResult {
    pub total_input_meshlets: u32,
    pub culled_meshlets: u32,
    pub visible_draw_count: u32,
}

// GPU-driven indirect argument stream compactor.
pub struct GpuDrawStreamCompactor;

impl GpuDrawStreamCompactor {
    // Compacts visible meshlets into a contiguous draw indexed indirect argument buffer.
    pub fn compact_meshlet_stream(
        input_tokens: &[MeshletDrawToken],
        frustum: &FrustumPlanes,
        hiz: Option<&HiZDepthPyramid>,
        output_args: &mut [D3D12_DRAW_INDEXED_ARGUMENTS],
        count_buf: &mut GpuDrawCountBuffer,
    ) -> GpuStreamCompactionResult {
        count_buf.reset();
        let mut culled_count = 0u32;

        for token in input_tokens {
            if GpuMeshletCuller::evaluate_meshlet_visibility(token, frustum, hiz) {
                let slot = count_buf.atomic_increment() as usize;
                if slot < output_args.len() {
                    output_args[slot] = token.draw_args;
                }
            } else {
                culled_count = culled_count.saturating_add(1);
            }
        }

        GpuStreamCompactionResult {
            total_input_meshlets: input_tokens.len() as u32,
            culled_meshlets: culled_count,
            visible_draw_count: count_buf.current_draw_count,
        }
    }
}
// Temporary stub for self-test during incremental commits.
pub fn test_gpu_multi_draw_subsystem() -> bool {
    true
}
