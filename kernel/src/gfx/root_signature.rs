//! Direct3D 12 Root Signature & Descriptor Heap Binding Table Engine.
//! Parameter descriptors, descriptor ranges, and pipeline visibility bindings.

#![allow(dead_code, non_camel_case_types)]

extern crate alloc;
use alloc::vec::Vec;

/// Direct3D 12 root parameter types defining how shader arguments are passed.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum D3D12_ROOT_PARAMETER_TYPE {
    /// Pointer to a contiguous table of descriptors within a descriptor heap.
    DescriptorTable = 0,
    /// Inline 32-bit constants stored directly in the root signature payload.
    Constants32Bit = 1,
    /// Direct 64-bit GPU virtual address pointing to a constant buffer (b#).
    Cbv = 2,
    /// Direct 64-bit GPU virtual address pointing to a shader resource view (t#).
    Srv = 3,
    /// Direct 64-bit GPU virtual address pointing to an unordered access view (u#).
    Uav = 4,
}

/// Category of descriptors grouped into descriptor ranges.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum D3D12_DESCRIPTOR_RANGE_TYPE {
    /// Shader Resource View (texture, buffer read).
    Srv = 0,
    /// Unordered Access View (read/write compute surface).
    Uav = 1,
    /// Constant Buffer View (uniform constant buffer).
    Cbv = 2,
    /// Texture sampler state descriptor.
    Sampler = 3,
}

/// Hardware shader pipeline visibility for root signature parameters.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum D3D12_SHADER_VISIBILITY {
    All = 0,
    Vertex = 1,
    Hull = 2,
    Domain = 3,
    Geometry = 4,
    Pixel = 5,
    Amplification = 6,
    Mesh = 7,
}

/// Root signature execution and hardware optimization flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_ROOT_SIGNATURE_FLAGS(pub u32);

impl D3D12_ROOT_SIGNATURE_FLAGS {
    pub const NONE: Self = Self(0);
    pub const ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT: Self = Self(1 << 0);
    pub const DENY_VERTEX_SHADER_ROOT_ACCESS: Self = Self(1 << 1);
    pub const DENY_HULL_SHADER_ROOT_ACCESS: Self = Self(1 << 2);
    pub const DENY_DOMAIN_SHADER_ROOT_ACCESS: Self = Self(1 << 3);
    pub const DENY_GEOMETRY_SHADER_ROOT_ACCESS: Self = Self(1 << 4);
    pub const DENY_PIXEL_SHADER_ROOT_ACCESS: Self = Self(1 << 5);
    pub const ALLOW_STREAM_OUTPUT: Self = Self(1 << 6);
    pub const LOCAL_ROOT_SIGNATURE: Self = Self(1 << 7);
    pub const DENY_AMPLIFICATION_SHADER_ROOT_ACCESS: Self = Self(1 << 8);
    pub const DENY_MESH_SHADER_ROOT_ACCESS: Self = Self(1 << 9);
    pub const CBV_SRV_UAV_HEAP_DIRECTLY_INDEXED: Self = Self(1 << 10);
    pub const SAMPLER_HEAP_DIRECTLY_INDEXED: Self = Self(1 << 11);

    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// Contiguous range of descriptors within a descriptor table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_DESCRIPTOR_RANGE {
    pub range_type: D3D12_DESCRIPTOR_RANGE_TYPE,
    pub num_descriptors: u32,
    pub base_shader_register: u32,
    pub register_space: u32,
    pub offset_in_descriptors_from_table_start: u32,
}

impl D3D12_DESCRIPTOR_RANGE {
    /// Creates a new descriptor range descriptor.
    pub const fn new(
        range_type: D3D12_DESCRIPTOR_RANGE_TYPE,
        num_descriptors: u32,
        base_shader_register: u32,
        register_space: u32,
        offset_in_descriptors_from_table_start: u32,
    ) -> Self {
        Self {
            range_type,
            num_descriptors,
            base_shader_register,
            register_space,
            offset_in_descriptors_from_table_start,
        }
    }
}

/// Descriptor table parameter specification holding multiple descriptor ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3D12_ROOT_DESCRIPTOR_TABLE {
    pub descriptor_ranges: Vec<D3D12_DESCRIPTOR_RANGE>,
}

impl D3D12_ROOT_DESCRIPTOR_TABLE {
    pub fn new(descriptor_ranges: Vec<D3D12_DESCRIPTOR_RANGE>) -> Self {
        Self { descriptor_ranges }
    }
}

/// Inline 32-bit constants parameter specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_ROOT_CONSTANTS {
    pub shader_register: u32,
    pub register_space: u32,
    pub num_32bit_values: u32,
}

impl D3D12_ROOT_CONSTANTS {
    pub const fn new(shader_register: u32, register_space: u32, num_32bit_values: u32) -> Self {
        Self {
            shader_register,
            register_space,
            num_32bit_values,
        }
    }
}

/// Root descriptor parameter specification (direct 64-bit GPU virtual address).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_ROOT_DESCRIPTOR {
    pub shader_register: u32,
    pub register_space: u32,
}

impl D3D12_ROOT_DESCRIPTOR {
    pub const fn new(shader_register: u32, register_space: u32) -> Self {
        Self {
            shader_register,
            register_space,
        }
    }
}

/// Polymorphic payload variant for a root parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootParameterPayload {
    DescriptorTable(D3D12_ROOT_DESCRIPTOR_TABLE),
    Constants32Bit(D3D12_ROOT_CONSTANTS),
    Cbv(D3D12_ROOT_DESCRIPTOR),
    Srv(D3D12_ROOT_DESCRIPTOR),
    Uav(D3D12_ROOT_DESCRIPTOR),
}

/// Single root signature parameter slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3D12_ROOT_PARAMETER {
    pub parameter_type: D3D12_ROOT_PARAMETER_TYPE,
    pub shader_visibility: D3D12_SHADER_VISIBILITY,
    pub payload: RootParameterPayload,
}

impl D3D12_ROOT_PARAMETER {
    /// Creates a descriptor table root parameter.
    pub fn descriptor_table(
        visibility: D3D12_SHADER_VISIBILITY,
        ranges: Vec<D3D12_DESCRIPTOR_RANGE>,
    ) -> Self {
        Self {
            parameter_type: D3D12_ROOT_PARAMETER_TYPE::DescriptorTable,
            shader_visibility: visibility,
            payload: RootParameterPayload::DescriptorTable(D3D12_ROOT_DESCRIPTOR_TABLE::new(ranges)),
        }
    }

    /// Creates an inline 32-bit constants root parameter.
    pub const fn constants(
        visibility: D3D12_SHADER_VISIBILITY,
        shader_register: u32,
        register_space: u32,
        num_32bit_values: u32,
    ) -> Self {
        Self {
            parameter_type: D3D12_ROOT_PARAMETER_TYPE::Constants32Bit,
            shader_visibility: visibility,
            payload: RootParameterPayload::Constants32Bit(D3D12_ROOT_CONSTANTS::new(
                shader_register,
                register_space,
                num_32bit_values,
            )),
        }
    }

    /// Creates a direct constant buffer view (CBV) root descriptor.
    pub const fn cbv(
        visibility: D3D12_SHADER_VISIBILITY,
        shader_register: u32,
        register_space: u32,
    ) -> Self {
        Self {
            parameter_type: D3D12_ROOT_PARAMETER_TYPE::Cbv,
            shader_visibility: visibility,
            payload: RootParameterPayload::Cbv(D3D12_ROOT_DESCRIPTOR::new(shader_register, register_space)),
        }
    }

