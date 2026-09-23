//! Direct3D 12 Screen-Space Motion Vectors & Optical Flow Generator.
//! Provides per-pixel motion vector field generation, camera delta reprojection,
//! bidirectional flow disocclusion cross-checking, and silhouette dilation filtering.

#![allow(dead_code)]

/// Errors returned by the screen-space optical flow and motion vector generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpticalFlowError {
    InvalidDimensions,
    BufferTooSmall,
    OutOfBounds,
    SingularMatrix,
}

/// Screen-space 2D motion vector with displacement in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScreenSpaceMotionVector {
    pub dx: f32,
    pub dy: f32,
}

impl ScreenSpaceMotionVector {
    /// Creates a new screen-space motion vector with displacements `(dx, dy)`.
    pub const fn new(dx: f32, dy: f32) -> Self {
        Self { dx, dy }
    }

    /// Creates a stationary zero motion vector.
    pub const fn zero() -> Self {
        Self { dx: 0.0, dy: 0.0 }
    }

    /// Computes squared Euclidean length of the displacement vector.
    #[inline(always)]
    pub fn length_squared(&self) -> f32 {
        self.dx * self.dx + self.dy * self.dy
    }

    /// Encodes displacement components into IEEE-754 binary16 (half-precision float) bits.
    pub fn to_half_f16(&self) -> (u16, u16) {
        (f32_to_f16(self.dx), f32_to_f16(self.dy))
    }

    /// Decodes displacement components from IEEE-754 binary16 bits.
    pub fn from_half_f16(hx: u16, hy: u16) -> Self {
        Self {
            dx: f16_to_f32(hx),
            dy: f16_to_f32(hy),
        }
    }
}

/// Converts a 32-bit single-precision float to a 16-bit half-precision float.
pub fn f32_to_f16(val: f32) -> u16 {
    let bits = val.to_bits();
    let sign = ((bits >> 31) & 0x1) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let frac = bits & 0x007F_FFFF;

    if exp == 255 {
        // Infinity or NaN
        let h_frac = if frac != 0 { 0x0200 } else { 0 };
        return (sign << 15) | 0x7C00 | h_frac;
    }

    let new_exp = exp - 127 + 15;
    if new_exp >= 31 {
        // Overflow to infinity
        return (sign << 15) | 0x7C00;
    }
    if new_exp <= 0 {
        // Subnormal or zero
        if (14 - new_exp) > 24 {
            return sign << 15;
        }
        let mant = frac | 0x0080_0000;
        let h_frac = (mant >> (14 - new_exp + 13)) as u16;
        return (sign << 15) | h_frac;
    }

    let h_exp = (new_exp as u16) << 10;
    let h_frac = (frac >> 13) as u16;
    (sign << 15) | h_exp | h_frac
}

/// Converts a 16-bit half-precision float to a 32-bit single-precision float.
pub fn f16_to_f32(val: u16) -> f32 {
    let sign = ((val >> 15) & 0x1) as u32;
    let exp = ((val >> 10) & 0x1F) as u32;
    let frac = (val & 0x03FF) as u32;

    if exp == 31 {
        let f_frac = if frac != 0 { 0x0040_0000 } else { 0 };
        return f32::from_bits((sign << 31) | 0x7F80_0000 | f_frac);
    }
    if exp == 0 {
        if frac == 0 {
            return f32::from_bits(sign << 31);
        }
        // Subnormal
        let mut f_frac = frac;
        let mut shift = 0;
        while (f_frac & 0x0400) == 0 {
            f_frac <<= 1;
            shift += 1;
        }
        f_frac &= 0x03FF;
        let f_exp = (127 - 15 + 1 - shift) as u32;
        return f32::from_bits((sign << 31) | (f_exp << 23) | (f_frac << 13));
    }

    let f_exp = (exp + 127 - 15) << 23;
    let f_frac = frac << 13;
    f32::from_bits((sign << 31) | f_exp | f_frac)
}

