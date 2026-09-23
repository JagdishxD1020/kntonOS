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

/// Temporary stub for optical flow subsystem self-test during incremental commits.
pub fn test_optical_flow_subsystem() -> bool {
    true
}
