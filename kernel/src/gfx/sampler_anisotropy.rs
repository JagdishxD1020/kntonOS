//! Direct3D 12 Agility Dynamic Sampler LOD Bias Injection & Texture Filtering Anisotropy Governor.
//! Implements dynamic per-draw LOD bias injection based on GPU frametime and thermal throttle headroom,
//! dynamic anisotropic filtering level adaptation, and sampler descriptor heap deduplication.

#![allow(dead_code, non_camel_case_types)]

extern crate alloc;

// Filter reduction and comparison modes.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DynamicFilterType {
    #[default]
    Point = 0,
    Linear = 1,
    Anisotropic = 2,
}

// Thermal and frametime throttle headroom state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuPerformanceState {
    pub current_frametime_ms: f32,
    pub target_frametime_ms: f32,
    pub gpu_temperature_celsius: f32,
    pub thermal_throttle_temp_celsius: f32,
}

impl GpuPerformanceState {
    pub const fn new(current_frametime_ms: f32, target_frametime_ms: f32, temp: f32, throttle_temp: f32) -> Self {
        Self {
            current_frametime_ms,
            target_frametime_ms,
            gpu_temperature_celsius: temp,
            thermal_throttle_temp_celsius: throttle_temp,
        }
    }

    #[inline(always)]
    pub fn is_under_heavy_load(&self) -> bool {
        self.current_frametime_ms > self.target_frametime_ms * 1.05
            || self.gpu_temperature_celsius >= self.thermal_throttle_temp_celsius - 2.0
    }
}

// Dynamic per-draw LOD bias injector governor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicLodBiasGovernor {
    pub base_lod_bias: f32,
    pub max_injected_bias: f32,
    pub current_injected_bias: f32,
    pub total_bias_evaluations: u64,
    pub total_bias_injections: u64,
}

impl DynamicLodBiasGovernor {
    pub fn new(max_bias: f32) -> Self {
        Self {
            base_lod_bias: 0.0,
            max_injected_bias: max_bias,
            current_injected_bias: 0.0,
            total_bias_evaluations: 0,
            total_bias_injections: 0,
        }
    }

    // Computes dynamic per-draw LOD bias adjustment based on GPU load and thermal headroom.
    pub fn calculate_injected_lod_bias(&mut self, perf: &GpuPerformanceState) -> f32 {
        self.total_bias_evaluations += 1;

        if perf.is_under_heavy_load() {
            // Under load or thermal stress, inject positive LOD bias (0.5 to max_bias) to select coarser mips
            let frametime_ratio = perf.current_frametime_ms / perf.target_frametime_ms.max(1.0);
            let bias = ((frametime_ratio - 1.0) * 1.5).min(self.max_injected_bias).max(0.5);
            self.current_injected_bias = bias;
            self.total_bias_injections += 1;
        } else {
            // Normal operation, zero extra bias
            self.current_injected_bias = 0.0;
        }

        self.base_lod_bias + self.current_injected_bias
    }
}


// Surface projection viewing angle in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceViewAngle {
    pub grazing_angle_degrees: f32,
}

impl SurfaceViewAngle {
    pub const fn new(degrees: f32) -> Self {
        Self { grazing_angle_degrees: degrees }
    }
}

// Memory bus bandwidth saturation state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryBandwidthState {
    pub current_vram_bandwidth_utilization_ratio: f32,
    pub max_sustainable_utilization_ratio: f32,
}

impl MemoryBandwidthState {
    pub const fn new(current_util: f32, max_util: f32) -> Self {
        Self {
            current_vram_bandwidth_utilization_ratio: current_util,
            max_sustainable_utilization_ratio: max_util,
        }
    }

    #[inline(always)]
    pub fn is_bandwidth_saturated(&self) -> bool {
        self.current_vram_bandwidth_utilization_ratio >= self.max_sustainable_utilization_ratio
    }
}

// Dynamic anisotropic filtering level governor (1x to 16x).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicAnisotropyGovernor {
    pub max_permitted_anisotropy: u32,
    pub active_anisotropy_level: u32,
    pub total_anisotropy_evaluations: u64,
    pub total_throttled_anisotropy_draws: u64,
}

impl DynamicAnisotropyGovernor {
    pub fn new(max_aniso: u32) -> Self {
        let max_val = max_aniso.clamp(1, 16);
        Self {
            max_permitted_anisotropy: max_val,
            active_anisotropy_level: max_val,
            total_anisotropy_evaluations: 0,
            total_throttled_anisotropy_draws: 0,
        }
    }

