//! Direct3D 12 DXR 1.1 Inline Ray Query & Procedural Intersection Engine.
//! Provides HLSL `RayQuery` intrinsics emulation, traversal state tracking, and ray-AABB math.

#![allow(dead_code)]

/// Traversal state of an inline ray query (`RayQuery::Proceed` progression).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RayQueryStatus {
    #[default]
    NotStarted = 0,
    InProgress = 1,
    CommittedTriangleHit = 2,
    CommittedProceduralHit = 3,
    Finished = 4,
}

/// DXR Ray Traversal Flags controlling BVH navigation and culling behavior.
pub const RAY_FLAG_NONE: u32 = 0x0;
pub const RAY_FLAG_FORCE_OPAQUE: u32 = 0x1;
pub const RAY_FLAG_FORCE_NON_OPAQUE: u32 = 0x2;
pub const RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH: u32 = 0x4;
pub const RAY_FLAG_SKIP_CLOSEST_HIT_SHADER: u32 = 0x8;
pub const RAY_FLAG_CULL_BACK_FACING_TRIANGLES: u32 = 0x10;
pub const RAY_FLAG_CULL_FRONT_FACING_TRIANGLES: u32 = 0x20;
pub const RAY_FLAG_CULL_OPAQUE: u32 = 0x40;
pub const RAY_FLAG_CULL_NON_OPAQUE: u32 = 0x80;
pub const RAY_FLAG_SKIP_TRIANGLES: u32 = 0x100;
pub const RAY_FLAG_SKIP_PROCEDURAL_PRIMITIVES: u32 = 0x200;

/// Geometric ray representation with origin, direction, and parametric distance bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayDesc {
    pub origin: [f32; 3],
    pub t_min: f32,
    pub direction: [f32; 3],
    pub t_max: f32,
}

impl RayDesc {
    /// Creates a new ray descriptor with validation of parameter bounds.
    pub const fn new(origin: [f32; 3], t_min: f32, direction: [f32; 3], t_max: f32) -> Self {
        Self {
            origin,
            t_min,
            direction,
            t_max,
        }
    }

    /// Evaluates world position along the ray at parametric distance $t$.
    #[inline(always)]
    pub fn point_at(&self, t: f32) -> [f32; 3] {
        [
            self.origin[0] + self.direction[0] * t,
            self.origin[1] + self.direction[1] * t,
            self.origin[2] + self.direction[2] * t,
        ]
    }
}

/// Configuration parameters for initializing an inline ray query (`RayQuery::TraceRayInline`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InlineRayQueryDesc {
    pub ray: RayDesc,
    pub ray_flags: u32,
    pub instance_inclusion_mask: u8,
    pub tlas_gpu_va: u64,
}

impl InlineRayQueryDesc {
    /// Creates a new inline ray query configuration.
    pub const fn new(ray: RayDesc, ray_flags: u32, instance_inclusion_mask: u8, tlas_gpu_va: u64) -> Self {
        Self {
            ray,
            ray_flags,
            instance_inclusion_mask,
            tlas_gpu_va,
        }
    }
}
