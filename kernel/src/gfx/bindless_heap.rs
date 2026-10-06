//! Direct3D 12 Shader Model 6.8 Extended Resource Descriptors & Bindless Heap Service.
//! Provides global indexing tables for ResourceDescriptorHeap[] and SamplerDescriptorHeap[].

#![allow(dead_code, non_camel_case_types, non_snake_case)]

// Maximum standard descriptor counts for SM 6.8 bindless heaps.
pub const SM68_MAX_RESOURCE_DESCRIPTORS: usize = 4096;
pub const SM68_MAX_SAMPLER_DESCRIPTORS: usize = 2048;

// Extended descriptor classifications supported by SM 6.8 Agility SDK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedDescriptorType {
    ConstantBufferView,
    ShaderResourceTypedTexture,
    ShaderResourceRawBuffer,
    ShaderResourceRaytracingAs,
    UnorderedAccessView,
    Sampler,
}

// Error conditions returned by the bindless descriptor heap subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindlessHeapError {
    IndexOutOfBounds,
    SlotNotAllocated,
    GenerationalMismatch,
    HeapFull,
    InvalidDescriptor,
}

// Extended resource descriptor layout representing CBV/SRV/UAV in ResourceDescriptorHeap[].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtendedResourceDescriptor {
    // Descriptor category and resource view semantics.
    pub desc_type: ExtendedDescriptorType,
    // Base GPU virtual address of the bound backing resource.
    pub gpu_virtual_address: u64,
    // Byte length or element capacity of the resource view.
    pub size_in_bytes: u32,
    // Pixel or element DXGI/FourCC format identifier.
    pub format_fourcc: u32,
    // View flags controlling typed access and read-only semantics.
    pub view_flags: u32,
    // Total mip levels or array slice depth.
    pub mip_levels: u16,
    // Indicates whether this descriptor slot contains an active valid descriptor.
    pub is_valid: bool,
}

impl ExtendedResourceDescriptor {
    // Creates an empty null descriptor.
    pub const fn null() -> Self {
        Self {
            desc_type: ExtendedDescriptorType::ConstantBufferView,
            gpu_virtual_address: 0,
            size_in_bytes: 0,
            format_fourcc: 0,
            view_flags: 0,
            mip_levels: 0,
            is_valid: false,
        }
    }

    // Creates an active typed texture SRV descriptor.
    pub fn new_typed_texture(
        gpu_va: u64,
        size_bytes: u32,
        format_fourcc: u32,
        mip_levels: u16,
    ) -> Self {
        Self {
            desc_type: ExtendedDescriptorType::ShaderResourceTypedTexture,
            gpu_virtual_address: gpu_va,
            size_in_bytes: size_bytes,
            format_fourcc,
            view_flags: 0,
            mip_levels,
            is_valid: true,
        }
    }

    // Creates an active raw buffer SRV/UAV descriptor.
    pub fn new_raw_buffer(gpu_va: u64, size_bytes: u32, is_uav: bool) -> Self {
        Self {
            desc_type: if is_uav {
                ExtendedDescriptorType::UnorderedAccessView
            } else {
                ExtendedDescriptorType::ShaderResourceRawBuffer
            },
            gpu_virtual_address: gpu_va,
            size_in_bytes: size_bytes,
            format_fourcc: 0x52415742, // 'RAWB'
            view_flags: 0,
            mip_levels: 1,
            is_valid: true,
        }
    }
}

// Extended sampler descriptor layout representing SamplerDescriptorHeap[].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtendedSamplerDescriptor {
    // Texture filtering mode (Point, Bilinear, Trilinear, Anisotropic).
    pub filter_mode: u32,
    // Addressing wrap mode along U, V, and W axes.
    pub address_u: u8,
    pub address_v: u8,
    pub address_w: u8,
    // Maximum anisotropic filtering clamping level (1 to 16).
    pub max_anisotropy: u8,
    // Comparison function for shadow comparison samplers.
    pub comparison_func: u8,
    // Indicates whether this sampler slot contains a valid configuration.
    pub is_valid: bool,
}

