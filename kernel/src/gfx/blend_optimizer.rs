//! Direct3D 12 Agility Dynamic Blend State Hashing & Color Blend Optimization Engine.
//! Implements blend state 64-bit FNV-1a hashing and cache deduplication, alpha-to-coverage and
//! independent render target blending optimization, and color write mask bypass optimization.

#![allow(dead_code, non_camel_case_types)]

extern crate alloc;
use alloc::vec::Vec;

// Color write enable bitflags matching D3D12_COLOR_WRITE_ENABLE.
pub const D3D12_COLOR_WRITE_MASK_RED: u8 = 1 << 0;
pub const D3D12_COLOR_WRITE_MASK_GREEN: u8 = 1 << 1;
pub const D3D12_COLOR_WRITE_MASK_BLUE: u8 = 1 << 2;
pub const D3D12_COLOR_WRITE_MASK_ALPHA: u8 = 1 << 3;
pub const D3D12_COLOR_WRITE_MASK_ALL: u8 = 0x0F;

// Blend factor operations.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DynamicBlendFactor {
    Zero = 1,
    #[default]
    One = 2,
    SrcColor = 3,
    InvSrcColor = 4,
    SrcAlpha = 5,
    InvSrcAlpha = 6,
    DestAlpha = 7,
    InvDestAlpha = 8,
    DestColor = 9,
    InvDestColor = 10,
    BlendFactor = 14,
}

// Blend arithmetic equation.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DynamicBlendOp {
    #[default]
    Add = 1,
    Subtract = 2,
    RevSubtract = 3,
    Min = 4,
    Max = 5,
}

// Per-render target blend state descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DynamicRenderTargetBlendDesc {
    pub blend_enable: bool,
    pub src_blend: DynamicBlendFactor,
    pub dest_blend: DynamicBlendFactor,
    pub blend_op: DynamicBlendOp,
    pub src_blend_alpha: DynamicBlendFactor,
    pub dest_blend_alpha: DynamicBlendFactor,
    pub blend_op_alpha: DynamicBlendOp,
    pub write_mask: u8,
}

impl Default for DynamicRenderTargetBlendDesc {
    fn default() -> Self {
        Self {
            blend_enable: false,
            src_blend: DynamicBlendFactor::One,
            dest_blend: DynamicBlendFactor::Zero,
            blend_op: DynamicBlendOp::Add,
            src_blend_alpha: DynamicBlendFactor::One,
            dest_blend_alpha: DynamicBlendFactor::Zero,
            blend_op_alpha: DynamicBlendOp::Add,
            write_mask: D3D12_COLOR_WRITE_MASK_ALL,
        }
    }
}

// Multi-render target global blend state descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DynamicBlendStateDesc {
    pub alpha_to_coverage_enable: bool,
    pub independent_blend_enable: bool,
    pub render_targets: [DynamicRenderTargetBlendDesc; 8],
}

impl Default for DynamicBlendStateDesc {
    fn default() -> Self {
        Self {
            alpha_to_coverage_enable: false,
            independent_blend_enable: false,
            render_targets: [DynamicRenderTargetBlendDesc::default(); 8],
        }
    }
}