    /// Creates a direct shader resource view (SRV) root descriptor.
    pub const fn srv(
        visibility: D3D12_SHADER_VISIBILITY,
        shader_register: u32,
        register_space: u32,
    ) -> Self {
        Self {
            parameter_type: D3D12_ROOT_PARAMETER_TYPE::Srv,
            shader_visibility: visibility,
            payload: RootParameterPayload::Srv(D3D12_ROOT_DESCRIPTOR::new(shader_register, register_space)),
        }
    }

    /// Creates a direct unordered access view (UAV) root descriptor.
    pub const fn uav(
        visibility: D3D12_SHADER_VISIBILITY,
        shader_register: u32,
        register_space: u32,
    ) -> Self {
        Self {
            parameter_type: D3D12_ROOT_PARAMETER_TYPE::Uav,
            shader_visibility: visibility,
            payload: RootParameterPayload::Uav(D3D12_ROOT_DESCRIPTOR::new(shader_register, register_space)),
        }
    }
}

/// Maximum allowed root signature size in 32-bit DWORDs (256 bytes).
pub const D3D12_MAX_ROOT_COST_DWORDS: u32 = 64;

/// Specification descriptor used to construct and compile a root signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3D12_ROOT_SIGNATURE_DESC {
    pub parameters: Vec<D3D12_ROOT_PARAMETER>,
    pub flags: D3D12_ROOT_SIGNATURE_FLAGS,
}

impl D3D12_ROOT_SIGNATURE_DESC {
    /// Creates a new root signature descriptor.
    pub fn new(parameters: Vec<D3D12_ROOT_PARAMETER>, flags: D3D12_ROOT_SIGNATURE_FLAGS) -> Self {
        Self { parameters, flags }
    }
}

/// Compiled hardware layout of a Direct3D 12 root signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootSignatureLayout {
    pub total_dwords: u32,
    pub parameter_offsets: Vec<u32>,
    pub parameter_dwords: Vec<u32>,
    pub table_descriptor_counts: Vec<u32>,
    pub flags: D3D12_ROOT_SIGNATURE_FLAGS,
    pub parameters: Vec<D3D12_ROOT_PARAMETER>,
}

impl RootSignatureLayout {
    /// Compiles and validates a root signature descriptor against D3D12 hardware budget limits.
    pub fn compile(desc: D3D12_ROOT_SIGNATURE_DESC) -> Result<Self, &'static str> {
        let mut total_dwords: u32 = 0;
        let mut parameter_offsets = Vec::with_capacity(desc.parameters.len());
        let mut parameter_dwords = Vec::with_capacity(desc.parameters.len());
        let mut table_descriptor_counts = Vec::new();

        for param in &desc.parameters {
            parameter_offsets.push(total_dwords);

            let dwords = match &param.payload {
                RootParameterPayload::DescriptorTable(table) => {
                    let mut desc_count: u32 = 0;
                    for range in &table.descriptor_ranges {
                        if range.num_descriptors == 0 {
                            return Err("Descriptor range cannot contain 0 descriptors");
                        }
                        desc_count = desc_count.saturating_add(range.num_descriptors);
                    }
                    table_descriptor_counts.push(desc_count);
                    // A descriptor table pointer in root memory occupies 1 DWORD (32-bit handle/offset)
                    1
                }
                RootParameterPayload::Constants32Bit(c) => {
                    if c.num_32bit_values == 0 {
                        return Err("Root constants count must be greater than 0");
                    }
                    c.num_32bit_values
                }
                RootParameterPayload::Cbv(_) | RootParameterPayload::Srv(_) | RootParameterPayload::Uav(_) => {
                    // A direct 64-bit GPU virtual address occupies 2 DWORDs
                    2
                }
            };

            parameter_dwords.push(dwords);
            total_dwords = total_dwords.saturating_add(dwords);

            if total_dwords > D3D12_MAX_ROOT_COST_DWORDS {
                return Err("Root signature exceeds maximum 64 DWORD hardware budget");
            }
        }

        Ok(Self {
            total_dwords,
            parameter_offsets,
            parameter_dwords,
            table_descriptor_counts,
            flags: desc.flags,
            parameters: desc.parameters,
        })
    }

    /// Returns the number of parameter slots in the compiled root signature.
    pub fn num_parameters(&self) -> usize {
        self.parameters.len()
    }

    /// Returns the DWORD offset of the specified parameter slot in root memory.
    pub fn get_parameter_offset(&self, slot: usize) -> Option<u32> {
        self.parameter_offsets.get(slot).copied()
    }

    /// Returns the size in DWORDs of the specified parameter slot.
    pub fn get_parameter_dwords(&self, slot: usize) -> Option<u32> {
        self.parameter_dwords.get(slot).copied()
    }
}

// Descriptor Heaps & Handle Arithmetic

/// Direct3D 12 descriptor heap categories.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum D3D12_DESCRIPTOR_HEAP_TYPE {
    CbvSrvUav = 0,
    Sampler = 1,
    Rtv = 2,
    Dsv = 3,
}

/// Descriptor heap execution visibility flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_DESCRIPTOR_HEAP_FLAGS(pub u32);

impl D3D12_DESCRIPTOR_HEAP_FLAGS {
    pub const NONE: Self = Self(0);
    pub const SHADER_VISIBLE: Self = Self(1 << 0);

    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// Descriptor heap creation parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_DESCRIPTOR_HEAP_DESC {
    pub heap_type: D3D12_DESCRIPTOR_HEAP_TYPE,
    pub num_descriptors: u32,
    pub flags: D3D12_DESCRIPTOR_HEAP_FLAGS,
    pub node_mask: u32,
}

impl D3D12_DESCRIPTOR_HEAP_DESC {
    pub const fn new(
        heap_type: D3D12_DESCRIPTOR_HEAP_TYPE,
        num_descriptors: u32,
        flags: D3D12_DESCRIPTOR_HEAP_FLAGS,
    ) -> Self {
        Self {
            heap_type,
            num_descriptors,
            flags,
            node_mask: 0,
        }
    }
}

/// CPU-visible memory handle to an individual descriptor inside a heap.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_CPU_DESCRIPTOR_HANDLE {
    pub ptr: usize,
}

impl D3D12_CPU_DESCRIPTOR_HANDLE {
    pub const NULL: Self = Self { ptr: 0 };

    #[inline(always)]
    pub const fn new(ptr: usize) -> Self {
        Self { ptr }
    }

    #[inline(always)]
    pub const fn is_valid(&self) -> bool {
        self.ptr != 0
    }

    /// Advances the handle forward by `index * increment_size` bytes.
    #[inline(always)]
    pub const fn offset(&self, index: usize, increment_size: usize) -> Self {
        Self {
            ptr: self.ptr.saturating_add(index.saturating_mul(increment_size)),
        }
    }
}

/// GPU-visible memory handle to an individual descriptor inside a shader-visible heap.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_GPU_DESCRIPTOR_HANDLE {
    pub ptr: u64,
}

impl D3D12_GPU_DESCRIPTOR_HANDLE {
    pub const NULL: Self = Self { ptr: 0 };