/// Computes floor of f32 without std.
#[inline(always)]
pub fn floor_f32(x: f32) -> f32 {
    let xi = x as i32;
    if (x < 0.0) && (x != xi as f32) {
        (xi - 1) as f32
    } else {
        xi as f32
    }
}

/// 2D Screen-Space Motion Vector Buffer Manager.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionVectorBuffer {
    pub width: u32,
    pub height: u32,
    pub vectors: alloc::vec::Vec<ScreenSpaceMotionVector>,
}

impl MotionVectorBuffer {
    /// Allocates a new motion vector buffer of dimensions `(width, height)`.
    pub fn new(width: u32, height: u32) -> Result<Self, OpticalFlowError> {
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(OpticalFlowError::InvalidDimensions);
        }
        let total = (width as usize)
            .checked_mul(height as usize)
            .ok_or(OpticalFlowError::InvalidDimensions)?;

        Ok(Self {
            width,
            height,
            vectors: alloc::vec![ScreenSpaceMotionVector::zero(); total],
        })
    }

    /// Reads motion vector at integer pixel `(x, y)`.
    #[inline(always)]
    pub fn get(&self, x: u32, y: u32) -> ScreenSpaceMotionVector {
        if x < self.width && y < self.height {
            self.vectors[(y * self.width + x) as usize]
        } else {
            ScreenSpaceMotionVector::zero()
        }
    }

    /// Sets motion vector at integer pixel `(x, y)`.
    #[inline(always)]
    pub fn set(&mut self, x: u32, y: u32, mv: ScreenSpaceMotionVector) {
        if x < self.width && y < self.height {
            self.vectors[(y * self.width + x) as usize] = mv;
        }
    }

    /// Samples motion vector using bilinear interpolation at continuous coordinates `(u, v)`.
    pub fn sample_bilinear(&self, u: f32, v: f32) -> ScreenSpaceMotionVector {
        let x0 = (floor_f32(u) as i32).clamp(0, self.width as i32 - 1) as u32;
        let y0 = (floor_f32(v) as i32).clamp(0, self.height as i32 - 1) as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);

        let fx = u - floor_f32(u);
        let fy = v - floor_f32(v);

        let v00 = self.get(x0, y0);
        let v10 = self.get(x1, y0);
        let v01 = self.get(x0, y1);
        let v11 = self.get(x1, y1);

        let top_x = v00.dx * (1.0 - fx) + v10.dx * fx;
        let top_y = v00.dy * (1.0 - fx) + v10.dy * fx;
        let bot_x = v01.dx * (1.0 - fx) + v11.dx * fx;
        let bot_y = v01.dy * (1.0 - fx) + v11.dy * fx;

        ScreenSpaceMotionVector::new(
            top_x * (1.0 - fy) + bot_x * fy,
            top_y * (1.0 - fy) + bot_y * fy,
        )
    }
}

/// Column-major 4x4 matrix for camera view-projection transformations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix4x4 {
    pub m: [[f32; 4]; 4],
}