impl ExtendedSamplerDescriptor {
    // Creates an empty null sampler descriptor.
    pub const fn null() -> Self {
        Self {
            filter_mode: 0,
            address_u: 0,
            address_v: 0,
            address_w: 0,
            max_anisotropy: 1,
            comparison_func: 0,
            is_valid: false,
        }
    }

    // Creates a standard anisotropic repeat sampler.
    pub fn new_anisotropic(anisotropy: u8) -> Self {
        Self {
            filter_mode: 0x55, // Anisotropic
            address_u: 1,      // Wrap
            address_v: 1,      // Wrap
            address_w: 1,      // Wrap
            max_anisotropy: anisotropy,
            comparison_func: 0,
            is_valid: true,
        }
    }
}

// Global ResourceDescriptorHeap[] table for SM 6.8 dynamic indexing.
#[derive(Debug, Clone)]
pub struct ResourceDescriptorHeap {
    // Contiguous table of extended resource descriptors.
    pub descriptors: alloc::vec::Vec<ExtendedResourceDescriptor>,
    // Maximum descriptor capacity of this heap.
    pub capacity: u32,
    // Total actively populated descriptors in this heap.
    pub active_count: u32,
}

impl ResourceDescriptorHeap {
    // Constructs a new bindless resource descriptor heap with fixed capacity.
    pub fn new(capacity: u32) -> Self {
        let cap = core::cmp::min(capacity as usize, SM68_MAX_RESOURCE_DESCRIPTORS);
        Self {
            descriptors: alloc::vec![ExtendedResourceDescriptor::null(); cap],
            capacity: cap as u32,
            active_count: 0,
        }
    }

    // Populates a descriptor at the specified heap index.
    pub fn set_descriptor(
        &mut self,
        index: u32,
        desc: ExtendedResourceDescriptor,
    ) -> Result<(), BindlessHeapError> {
        if index >= self.capacity {
            return Err(BindlessHeapError::IndexOutOfBounds);
        }
        let was_valid = self.descriptors[index as usize].is_valid;
        if desc.is_valid && !was_valid {
            self.active_count = self.active_count.saturating_add(1);
        } else if !desc.is_valid && was_valid {
            self.active_count = self.active_count.saturating_sub(1);
        }
        self.descriptors[index as usize] = desc;
        Ok(())
    }

    // Queries a descriptor by global index from ResourceDescriptorHeap[].
    pub fn get_descriptor(&self, index: u32) -> Result<&ExtendedResourceDescriptor, BindlessHeapError> {
        if index >= self.capacity {
            return Err(BindlessHeapError::IndexOutOfBounds);
        }
        let desc = &self.descriptors[index as usize];
        if !desc.is_valid {
            return Err(BindlessHeapError::SlotNotAllocated);
        }
        Ok(desc)
    }
}

// Global SamplerDescriptorHeap[] table for SM 6.8 dynamic sampler indexing.
#[derive(Debug, Clone)]
pub struct SamplerDescriptorHeap {
    // Contiguous table of sampler descriptors.
    pub samplers: alloc::vec::Vec<ExtendedSamplerDescriptor>,
    // Maximum sampler capacity of this heap.
    pub capacity: u32,
    // Total actively populated samplers in this heap.
    pub active_count: u32,
}

impl SamplerDescriptorHeap {
    // Constructs a new bindless sampler descriptor heap with fixed capacity.
    pub fn new(capacity: u32) -> Self {
        let cap = core::cmp::min(capacity as usize, SM68_MAX_SAMPLER_DESCRIPTORS);
        Self {
            samplers: alloc::vec![ExtendedSamplerDescriptor::null(); cap],
            capacity: cap as u32,
            active_count: 0,
        }
    }

    // Populates a sampler at the specified heap index.
    pub fn set_sampler(
        &mut self,
        index: u32,
        sampler: ExtendedSamplerDescriptor,
    ) -> Result<(), BindlessHeapError> {
        if index >= self.capacity {
            return Err(BindlessHeapError::IndexOutOfBounds);
        }
        let was_valid = self.samplers[index as usize].is_valid;
        if sampler.is_valid && !was_valid {
            self.active_count = self.active_count.saturating_add(1);
        } else if !sampler.is_valid && was_valid {
            self.active_count = self.active_count.saturating_sub(1);
        }
        self.samplers[index as usize] = sampler;
        Ok(())
    }

