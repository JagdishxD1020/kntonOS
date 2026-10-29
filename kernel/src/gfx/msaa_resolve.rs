// Direct3D 12 Agility Dynamic Multisample Anti-Aliasing (MSAA) Resolve Governor

// Hardware MSAA 2x/4x/8x resolve subpass optimizer and programmable sample patterns.



use core::sync::atomic::{AtomicU64, Ordering};



// MSAA sample count selection for color, depth, and stencil surfaces.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]

#[repr(u32)]

pub enum MsaaSampleCount {

    Count1x = 1,

    Count2x = 2,

    Count4x = 4,

    Count8x = 8,

    Count16x = 16,

}



impl MsaaSampleCount {

    // Return numerical sample count.

    pub const fn count(self) -> u32 {

        self as u32

    }



    // Check if multi-sampling is active.

    pub const fn is_multisampled(self) -> bool {

        (self as u32) > 1

    }

}



// Sub-pixel sample position offset within standard 16x16 sub-pixel coordinate space.

// Coordinate components range from -8 to 7, corresponding to [-8/16, +7/16] from pixel center.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]

pub struct SamplePosition {

    pub x: i8,

    pub y: i8,

}



impl SamplePosition {

    // Create new sample position offset clamped to [-8, 7].

    pub const fn new(x: i8, y: i8) -> Self {

        let cx = if x < -8 { -8 } else if x > 7 { 7 } else { x };

        let cy = if y < -8 { -8 } else if y > 7 { 7 } else { y };

        Self { x: cx, y: cy }

    }



    // Convert sub-pixel offset into normalized float offset [-0.5, +0.4375].

    pub fn normalized_offset(self) -> (f32, f32) {

        ((self.x as f32) / 16.0, (self.y as f32) / 16.0)

    }

}



// Maximum sample count supported for programmable sample positions (up to 16x).

pub const MAX_PROGRAMMABLE_SAMPLES: usize = 16;



// Programmable sample pattern table for a specific MSAA level.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]

pub struct ProgrammableSamplePattern {

    pub sample_count: MsaaSampleCount,

    pub positions: [SamplePosition; MAX_PROGRAMMABLE_SAMPLES],

    pub is_custom: bool,

}



impl ProgrammableSamplePattern {

    // Construct standard standard-conformant D3D12 sample locations.

    pub const fn standard(sample_count: MsaaSampleCount) -> Self {

        let mut positions = [SamplePosition::new(0, 0); MAX_PROGRAMMABLE_SAMPLES];

        match sample_count {

            MsaaSampleCount::Count1x => {

                positions[0] = SamplePosition::new(0, 0);

            }

            MsaaSampleCount::Count2x => {

                // Standard 2x sample pattern (diagonal)

                positions[0] = SamplePosition::new(4, 4);

                positions[1] = SamplePosition::new(-4, -4);

            }

            MsaaSampleCount::Count4x => {

                // Standard 4x sample pattern (rotated grid)

                positions[0] = SamplePosition::new(-2, -6);

                positions[1] = SamplePosition::new(6, -2);

                positions[2] = SamplePosition::new(-6, 2);

                positions[3] = SamplePosition::new(2, 6);

            }

            MsaaSampleCount::Count8x => {

                // Standard 8x sample pattern

                positions[0] = SamplePosition::new(1, -3);

                positions[1] = SamplePosition::new(-1, 3);

                positions[2] = SamplePosition::new(5, 1);

                positions[3] = SamplePosition::new(-3, -5);

                positions[4] = SamplePosition::new(-5, 5);

                positions[5] = SamplePosition::new(-7, -1);

                positions[6] = SamplePosition::new(3, 7);

                positions[7] = SamplePosition::new(7, -7);

            }

            MsaaSampleCount::Count16x => {

                // Standard 16x sample pattern

                positions[0] = SamplePosition::new(1, 1);

                positions[1] = SamplePosition::new(-1, -3);

                positions[2] = SamplePosition::new(-3, 2);

                positions[3] = SamplePosition::new(4, -1);

                positions[4] = SamplePosition::new(-5, -2);

                positions[5] = SamplePosition::new(2, 5);

                positions[6] = SamplePosition::new(5, 3);

                positions[7] = SamplePosition::new(3, -5);

                positions[8] = SamplePosition::new(-2, 6);

                positions[9] = SamplePosition::new(0, -7);

                positions[10] = SamplePosition::new(-4, -6);

                positions[11] = SamplePosition::new(-6, 4);

                positions[12] = SamplePosition::new(-8, 0);

                positions[13] = SamplePosition::new(7, -4);

                positions[14] = SamplePosition::new(6, 7);

                positions[15] = SamplePosition::new(-7, -8);

            }

        }

        Self {

            sample_count,

            positions,

            is_custom: false,

        }

    }