    #[inline(always)]
    pub const fn new(ptr: u64) -> Self {
        Self { ptr }
    }

    #[inline(always)]
    pub const fn is_valid(&self) -> bool {
        self.ptr != 0
    }

    /// Advances the GPU handle forward by `index * increment_size` bytes.
    #[inline(always)]
    pub const fn offset(&self, index: usize, increment_size: usize) -> Self {
        Self {
            ptr: self.ptr.saturating_add((index as u64).saturating_mul(increment_size as u64)),
        }
    }
}

/// GPU Descriptor Heap managing contiguous descriptor pools and handle allocation.
#[derive(Debug)]
pub struct DescriptorHeap {
    pub heap_type: D3D12_DESCRIPTOR_HEAP_TYPE,
    pub flags: D3D12_DESCRIPTOR_HEAP_FLAGS,
    pub num_descriptors: u32,
    pub descriptor_increment_size: usize,
    pub cpu_start: D3D12_CPU_DESCRIPTOR_HANDLE,
    pub gpu_start: D3D12_GPU_DESCRIPTOR_HANDLE,
    pub allocated_count: u32,
}

impl DescriptorHeap {
    /// Returns the hardware byte stride between consecutive descriptors for a given heap type.
    pub const fn get_descriptor_increment_size(heap_type: D3D12_DESCRIPTOR_HEAP_TYPE) -> usize {
        match heap_type {
            D3D12_DESCRIPTOR_HEAP_TYPE::CbvSrvUav => 32,
            D3D12_DESCRIPTOR_HEAP_TYPE::Sampler => 16,
            D3D12_DESCRIPTOR_HEAP_TYPE::Rtv => 32,
            D3D12_DESCRIPTOR_HEAP_TYPE::Dsv => 32,
        }
    }

    /// Creates a new descriptor heap mapped at the specified base addresses.
    pub fn new(
        desc: D3D12_DESCRIPTOR_HEAP_DESC,
        cpu_base: usize,
        gpu_base: u64,
    ) -> Result<Self, &'static str> {
        if desc.num_descriptors == 0 {
            return Err("Descriptor heap descriptor count must be greater than zero");
        }
        let descriptor_increment_size = Self::get_descriptor_increment_size(desc.heap_type);
        let gpu_start = if desc.flags.contains(D3D12_DESCRIPTOR_HEAP_FLAGS::SHADER_VISIBLE) {
            D3D12_GPU_DESCRIPTOR_HANDLE::new(gpu_base)
        } else {
            D3D12_GPU_DESCRIPTOR_HANDLE::NULL
        };

        Ok(Self {
            heap_type: desc.heap_type,
            flags: desc.flags,
            num_descriptors: desc.num_descriptors,
            descriptor_increment_size,
            cpu_start: D3D12_CPU_DESCRIPTOR_HANDLE::new(cpu_base),
            gpu_start,
            allocated_count: 0,
        })
    }

    /// Sub-allocates a contiguous slice of descriptors from the heap.
    pub fn allocate_range(
        &mut self,
        count: u32,
    ) -> Result<(D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_GPU_DESCRIPTOR_HANDLE), &'static str> {
        if count == 0 {
            return Err("Cannot allocate range of zero descriptors");
        }
        let available = self.num_descriptors.saturating_sub(self.allocated_count);
        if count > available {
            return Err("Out of descriptor heap memory");
        }

        let base_idx = self.allocated_count as usize;
        self.allocated_count = self.allocated_count.saturating_add(count);

        let cpu_handle = self.cpu_start.offset(base_idx, self.descriptor_increment_size);
        let gpu_handle = if self.gpu_start.is_valid() {
            self.gpu_start.offset(base_idx, self.descriptor_increment_size)
        } else {
            D3D12_GPU_DESCRIPTOR_HANDLE::NULL
        };

        Ok((cpu_handle, gpu_handle))
    }

    /// Returns the CPU descriptor handle at the specified zero-based index.
    pub fn get_cpu_handle_at(&self, index: usize) -> Option<D3D12_CPU_DESCRIPTOR_HANDLE> {
        if index < self.num_descriptors as usize {
            Some(self.cpu_start.offset(index, self.descriptor_increment_size))
        } else {
            None
        }
    }

    /// Returns the GPU descriptor handle at the specified zero-based index.
    pub fn get_gpu_handle_at(&self, index: usize) -> Option<D3D12_GPU_DESCRIPTOR_HANDLE> {
        if index < self.num_descriptors as usize && self.gpu_start.is_valid() {
            Some(self.gpu_start.offset(index, self.descriptor_increment_size))
        } else {
            None
        }
    }

    /// Returns the remaining available descriptor capacity.
    pub fn remaining_capacity(&self) -> u32 {
        self.num_descriptors.saturating_sub(self.allocated_count)
    }
}

// Dynamic Root Parameter Binding Table

/// Bound state value for an individual root parameter slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundRootParameter {
    /// Descriptor table bound to a base GPU descriptor handle.
    DescriptorTable { gpu_handle: D3D12_GPU_DESCRIPTOR_HANDLE },
    /// Inline 32-bit constants buffer.
    Constants32Bit { values: Vec<u32> },
    /// Direct 64-bit GPU virtual address descriptor.
    RootDescriptor { gpu_virtual_address: u64 },
}

/// Dynamic runtime binding table managing active shader parameters for a root signature layout.
#[derive(Debug, Clone)]
pub struct RootBindingTable {
    pub layout: RootSignatureLayout,
    pub bindings: Vec<Option<BoundRootParameter>>,
    pub dirty_slots: u64,
}

impl RootBindingTable {
    /// Creates a new binding table configured for the given root signature layout.
    pub fn new(layout: RootSignatureLayout) -> Self {
        let num_params = layout.num_parameters();
        Self {
            layout,
            bindings: alloc::vec![None; num_params],
            dirty_slots: 0,
        }
    }

    /// Binds a GPU descriptor handle to a descriptor table root parameter slot.
    pub fn bind_descriptor_table(
        &mut self,
        slot: usize,
        handle: D3D12_GPU_DESCRIPTOR_HANDLE,
    ) -> Result<(), &'static str> {
        if slot >= self.bindings.len() {
            return Err("Root parameter slot out of bounds");
        }
        if self.layout.parameters[slot].parameter_type != D3D12_ROOT_PARAMETER_TYPE::DescriptorTable {
            return Err("Target slot is not a descriptor table parameter");
        }
        if !handle.is_valid() {
            return Err("Descriptor table GPU handle must be non-null and valid");
        }