    // Queries a sampler by global index from SamplerDescriptorHeap[].
    pub fn get_sampler(&self, index: u32) -> Result<&ExtendedSamplerDescriptor, BindlessHeapError> {
        if index >= self.capacity {
            return Err(BindlessHeapError::IndexOutOfBounds);
        }
        let s = &self.samplers[index as usize];
        if !s.is_valid {
            return Err(BindlessHeapError::SlotNotAllocated);
        }
        Ok(s)
    }
}

// ============================================================================
// Generational Descriptor Handle Recycling Pool with ABA Prevention
// ============================================================================

// 64-bit generational descriptor handle pairing a slot index with an ABA guard generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BindlessDescriptorHandle {
    // Packed handle bits: high 32 bits = generation, low 32 bits = slot index.
    pub raw_handle: u64,
}

impl BindlessDescriptorHandle {
    // Constructs a new generational descriptor handle.
    pub const fn new(index: u32, generation: u32) -> Self {
        let raw = ((generation as u64) << 32) | (index as u64);
        Self { raw_handle: raw }
    }

    // Returns a null/invalid handle representation.
    pub const fn null() -> Self {
        Self { raw_handle: 0 }
    }

    // Extracts the 32-bit slot index from this handle.
    #[inline]
    pub const fn index(&self) -> u32 {
        self.raw_handle as u32
    }

    // Extracts the 32-bit ABA guard generation from this handle.
    #[inline]
    pub const fn generation(&self) -> u32 {
        (self.raw_handle >> 32) as u32
    }

    // Returns whether this handle is non-null and possesses a valid generation (> 0).
    #[inline]
    pub const fn is_valid(&self) -> bool {
        self.generation() > 0
    }
}

// Internal slot metadata tracking allocation status and generational increment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorSlotMeta {
    // Current generational tag incremented on each recycle to prevent ABA hazards.
    pub generation: u32,
    // Allocation state flag.
    pub is_allocated: bool,
    // Descriptor category currently occupying this slot.
    pub desc_type: ExtendedDescriptorType,
}

impl DescriptorSlotMeta {
    // Creates a new unallocated descriptor slot metadata entry with initial generation 1.
    pub const fn new() -> Self {
        Self {
            generation: 1,
            is_allocated: false,
            desc_type: ExtendedDescriptorType::ConstantBufferView,
        }
    }
}

// Generational recycling pool for dynamic descriptor handles preventing stale wave access.
#[derive(Debug, Clone)]
pub struct GenerationalDescriptorPool {
    // Per-slot metadata containing active generations.
    pub slots: alloc::vec::Vec<DescriptorSlotMeta>,
    // LIFO freelist storing available slot indices for O(1) recycling.
    pub freelist: alloc::vec::Vec<u32>,
    // Total capacity of this descriptor pool.
    pub capacity: u32,
    // Current number of actively allocated descriptor slots.
    pub allocated_count: u32,
    // Total lifetime allocations fulfilled.
    pub total_allocations: u64,
    // Total lifetime descriptor releases.
    pub total_frees: u64,
    // Total stale/stray descriptor accesses blocked due to generation mismatch.
    pub aba_mismatches_prevented: u64,
}

impl GenerationalDescriptorPool {
    // Constructs a new generational descriptor pool with the specified capacity.
    pub fn new(capacity: u32) -> Self {
        let cap = core::cmp::min(capacity as usize, SM68_MAX_RESOURCE_DESCRIPTORS);
        let mut freelist = alloc::vec::Vec::with_capacity(cap);
        for i in (0..cap).rev() {
            freelist.push(i as u32);
        }
        Self {
            slots: alloc::vec![DescriptorSlotMeta::new(); cap],
            freelist,
            capacity: cap as u32,
            allocated_count: 0,
            total_allocations: 0,
            total_frees: 0,
            aba_mismatches_prevented: 0,
        }
    }