    // Construct custom programmable sample pattern.

    pub fn custom(sample_count: MsaaSampleCount, slice: &[SamplePosition]) -> Self {

        let mut positions = [SamplePosition::new(0, 0); MAX_PROGRAMMABLE_SAMPLES];

        let n = core::cmp::min(slice.len(), sample_count.count() as usize);

        let mut i = 0;

        while i < n {

            positions[i] = slice[i];

            i += 1;

        }

        Self {

            sample_count,

            positions,

            is_custom: true,

        }

    }

}



// Rectangle bounding subpass region for MSAA resolve.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]

pub struct ResolveRect {

    pub left: u32,

    pub top: u32,

    pub right: u32,

    pub bottom: u32,

}



impl ResolveRect {

    // Construct resolve subresource rectangle.

    pub const fn new(left: u32, top: u32, right: u32, bottom: u32) -> Self {

        Self { left, top, right, bottom }

    }



    // Compute width of the resolve bounds.

    pub const fn width(&self) -> u32 {

        if self.right > self.left { self.right - self.left } else { 0 }

    }



    // Compute height of the resolve bounds.

    pub const fn height(&self) -> u32 {

        if self.bottom > self.top { self.bottom - self.top } else { 0 }

    }

}



// Optimizer managing hardware MSAA resolve subpasses and sample pattern state.

pub struct MsaaResolveSubpassOptimizer {

    active_pattern_2x: ProgrammableSamplePattern,

    active_pattern_4x: ProgrammableSamplePattern,

    active_pattern_8x: ProgrammableSamplePattern,

    subpass_resolve_count: AtomicU64,

}



impl MsaaResolveSubpassOptimizer {

    // Initialize optimizer with standard Jitter sample patterns.

    pub const fn new() -> Self {

        Self {

            active_pattern_2x: ProgrammableSamplePattern::standard(MsaaSampleCount::Count2x),

            active_pattern_4x: ProgrammableSamplePattern::standard(MsaaSampleCount::Count4x),

            active_pattern_8x: ProgrammableSamplePattern::standard(MsaaSampleCount::Count8x),

            subpass_resolve_count: AtomicU64::new(0),

        }

    }



    // Retrieve active sample pattern for the requested MSAA count.

    pub fn get_pattern(&self, count: MsaaSampleCount) -> &ProgrammableSamplePattern {

        match count {

            MsaaSampleCount::Count2x => &self.active_pattern_2x,

            MsaaSampleCount::Count4x => &self.active_pattern_4x,

            MsaaSampleCount::Count8x | MsaaSampleCount::Count16x => &self.active_pattern_8x,

            _ => &self.active_pattern_4x,

        }

    }



    // Update programmable sample positions for specified MSAA level.

    pub fn set_custom_pattern(&mut self, pattern: ProgrammableSamplePattern) {

        match pattern.sample_count {

            MsaaSampleCount::Count2x => self.active_pattern_2x = pattern,

            MsaaSampleCount::Count4x => self.active_pattern_4x = pattern,

            MsaaSampleCount::Count8x => self.active_pattern_8x = pattern,

            _ => {}

        }

    }



    // Reset sample positions to standard D3D12 hardware defaults.

    pub fn reset_to_standard(&mut self, count: MsaaSampleCount) {

        self.set_custom_pattern(ProgrammableSamplePattern::standard(count));

    }



    // Track resolve invocation.

    pub fn record_resolve(&self) {

        self.subpass_resolve_count.fetch_add(1, Ordering::Relaxed);

    }



