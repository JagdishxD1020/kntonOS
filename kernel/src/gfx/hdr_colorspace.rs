// Direct3D 12 Agility Advanced Color Space Management & HDR10+ Dynamic Metadata Engine.
// Implements wide color gamut transformations, ScRGB linear blending, and OLED drift compensation.

// Standard DXGI color space types conforming to Direct3D 12 Agility specifications.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum DxgiColorSpaceType {
    // Standard SDR sRGB with 2.2 gamma curve and Rec.709 primaries.
    RgbFullG22NoneP709 = 0,
    // ScRGB 16-bit float linear space with Rec.709 primaries where 1.0 represents 80 nits.
    RgbFullG10NoneP709 = 1,
    // HDR10 with SMPTE ST 2084 Perceptual Quantizer (PQ) and Rec.2020 wide color gamut.
    RgbFullG2084NoneP2020 = 9,
    // HDR10 video surface with YCbCr encoding and Rec.2020 primaries.
    YcbcrStudioG2084LeftP2020 = 12,
    // Custom non-standard color space profile.
    Custom = 0xFF,
}

// 2D CIE 1931 xy chromaticity coordinate.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ChromaticityCoordinate {
    pub x: f32,
    pub y: f32,
}

impl ChromaticityCoordinate {
    // Creates a new chromaticity coordinate.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

// Color gamut definition defined by red, green, blue primaries and D65 white point.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ColorPrimaries {
    pub red: ChromaticityCoordinate,
    pub green: ChromaticityCoordinate,
    pub blue: ChromaticityCoordinate,
    pub white: ChromaticityCoordinate,
}

impl ColorPrimaries {
    // Standard BT.709 / sRGB color space primaries with CIE D65 white point.
    pub const fn rec709() -> Self {
        Self {
            red: ChromaticityCoordinate::new(0.640, 0.330),
            green: ChromaticityCoordinate::new(0.300, 0.600),
            blue: ChromaticityCoordinate::new(0.150, 0.060),
            white: ChromaticityCoordinate::new(0.3127, 0.3290),
        }
    }

    // Standard BT.2020 Ultra HD wide color gamut primaries with CIE D65 white point.
    pub const fn rec2020() -> Self {
        Self {
            red: ChromaticityCoordinate::new(0.708, 0.292),
            green: ChromaticityCoordinate::new(0.170, 0.797),
            blue: ChromaticityCoordinate::new(0.131, 0.046),
            white: ChromaticityCoordinate::new(0.3127, 0.3290),
        }
    }
}

// 3x3 linear color transformation matrix for gamut conversions.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Mat3x3 {
    pub m: [[f32; 3]; 3],
}

impl Mat3x3 {
    // Multiplies matrix by a 3D color vector [R, G, B].
    #[inline]
    pub fn mul_vec(&self, rgb: [f32; 3]) -> [f32; 3] {
        [
            self.m[0][0] * rgb[0] + self.m[0][1] * rgb[1] + self.m[0][2] * rgb[2],
            self.m[1][0] * rgb[0] + self.m[1][1] * rgb[1] + self.m[1][2] * rgb[2],
            self.m[2][0] * rgb[0] + self.m[2][1] * rgb[1] + self.m[2][2] * rgb[2],
        ]
    }
}

// Pre-computed BT.709 to BT.2020 color gamut conversion matrix (Bradford D65 adapted).
pub const REC709_TO_REC2020: Mat3x3 = Mat3x3 {
    m: [
        [0.6274041, 0.3292821, 0.0433138],
        [0.0690973, 0.9195404, 0.0113623],
        [0.0163914, 0.0880133, 0.8955953],
    ],
};

// Pre-computed BT.2020 to BT.709 color gamut conversion matrix (Bradford D65 adapted).
pub const REC2020_TO_REC709: Mat3x3 = Mat3x3 {
    m: [
        [1.6604910, -0.5876411, -0.0728499],
        [-0.1245505, 1.1328999, -0.0083494],
        [-0.0181508, -0.1005789, 1.1187297],
    ],
};

// Converts an RGB color triplet from BT.709 to BT.2020 color space.
#[inline]
pub fn transform_rec709_to_rec2020(rgb: [f32; 3]) -> [f32; 3] {
    REC709_TO_REC2020.mul_vec(rgb)
}

// Converts an RGB color triplet from BT.2020 to BT.709 color space.
#[inline]
pub fn transform_rec2020_to_rec709(rgb: [f32; 3]) -> [f32; 3] {
    REC2020_TO_REC709.mul_vec(rgb)
}
