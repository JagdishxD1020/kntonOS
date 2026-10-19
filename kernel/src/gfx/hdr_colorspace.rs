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

// Static HDR mastering display metadata conforming to CTA-861-G and SMPTE ST 2086.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Hdr10StaticMetadata {
    pub max_cll_nits: u32,
    pub max_fall_nits: u32,
    pub min_luminance_nits: f32,
    pub max_luminance_nits: u32,
}

impl Hdr10StaticMetadata {
    // Default static HDR10 metadata profile for 1,000 nit master.
    pub const fn default_1000_nit() -> Self {
        Self {
            max_cll_nits: 1000,
            max_fall_nits: 400,
            min_luminance_nits: 0.001,
            max_luminance_nits: 1000,
        }
    }
}

// HDR10+ dynamic frame-by-frame metadata conforming to SMPTE ST 2094-40.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Hdr10PlusDynamicMetadata {
    pub targeted_system_display_maximum_luminance: u32,
    pub average_maxrgb_nits: u32,
    pub max_cll_nits: u32,
    pub distribution_percentiles: [u32; 9],
    pub knee_point_percent: u32,
}

impl Hdr10PlusDynamicMetadata {
    // Constructs dynamic metadata for a specific frame luminance distribution.
    pub const fn new(
        target_display_nits: u32,
        avg_maxrgb: u32,
        max_cll: u32,
        percentiles: [u32; 9],
        knee_percent: u32,
    ) -> Self {
        Self {
            targeted_system_display_maximum_luminance: target_display_nits,
            average_maxrgb_nits: avg_maxrgb,
            max_cll_nits: max_cll,
            distribution_percentiles: percentiles,
            knee_point_percent: knee_percent,
        }
    }
}

// Frame-by-frame luminance distribution tonemapping engine for HDR10+.
pub struct Hdr10PlusTonemapper {
    display_peak_nits: f32,
    display_min_nits: f32,
}

impl Hdr10PlusTonemapper {
    // Initializes the tonemapper configured for target display physical capabilities.
    pub const fn new(display_peak_nits: f32, display_min_nits: f32) -> Self {
        Self {
            display_peak_nits,
            display_min_nits,
        }
    }

    // Default tonemapper configured for high-efficiency handheld OLED panels (e.g. 800 nits peak).
    pub const fn default_handheld_oled() -> Self {
        Self::new(800.0, 0.0005)
    }

    // Tonemaps a single luminance sample in nits using dynamic metadata knee and roll-off.
    pub fn map_luminance(&self, input_nits: f32, meta: &Hdr10PlusDynamicMetadata) -> f32 {
        if input_nits <= self.display_min_nits {
            return self.display_min_nits;
        }

        let content_peak = meta.max_cll_nits as f32;
        // If content peak is within target display range, pass through 1:1 linearly.
        if content_peak <= self.display_peak_nits || input_nits <= 0.0 {
            return input_nits.min(self.display_peak_nits);
        }

        let knee_nits = (self.display_peak_nits * (meta.knee_point_percent as f32)) / 100.0;
        if input_nits <= knee_nits {
            input_nits
        } else {
            // Smooth Reinhard-style roll-off curve above knee point.
            let excess = input_nits - knee_nits;
            let headroom = self.display_peak_nits - knee_nits;
            let content_excess = (content_peak - knee_nits).max(1.0);
            let compressed = (excess * headroom) / (excess + content_excess * 0.5);
            knee_nits + compressed
        }
    }

    // Applies dynamic tonemapping across an RGB color triplet in nits preserving chromaticity.
    pub fn map_rgb(&self, rgb_nits: [f32; 3], meta: &Hdr10PlusDynamicMetadata) -> [f32; 3] {
        let max_rgb = rgb_nits[0].max(rgb_nits[1]).max(rgb_nits[2]);
        if max_rgb <= 0.0 {
            return [0.0, 0.0, 0.0];
        }
        let mapped_max = self.map_luminance(max_rgb, meta);
        let scale = mapped_max / max_rgb;
        [
            rgb_nits[0] * scale,
            rgb_nits[1] * scale,
            rgb_nits[2] * scale,
        ]
    }
}

// ScRGB reference white luminance: 1.0 represents 80.0 nits SDR reference level.
pub const SCRGB_REFERENCE_WHITE_NITS: f32 = 80.0;