        self.bindings[slot] = Some(BoundRootParameter::DescriptorTable { gpu_handle: handle });
        self.mark_dirty(slot);
        Ok(())
    }

    /// Binds inline 32-bit constants into a root constants parameter slot.
    pub fn bind_32bit_constants(
        &mut self,
        slot: usize,
        values: &[u32],
        offset_dwords: usize,
    ) -> Result<(), &'static str> {
        if slot >= self.bindings.len() {
            return Err("Root parameter slot out of bounds");
        }
        let expected_count = match &self.layout.parameters[slot].payload {
            RootParameterPayload::Constants32Bit(c) => c.num_32bit_values as usize,
            _ => return Err("Target slot is not a 32-bit constants parameter"),
        };

        if offset_dwords.saturating_add(values.len()) > expected_count {
            return Err("Constant values exceed root parameter allocated DWORD budget");
        }

        let mut current_vals = if let Some(BoundRootParameter::Constants32Bit { values: cur }) = &self.bindings[slot] {
            cur.clone()
        } else {
            alloc::vec![0u32; expected_count]
        };

        for (i, &val) in values.iter().enumerate() {
            current_vals[offset_dwords + i] = val;
        }

        self.bindings[slot] = Some(BoundRootParameter::Constants32Bit { values: current_vals });
        self.mark_dirty(slot);
        Ok(())
    }

    /// Binds a direct 64-bit GPU virtual address to a CBV, SRV, or UAV root descriptor slot.
    pub fn bind_root_descriptor(
        &mut self,
        slot: usize,
        gpu_virtual_address: u64,
    ) -> Result<(), &'static str> {
        if slot >= self.bindings.len() {
            return Err("Root parameter slot out of bounds");
        }
        match self.layout.parameters[slot].parameter_type {
            D3D12_ROOT_PARAMETER_TYPE::Cbv
            | D3D12_ROOT_PARAMETER_TYPE::Srv
            | D3D12_ROOT_PARAMETER_TYPE::Uav => {}
            _ => return Err("Target slot is not a direct root descriptor parameter"),
        }

        if gpu_virtual_address == 0 {
            return Err("Root descriptor GPU virtual address cannot be zero");
        }
        if (gpu_virtual_address % 8) != 0 {
            return Err("Root descriptor GPU virtual address must be 8-byte aligned");
        }

        self.bindings[slot] = Some(BoundRootParameter::RootDescriptor { gpu_virtual_address });
        self.mark_dirty(slot);
        Ok(())
    }

    /// Computes the hardware byte offset in the root argument ring for the given parameter slot.
    pub fn compute_hardware_offset(&self, slot: usize) -> Option<u32> {
        self.layout.get_parameter_offset(slot).map(|dw| dw * 4)
    }

    /// Returns a reference to the bound parameter value at the given slot.
    pub fn get_binding(&self, slot: usize) -> Option<&BoundRootParameter> {
        self.bindings.get(slot).and_then(|opt| opt.as_ref())
    }

    /// Checks if all parameters required by the root signature layout are bound.
    pub fn all_parameters_bound(&self) -> bool {
        self.bindings.iter().all(|b| b.is_some())
    }

    /// Marks the specified parameter slot as dirty, requiring hardware flush.
    #[inline(always)]
    pub fn mark_dirty(&mut self, slot: usize) {
        if slot < 64 {
            self.dirty_slots |= 1 << slot;
        }
    }

    /// Checks if the specified parameter slot has unflushed modifications.
    #[inline(always)]
    pub fn is_dirty(&self, slot: usize) -> bool {
        if slot < 64 {
            (self.dirty_slots & (1 << slot)) != 0
        } else {
            false
        }
    }

    /// Returns true if any parameter slot is currently marked dirty.
    #[inline(always)]
    pub fn has_dirty_slots(&self) -> bool {
        self.dirty_slots != 0
    }

    /// Clears the dirty flag on a single parameter slot after hardware synchronization.
    #[inline(always)]
    pub fn clear_dirty_slot(&mut self, slot: usize) {
        if slot < 64 {
            self.dirty_slots &= !(1 << slot);
        }
    }

    /// Resets all dirty bits across the binding table.
    #[inline(always)]
    pub fn reset_dirty(&mut self) {
        self.dirty_slots = 0;
    }

    /// Flushes all dirty parameters through a callback and clears the dirty bitmask.
    pub fn flush_dirty_parameters<F>(&mut self, mut flush_fn: F) -> usize
    where
        F: FnMut(usize, &BoundRootParameter, u32),
    {
        let mut flushed_count = 0;
        let mut mask = self.dirty_slots;
        while mask != 0 {
            let slot = mask.trailing_zeros() as usize;
            if let Some(param) = self.get_binding(slot) {
                let hw_offset = self.compute_hardware_offset(slot).unwrap_or(0);
                flush_fn(slot, param, hw_offset);
                flushed_count += 1;
            }
            mask &= mask - 1;
        }
        self.dirty_slots = 0;
        flushed_count
    }

}

// Root Signature Cache & Deduplication

/// Computes an FNV-1a 64-bit hash over a root signature descriptor for cache lookup.
pub fn hash_root_signature_desc(desc: &D3D12_ROOT_SIGNATURE_DESC) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    hash = (hash ^ (desc.flags.0 as u64)).wrapping_mul(FNV_PRIME);
    hash = (hash ^ (desc.parameters.len() as u64)).wrapping_mul(FNV_PRIME);

    for (i, p) in desc.parameters.iter().enumerate() {
        hash = (hash ^ (i as u64)).wrapping_mul(FNV_PRIME);
        hash = (hash ^ (p.parameter_type as u64)).wrapping_mul(FNV_PRIME);
        hash = (hash ^ (p.shader_visibility as u64)).wrapping_mul(FNV_PRIME);

        match &p.payload {
            RootParameterPayload::DescriptorTable(t) => {
                hash = (hash ^ (t.descriptor_ranges.len() as u64)).wrapping_mul(FNV_PRIME);
                for r in &t.descriptor_ranges {
                    hash = (hash ^ (r.range_type as u64)).wrapping_mul(FNV_PRIME);
                    hash = (hash ^ (r.num_descriptors as u64)).wrapping_mul(FNV_PRIME);
                    hash = (hash ^ (r.base_shader_register as u64)).wrapping_mul(FNV_PRIME);
                    hash = (hash ^ (r.register_space as u64)).wrapping_mul(FNV_PRIME);
                }
            }
            RootParameterPayload::Constants32Bit(c) => {
                hash = (hash ^ (c.shader_register as u64)).wrapping_mul(FNV_PRIME);
                hash = (hash ^ (c.register_space as u64)).wrapping_mul(FNV_PRIME);
                hash = (hash ^ (c.num_32bit_values as u64)).wrapping_mul(FNV_PRIME);
            }
            RootParameterPayload::Cbv(d) | RootParameterPayload::Srv(d) | RootParameterPayload::Uav(d) => {
                hash = (hash ^ (d.shader_register as u64)).wrapping_mul(FNV_PRIME);
                hash = (hash ^ (d.register_space as u64)).wrapping_mul(FNV_PRIME);
            }
        }
    }
    hash
}

/// Cached compiled root signature entry with unique 64-bit signature ID.
#[derive(Debug, Clone)]
pub struct CachedRootSignature {
    pub hash_id: u64,
    pub layout: RootSignatureLayout,
}

/// Centralized root signature cache eliminating redundant compilation overhead.
#[derive(Debug, Default)]
pub struct RootSignatureCache {
    pub entries: Vec<CachedRootSignature>,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

impl RootSignatureCache {
    /// Creates an empty root signature cache.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    /// Fetches an existing compiled root signature layout or compiles and caches a new one.
    pub fn get_or_compile(
        &mut self,
        desc: D3D12_ROOT_SIGNATURE_DESC,
    ) -> Result<RootSignatureLayout, &'static str> {
        let hash = hash_root_signature_desc(&desc);

        for entry in &self.entries {
            if entry.hash_id == hash {
                self.cache_hits = self.cache_hits.saturating_add(1);
                return Ok(entry.layout.clone());
            }
        }