    // Evaluates optimal anisotropic level adapting to viewing angle and memory bandwidth pressure.
    pub fn calculate_anisotropy_level(&mut self, angle: SurfaceViewAngle, bw: &MemoryBandwidthState) -> u32 {
        self.total_anisotropy_evaluations += 1;

        // Base anisotropy calculation from grazing angle: higher angles require higher aniso
        let base_level = if angle.grazing_angle_degrees >= 75.0 {
            16
        } else if angle.grazing_angle_degrees >= 60.0 {
            8
        } else if angle.grazing_angle_degrees >= 45.0 {
            4
        } else if angle.grazing_angle_degrees >= 30.0 {
            2
        } else {
            1
        };

        let clamped_base = base_level.min(self.max_permitted_anisotropy);

        // If memory bus is saturated, throttle anisotropy by half to alleviate cache thrashing
        let resolved = if bw.is_bandwidth_saturated() {
            self.total_throttled_anisotropy_draws += 1;
            (clamped_base / 2).max(1)
        } else {
            clamped_base
        };

        self.active_anisotropy_level = resolved;
        resolved
    }
}

// Sampler descriptor configuration key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SamplerDescriptorKey {
    pub filter: DynamicFilterType,
    pub max_anisotropy: u32,
    pub lod_bias_fixed_pt: i32, // Fixed point representation of LOD bias
    pub address_u: u8,
    pub address_v: u8,
    pub address_w: u8,
}

impl SamplerDescriptorKey {
    pub fn new(filter: DynamicFilterType, max_anisotropy: u32, lod_bias: f32) -> Self {
        Self {
            filter,
            max_anisotropy,
            lod_bias_fixed_pt: (lod_bias * 256.0) as i32,
            address_u: 0,
            address_v: 0,
            address_w: 0,
        }
    }
}

// Allocated sampler descriptor slot in the heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SamplerHeapSlot {
    pub heap_index: u32,
    pub key: SamplerDescriptorKey,
    pub reference_count: u32,
}

// Sampler descriptor heap table deduplication and cache locality optimizer.
#[derive(Debug)]
pub struct SamplerDescriptorTableOptimizer {
    pub heap_capacity: u32,
    pub allocated_slots: alloc::vec::Vec<SamplerHeapSlot>,
    pub total_lookup_requests: u64,
    pub deduplication_hits: u64,
}

impl SamplerDescriptorTableOptimizer {
    pub fn new(capacity: u32) -> Self {
        Self {
            heap_capacity: capacity,
            allocated_slots: alloc::vec::Vec::new(),
            total_lookup_requests: 0,
            deduplication_hits: 0,
        }
    }

    // Finds an identical sampler descriptor slot or allocates a new deduplicated slot.
    pub fn get_or_create_sampler(&mut self, key: SamplerDescriptorKey) -> Result<u32, &'static str> {
        self.total_lookup_requests += 1;

        // Check for existing identical descriptor (deduplication)
        for slot in &mut self.allocated_slots {
            if slot.key == key {
                slot.reference_count += 1;
                self.deduplication_hits += 1;
                return Ok(slot.heap_index);
            }
        }

        // Allocate new slot if capacity allows
        if self.allocated_slots.len() as u32 >= self.heap_capacity {
            return Err("Sampler descriptor heap full");
        }

        let new_index = self.allocated_slots.len() as u32;
        self.allocated_slots.push(SamplerHeapSlot {
            heap_index: new_index,
            key,
            reference_count: 1,
        });

        Ok(new_index)
    }

    #[inline(always)]
    pub fn deduplication_ratio_percentage(&self) -> u32 {
        if self.total_lookup_requests == 0 {
            0
        } else {
            ((self.deduplication_hits * 100) / self.total_lookup_requests) as u32
        }
    }
}

// Runtime telemetry counters for dynamic sampler tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SamplerTuningMetrics {
    pub total_draw_samplers_evaluated: u64,
    pub total_lod_biases_injected: u64,
    pub total_anisotropy_throttles: u64,
    pub total_deduplication_reuses: u64,
}

// Integrated Direct3D 12 Agility Dynamic Sampler LOD Bias & Anisotropy Governor.
#[derive(Debug)]
pub struct D3d12DynamicSamplerGovernor {
    pub lod_governor: DynamicLodBiasGovernor,
    pub aniso_governor: DynamicAnisotropyGovernor,
    pub descriptor_optimizer: SamplerDescriptorTableOptimizer,
    pub metrics: SamplerTuningMetrics,
}

impl D3d12DynamicSamplerGovernor {
    pub fn new(max_bias: f32, max_aniso: u32, heap_capacity: u32) -> Self {
        Self {
            lod_governor: DynamicLodBiasGovernor::new(max_bias),
            aniso_governor: DynamicAnisotropyGovernor::new(max_aniso),
            descriptor_optimizer: SamplerDescriptorTableOptimizer::new(heap_capacity),
            metrics: SamplerTuningMetrics::default(),
        }
    }

