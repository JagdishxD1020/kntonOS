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

// Self-test entrypoint stub during incremental commits.
pub fn test_bindless_heap_subsystem() -> bool {
    true
}