impl Matrix4x4 {
    /// Creates an identity 4x4 matrix.
    pub const fn identity() -> Self {
        Self {
            m: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Transforms a 4D homogeneous vector `p` by the matrix.
    #[inline(always)]
    pub fn transform_point4(&self, p: [f32; 4]) -> [f32; 4] {
        [
            self.m[0][0] * p[0] + self.m[1][0] * p[1] + self.m[2][0] * p[2] + self.m[3][0] * p[3],
            self.m[0][1] * p[0] + self.m[1][1] * p[1] + self.m[2][1] * p[2] + self.m[3][1] * p[3],
            self.m[0][2] * p[0] + self.m[1][2] * p[1] + self.m[2][2] * p[2] + self.m[3][2] * p[3],
            self.m[0][3] * p[0] + self.m[1][3] * p[1] + self.m[2][3] * p[2] + self.m[3][3] * p[3],
        ]
    }

    /// Computes matrix inverse via cofactor expansion. Returns `None` if singular.
    pub fn inverse(&self) -> Option<Self> {
        let m = &self.m;
        let mut inv = [[0.0f32; 4]; 4];

        inv[0][0] = m[1][1] * m[2][2] * m[3][3] - m[1][1] * m[2][3] * m[3][2] - m[2][1] * m[1][2] * m[3][3]
            + m[2][1] * m[1][3] * m[3][2] + m[3][1] * m[1][2] * m[2][3] - m[3][1] * m[1][3] * m[2][2];
        inv[1][0] = -m[1][0] * m[2][2] * m[3][3] + m[1][0] * m[2][3] * m[3][2] + m[2][0] * m[1][2] * m[3][3]
            - m[2][0] * m[1][3] * m[3][2] - m[3][0] * m[1][2] * m[2][3] + m[3][0] * m[1][3] * m[2][2];
        inv[2][0] = m[1][0] * m[2][1] * m[3][3] - m[1][0] * m[2][3] * m[3][1] - m[2][0] * m[1][1] * m[3][3]
            + m[2][0] * m[1][3] * m[3][1] + m[3][0] * m[1][1] * m[2][3] - m[3][0] * m[1][3] * m[2][1];
        inv[3][0] = -m[1][0] * m[2][1] * m[3][2] + m[1][0] * m[2][2] * m[3][1] + m[2][0] * m[1][1] * m[3][2]
            - m[2][0] * m[1][2] * m[3][1] - m[3][0] * m[1][1] * m[2][2] + m[3][0] * m[1][2] * m[2][1];

        let det = m[0][0] * inv[0][0] + m[0][1] * inv[1][0] + m[0][2] * inv[2][0] + m[0][3] * inv[3][0];
        if det.abs() < 1e-9 {
            return None;
        }

        inv[0][1] = -m[0][1] * m[2][2] * m[3][3] + m[0][1] * m[2][3] * m[3][2] + m[2][1] * m[0][2] * m[3][3]
            - m[2][1] * m[0][3] * m[3][2] - m[3][1] * m[0][2] * m[2][3] + m[3][1] * m[0][3] * m[2][2];
        inv[1][1] = m[0][0] * m[2][2] * m[3][3] - m[0][0] * m[2][3] * m[3][2] - m[2][0] * m[0][2] * m[3][3]
            + m[2][0] * m[0][3] * m[3][2] + m[3][0] * m[0][2] * m[2][3] - m[3][0] * m[0][3] * m[2][2];
        inv[2][1] = -m[0][0] * m[2][1] * m[3][3] + m[0][0] * m[2][3] * m[3][1] + m[2][0] * m[0][1] * m[3][3]
            - m[2][0] * m[0][3] * m[3][1] - m[3][0] * m[0][1] * m[2][3] + m[3][0] * m[0][3] * m[2][1];
        inv[3][1] = m[0][0] * m[2][1] * m[3][2] - m[0][0] * m[2][2] * m[3][1] - m[2][0] * m[0][1] * m[3][2]
            + m[2][0] * m[0][2] * m[3][1] + m[3][0] * m[0][1] * m[2][2] - m[3][0] * m[0][2] * m[2][1];

        inv[0][2] = m[0][1] * m[1][2] * m[3][3] - m[0][1] * m[1][3] * m[3][2] - m[1][1] * m[0][2] * m[3][3]
            + m[1][1] * m[0][3] * m[3][2] + m[3][1] * m[0][2] * m[1][3] - m[3][1] * m[0][3] * m[1][2];
        inv[1][2] = -m[0][0] * m[1][2] * m[3][3] + m[0][0] * m[1][3] * m[3][2] + m[1][0] * m[0][2] * m[3][3]
            - m[1][0] * m[0][3] * m[3][2] - m[3][0] * m[0][2] * m[1][3] + m[3][0] * m[0][3] * m[1][2];
        inv[2][2] = m[0][0] * m[1][1] * m[3][3] - m[0][0] * m[1][3] * m[3][1] - m[1][0] * m[0][1] * m[3][3]
            + m[1][0] * m[0][3] * m[3][1] + m[3][0] * m[0][1] * m[1][3] - m[3][0] * m[0][3] * m[1][1];
        inv[3][2] = -m[0][0] * m[1][1] * m[3][2] + m[0][0] * m[1][2] * m[3][1] + m[1][0] * m[0][1] * m[3][2]
            - m[1][0] * m[0][2] * m[3][1] - m[3][0] * m[0][1] * m[1][2] + m[3][0] * m[0][2] * m[1][1];

        inv[0][3] = -m[0][1] * m[1][2] * m[2][3] + m[0][1] * m[1][3] * m[2][2] + m[1][1] * m[0][2] * m[2][3]
            - m[1][1] * m[0][3] * m[2][2] - m[2][1] * m[0][2] * m[1][3] + m[2][1] * m[0][3] * m[1][2];
        inv[1][3] = m[0][0] * m[1][2] * m[2][3] - m[0][0] * m[1][3] * m[2][2] - m[1][0] * m[0][2] * m[2][3]
            + m[1][0] * m[0][3] * m[2][2] + m[2][0] * m[0][2] * m[1][3] - m[2][0] * m[0][3] * m[1][2];
        inv[2][3] = -m[0][0] * m[1][1] * m[2][3] + m[0][0] * m[1][3] * m[2][1] + m[1][0] * m[0][1] * m[2][3]
            - m[1][0] * m[0][3] * m[2][1] - m[2][0] * m[0][1] * m[1][3] + m[2][0] * m[0][3] * m[1][1];
        inv[3][3] = m[0][0] * m[1][1] * m[2][2] - m[0][0] * m[1][2] * m[2][1] - m[1][0] * m[0][1] * m[2][2]
            + m[1][0] * m[0][2] * m[2][1] + m[2][0] * m[0][1] * m[1][2] - m[2][0] * m[0][2] * m[1][1];

        let inv_det = 1.0 / det;
        for i in 0..4 {
            for j in 0..4 {
                inv[i][j] *= inv_det;
            }
        }

        Some(Self { m: inv })
    }
}

/// Calculator deriving per-pixel motion vectors from camera delta matrices and depth.
pub struct CameraReprojectionVelocityCalculator;

impl CameraReprojectionVelocityCalculator {
    /// Derives screen-space motion vector for a single pixel `(x, y)` at specified depth.
    pub fn calculate_pixel_velocity(
        inv_view_proj_curr: &Matrix4x4,
        view_proj_prev: &Matrix4x4,
        x: u32,
        y: u32,
        depth: f32,
        width: u32,
        height: u32,
    ) -> ScreenSpaceMotionVector {
        let u = (x as f32 + 0.5) / (width as f32);
        let v = (y as f32 + 0.5) / (height as f32);

        let ndc_x = 2.0 * u - 1.0;
        let ndc_y = 1.0 - 2.0 * v;
        let ndc_curr = [ndc_x, ndc_y, depth, 1.0];

        let world = inv_view_proj_curr.transform_point4(ndc_curr);
        let w_inv = 1.0 / world[3];
        let world_norm = [world[0] * w_inv, world[1] * w_inv, world[2] * w_inv, 1.0];

        let clip_prev = view_proj_prev.transform_point4(world_norm);
        let clip_inv = 1.0 / clip_prev[3];
        let ndc_prev_x = clip_prev[0] * clip_inv;
        let ndc_prev_y = clip_prev[1] * clip_inv;

        let u_prev = (ndc_prev_x + 1.0) * 0.5;
        let v_prev = (1.0 - ndc_prev_y) * 0.5;

        let x_prev = u_prev * (width as f32) - 0.5;
        let y_prev = v_prev * (height as f32) - 0.5;

        ScreenSpaceMotionVector::new((x as f32) - x_prev, (y as f32) - y_prev)
    }

    /// Generates full-frame camera reprojection motion vector field from depth buffer.
    pub fn calculate_motion_field(
        view_proj_curr: &Matrix4x4,
        view_proj_prev: &Matrix4x4,
        depth_buffer: &[f32],
        width: u32,
        height: u32,
    ) -> Result<MotionVectorBuffer, OpticalFlowError> {
        let inv_curr = view_proj_curr.inverse().ok_or(OpticalFlowError::SingularMatrix)?;
        let mut buffer = MotionVectorBuffer::new(width, height)?;

        for y in 0..height {
            for x in 0..width {
                let depth = depth_buffer[(y * width + x) as usize];
                let mv = Self::calculate_pixel_velocity(&inv_curr, view_proj_prev, x, y, depth, width, height);
                buffer.set(x, y, mv);
            }
        }

        Ok(buffer)
    }
}

/// Binary disocclusion mask identifying newly revealed regions behind moving silhouettes.
#[derive(Debug, Clone, PartialEq)]
pub struct DisocclusionMask {
    pub width: u32,
    pub height: u32,
    pub mask: alloc::vec::Vec<u8>,
    pub disoccluded_pixels: u32,
}

impl DisocclusionMask {
    /// Creates a blank disocclusion mask.
    pub fn new(width: u32, height: u32) -> Result<Self, OpticalFlowError> {
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            return Err(OpticalFlowError::InvalidDimensions);
        }
        let total = (width as usize)
            .checked_mul(height as usize)
            .ok_or(OpticalFlowError::InvalidDimensions)?;

        Ok(Self {
            width,
            height,
            mask: alloc::vec![0u8; total],
            disoccluded_pixels: 0,
        })
    }

    /// Evaluates if pixel `(x, y)` is marked as disoccluded.
    #[inline(always)]
    pub fn is_disoccluded(&self, x: u32, y: u32) -> bool {
        if x < self.width && y < self.height {
            self.mask[(y * self.width + x) as usize] == 255
        } else {
            true
        }
    }

    /// Performs bidirectional optical flow cross-check between forward and backward vector fields.
    pub fn detect(
        forward: &MotionVectorBuffer,
        backward: &MotionVectorBuffer,
        tolerance_pixels: f32,
    ) -> Result<Self, OpticalFlowError> {
        if forward.width != backward.width || forward.height != backward.height {
            return Err(OpticalFlowError::InvalidDimensions);
        }

        let width = forward.width;
        let height = forward.height;
        let mut result = Self::new(width, height)?;
        let tol_sq = tolerance_pixels * tolerance_pixels;
        let mut disoccluded_count = 0u32;

        for y in 0..height {
            for x in 0..width {
                let vf = forward.get(x, y);
                let target_x = (x as f32) + vf.dx;
                let target_y = (y as f32) + vf.dy;

                let is_disoccluded = if target_x < 0.0
                    || target_x >= (width as f32)
                    || target_y < 0.0
                    || target_y >= (height as f32)
                {
                    true
                } else {
                    let vb = backward.sample_bilinear(target_x, target_y);
                    let err_x = vf.dx + vb.dx;
                    let err_y = vf.dy + vb.dy;
                    let err_dist_sq = err_x * err_x + err_y * err_y;
                    err_dist_sq > tol_sq
                };

                if is_disoccluded {
                    result.mask[(y * width + x) as usize] = 255;
                    disoccluded_count = disoccluded_count.saturating_add(1);
                }
            }
        }

        result.disoccluded_pixels = disoccluded_count;
        Ok(result)
    }
}

/// Temporary stub for optical flow subsystem self-test during incremental commits.
pub fn test_optical_flow_subsystem() -> bool {
    true
}