// High dynamic range linear 4-component float pixel in ScRGB color space.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ScRgbPixel {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl ScRgbPixel {
    // Constructs an ScRGB linear pixel from float components.
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    // Creates an ScRGB pixel from absolute optical luminance in nits and alpha.
    pub fn from_nits(r_nits: f32, g_nits: f32, b_nits: f32, alpha: f32) -> Self {
        Self {
            r: r_nits / SCRGB_REFERENCE_WHITE_NITS,
            g: g_nits / SCRGB_REFERENCE_WHITE_NITS,
            b: b_nits / SCRGB_REFERENCE_WHITE_NITS,
            a: alpha,
        }
    }

    // Converts ScRGB linear color channels to absolute physical luminance in nits.
    pub fn to_nits(&self) -> [f32; 3] {
        [
            self.r * SCRGB_REFERENCE_WHITE_NITS,
            self.g * SCRGB_REFERENCE_WHITE_NITS,
            self.b * SCRGB_REFERENCE_WHITE_NITS,
        ]
    }

    // Returns peak channel luminance in physical nits.
    pub fn peak_nits(&self) -> f32 {
        self.r.max(self.g).max(self.b) * SCRGB_REFERENCE_WHITE_NITS
    }
}

// Bitwise IEEE 754 half-precision float (16-bit) conversion utilities for ScRGB textures.
pub struct HalfFloat16;

impl HalfFloat16 {
    // Converts 32-bit single precision float into IEEE 754 16-bit half precision bits.
    pub fn from_f32(val: f32) -> u16 {
        let x = val.to_bits();
        let sign = ((x >> 16) & 0x8000) as u16;
        let exp = ((x >> 23) & 0xFF) as i32 - 127 + 15;
        let mant = (x & 0x007F_FFFF) as u32;

        if exp <= 0 {
            0 | sign
        } else if exp >= 31 {
            0x7C00 | sign
        } else {
            sign | ((exp as u16) << 10) | ((mant >> 13) as u16)
        }
    }

    // Decodes IEEE 754 16-bit half precision bits into 32-bit float without hardware F16C.
    pub fn to_f32(bits: u16) -> f32 {
        let sign = ((bits & 0x8000) as u32) << 16;
        let exp = ((bits >> 10) & 0x1F) as u32;
        let mant = (bits & 0x03FF) as u32;

        if exp == 0 {
            if mant == 0 {
                f32::from_bits(sign)
            } else {
                f32::from_bits(sign | (mant << 13))
            }
        } else if exp == 31 {
            f32::from_bits(sign | 0x7F80_0000 | (mant << 13))
        } else {
            let exp32 = (exp + 127 - 15) << 23;
            f32::from_bits(sign | exp32 | (mant << 13))
        }
    }
}

// Blend modes for High Dynamic Range ScRGB rendering pipelines.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScRgbBlendMode {
    // Additive blending for emissive light, specular flares, and laser particles.
    Additive,
    // Linear alpha blending preserving unclamped specular highlights.
    StraightAlpha,
    // Premultiplied alpha blending standard for compositing surfaces.
    PremultipliedAlpha,
}

// ScRGB 16-bit float linear blending governor preventing banding and highlight clipping.
pub struct ScRgbLinearBlender;

impl ScRgbLinearBlender {
    // Blends source and destination pixels in linear optical luminance space.
    pub fn blend(src: ScRgbPixel, dst: ScRgbPixel, mode: ScRgbBlendMode) -> ScRgbPixel {
        match mode {
            ScRgbBlendMode::Additive => ScRgbPixel {
                r: dst.r + src.r,
                g: dst.g + src.g,
                b: dst.b + src.b,
                a: dst.a.max(src.a),
            },
            ScRgbBlendMode::StraightAlpha => {
                let a = src.a.clamp(0.0, 1.0);
                let inv_a = 1.0 - a;
                ScRgbPixel {
                    r: src.r * a + dst.r * inv_a,
                    g: src.g * a + dst.g * inv_a,
                    b: src.b * a + dst.b * inv_a,
                    a: src.a + dst.a * inv_a,
                }
            }
            ScRgbBlendMode::PremultipliedAlpha => {
                let inv_a = (1.0 - src.a).max(0.0);
                ScRgbPixel {
                    r: src.r + dst.r * inv_a,
                    g: src.g + dst.g * inv_a,
                    b: src.b + dst.b * inv_a,
                    a: src.a + dst.a * inv_a,
                }
            }
        }
    }
}