    // Resolves a dynamically tuned sampler descriptor for an upcoming draw call.
    pub fn resolve_tuned_sampler(
        &mut self,
        filter: DynamicFilterType,
        angle: SurfaceViewAngle,
        perf: &GpuPerformanceState,
        bw: &MemoryBandwidthState,
    ) -> Result<u32, &'static str> {
        self.metrics.total_draw_samplers_evaluated += 1;

        // 1. Calculate dynamic LOD bias based on GPU thermal/frametime headroom
        let lod_bias = self.lod_governor.calculate_injected_lod_bias(perf);
        if lod_bias > 0.0 {
            self.metrics.total_lod_biases_injected += 1;
        }

        // 2. Calculate dynamic anisotropy level adapting to grazing angle & memory bandwidth
        let aniso = if filter == DynamicFilterType::Anisotropic {
            let val = self.aniso_governor.calculate_anisotropy_level(angle, bw);
            if bw.is_bandwidth_saturated() {
                self.metrics.total_anisotropy_throttles += 1;
            }
            val
        } else {
            1
        };

        // 3. Deduplicate descriptor in heap
        let key = SamplerDescriptorKey::new(filter, aniso, lod_bias);
        let slot = self.descriptor_optimizer.get_or_create_sampler(key)?;
        if self.descriptor_optimizer.deduplication_hits > self.metrics.total_deduplication_reuses {
            self.metrics.total_deduplication_reuses = self.descriptor_optimizer.deduplication_hits;
        }

        Ok(slot)
    }
}

// Verification self-test suite for D3D12 Agility Dynamic Sampler LOD Bias & Anisotropy Governor.
pub fn test_sampler_anisotropy_subsystem() -> bool {
    crate::serial_write_str("[KNT-GFX] Testing Direct3D 12 dynamic sampler LOD bias & anisotropy governor...\r\n");

    let mut governor = D3d12DynamicSamplerGovernor::new(2.5, 16, 64);

    // 1. Normal state: no thermal pressure, no bandwidth saturation.
    let normal_perf = GpuPerformanceState::new(16.0, 16.6, 65.0, 85.0);
    let normal_bw = MemoryBandwidthState::new(0.50, 0.85);
    let angle_flat = SurfaceViewAngle::new(20.0);

    let slot0 = match governor.resolve_tuned_sampler(DynamicFilterType::Anisotropic, angle_flat, &normal_perf, &normal_bw) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if slot0 != 0 {
        return false;
    }
    if governor.lod_governor.current_injected_bias != 0.0 {
        return false;
    }
    if governor.aniso_governor.active_anisotropy_level != 1 {
        return false;
    }

    // 2. High grazing angle (75 deg) with no BW pressure -> Anisotropy should be 16x.
    let angle_steep = SurfaceViewAngle::new(80.0);
    let slot1 = match governor.resolve_tuned_sampler(DynamicFilterType::Anisotropic, angle_steep, &normal_perf, &normal_bw) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if slot1 != 1 {
        return false;
    }
    if governor.aniso_governor.active_anisotropy_level != 16 {
        return false;
    }

    // 3. High grazing angle under memory bandwidth saturation -> Anisotropy should be throttled to 8x.
    let saturated_bw = MemoryBandwidthState::new(0.92, 0.85);
    let slot2 = match governor.resolve_tuned_sampler(DynamicFilterType::Anisotropic, angle_steep, &normal_perf, &saturated_bw) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if slot2 != 2 {
        return false;
    }
    if governor.aniso_governor.active_anisotropy_level != 8 {
        return false;
    }
    if governor.metrics.total_anisotropy_throttles != 1 {
        return false;
    }

    // 4. Heavy load / thermal throttle -> LOD bias injection active.
    let heavy_perf = GpuPerformanceState::new(25.0, 16.6, 84.0, 85.0);
    let slot3 = match governor.resolve_tuned_sampler(DynamicFilterType::Linear, angle_flat, &heavy_perf, &normal_bw) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if slot3 != 3 {
        return false;
    }
    if governor.lod_governor.current_injected_bias <= 0.0 {
        return false;
    }
    if governor.metrics.total_lod_biases_injected != 1 {
        return false;
    }

    // 5. Deduplication hit: request same parameters as slot0.
    let slot0_reuse = match governor.resolve_tuned_sampler(DynamicFilterType::Anisotropic, angle_flat, &normal_perf, &normal_bw) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if slot0_reuse != slot0 {
        return false;
    }
    if governor.descriptor_optimizer.deduplication_hits != 1 {
        return false;
    }

    // 6. Emit deterministic boot sentinel.
    crate::serial_write_str("[KNT-D3D12-SAMPLER-ANISO-PASS] Direct3D 12 dynamic sampler LOD bias and anisotropy governor verified\r\n");

    true
}