    // Get total subpass resolves processed.

    pub fn total_resolves(&self) -> u64 {

        self.subpass_resolve_count.load(Ordering::Relaxed)

    }

}



// Depth resolve filter mode according to D3D12_RESOLVE_MODE.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]

pub enum DepthResolveFilter {

    // Select minimum depth across all active sample locations (standard for reverse-Z occlusion).

    Min,

    // Select maximum depth across all active sample locations (standard for normal-Z occlusion).

    Max,

    // Average sample depths across all locations.

    Average,

    // Take sample zero directly without interpolation.

    SampleZero,

}



// Stencil resolve filter mode according to D3D12_RESOLVE_MODE.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]

pub enum StencilResolveFilter {

    Min,

    Max,

    SampleZero,

}



// Depth and stencil resolve execution engine operating over multisampled subresources.

pub struct DepthStencilResolveEngine {

    total_depth_resolves: AtomicU64,

    total_stencil_resolves: AtomicU64,

}



impl DepthStencilResolveEngine {

    // Initialize depth/stencil resolve filter selector.

    pub const fn new() -> Self {

        Self {

            total_depth_resolves: AtomicU64::new(0),

            total_stencil_resolves: AtomicU64::new(0),

        }

    }



    // Resolve multisampled depth samples into a single resolved depth value without CPU stall.

    pub fn resolve_depth(&self, samples: &[f32], filter: DepthResolveFilter) -> f32 {

        self.total_depth_resolves.fetch_add(1, Ordering::Relaxed);

        if samples.is_empty() {

            return 1.0;

        }

        match filter {

            DepthResolveFilter::SampleZero => samples[0],

            DepthResolveFilter::Min => {

                let mut min_val = samples[0];

                let mut i = 1;

                while i < samples.len() {

                    if samples[i] < min_val {

                        min_val = samples[i];

                    }

                    i += 1;

                }

                min_val

            }

            DepthResolveFilter::Max => {

                let mut max_val = samples[0];

                let mut i = 1;

                while i < samples.len() {

                    if samples[i] > max_val {

                        max_val = samples[i];

                    }

                    i += 1;

                }

                max_val

            }

            DepthResolveFilter::Average => {

                let mut sum = 0.0f32;

                let mut i = 0;

                while i < samples.len() {

                    sum += samples[i];

                    i += 1;

                }

                sum / (samples.len() as f32)

            }

        }

    }



    // Resolve multisampled stencil samples into single stencil byte value.

    pub fn resolve_stencil(&self, samples: &[u8], filter: StencilResolveFilter) -> u8 {

        self.total_stencil_resolves.fetch_add(1, Ordering::Relaxed);

        if samples.is_empty() {

            return 0;

        }

        match filter {

            StencilResolveFilter::SampleZero => samples[0],

            StencilResolveFilter::Min => {

                let mut min_val = samples[0];

                let mut i = 1;

                while i < samples.len() {

                    if samples[i] < min_val {

                        min_val = samples[i];

                    }

                    i += 1;

                }

                min_val

            }

            StencilResolveFilter::Max => {

                let mut max_val = samples[0];

                let mut i = 1;

                while i < samples.len() {

                    if samples[i] > max_val {

                        max_val = samples[i];

                    }

                    i += 1;

                }

                max_val

            }

        }

    }



    // Get total depth resolves performed.

    pub fn total_depth_resolves(&self) -> u64 {

        self.total_depth_resolves.load(Ordering::Relaxed)

    }



    // Get total stencil resolves performed.

    pub fn total_stencil_resolves(&self) -> u64 {

        self.total_stencil_resolves.load(Ordering::Relaxed)

    }

}

// High Dynamic Range color sample representation with linear floating-point channels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HdrColorSample {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl HdrColorSample {
    // Construct linear HDR color sample.
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    // Calculate perceived luminance using Rec.709 coefficients.
    pub fn luminance(&self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }
}

// Tonemapping curve selection for high-contrast HDR MSAA resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HdrResolveToneMapMode {
    // Standard Reinhard tone mapping weight: 1 / (1 + L).
    Reinhard,
    // Luminance-weighted exponential curve.
    LuminanceWeighted,
    // Direct unweighted linear average (fallback).
    BoxFilter,
}

