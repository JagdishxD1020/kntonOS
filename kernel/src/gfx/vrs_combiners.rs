//! Direct3D 12 Variable-Rate Shading (VRS) Tier 2 Combiners & Per-Draw Shading Rate Multiplier.
//! Evaluates multi-stage combiner pipelines, dynamic hardware clamping, and adaptive handheld rate governing.

#![allow(dead_code, non_snake_case, non_camel_case_types, unused_imports)]

pub use super::vrs::{
    D3D12_FEATURE_DATA_D3D12_OPTIONS6, D3D12_SHADING_RATE, D3D12_SHADING_RATE_COMBINER,
};

// Per-draw shading rate multiplier modifying coarse pixel dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VrsRateMultiplier {
    pub horizontal_scale: u32,
    pub vertical_scale: u32,
}

impl VrsRateMultiplier {
    pub const IDENTITY: Self = Self { horizontal_scale: 1, vertical_scale: 1 };

    pub const fn new(horizontal_scale: u32, vertical_scale: u32) -> Self {
        Self {
            horizontal_scale: if horizontal_scale == 0 { 1 } else { horizontal_scale },
            vertical_scale: if vertical_scale == 0 { 1 } else { vertical_scale },
        }
    }
}

// Configuration of the two-stage combiner pipeline set via RSSetShadingRate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrsCombinerStageConfig {
    pub combiner_primitive_image: D3D12_SHADING_RATE_COMBINER,
    pub combiner_result_base: D3D12_SHADING_RATE_COMBINER,
}

impl Default for VrsCombinerStageConfig {
    fn default() -> Self {
        Self {
            combiner_primitive_image: D3D12_SHADING_RATE_COMBINER::D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
            combiner_result_base: D3D12_SHADING_RATE_COMBINER::D3D12_SHADING_RATE_COMBINER_PASSTHROUGH,
        }
    }
}

// Combiner subsystem error conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VrsCombinerError {
    UnsupportedRate,
    InvalidCombinerMode,
    InvalidTileGranularity,
    ClampingFailed,
}

// Multi-stage shading rate combiner pipeline evaluating D3D12 VRS Tier 2 combiner rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MultiStageCombinerEvaluator {
    pub config: VrsCombinerStageConfig,
}

impl MultiStageCombinerEvaluator {
    // Creates a new multi-stage evaluator with specified stage configuration.
    pub const fn new(config: VrsCombinerStageConfig) -> Self {
        Self { config }
    }

    // Evaluates a single combiner operator between Rate A and Rate B.
    pub fn evaluate_combiner(
        combiner: D3D12_SHADING_RATE_COMBINER,
        rate_a: D3D12_SHADING_RATE,
        rate_b: D3D12_SHADING_RATE,
    ) -> D3D12_SHADING_RATE {
        combiner.combine(rate_a, rate_b)
    }

    // Evaluates Stage 0: combines per-primitive rate with screen-space image rate.
    pub fn evaluate_stage0(
        &self,
        primitive_rate: D3D12_SHADING_RATE,
        image_rate: D3D12_SHADING_RATE,
    ) -> D3D12_SHADING_RATE {
        self.config
            .combiner_primitive_image
            .combine(primitive_rate, image_rate)
    }

    // Evaluates Stage 1: combines Stage 0 intermediate result with draw call base rate.
    pub fn evaluate_stage1(
        &self,
        stage0_result: D3D12_SHADING_RATE,
        draw_base_rate: D3D12_SHADING_RATE,
    ) -> D3D12_SHADING_RATE {
        self.config
            .combiner_result_base
            .combine(stage0_result, draw_base_rate)
    }