        self.cache_misses = self.cache_misses.saturating_add(1);
        let layout = RootSignatureLayout::compile(desc)?;
        self.entries.push(CachedRootSignature {
            hash_id: hash,
            layout: layout.clone(),
        });

        Ok(layout)
    }

    /// Returns cache telemetry performance statistics: (hits, misses).
    pub fn stats(&self) -> (u64, u64) {
        (self.cache_hits, self.cache_misses)
    }

    /// Clears all entries from the root signature cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.cache_hits = 0;
        self.cache_misses = 0;
    }
}

/// Comprehensive verification self-test suite for Direct3D 12 Root Signature & Descriptor Heap Binding Table Engine.


// --- Direct3D 12 Agility Dynamic Root Signature Version 1.2 Serialization & Deserialization Engine ---



// Binary bytecode chunk header identifying serialized D3D12 root signature.

pub const D3D12_ROOT_SIGNATURE_MAGIC_RTS0: u32 = 0x30535452; // 'RTS0' in little endian



// Root parameter type codes encoded into serialized binary bytecode.

#[repr(u32)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]

pub enum D3d12SerializedParamType {

    DescriptorTable = 0,

    Constants32Bit = 1,

    Cbv = 2,

    Srv = 3,

    Uav = 4,

}



// Deserialized or serialized root parameter item for Root Signature 1.2.

#[derive(Debug, Clone, PartialEq, Eq)]

pub struct RootParameter12Desc {

    pub param_type: D3d12SerializedParamType,

    pub shader_visibility: D3D12_SHADER_VISIBILITY,

    pub shader_register: u32,

    pub register_space: u32,

    pub num_32bit_values: u32,

    pub descriptor_count: u32,

}



impl RootParameter12Desc {

    // Construct 32-bit root constants parameter.

    pub const fn constants(vis: D3D12_SHADER_VISIBILITY, reg: u32, space: u32, count: u32) -> Self {

        Self {

            param_type: D3d12SerializedParamType::Constants32Bit,

            shader_visibility: vis,

            shader_register: reg,

            register_space: space,

            num_32bit_values: count,

            descriptor_count: 0,

        }

    }



    // Construct root CBV descriptor.

    pub const fn cbv(vis: D3D12_SHADER_VISIBILITY, reg: u32, space: u32) -> Self {

        Self {

            param_type: D3d12SerializedParamType::Cbv,

            shader_visibility: vis,

            shader_register: reg,

            register_space: space,

            num_32bit_values: 0,

            descriptor_count: 0,

        }

    }



    // Construct root SRV descriptor.

    pub const fn srv(vis: D3D12_SHADER_VISIBILITY, reg: u32, space: u32) -> Self {

        Self {

            param_type: D3d12SerializedParamType::Srv,

            shader_visibility: vis,

            shader_register: reg,

            register_space: space,

            num_32bit_values: 0,

            descriptor_count: 0,

        }

    }



    // Construct root UAV descriptor.

    pub const fn uav(vis: D3D12_SHADER_VISIBILITY, reg: u32, space: u32) -> Self {

        Self {

            param_type: D3d12SerializedParamType::Uav,

            shader_visibility: vis,

            shader_register: reg,

            register_space: space,

            num_32bit_values: 0,

            descriptor_count: 0,

        }

    }



    // Construct root descriptor table parameter.

    pub const fn descriptor_table(vis: D3D12_SHADER_VISIBILITY, num_descriptors: u32) -> Self {

        Self {

            param_type: D3d12SerializedParamType::DescriptorTable,

            shader_visibility: vis,

            shader_register: 0,

            register_space: 0,

            num_32bit_values: 0,

            descriptor_count: num_descriptors,

        }

    }



    // Compute hardware DWORD cost for this parameter.

    pub const fn dword_cost(&self) -> u32 {

        match self.param_type {

            D3d12SerializedParamType::DescriptorTable => 1,

            D3d12SerializedParamType::Constants32Bit => self.num_32bit_values,

            D3d12SerializedParamType::Cbv

            | D3d12SerializedParamType::Srv

            | D3d12SerializedParamType::Uav => 2,

        }

    }

}



// Bytecode serializer and deserializer for Root Signature Version 1.2.

pub struct RootSignature12Serializer;



impl RootSignature12Serializer {

    // Serialize root signature parameters into compact D3D12 bytecode stream.

    pub fn serialize(flags: u32, params: &[RootParameter12Desc], output: &mut Vec<u8>) {

        output.clear();

        output.extend_from_slice(&D3D12_ROOT_SIGNATURE_MAGIC_RTS0.to_le_bytes());

        output.extend_from_slice(&0x3u32.to_le_bytes()); // Version 1.2

        output.extend_from_slice(&flags.to_le_bytes());

        output.extend_from_slice(&(params.len() as u32).to_le_bytes());



        for p in params {

            output.extend_from_slice(&(p.param_type as u32).to_le_bytes());

            output.extend_from_slice(&(p.shader_visibility as u32).to_le_bytes());

            output.extend_from_slice(&p.shader_register.to_le_bytes());

            output.extend_from_slice(&p.register_space.to_le_bytes());

            output.extend_from_slice(&p.num_32bit_values.to_le_bytes());

            output.extend_from_slice(&p.descriptor_count.to_le_bytes());

        }

    }



    // Deserialize bytecode stream into root parameter layout.

    pub fn deserialize(bytes: &[u8]) -> Result<(u32, Vec<RootParameter12Desc>), &'static str> {

        if bytes.len() < 16 {

            return Err("Bytecode smaller than minimum D3D12 root signature header");

        }

        let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);

        if magic != D3D12_ROOT_SIGNATURE_MAGIC_RTS0 {

            return Err("Invalid root signature RTS0 magic header");

        }

        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);

        if version != 3 && version != 2 && version != 1 {

            return Err("Unsupported root signature version in bytecode");

        }

        let flags = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);

        let param_count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;



        let expected_len = 16 + param_count * 24;

        if bytes.len() < expected_len {

            return Err("Bytecode truncated or corrupted for specified parameter count");

        }



        let mut params = Vec::with_capacity(param_count);

        let mut offset = 16;

        for _ in 0..param_count {

            let ptype_raw = u32::from_le_bytes([bytes[offset], bytes[offset+1], bytes[offset+2], bytes[offset+3]]);

            let vis_raw = u32::from_le_bytes([bytes[offset+4], bytes[offset+5], bytes[offset+6], bytes[offset+7]]);

            let reg = u32::from_le_bytes([bytes[offset+8], bytes[offset+9], bytes[offset+10], bytes[offset+11]]);

            let space = u32::from_le_bytes([bytes[offset+12], bytes[offset+13], bytes[offset+14], bytes[offset+15]]);

            let num_vals = u32::from_le_bytes([bytes[offset+16], bytes[offset+17], bytes[offset+18], bytes[offset+19]]);

            let desc_cnt = u32::from_le_bytes([bytes[offset+20], bytes[offset+21], bytes[offset+22], bytes[offset+23]]);



            let param_type = match ptype_raw {

                0 => D3d12SerializedParamType::DescriptorTable,

                1 => D3d12SerializedParamType::Constants32Bit,

                2 => D3d12SerializedParamType::Cbv,

                3 => D3d12SerializedParamType::Srv,

                4 => D3d12SerializedParamType::Uav,

                _ => return Err("Unknown root parameter type in bytecode"),

            };



            let shader_visibility = match vis_raw {

                0 => D3D12_SHADER_VISIBILITY::All,

                1 => D3D12_SHADER_VISIBILITY::Vertex,

                2 => D3D12_SHADER_VISIBILITY::Hull,

                3 => D3D12_SHADER_VISIBILITY::Domain,

                4 => D3D12_SHADER_VISIBILITY::Geometry,

                5 => D3D12_SHADER_VISIBILITY::Pixel,

                6 => D3D12_SHADER_VISIBILITY::Amplification,

                7 => D3D12_SHADER_VISIBILITY::Mesh,

                _ => return Err("Invalid shader visibility enumeration in bytecode"),

            };



            params.push(RootParameter12Desc {

                param_type,

                shader_visibility,

                shader_register: reg,

                register_space: space,

                num_32bit_values: num_vals,

                descriptor_count: desc_cnt,

            });

            offset += 24;

        }



        Ok((flags, params))

    }

}



