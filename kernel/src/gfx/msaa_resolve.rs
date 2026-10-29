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