    // Allocates a descriptor slot and returns a generational handle.
    pub fn allocate(
        &mut self,
        desc_type: ExtendedDescriptorType,
    ) -> Result<BindlessDescriptorHandle, BindlessHeapError> {
        let index = match self.freelist.pop() {
            Some(idx) => idx,
            None => return Err(BindlessHeapError::HeapFull),
        };

        let slot = &mut self.slots[index as usize];
        slot.is_allocated = true;
        slot.desc_type = desc_type;

        self.allocated_count = self.allocated_count.saturating_add(1);
        self.total_allocations = self.total_allocations.saturating_add(1);

        Ok(BindlessDescriptorHandle::new(index, slot.generation))
    }

    // Releases an allocated descriptor slot, advancing its generation to invalidate outstanding handles.
    pub fn free(&mut self, handle: BindlessDescriptorHandle) -> Result<(), BindlessHeapError> {
        let index = handle.index();
        if index >= self.capacity {
            return Err(BindlessHeapError::IndexOutOfBounds);
        }

        let slot = &mut self.slots[index as usize];
        if !slot.is_allocated {
            return Err(BindlessHeapError::SlotNotAllocated);
        }

        if slot.generation != handle.generation() {
            self.aba_mismatches_prevented = self.aba_mismatches_prevented.saturating_add(1);
            return Err(BindlessHeapError::GenerationalMismatch);
        }

        slot.is_allocated = false;
        // Increment generation and ensure it wraps around cleanly without reaching 0
        slot.generation = slot.generation.wrapping_add(1);
        if slot.generation == 0 {
            slot.generation = 1;
        }

        self.freelist.push(index);
        self.allocated_count = self.allocated_count.saturating_sub(1);
        self.total_frees = self.total_frees.saturating_add(1);
        Ok(())
    }

    // Validates that an incoming handle matches the active slot and current generation.
    pub fn validate_handle(&mut self, handle: BindlessDescriptorHandle) -> Result<u32, BindlessHeapError> {
        let index = handle.index();
        if index >= self.capacity {
            return Err(BindlessHeapError::IndexOutOfBounds);
        }

        let slot = &self.slots[index as usize];
        if !slot.is_allocated {
            return Err(BindlessHeapError::SlotNotAllocated);
        }

        if slot.generation != handle.generation() {
            self.aba_mismatches_prevented = self.aba_mismatches_prevented.saturating_add(1);
            return Err(BindlessHeapError::GenerationalMismatch);
        }

        Ok(index)
    }
}

// ============================================================================
// GPU Descriptor Table Bounds Checking & Safety Trap Engine
// ============================================================================

// Policy enacted when a shader wave executes an out-of-bounds descriptor access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorTrapPolicy {
    // Returns a safe zero-filled null descriptor, dropping writes and reading zeros.
    ReturnNullDescriptor,
    // Emits a hardware trap interrupt and halts the offending wave.
    TrapAndHalt,
}

// Diagnostic telemetry recorded during a descriptor table bounds violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorFaultDiagnostic {
    // Attempted descriptor heap index that triggered the fault.
    pub fault_index: u32,
    // Active upper bound of the descriptor table at the time of access.
    pub heap_bound: u32,
    // Virtual program counter (PC) of the offending shader instruction.
    pub shader_pc: u64,
    // Hardware execution wave / SIMD lane cluster identifier.
    pub wave_id: u32,
    // Whether the fault occurred in SamplerDescriptorHeap[] (true) or ResourceDescriptorHeap[] (false).
    pub is_sampler_heap: bool,
    // Monotonic hardware cycle or event timestamp.
    pub timestamp: u64,
}

// GPU-side descriptor table bounds checker and safety trap dispatcher.
#[derive(Debug, Clone)]
pub struct DescriptorBoundsTrapEngine {
    // Dynamic upper bound for ResourceDescriptorHeap[].
    pub resource_heap_bound: u32,
    // Dynamic upper bound for SamplerDescriptorHeap[].
    pub sampler_heap_bound: u32,
    // Active safety policy for out-of-bounds containment.
    pub trap_policy: DescriptorTrapPolicy,
    // Total descriptor lookups validated.
    pub total_accesses_checked: u64,
    // Total out-of-bounds or invalid descriptor accesses trapped.
    pub total_bounds_traps_triggered: u64,
    // Diagnostic details of the most recent trapped access fault.
    pub last_fault: Option<DescriptorFaultDiagnostic>,
    // Fallback static null resource descriptor.
    pub null_resource_descriptor: ExtendedResourceDescriptor,
    // Fallback static null sampler descriptor.
    pub null_sampler_descriptor: ExtendedSamplerDescriptor,
}

