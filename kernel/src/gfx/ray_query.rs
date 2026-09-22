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

/// Axis-Aligned Bounding Box (AABB) enclosing procedural primitives in BVH leaves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayAABB {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl RayAABB {
    /// Creates a new Axis-Aligned Bounding Box.
    pub const fn new(min: [f32; 3], max: [f32; 3]) -> Self {
        Self { min, max }
    }

    /// Evaluates ray-box slab intersection returning entry and exit distances `(t_near, t_far)`.
    pub fn intersect_ray(&self, ray: &RayDesc) -> Option<(f32, f32)> {
        let mut t_near = ray.t_min;
        let mut t_far = ray.t_max;

        for i in 0..3 {
            let dir = ray.direction[i];
            let origin = ray.origin[i];
            let b_min = self.min[i];
            let b_max = self.max[i];

            if dir.abs() < 1e-6 {
                // Ray is parallel to slab; check if origin is outside box
                if origin < b_min || origin > b_max {
                    return None;
                }
            } else {
                let inv_d = 1.0 / dir;
                let mut t0 = (b_min - origin) * inv_d;
                let mut t1 = (b_max - origin) * inv_d;

                if t0 > t1 {
                    core::mem::swap(&mut t0, &mut t1);
                }

                if t0 > t_near {
                    t_near = t0;
                }
                if t1 < t_far {
                    t_far = t1;
                }

                if t_near > t_far {
                    return None;
                }
            }
        }

        Some((t_near, t_far))
    }

    /// Computes the center position of the bounding box.
    #[inline(always)]
    pub fn center(&self) -> [f32; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }
}

/// Candidate intersection yielded by `RayQuery::Proceed` for shader inspection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RayQueryCandidate {
    NonOpaqueTriangle {
        primitive_index: u32,
        barycentrics: [f32; 2],
        t_hit: f32,
        front_face: bool,
    },
    ProceduralPrimitive {
        primitive_index: u32,
        aabb: RayAABB,
        t_near: f32,
        t_far: f32,
    },
}

/// Final committed intersection state of an inline ray query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CommittedHit {
    None,
    Triangle {
        instance_index: u32,
        primitive_index: u32,
        barycentrics: [f32; 2],
        t_hit: f32,
        front_face: bool,
    },
    Procedural {
        instance_index: u32,
        primitive_index: u32,
        t_hit: f32,
    },
}

impl Default for CommittedHit {
    fn default() -> Self {
        Self::None
    }
}

/// Inline Ray Query state machine coordinating ray traversal and candidate commitment.
#[derive(Debug, Clone)]
pub struct InlineRayQuery {
    pub desc: InlineRayQueryDesc,
    pub status: RayQueryStatus,
    pub candidate: Option<RayQueryCandidate>,
    pub committed_hit: CommittedHit,
    pub ray_t_current: f32,
}

impl InlineRayQuery {
    /// Initializes an inline ray query in `NotStarted` state.
    pub fn new(desc: InlineRayQueryDesc) -> Self {
        let ray_t_current = desc.ray.t_max;
        Self {
            desc,
            status: RayQueryStatus::NotStarted,
            candidate: None,
            committed_hit: CommittedHit::None,
            ray_t_current,
        }
    }

    /// Current traversal state of the query.
    #[inline(always)]
    pub fn status(&self) -> RayQueryStatus {
        self.status
    }

    /// Committed intersection result.
    #[inline(always)]
    pub fn committed_hit(&self) -> &CommittedHit {
        &self.committed_hit
    }

    /// Active candidate awaiting shader accept/commit or reject decision.
    #[inline(always)]
    pub fn candidate(&self) -> Option<&RayQueryCandidate> {
        self.candidate.as_ref()
    }

    /// Commits a candidate procedural primitive hit with caller-calculated intersection distance $t$.
    pub fn commit_procedural_primitive_hit(&mut self, t_hit: f32) -> bool {
        if self.status != RayQueryStatus::InProgress {
            return false;
        }

        if let Some(RayQueryCandidate::ProceduralPrimitive { primitive_index, .. }) = self.candidate {
            if t_hit >= self.desc.ray.t_min && t_hit <= self.ray_t_current {
                self.ray_t_current = t_hit;
                self.committed_hit = CommittedHit::Procedural {
                    instance_index: 0,
                    primitive_index,
                    t_hit,
                };
                self.candidate = None;

                if (self.desc.ray_flags & RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH) != 0 {
                    self.status = RayQueryStatus::Finished;
                } else {
                    self.status = RayQueryStatus::CommittedProceduralHit;
                }
                return true;
            }
        }
        false
    }

    /// Commits a candidate non-opaque triangle hit.
    pub fn commit_non_opaque_triangle_hit(&mut self) -> bool {
        if self.status != RayQueryStatus::InProgress {
            return false;
        }

        if let Some(RayQueryCandidate::NonOpaqueTriangle {
            primitive_index,
            barycentrics,
            t_hit,
            front_face,
        }) = self.candidate
        {
            if t_hit >= self.desc.ray.t_min && t_hit <= self.ray_t_current {
                self.ray_t_current = t_hit;
                self.committed_hit = CommittedHit::Triangle {
                    instance_index: 0,
                    primitive_index,
                    barycentrics,
                    t_hit,
                    front_face,
                };
                self.candidate = None;

                if (self.desc.ray_flags & RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH) != 0 {
                    self.status = RayQueryStatus::Finished;
                } else {
                    self.status = RayQueryStatus::CommittedTriangleHit;
                }
                return true;
            }
        }
        false
    }

    /// Aborts the inline ray query, immediately terminating further traversal.
    pub fn abort(&mut self) {
        self.status = RayQueryStatus::Finished;
        self.candidate = None;
    }
}