// Key identifying a descriptor table range layout for deduplication.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]

pub struct DescriptorTableLayoutKey {

    pub range_type: D3D12_DESCRIPTOR_RANGE_TYPE,

    pub num_descriptors: u32,

    pub base_shader_register: u32,

    pub register_space: u32,

}



// Cached local root signature binding metadata for DXR shader record tables and Work Graphs.

#[derive(Debug, Clone, PartialEq, Eq)]

pub struct LocalRootBindingEntry {

    pub local_signature_id: u64,

    pub shader_identifier: [u8; 32],

    pub local_arguments_size: u32,

    pub sbt_record_stride: u32,

}



// Local root signature binding cache for DXR ray tracing and Work Graphs.

#[derive(Debug, Default)]

pub struct LocalRootBindingCache {

    entries: Vec<LocalRootBindingEntry>,

    cache_hits: u64,

    cache_misses: u64,

}



impl LocalRootBindingCache {

    // Construct new empty local root binding cache.

    pub fn new() -> Self {

        Self {

            entries: Vec::new(),

            cache_hits: 0,

            cache_misses: 0,

        }

    }



    // Register or retrieve cached local root signature record stride.

    pub fn get_or_register(

        &mut self,

        sig_id: u64,

        shader_id: &[u8; 32],

        args_size: u32,

    ) -> (u32, bool) {

        for entry in &self.entries {

            if entry.local_signature_id == sig_id && &entry.shader_identifier == shader_id {

                self.cache_hits = self.cache_hits.saturating_add(1);

                return (entry.sbt_record_stride, true);

            }

        }



        self.cache_misses = self.cache_misses.saturating_add(1);

        let raw_size = 32 + args_size;

        // D3D12 DXR Shader Record Table records must be aligned to 64 bytes

        let aligned_stride = (raw_size + 63) & !63;



        self.entries.push(LocalRootBindingEntry {

            local_signature_id: sig_id,

            shader_identifier: *shader_id,

            local_arguments_size: args_size,

            sbt_record_stride: aligned_stride,

        });



        (aligned_stride, false)

    }



    // Query cache statistics: (hits, misses).

    pub fn stats(&self) -> (u64, u64) {

        (self.cache_hits, self.cache_misses)

    }



    // Count of stored unique local root signature records.

    pub fn len(&self) -> usize {

        self.entries.len()

    }

}



// Deduplication table eliminating redundant descriptor table allocations across draw/dispatch calls.

#[derive(Debug, Default)]

pub struct RootDescriptorTableDeduplicator {

    table_hashes: Vec<u64>,

    descriptor_heap_offsets: Vec<u32>,

    dedup_hits: u64,

}



impl RootDescriptorTableDeduplicator {

    // Create new root descriptor table deduplicator.

    pub fn new() -> Self {

        Self {

            table_hashes: Vec::new(),

            descriptor_heap_offsets: Vec::new(),

            dedup_hits: 0,

        }

    }



    // Lookup or record descriptor table range, returning deduplicated descriptor heap offset.

    pub fn deduplicate_table(&mut self, table_hash: u64, current_offset: u32) -> (u32, bool) {

        for (idx, &h) in self.table_hashes.iter().enumerate() {

            if h == table_hash {

                self.dedup_hits = self.dedup_hits.saturating_add(1);

                return (self.descriptor_heap_offsets[idx], true);

            }

        }



        self.table_hashes.push(table_hash);

        self.descriptor_heap_offsets.push(current_offset);

        (current_offset, false)

    }



    // Deduplication hits count.

    pub fn dedup_hits(&self) -> u64 {

        self.dedup_hits

    }

}



// Texture filter mode for static samplers embedded into root signature.

#[repr(u32)]

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]

pub enum D3d12StaticFilter {

    MinMagMipPoint = 0,

    MinMagPointMipLinear = 1,

    MinPointMagLinearMipPoint = 2,

    MinPointMagMipLinear = 3,

    MinLinearMagMipPoint = 4,

    MinLinearMagPointMipLinear = 5,

    MinMagLinearMipPoint = 6,

    MinMagMipLinear = 7,

    Anisotropic = 8,

}



// Texture addressing mode for static samplers.

#[repr(u32)]

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]

pub enum D3d12StaticTextureAddressMode {

    Wrap = 1,

    Mirror = 2,

    Clamp = 3,

    Border = 4,

    MirrorOnce = 5,

}



// Static sampler descriptor embedded directly in root signature.

// Eliminates CPU descriptor heap allocation and binding overhead at runtime.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]

pub struct D3d12StaticSamplerDesc {

    pub filter: D3d12StaticFilter,

    pub address_u: D3d12StaticTextureAddressMode,

    pub address_v: D3d12StaticTextureAddressMode,

    pub address_w: D3d12StaticTextureAddressMode,

    pub mip_lod_bias_fixed: i16,

    pub max_anisotropy: u8,

    pub shader_register: u32,

    pub register_space: u32,

    pub shader_visibility: D3D12_SHADER_VISIBILITY,

}



impl D3d12StaticSamplerDesc {

    // Construct new static sampler descriptor.

    pub const fn new(

        filter: D3d12StaticFilter,

        address: D3d12StaticTextureAddressMode,

        shader_register: u32,

        register_space: u32,

        shader_visibility: D3D12_SHADER_VISIBILITY,

    ) -> Self {

        Self {

            filter,

            address_u: address,

            address_v: address,

            address_w: address,

            mip_lod_bias_fixed: 0,

            max_anisotropy: 16,

            shader_register,

            register_space,

            shader_visibility,

        }

    }

}



// Optimizer managing static sampler register table indexing.

// Bypasses runtime descriptor heap allocations entirely.

#[derive(Debug, Default)]

pub struct StaticSamplerTableOptimizer {

    samplers: Vec<D3d12StaticSamplerDesc>,

    bypassed_descriptor_allocations: u64,

}



impl StaticSamplerTableOptimizer {

    // Construct new empty static sampler optimizer.

    pub fn new() -> Self {

        Self {

            samplers: Vec::new(),

            bypassed_descriptor_allocations: 0,

        }

    }



    // Register a static sampler in the root signature static sampler table.

    pub fn add_static_sampler(&mut self, sampler: D3d12StaticSamplerDesc) -> Result<u32, &'static str> {

        // Direct3D 12 limits static samplers to maximum 2032 samplers per root signature

        if self.samplers.len() >= 2032 {

            return Err("Maximum static samplers limit (2032) reached");

        }