impl DescriptorBoundsTrapEngine {
    // Constructs a descriptor bounds checking and safety trap engine.
    pub fn new(
        resource_heap_bound: u32,
        sampler_heap_bound: u32,
        trap_policy: DescriptorTrapPolicy,
    ) -> Self {
        Self {
            resource_heap_bound,
            sampler_heap_bound,
            trap_policy,
            total_accesses_checked: 0,
            total_bounds_traps_triggered: 0,
            last_fault: None,
            null_resource_descriptor: ExtendedResourceDescriptor::null(),
            null_sampler_descriptor: ExtendedSamplerDescriptor::null(),
        }
    }

    // Validates a shader dynamic indexing lookup into ResourceDescriptorHeap[].
    pub fn check_resource_access<'a>(
        &'a mut self,
        heap: &'a ResourceDescriptorHeap,
        index: u32,
        shader_pc: u64,
        wave_id: u32,
        timestamp: u64,
    ) -> Result<&'a ExtendedResourceDescriptor, BindlessHeapError> {
        self.total_accesses_checked = self.total_accesses_checked.saturating_add(1);

        if index >= self.resource_heap_bound || index >= heap.capacity {
            self.total_bounds_traps_triggered = self.total_bounds_traps_triggered.saturating_add(1);
            self.last_fault = Some(DescriptorFaultDiagnostic {
                fault_index: index,
                heap_bound: self.resource_heap_bound,
                shader_pc,
                wave_id,
                is_sampler_heap: false,
                timestamp,
            });

            return match self.trap_policy {
                DescriptorTrapPolicy::ReturnNullDescriptor => Ok(&self.null_resource_descriptor),
                DescriptorTrapPolicy::TrapAndHalt => Err(BindlessHeapError::IndexOutOfBounds),
            };
        }

        match heap.get_descriptor(index) {
            Ok(desc) => Ok(desc),
            Err(BindlessHeapError::SlotNotAllocated) => {
                self.total_bounds_traps_triggered = self.total_bounds_traps_triggered.saturating_add(1);
                match self.trap_policy {
                    DescriptorTrapPolicy::ReturnNullDescriptor => Ok(&self.null_resource_descriptor),
                    DescriptorTrapPolicy::TrapAndHalt => Err(BindlessHeapError::SlotNotAllocated),
                }
            }
            Err(e) => Err(e),
        }
    }

    // Validates a shader dynamic indexing lookup into SamplerDescriptorHeap[].
    pub fn check_sampler_access<'a>(
        &'a mut self,
        heap: &'a SamplerDescriptorHeap,
        index: u32,
        shader_pc: u64,
        wave_id: u32,
        timestamp: u64,
    ) -> Result<&'a ExtendedSamplerDescriptor, BindlessHeapError> {
        self.total_accesses_checked = self.total_accesses_checked.saturating_add(1);

        if index >= self.sampler_heap_bound || index >= heap.capacity {
            self.total_bounds_traps_triggered = self.total_bounds_traps_triggered.saturating_add(1);
            self.last_fault = Some(DescriptorFaultDiagnostic {
                fault_index: index,
                heap_bound: self.sampler_heap_bound,
                shader_pc,
                wave_id,
                is_sampler_heap: true,
                timestamp,
            });

            return match self.trap_policy {
                DescriptorTrapPolicy::ReturnNullDescriptor => Ok(&self.null_sampler_descriptor),
                DescriptorTrapPolicy::TrapAndHalt => Err(BindlessHeapError::IndexOutOfBounds),
            };
        }

        match heap.get_sampler(index) {
            Ok(sampler) => Ok(sampler),
            Err(BindlessHeapError::SlotNotAllocated) => {
                self.total_bounds_traps_triggered = self.total_bounds_traps_triggered.saturating_add(1);
                match self.trap_policy {
                    DescriptorTrapPolicy::ReturnNullDescriptor => Ok(&self.null_sampler_descriptor),
                    DescriptorTrapPolicy::TrapAndHalt => Err(BindlessHeapError::SlotNotAllocated),
                }
            }
            Err(e) => Err(e),
        }
    }
}