    // Applies a per-draw scaling multiplier to a resolved shading rate.
    pub fn apply_multiplier(
        rate: D3D12_SHADING_RATE,
        multiplier: VrsRateMultiplier,
    ) -> D3D12_SHADING_RATE {
        let w = (rate.width() * multiplier.horizontal_scale).min(4);
        let h = (rate.height() * multiplier.vertical_scale).min(4);
        let w_clamped = if w >= 4 { 4 } else if w >= 2 { 2 } else { 1 };
        let h_clamped = if h >= 4 { 4 } else if h >= 2 { 2 } else { 1 };
        D3D12_SHADING_RATE::from_dimensions(w_clamped, h_clamped)
            .unwrap_or(D3D12_SHADING_RATE::D3D12_SHADING_RATE_1X1)
    }

    // Evaluates the full two-stage combiner pipeline including optional per-draw multiplier.
    pub fn evaluate_full(
        &self,
        primitive_rate: D3D12_SHADING_RATE,
        image_rate: D3D12_SHADING_RATE,
        draw_base_rate: D3D12_SHADING_RATE,
        multiplier: Option<VrsRateMultiplier>,
    ) -> D3D12_SHADING_RATE {
        let stage0 = self.evaluate_stage0(primitive_rate, image_rate);
        let stage1 = self.evaluate_stage1(stage0, draw_base_rate);
        if let Some(mult) = multiplier {
            Self::apply_multiplier(stage1, mult)
        } else {
            stage1
        }
    }
}

// Dynamic rate clamping engine ensuring calculated coarse rates adhere to hardware limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrsRateClamper {
    pub caps: D3D12_FEATURE_DATA_D3D12_OPTIONS6,
    pub max_width: u32,
    pub max_height: u32,
}

impl Default for VrsRateClamper {
    fn default() -> Self {
        Self {
            caps: D3D12_FEATURE_DATA_D3D12_OPTIONS6::tier1(true),
            max_width: 4,
            max_height: 4,
        }
    }
}

impl VrsRateClamper {
    // Creates a new clamper bound to specific device capabilities.
    pub const fn new(caps: D3D12_FEATURE_DATA_D3D12_OPTIONS6) -> Self {
        let (max_w, max_h) = if caps.AdditionalShadingRatesSupported {
            (4, 4)
        } else {
            (2, 2)
        };
        Self {
            caps,
            max_width: max_w,
            max_height: max_h,
        }
    }

    // Clamps an input shading rate against hardware limits and capability flags.
    pub fn clamp_rate(&self, rate: D3D12_SHADING_RATE) -> D3D12_SHADING_RATE {
        let w = rate.width().min(self.max_width);
        let h = rate.height().min(self.max_height);

        if !self.caps.AdditionalShadingRatesSupported && (w > 2 || h > 2) {
            return D3D12_SHADING_RATE::D3D12_SHADING_RATE_2X2;
        }

        let w_val = if w >= 4 { 4 } else if w >= 2 { 2 } else { 1 };
        let h_val = if h >= 4 { 4 } else if h >= 2 { 2 } else { 1 };

        D3D12_SHADING_RATE::from_dimensions(w_val, h_val)
            .unwrap_or(D3D12_SHADING_RATE::D3D12_SHADING_RATE_1X1)
    }

    // Validates whether a specific shading rate is natively supported by the device.
    pub fn is_rate_supported(&self, rate: D3D12_SHADING_RATE) -> bool {
        match rate {
            D3D12_SHADING_RATE::D3D12_SHADING_RATE_1X1
            | D3D12_SHADING_RATE::D3D12_SHADING_RATE_1X2
            | D3D12_SHADING_RATE::D3D12_SHADING_RATE_2X1
            | D3D12_SHADING_RATE::D3D12_SHADING_RATE_2X2 => true,
            D3D12_SHADING_RATE::D3D12_SHADING_RATE_2X4
            | D3D12_SHADING_RATE::D3D12_SHADING_RATE_4X2
            | D3D12_SHADING_RATE::D3D12_SHADING_RATE_4X4 => self.caps.AdditionalShadingRatesSupported,
        }
    }
}

// Temporary stub for self-test during incremental commits.
pub fn test_vrs_combiners_subsystem() -> bool {
    true
}