        for s in &self.samplers {

            if s.shader_register == sampler.shader_register

                && s.register_space == sampler.register_space

                && s.shader_visibility == sampler.shader_visibility

            {

                return Err("Static sampler register conflict in same register space and visibility");

            }

        }

        let index = self.samplers.len() as u32;

        self.samplers.push(sampler);

        Ok(index)

    }



    // Lookup static sampler by register and space, bypassing descriptor heap allocation.

    pub fn lookup_static_sampler(

        &mut self,

        shader_register: u32,

        register_space: u32,

    ) -> Option<&D3d12StaticSamplerDesc> {

        for s in &self.samplers {

            if s.shader_register == shader_register && s.register_space == register_space {

                self.bypassed_descriptor_allocations = self.bypassed_descriptor_allocations.saturating_add(1);

                return Some(s);

            }

        }

        None

    }



    // Count of registered static samplers.

    pub fn len(&self) -> usize {

        self.samplers.len()

    }



    // Number of runtime descriptor allocations bypassed by static samplers.

    pub fn bypassed_allocations(&self) -> u64 {

        self.bypassed_descriptor_allocations

    }

}



// Unified Direct3D 12 Agility Dynamic Root Signature 1.2 Serialization & Static Sampler Table Governor.

pub struct D3d12RootSignature12Governor {

    local_binding_cache: LocalRootBindingCache,

    table_deduplicator: RootDescriptorTableDeduplicator,

    static_samplers: StaticSamplerTableOptimizer,

    serialized_bytecode: Vec<u8>,

    active_flags: u32,

    active_params: Vec<RootParameter12Desc>,

}



impl D3d12RootSignature12Governor {

    // Construct new Root Signature 1.2 governor.

    pub fn new() -> Self {

        Self {

            local_binding_cache: LocalRootBindingCache::new(),

            table_deduplicator: RootDescriptorTableDeduplicator::new(),

            static_samplers: StaticSamplerTableOptimizer::new(),

            serialized_bytecode: Vec::new(),

            active_flags: 0,

            active_params: Vec::new(),

        }

    }



    // Set and serialize root signature parameters and configuration flags.

    pub fn configure_signature(&mut self, flags: u32, params: &[RootParameter12Desc]) {

        self.active_flags = flags;

        self.active_params = params.to_vec();

        RootSignature12Serializer::serialize(flags, params, &mut self.serialized_bytecode);

    }



    // Deserialize bytecode into active layout.

    pub fn load_bytecode(&mut self, bytes: &[u8]) -> Result<(), &'static str> {

        let (flags, params) = RootSignature12Serializer::deserialize(bytes)?;

        self.active_flags = flags;

        self.active_params = params;

        self.serialized_bytecode = bytes.to_vec();

        Ok(())

    }



    // Access local root binding cache.

    pub fn local_binding_cache_mut(&mut self) -> &mut LocalRootBindingCache {

        &mut self.local_binding_cache

    }



    // Access table deduplicator.

    pub fn table_deduplicator_mut(&mut self) -> &mut RootDescriptorTableDeduplicator {

        &mut self.table_deduplicator

    }



    // Access static sampler optimizer.

    pub fn static_samplers_mut(&mut self) -> &mut StaticSamplerTableOptimizer {

        &mut self.static_samplers

    }



    // Retrieve serialized bytecode slice.

    pub fn bytecode(&self) -> &[u8] {

        &self.serialized_bytecode

    }



    // Total DWORD cost of active parameters.

    pub fn total_dwords(&self) -> u32 {

        let mut sum = 0;

        for p in &self.active_params {

            sum += p.dword_cost();

        }

        sum

    }

}