// ============================================================================
// Handheld L2/L3 Cache Warming Pipeline for Bindless Material Lookups
// ============================================================================

// Target APU cache hierarchy tier for prefetched bindless descriptor data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheWarmLevel {
    // Local compute unit L2 cache (lowest latency ~20 cycles).
    L2Cache,
    // On-die L3 Infinity Cache / MALL (moderate latency ~90 cycles vs 300+ VRAM cycles).
    L3InfinityCache,
    // Main memory / VRAM fallback on cache miss.
    VramUncached,
}

// Tracked 64-byte hardware cache line residency in APU L2/L3 caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarmedCacheLine {
    // 64-byte aligned physical cache line tag address.
    pub cache_line_tag: u64,
    // Cache hierarchy level where line is currently resident.
    pub level: CacheWarmLevel,
    // Valid residency marker.
    pub is_valid: bool,
}

// Handheld APU hardware cache warming pipeline prefetching hot bindless material descriptors.
#[derive(Debug, Clone)]
pub struct HandheldCacheWarmingPipeline {
    // Active table of resident warmed 64-byte cache lines.
    pub warmed_lines: alloc::vec::Vec<WarmedCacheLine>,
    // Maximum cache lines budgeted for prefetch warming.
    pub max_warmed_lines: usize,
    // Total 64-byte descriptor cache lines warmed lifetime.
    pub total_lines_warmed: u64,
    // Total simulated material lookups hitting L2 cache.
    pub estimated_l2_hits: u64,
    // Total simulated material lookups hitting L3 Infinity Cache.
    pub estimated_l3_hits: u64,
    // Total high-latency VRAM DRAM round-trips avoided.
    pub vram_fetches_avoided: u64,
}

impl HandheldCacheWarmingPipeline {
    // Constructs a handheld cache warming pipeline with the specified line capacity.
    pub fn new(max_warmed_lines: usize) -> Self {
        Self {
            warmed_lines: alloc::vec::Vec::with_capacity(max_warmed_lines),
            max_warmed_lines,
            total_lines_warmed: 0,
            estimated_l2_hits: 0,
            estimated_l3_hits: 0,
            vram_fetches_avoided: 0,
        }
    }

    // Prefetches and warms a contiguous range of bindless descriptors into L2/L3 cache.
    pub fn warm_descriptor_range(
        &mut self,
        heap_base_va: u64,
        start_index: u32,
        count: u32,
        level: CacheWarmLevel,
    ) -> u32 {
        if count == 0 {
            return 0;
        }

        // 32 bytes per descriptor -> 2 descriptors per 64-byte cache line
        let start_addr = heap_base_va + (start_index as u64) * 32;
        let end_addr = start_addr + (count as u64) * 32;

        let start_tag = start_addr / 64;
        let end_tag = (end_addr.saturating_add(63)) / 64;

        let mut lines_warmed: u32 = 0;

        for tag in start_tag..end_tag {
            if let Some(existing) = self.warmed_lines.iter_mut().find(|l| l.cache_line_tag == tag && l.is_valid) {
                // Upgrade from L3 to L2 if warmer level requested
                if level == CacheWarmLevel::L2Cache && existing.level == CacheWarmLevel::L3InfinityCache {
                    existing.level = CacheWarmLevel::L2Cache;
                }
            } else {
                if self.warmed_lines.len() >= self.max_warmed_lines {
                    self.warmed_lines.remove(0); // FIFO line eviction
                }
                self.warmed_lines.push(WarmedCacheLine {
                    cache_line_tag: tag,
                    level,
                    is_valid: true,
                });
                lines_warmed = lines_warmed.saturating_add(1);
                self.total_lines_warmed = self.total_lines_warmed.saturating_add(1);
            }
        }

        lines_warmed
    }