impl DynamicBlendStateDesc {
    // Computes deterministic 64-bit FNV-1a hash of the blend state.
    pub fn compute_hash(&self) -> u64 {
        const FNV_OFFSET_BASIS: u64 = 0xCBF29CE484222325;
        const FNV_PRIME: u64 = 0x100000001B3;

        let mut hash = FNV_OFFSET_BASIS;
        hash = (hash ^ (self.alpha_to_coverage_enable as u64)).wrapping_mul(FNV_PRIME);
        hash = (hash ^ (self.independent_blend_enable as u64)).wrapping_mul(FNV_PRIME);

        for (i, rt) in self.render_targets.iter().enumerate() {
            hash = (hash ^ (i as u64)).wrapping_mul(FNV_PRIME);
            hash = (hash ^ (rt.blend_enable as u64)).wrapping_mul(FNV_PRIME);
            hash = (hash ^ (rt.src_blend as u64)).wrapping_mul(FNV_PRIME);
            hash = (hash ^ (rt.dest_blend as u64)).wrapping_mul(FNV_PRIME);
            hash = (hash ^ (rt.blend_op as u64)).wrapping_mul(FNV_PRIME);
            hash = (hash ^ (rt.src_blend_alpha as u64)).wrapping_mul(FNV_PRIME);
            hash = (hash ^ (rt.dest_blend_alpha as u64)).wrapping_mul(FNV_PRIME);
            hash = (hash ^ (rt.blend_op_alpha as u64)).wrapping_mul(FNV_PRIME);
            hash = (hash ^ (rt.write_mask as u64)).wrapping_mul(FNV_PRIME);
        }

        hash
    }
}

// Dynamic blend state hash cache eliminating redundant hardware ROP state programming.
#[derive(Debug)]
pub struct DynamicBlendStateHashCache {
    pub cached_hashes: Vec<u64>,
    pub active_state_hash: u64,
    pub total_state_evaluations: u64,
    pub redundant_rop_flushes_avoided: u64,
}

impl DynamicBlendStateHashCache {
    pub fn new() -> Self {
        Self {
            cached_hashes: Vec::new(),
            active_state_hash: 0,
            total_state_evaluations: 0,
            redundant_rop_flushes_avoided: 0,
        }
    }

    // Evaluates blend state descriptor; returns true if state changed (hardware ROP flush required).
    pub fn bind_blend_state(&mut self, desc: &DynamicBlendStateDesc) -> bool {
        self.total_state_evaluations += 1;
        let hash = desc.compute_hash();

        if hash == self.active_state_hash {
            self.redundant_rop_flushes_avoided += 1;
            return false;
        }

        self.active_state_hash = hash;
        if !self.cached_hashes.contains(&hash) {
            self.cached_hashes.push(hash);
        }

        true
    }
}


// Alpha-to-coverage and independent render target blending optimization governor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AlphaToCoverageGovernor {
    pub alpha_to_coverage_active: bool,
    pub independent_blend_active: bool,
    pub active_render_target_count: usize,
    pub total_draw_evaluations: u64,
    pub multisample_mask_generations: u64,
}

impl AlphaToCoverageGovernor {
    pub fn new() -> Self {
        Self::default()
    }

    // Configures state from active blend descriptor.
    pub fn configure(&mut self, desc: &DynamicBlendStateDesc, active_rts: usize) {
        self.alpha_to_coverage_active = desc.alpha_to_coverage_enable;
        self.independent_blend_active = desc.independent_blend_enable;
        self.active_render_target_count = active_rts.min(8);
    }

    // Computes 4-sample subpixel coverage mask from fragment alpha channel when alpha-to-coverage is enabled.
    pub fn calculate_coverage_mask(&mut self, fragment_alpha: f32) -> u8 {
        self.total_draw_evaluations += 1;
        if !self.alpha_to_coverage_active {
            return 0x0F; // Full coverage
        }

        self.multisample_mask_generations += 1;
        let alpha_clamped = if fragment_alpha < 0.0 { 0.0 } else if fragment_alpha > 1.0 { 1.0 } else { fragment_alpha };

        // D3D12 4xMSAA alpha-to-coverage threshold mapping
        if alpha_clamped >= 0.875 {
            0x0F
        } else if alpha_clamped >= 0.625 {
            0x07
        } else if alpha_clamped >= 0.375 {
            0x03
        } else if alpha_clamped >= 0.125 {
            0x01
        } else {
            0x00
        }
    }

    // Checks if render target slot uses independent blend equations.
    #[inline(always)]
    pub fn is_slot_independent(&self, slot_idx: usize) -> bool {
        self.independent_blend_active && slot_idx < self.active_render_target_count
    }
}