// HDR tone-mapped MSAA color resolve engine.
// Eliminates chromatic fringing and dark rim artifacts at high-contrast specular edges.
pub struct HdrMsaaColorResolver {
    active_tonemap_mode: HdrResolveToneMapMode,
    total_hdr_resolves: AtomicU64,
}

impl HdrMsaaColorResolver {
    // Initialize HDR MSAA color resolver.
    pub const fn new() -> Self {
        Self {
            active_tonemap_mode: HdrResolveToneMapMode::Reinhard,
            total_hdr_resolves: AtomicU64::new(0),
        }
    }

    // Set tone mapping curve mode.
    pub fn set_tonemap_mode(&mut self, mode: HdrResolveToneMapMode) {
        self.active_tonemap_mode = mode;
    }

    // Resolve multisampled HDR linear colors using tone-mapped sample weights.
    pub fn resolve_hdr_samples(&self, samples: &[HdrColorSample]) -> HdrColorSample {
        self.total_hdr_resolves.fetch_add(1, Ordering::Relaxed);
        if samples.is_empty() {
            return HdrColorSample::new(0.0, 0.0, 0.0, 0.0);
        }
        if samples.len() == 1 || self.active_tonemap_mode == HdrResolveToneMapMode::BoxFilter {
            let mut r = 0.0f32;
            let mut g = 0.0f32;
            let mut b = 0.0f32;
            let mut a = 0.0f32;
            let mut i = 0;
            while i < samples.len() {
                r += samples[i].r;
                g += samples[i].g;
                b += samples[i].b;
                a += samples[i].a;
                i += 1;
            }
            let inv_n = 1.0 / (samples.len() as f32);
            return HdrColorSample::new(r * inv_n, g * inv_n, b * inv_n, a * inv_n);
        }

        let mut weighted_r = 0.0f32;
        let mut weighted_g = 0.0f32;
        let mut weighted_b = 0.0f32;
        let mut weighted_a = 0.0f32;
        let mut total_weight = 0.0f32;

        let mut i = 0;
        while i < samples.len() {
            let lum = samples[i].luminance();
            let safe_lum = if lum < 0.0 { 0.0 } else { lum };
            let w = match self.active_tonemap_mode {
                HdrResolveToneMapMode::Reinhard => 1.0 / (1.0 + safe_lum),
                HdrResolveToneMapMode::LuminanceWeighted => 1.0 / (1.0 + safe_lum * 0.5),
                HdrResolveToneMapMode::BoxFilter => 1.0,
            };
            weighted_r += samples[i].r * w;
            weighted_g += samples[i].g * w;
            weighted_b += samples[i].b * w;
            weighted_a += samples[i].a * w;
            total_weight += w;
            i += 1;
        }

        if total_weight <= 0.00001 {
            return samples[0];
        }
        let inv_total_weight = 1.0 / total_weight;
        let avg_r = weighted_r * inv_total_weight;
        let avg_g = weighted_g * inv_total_weight;
        let avg_b = weighted_b * inv_total_weight;
        let avg_a = weighted_a * inv_total_weight;

        // Inverse tonemap re-expansion to restore full HDR luminance range
        let avg_lum = 0.2126 * avg_r + 0.7152 * avg_g + 0.0722 * avg_b;
        let safe_avg_lum = if avg_lum < 0.0 { 0.0 } else { avg_lum };
        let inv_scale = match self.active_tonemap_mode {
            HdrResolveToneMapMode::Reinhard => {
                if safe_avg_lum >= 0.999 { 100.0 } else { 1.0 / (1.0 - safe_avg_lum) }
            }
            _ => 1.0,
        };

        // Clamp extreme multipliers to prevent float overflows
        let scale = if inv_scale < 1.0 { 1.0 } else if inv_scale > 10.0 { 10.0 } else { inv_scale };
        HdrColorSample::new(avg_r * scale, avg_g * scale, avg_b * scale, avg_a)
    }

    // Total HDR color resolves performed.
    pub fn total_hdr_resolves(&self) -> u64 {
        self.total_hdr_resolves.load(Ordering::Relaxed)
    }
}