    // Simulates a GPU wave bindless material lookup and tracks cache hit metrics.
    pub fn simulate_material_lookup(&mut self, heap_base_va: u64, index: u32) -> CacheWarmLevel {
        let addr = heap_base_va + (index as u64) * 32;
        let tag = addr / 64;

        if let Some(line) = self.warmed_lines.iter().find(|l| l.cache_line_tag == tag && l.is_valid) {
            match line.level {
                CacheWarmLevel::L2Cache => {
                    self.estimated_l2_hits = self.estimated_l2_hits.saturating_add(1);
                    self.vram_fetches_avoided = self.vram_fetches_avoided.saturating_add(1);
                    CacheWarmLevel::L2Cache
                }
                CacheWarmLevel::L3InfinityCache => {
                    self.estimated_l3_hits = self.estimated_l3_hits.saturating_add(1);
                    self.vram_fetches_avoided = self.vram_fetches_avoided.saturating_add(1);
                    CacheWarmLevel::L3InfinityCache
                }
                CacheWarmLevel::VramUncached => CacheWarmLevel::VramUncached,
            }
        } else {
            CacheWarmLevel::VramUncached
        }
    }

    // Calculates estimated memory access latency cycles saved by L2/L3 warming.
    pub fn estimated_cycles_saved(&self) -> u64 {
        // L2 hit saves ~280 cycles (300 DRAM - 20 L2); L3 hit saves ~200 cycles (300 DRAM - 100 L3)
        self.estimated_l2_hits * 280 + self.estimated_l3_hits * 200
    }
}

