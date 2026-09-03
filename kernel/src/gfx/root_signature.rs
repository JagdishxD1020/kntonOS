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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl RootBindingTable {
    /// Creates a new binding table configured for the given root signature layout.
    pub fn new(layout: RootSignatureLayout) -> Self {
        let num_params = layout.num_parameters();
        Self {
            layout,
            bindings: alloc::vec![None; num_params],
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
}