pub fn test_root_signature_subsystem() -> bool {
    crate::serial_write_str("[KNT-GFX] Testing Direct3D 12 Root Signature & Descriptor Heap Binding Table...\r\n");

    // 1. Verify Root Parameter Construction and 64-DWORD Budget Enforcement
    let ranges = alloc::vec![
        D3D12_DESCRIPTOR_RANGE::new(D3D12_DESCRIPTOR_RANGE_TYPE::Srv, 4, 0, 0, 0),
        D3D12_DESCRIPTOR_RANGE::new(D3D12_DESCRIPTOR_RANGE_TYPE::Uav, 2, 0, 0, 4),
    ];
    let params = alloc::vec![
        D3D12_ROOT_PARAMETER::descriptor_table(D3D12_SHADER_VISIBILITY::Pixel, ranges),
        D3D12_ROOT_PARAMETER::constants(D3D12_SHADER_VISIBILITY::All, 0, 0, 4),
        D3D12_ROOT_PARAMETER::cbv(D3D12_SHADER_VISIBILITY::Vertex, 0, 0),
        D3D12_ROOT_PARAMETER::srv(D3D12_SHADER_VISIBILITY::Pixel, 1, 0),
    ];
    let desc = D3D12_ROOT_SIGNATURE_DESC::new(
        params,
        D3D12_ROOT_SIGNATURE_FLAGS::ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
    );

    let layout = match RootSignatureLayout::compile(desc.clone()) {
        Ok(l) => l,
        Err(_) => {
            crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Failed to compile valid root signature\r\n");
            return false;
        }
    };

    // Expected DWORD cost: Table (1) + Constants (4) + CBV (2) + SRV (2) = 9 DWORDs
    if layout.total_dwords != 9 {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Root signature DWORD cost mismatch\r\n");
        return false;
    }
    if layout.parameter_offsets != alloc::vec![0, 1, 5, 7] {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Root signature parameter offsets mismatch\r\n");
        return false;
    }

    // Budget overflow test (> 64 DWORDs)
    let overflow_params = alloc::vec![
        D3D12_ROOT_PARAMETER::constants(D3D12_SHADER_VISIBILITY::All, 0, 0, 60),
        D3D12_ROOT_PARAMETER::constants(D3D12_SHADER_VISIBILITY::All, 1, 0, 10),
    ];
    let overflow_desc = D3D12_ROOT_SIGNATURE_DESC::new(overflow_params, D3D12_ROOT_SIGNATURE_FLAGS::NONE);
    if RootSignatureLayout::compile(overflow_desc).is_ok() {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Budget overflow must be rejected\r\n");
        return false;
    }
    crate::serial_write_str("  * Root parameter descriptors and 64-DWORD budget validation verified\r\n");

    // 2. Verify DescriptorHeap Allocation and Handle Arithmetic
    let heap_desc = D3D12_DESCRIPTOR_HEAP_DESC::new(
        D3D12_DESCRIPTOR_HEAP_TYPE::CbvSrvUav,
        128,
        D3D12_DESCRIPTOR_HEAP_FLAGS::SHADER_VISIBLE,
    );
    let mut heap = match DescriptorHeap::new(heap_desc, 0x1000_0000, 0x2000_0000) {
        Ok(h) => h,
        Err(_) => {
            crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Failed to create descriptor heap\r\n");
            return false;
        }
    };

    if heap.descriptor_increment_size != 32 {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Descriptor increment size mismatch\r\n");
        return false;
    }

    let (cpu_h0, gpu_h0) = match heap.allocate_range(16) {
        Ok(res) => res,
        Err(_) => {
            crate::serial_write_str("[KNT-ROOT-SIG-FAIL] First range allocation failed\r\n");
            return false;
        }
    };
    if cpu_h0.ptr != 0x1000_0000 || gpu_h0.ptr != 0x2000_0000 {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Base handle allocation mismatch\r\n");
        return false;
    }

    let (cpu_h1, gpu_h1) = match heap.allocate_range(8) {
        Ok(res) => res,
        Err(_) => {
            crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Second range allocation failed\r\n");
            return false;
        }
    };
    // 16 descriptors * 32 bytes = 512 bytes = 0x200
    if cpu_h1.ptr != 0x1000_0200 || gpu_h1.ptr != 0x2000_0200 {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Second range offset handle mismatch\r\n");
        return false;
    }
    if heap.remaining_capacity() != 104 {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Heap remaining capacity mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Descriptor heap allocation and handle offset arithmetic verified\r\n");

    // 3. Verify Dynamic Root Binding Table and Hardware Offsets
    let mut binding_table = RootBindingTable::new(layout);

    if binding_table.all_parameters_bound() {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Initial binding table must not report all bound\r\n");
        return false;
    }

    // Bind slot 0: Descriptor Table
    if let Err(_) = binding_table.bind_descriptor_table(0, gpu_h0) {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Failed to bind descriptor table\r\n");
        return false;
    }

    // Bind slot 1: 32-bit Constants
    let const_vals = [0xCAFEBABE, 0xDEADBEEF, 0x12345678, 0x9ABCDEF0];
    if let Err(_) = binding_table.bind_32bit_constants(1, &const_vals, 0) {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Failed to bind 32-bit constants\r\n");
        return false;
    }

    // Bind slot 2: CBV
    if let Err(_) = binding_table.bind_root_descriptor(2, 0x3000_0000) {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Failed to bind CBV root descriptor\r\n");
        return false;
    }

    // Bind slot 3: SRV
    if let Err(_) = binding_table.bind_root_descriptor(3, 0x4000_0000) {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Failed to bind SRV root descriptor\r\n");
        return false;
    }

    if !binding_table.all_parameters_bound() {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] All parameters must be bound\r\n");
        return false;
    }
    if binding_table.compute_hardware_offset(0) != Some(0)
        || binding_table.compute_hardware_offset(1) != Some(4)
        || binding_table.compute_hardware_offset(2) != Some(20)
        || binding_table.compute_hardware_offset(3) != Some(28)
    {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Hardware byte offsets calculation mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Dynamic root parameter binding and hardware offset mapping verified\r\n");

    // 4. Verify Dirty Slot Bitmask and Incremental Update Pipeline
    if binding_table.dirty_slots != 0x0F {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Dirty slot bitmask mismatch\r\n");
        return false;
    }

    let mut flushed_count = 0;
    binding_table.flush_dirty_parameters(|_slot, _param, _hw_offset| {
        flushed_count += 1;
    });

    if flushed_count != 4 || binding_table.has_dirty_slots() {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Incremental dirty parameter flush failed\r\n");
        return false;
    }
    crate::serial_write_str("  * Incremental dirty slot tracking and hardware flush pipeline verified\r\n");

    // 5. Verify Centralized Root Signature Cache Deduplication
    let mut cache = RootSignatureCache::new();
    let l1 = match cache.get_or_compile(desc.clone()) {
        Ok(l) => l,
        Err(_) => return false,
    };
    let l2 = match cache.get_or_compile(desc) {
        Ok(l) => l,
        Err(_) => return false,
    };

    if l1 != l2 {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Cached layout mismatch\r\n");
        return false;
    }
    let (hits, misses) = cache.stats();
    if hits != 1 || misses != 1 {
        crate::serial_write_str("[KNT-ROOT-SIG-FAIL] Cache hits/misses telemetry mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * Centralized root signature cache deduplication verified\r\n");

    // 6. Verify Direct3D 12 Agility Dynamic Root Signature 1.2 Serialization & Deserialization
    let mut gov = D3d12RootSignature12Governor::new();
    let p_desc = alloc::vec![
        RootParameter12Desc::descriptor_table(D3D12_SHADER_VISIBILITY::Pixel, 8),
        RootParameter12Desc::constants(D3D12_SHADER_VISIBILITY::Vertex, 0, 0, 4),
        RootParameter12Desc::cbv(D3D12_SHADER_VISIBILITY::All, 1, 0),
        RootParameter12Desc::srv(D3D12_SHADER_VISIBILITY::Pixel, 0, 1),
    ];
    gov.configure_signature(0x1, &p_desc);
    assert_eq!(gov.total_dwords(), 1 + 4 + 2 + 2); // 9 DWORDs

    let bytecode = gov.bytecode().to_vec();
    let mut gov2 = D3d12RootSignature12Governor::new();
    assert!(gov2.load_bytecode(&bytecode).is_ok());
    assert_eq!(gov2.total_dwords(), 9);
    crate::serial_write_str("  * Direct3D 12 Root Signature 1.2 bytecode serialization and deserialization verified\r\n");

    // 7. Verify Descriptor Table Deduplication and Local Root Signature Binding Cache
    let (offset1, dedup1) = gov.table_deduplicator_mut().deduplicate_table(0xAABB_CCDD, 64);
    assert!(!dedup1);
    assert_eq!(offset1, 64);
    let (offset2, dedup2) = gov.table_deduplicator_mut().deduplicate_table(0xAABB_CCDD, 128);
    assert!(dedup2);
    assert_eq!(offset2, 64);
    assert_eq!(gov.table_deduplicator_mut().dedup_hits(), 1);

    let shader_id = [0x5Au8; 32];
    let (stride1, cached1) = gov.local_binding_cache_mut().get_or_register(1, &shader_id, 24);
    assert!(!cached1);
    assert_eq!(stride1, 64); // (32 + 24 = 56) -> aligned to 64
    let (stride2, cached2) = gov.local_binding_cache_mut().get_or_register(1, &shader_id, 24);
    assert!(cached2);
    assert_eq!(stride2, 64);
    let (l_hits, l_misses) = gov.local_binding_cache_mut().stats();
    assert_eq!(l_hits, 1);
    assert_eq!(l_misses, 1);
    crate::serial_write_str("  * Descriptor table deduplication and local root signature binding cache verified\r\n");

    // 8. Verify Static Sampler Register Table Indexing bypassing heap allocations
    let static_s = D3d12StaticSamplerDesc::new(
        D3d12StaticFilter::Anisotropic,
        D3d12StaticTextureAddressMode::Wrap,
        0,
        0,
        D3D12_SHADER_VISIBILITY::Pixel,
    );
    assert!(gov.static_samplers_mut().add_static_sampler(static_s).is_ok());
    assert_eq!(gov.static_samplers_mut().len(), 1);

    let looked_up = gov.static_samplers_mut().lookup_static_sampler(0, 0);
    assert!(looked_up.is_some());
    assert_eq!(looked_up.unwrap().filter, D3d12StaticFilter::Anisotropic);
    assert_eq!(gov.static_samplers_mut().bypassed_allocations(), 1);
    crate::serial_write_str("  * Static sampler register table indexing bypassing heap allocations verified\r\n");

    crate::serial_write_str("[KNT-D3D12-ROOT-SIG-PASS] Direct3D 12 Agility Dynamic Root Signature 1.2 & Static Samplers verified\r\n");
    crate::serial_write_str("[KNT-ROOT-SIG-PASS] Direct3D 12 root signature and descriptor heap binding verified\r\n");
    true
}