// Comprehensive self-test suite for Direct3D 12 SM 6.8 Bindless Heap & Extended Descriptors.
pub fn test_bindless_heap_subsystem() -> bool {
    // 1. Verify SM 6.8 ResourceDescriptorHeap & SamplerDescriptorHeap
    let mut res_heap = ResourceDescriptorHeap::new(512);
    let mut smp_heap = SamplerDescriptorHeap::new(64);

    let tex_desc = ExtendedResourceDescriptor::new_typed_texture(0x1000_0000, 1024 * 1024, 0x3231564E, 10);
    let buf_desc = ExtendedResourceDescriptor::new_raw_buffer(0x2000_0000, 65536, false);
    let uav_desc = ExtendedResourceDescriptor::new_raw_buffer(0x3000_0000, 65536, true);
    let sampler_desc = ExtendedSamplerDescriptor::new_anisotropic(16);

    if res_heap.set_descriptor(10, tex_desc).is_err()
        || res_heap.set_descriptor(11, buf_desc).is_err()
        || res_heap.set_descriptor(12, uav_desc).is_err()
    {
        return false;
    }
    if smp_heap.set_sampler(3, sampler_desc).is_err() {
        return false;
    }
    if res_heap.active_count != 3 || smp_heap.active_count != 1 {
        return false;
    }

    let q_tex = match res_heap.get_descriptor(10) {
        Ok(d) => d,
        Err(_) => return false,
    };
    if q_tex.gpu_virtual_address != 0x1000_0000 || q_tex.mip_levels != 10 {
        return false;
    }

    let q_smp = match smp_heap.get_sampler(3) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if q_smp.max_anisotropy != 16 {
        return false;
    }

    // Unallocated slot query returns SlotNotAllocated
    if res_heap.get_descriptor(50) != Err(BindlessHeapError::SlotNotAllocated) {
        return false;
    }

    // 2. Verify Generational Descriptor Recycling Pool & ABA Hazard Prevention
    let mut pool = GenerationalDescriptorPool::new(128);
    let h1 = match pool.allocate(ExtendedDescriptorType::ShaderResourceTypedTexture) {
        Ok(h) => h,
        Err(_) => return false,
    };
    if h1.index() != 0 || h1.generation() != 1 {
        return false;
    }

    // Valid handle resolves successfully
    if pool.validate_handle(h1) != Ok(0) {
        return false;
    }

    // Free handle h1 -> slot 0 generation increments from 1 to 2
    if pool.free(h1).is_err() {
        return false;
    }
    if pool.allocated_count != 0 {
        return false;
    }

    // Stale handle h1 access must fail with SlotNotAllocated
    if pool.validate_handle(h1) != Err(BindlessHeapError::SlotNotAllocated) {
        return false;
    }

    // Re-allocate slot 0 -> returns h2 with generation 2
    let h2 = match pool.allocate(ExtendedDescriptorType::UnorderedAccessView) {
        Ok(h) => h,
        Err(_) => return false,
    };
    if h2.index() != 0 || h2.generation() != 2 {
        return false;
    }

    // Stale handle h1 (generation 1) on active slot 0 with generation 2 -> ABA prevented!
    if pool.validate_handle(h1) != Err(BindlessHeapError::GenerationalMismatch) {
        return false;
    }
    if pool.aba_mismatches_prevented != 1 {
        return false;
    }

    // Double-free with stale handle h1 must fail with GenerationalMismatch
    if pool.free(h1) != Err(BindlessHeapError::GenerationalMismatch) {
        return false;
    }
    if pool.aba_mismatches_prevented != 2 {
        return false;
    }

    // Free with correct handle h2 succeeds
    if pool.free(h2).is_err() {
        return false;
    }

    // 3. Verify GPU Descriptor Bounds Checking & Safety Trap Engine
    let mut trap_engine = DescriptorBoundsTrapEngine::new(
        100, // resource heap upper bound
        16,  // sampler heap upper bound
        DescriptorTrapPolicy::ReturnNullDescriptor,
    );

    // In-bounds allocated access
    let checked_res = match trap_engine.check_resource_access(&res_heap, 10, 0x400010, 1, 100) {
        Ok(d) => d,
        Err(_) => return false,
    };
    if checked_res.gpu_virtual_address != 0x1000_0000 {
        return false;
    }

    // Out-of-bounds access (index 150 >= bound 100) -> returns null descriptor safely
    let oob_res = match trap_engine.check_resource_access(&res_heap, 150, 0x400020, 2, 101) {
        Ok(d) => d,
        Err(_) => return false,
    };
    if oob_res.is_valid || oob_res.gpu_virtual_address != 0 {
        return false;
    }
    if trap_engine.total_bounds_traps_triggered != 1 {
        return false;
    }
    let fault = match trap_engine.last_fault {
        Some(f) => f,
        None => return false,
    };
    if fault.fault_index != 150 || fault.heap_bound != 100 || fault.shader_pc != 0x400020 {
        return false;
    }

    // TrapAndHalt policy
    trap_engine.trap_policy = DescriptorTrapPolicy::TrapAndHalt;
    if trap_engine.check_resource_access(&res_heap, 200, 0x400030, 3, 102)
        != Err(BindlessHeapError::IndexOutOfBounds)
    {
        return false;
    }

    // 4. Verify Handheld L2/L3 Cache Warming Pipeline
    let mut cache_pipeline = HandheldCacheWarmingPipeline::new(128);
    let heap_base = 0x0000_7FFF_E000_0000;

    // Warm descriptors 0..16 into L2 cache (8 cache lines = 512 bytes)
    let warmed_count = cache_pipeline.warm_descriptor_range(heap_base, 0, 16, CacheWarmLevel::L2Cache);
    if warmed_count != 8 || cache_pipeline.total_lines_warmed != 8 {
        return false;
    }

    // Warm descriptors 16..32 into L3 Infinity Cache
    let l3_warmed = cache_pipeline.warm_descriptor_range(heap_base, 16, 16, CacheWarmLevel::L3InfinityCache);
    if l3_warmed != 8 || cache_pipeline.total_lines_warmed != 16 {
        return false;
    }

    // Simulate lookups
    if cache_pipeline.simulate_material_lookup(heap_base, 5) != CacheWarmLevel::L2Cache {
        return false;
    }
    if cache_pipeline.simulate_material_lookup(heap_base, 20) != CacheWarmLevel::L3InfinityCache {
        return false;
    }
    // Lookup unwarmed descriptor (index 80) -> VramUncached
    if cache_pipeline.simulate_material_lookup(heap_base, 80) != CacheWarmLevel::VramUncached {
        return false;
    }

    if cache_pipeline.estimated_l2_hits != 1
        || cache_pipeline.estimated_l3_hits != 1
        || cache_pipeline.vram_fetches_avoided != 2
    {
        return false;
    }
    if cache_pipeline.estimated_cycles_saved() != 480 {
        return false;
    }

    // 5. Emit deterministic boot sentinel
    crate::serial_write_str("[KNT-D3D12-BINDLESS-HEAP-PASS] Direct3D 12 SM 6.8 bindless heap & descriptor safety verified\r\n");
    true
}
