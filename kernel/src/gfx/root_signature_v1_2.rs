//! Direct3D 12 Root Signature Version 1.2 & Local Root Signature Engine.
//! Direct heap indexing (SM 6.6 bindless) and shader record table layouts.

#![allow(dead_code, non_camel_case_types)]

extern crate alloc;
use alloc::vec::Vec;

// Direct3D Root Signature versions supported by hardware runtime.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum D3D_ROOT_SIGNATURE_VERSION {
    Version_1_0 = 0x1,
    Version_1_1 = 0x2,
    Version_1_2 = 0x3,
}

// Root signature 1.2 configuration and execution flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_ROOT_SIGNATURE_FLAGS_V1_2(pub u32);

impl D3D12_ROOT_SIGNATURE_FLAGS_V1_2 {
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

    // Tests if flags contain specified bitmask.
    #[inline(always)]
    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    // Combines two flag sets.
    #[inline(always)]
    pub const fn union(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    // Checks if bindless resource indexing is enabled.
    #[inline(always)]
    pub const fn is_bindless_resources_enabled(&self) -> bool {
        self.contains(Self::CBV_SRV_UAV_HEAP_DIRECTLY_INDEXED)
    }

    // Checks if bindless sampler indexing is enabled.
    #[inline(always)]
    pub const fn is_bindless_samplers_enabled(&self) -> bool {
        self.contains(Self::SAMPLER_HEAP_DIRECTLY_INDEXED)
    }

    // Checks if root signature is marked for local shader record binding.
    #[inline(always)]
    pub const fn is_local_root_signature(&self) -> bool {
        self.contains(Self::LOCAL_ROOT_SIGNATURE)
    }
}

// Descriptor range behavior and volatility flags for Root Signature 1.1 / 1.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_DESCRIPTOR_RANGE_FLAGS(pub u32);

impl D3D12_DESCRIPTOR_RANGE_FLAGS {
    pub const NONE: Self = Self(0);
    pub const DESCRIPTORS_VOLATILE: Self = Self(1 << 0);
    pub const DATA_VOLATILE: Self = Self(1 << 1);
    pub const DATA_STATIC_WHILE_SET_AT_EXECUTE: Self = Self(1 << 2);
    pub const DATA_STATIC: Self = Self(1 << 3);
    pub const DESCRIPTORS_STATIC_KEEPING_BUFFER_BOUNDS_CHECKS: Self = Self(1 << 16);
}

// Data volatility flags for root descriptors under Root Signature 1.1 / 1.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_ROOT_DESCRIPTOR_FLAGS_V1_2(pub u32);

impl D3D12_ROOT_DESCRIPTOR_FLAGS_V1_2 {
    pub const NONE: Self = Self(0);
    pub const DATA_VOLATILE: Self = Self(1 << 1);
    pub const DATA_STATIC_WHILE_SET_AT_EXECUTE: Self = Self(1 << 2);
    pub const DATA_STATIC: Self = Self(1 << 3);
}

// Descriptor range descriptor with volatility flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_DESCRIPTOR_RANGE1 {
    pub range_type: super::root_signature::D3D12_DESCRIPTOR_RANGE_TYPE,
    pub num_descriptors: u32,
    pub base_shader_register: u32,
    pub register_space: u32,
    pub flags: D3D12_DESCRIPTOR_RANGE_FLAGS,
    pub offset_in_descriptors_from_table_start: u32,
}

impl D3D12_DESCRIPTOR_RANGE1 {
    // Creates a new descriptor range descriptor.
    pub const fn new(
        range_type: super::root_signature::D3D12_DESCRIPTOR_RANGE_TYPE,
        num_descriptors: u32,
        base_shader_register: u32,
        register_space: u32,
        flags: D3D12_DESCRIPTOR_RANGE_FLAGS,
        offset_in_descriptors_from_table_start: u32,
    ) -> Self {
        Self {
            range_type,
            num_descriptors,
            base_shader_register,
            register_space,
            flags,
            offset_in_descriptors_from_table_start,
        }
    }
}

// Descriptor table holding multiple descriptor ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3D12_ROOT_DESCRIPTOR_TABLE1 {
    pub descriptor_ranges: Vec<D3D12_DESCRIPTOR_RANGE1>,
}

impl D3D12_ROOT_DESCRIPTOR_TABLE1 {
    // Creates a new descriptor table from a range list.
    pub fn new(descriptor_ranges: Vec<D3D12_DESCRIPTOR_RANGE1>) -> Self {
        Self { descriptor_ranges }
    }
}

// Root descriptor with data volatility flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_ROOT_DESCRIPTOR1 {
    pub shader_register: u32,
    pub register_space: u32,
    pub flags: D3D12_ROOT_DESCRIPTOR_FLAGS_V1_2,
}

impl D3D12_ROOT_DESCRIPTOR1 {
    // Creates a new root descriptor specification.
    pub const fn new(
        shader_register: u32,
        register_space: u32,
        flags: D3D12_ROOT_DESCRIPTOR_FLAGS_V1_2,
    ) -> Self {
        Self {
            shader_register,
            register_space,
            flags,
        }
    }
}

// Polymorphic payload for a Root Parameter 1.2 slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootParameter12Payload {
    DescriptorTable(D3D12_ROOT_DESCRIPTOR_TABLE1),
    Constants32Bit(super::root_signature::D3D12_ROOT_CONSTANTS),
    Cbv(D3D12_ROOT_DESCRIPTOR1),
    Srv(D3D12_ROOT_DESCRIPTOR1),
    Uav(D3D12_ROOT_DESCRIPTOR1),
}

// Single root parameter slot under Root Signature 1.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3D12_ROOT_PARAMETER1_2 {
    pub parameter_type: super::root_signature::D3D12_ROOT_PARAMETER_TYPE,
    pub shader_visibility: super::root_signature::D3D12_SHADER_VISIBILITY,
    pub payload: RootParameter12Payload,
}

impl D3D12_ROOT_PARAMETER1_2 {
    // Creates a descriptor table root parameter.
    pub fn descriptor_table(
        visibility: super::root_signature::D3D12_SHADER_VISIBILITY,
        ranges: Vec<D3D12_DESCRIPTOR_RANGE1>,
    ) -> Self {
        Self {
            parameter_type: super::root_signature::D3D12_ROOT_PARAMETER_TYPE::DescriptorTable,
            shader_visibility: visibility,
            payload: RootParameter12Payload::DescriptorTable(D3D12_ROOT_DESCRIPTOR_TABLE1::new(ranges)),
        }
    }

    // Creates an inline 32-bit constants root parameter.
    pub const fn constants(
        visibility: super::root_signature::D3D12_SHADER_VISIBILITY,
        shader_register: u32,
        register_space: u32,
        num_32bit_values: u32,
    ) -> Self {
        Self {
            parameter_type: super::root_signature::D3D12_ROOT_PARAMETER_TYPE::Constants32Bit,
            shader_visibility: visibility,
            payload: RootParameter12Payload::Constants32Bit(super::root_signature::D3D12_ROOT_CONSTANTS::new(
                shader_register,
                register_space,
                num_32bit_values,
            )),
        }
    }

    // Creates a CBV root descriptor parameter.
    pub const fn cbv(
        visibility: super::root_signature::D3D12_SHADER_VISIBILITY,
        shader_register: u32,
        register_space: u32,
        flags: D3D12_ROOT_DESCRIPTOR_FLAGS_V1_2,
    ) -> Self {
        Self {
            parameter_type: super::root_signature::D3D12_ROOT_PARAMETER_TYPE::Cbv,
            shader_visibility: visibility,
            payload: RootParameter12Payload::Cbv(D3D12_ROOT_DESCRIPTOR1::new(
                shader_register,
                register_space,
                flags,
            )),
        }
    }

    // Creates an SRV root descriptor parameter.
    pub const fn srv(
        visibility: super::root_signature::D3D12_SHADER_VISIBILITY,
        shader_register: u32,
        register_space: u32,
        flags: D3D12_ROOT_DESCRIPTOR_FLAGS_V1_2,
    ) -> Self {
        Self {
            parameter_type: super::root_signature::D3D12_ROOT_PARAMETER_TYPE::Srv,
            shader_visibility: visibility,
            payload: RootParameter12Payload::Srv(D3D12_ROOT_DESCRIPTOR1::new(
                shader_register,
                register_space,
                flags,
            )),
        }
    }

    // Creates a UAV root descriptor parameter.
    pub const fn uav(
        visibility: super::root_signature::D3D12_SHADER_VISIBILITY,
        shader_register: u32,
        register_space: u32,
        flags: D3D12_ROOT_DESCRIPTOR_FLAGS_V1_2,
    ) -> Self {
        Self {
            parameter_type: super::root_signature::D3D12_ROOT_PARAMETER_TYPE::Uav,
            shader_visibility: visibility,
            payload: RootParameter12Payload::Uav(D3D12_ROOT_DESCRIPTOR1::new(
                shader_register,
                register_space,
                flags,
            )),
        }
    }
}


// Filtering reduction and interpolation modes.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum D3D12_FILTER {
    MinMagMipPoint = 0,
    MinMagPointMipLinear = 0x1,
    MinPointMagLinearMipPoint = 0x4,
    MinPointMagMipLinear = 0x5,
    MinLinearMagMipPoint = 0x10,
    MinLinearMagPointMipLinear = 0x11,
    MinMagLinearMipPoint = 0x14,
    MinMagMipLinear = 0x15,
    Anisotropic = 0x55,
    ComparisonMinMagMipPoint = 0x80,
    ComparisonMinMagMipLinear = 0x95,
    ComparisonAnisotropic = 0xd5,
}

// Texture coordinate wrap and clamping modes.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum D3D12_TEXTURE_ADDRESS_MODE {
    Wrap = 1,
    Mirror = 2,
    Clamp = 3,
    Border = 4,
    MirrorOnce = 5,
}

// Predefined static sampler border colors with high precision.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum D3D12_STATIC_BORDER_COLOR {
    TransparentBlack = 0,
    OpaqueBlack = 1,
    OpaqueWhite = 2,
    UIntOpaqueBlack = 3,
    UIntOpaqueWhite = 4,
}

// Extended flags for static samplers in Root Signature 1.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct D3D12_STATIC_SAMPLER_FLAGS(pub u32);

impl D3D12_STATIC_SAMPLER_FLAGS {
    pub const NONE: Self = Self(0);
    pub const UINT_BORDER_COLOR: Self = Self(1 << 0);
    pub const NON_NORMALIZED_COORDINATES: Self = Self(1 << 1);
}

// Static sampler specification with extended border color precision and anisotropy limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3D12_STATIC_SAMPLER_DESC1 {
    pub filter: D3D12_FILTER,
    pub address_u: D3D12_TEXTURE_ADDRESS_MODE,
    pub address_v: D3D12_TEXTURE_ADDRESS_MODE,
    pub address_w: D3D12_TEXTURE_ADDRESS_MODE,
    pub mip_lod_bias_fixed: i16,
    pub max_anisotropy: u32,
    pub comparison_func: super::pso::D3D12_COMPARISON_FUNC,
    pub border_color: D3D12_STATIC_BORDER_COLOR,
    pub min_lod_fixed: u16,
    pub max_lod_fixed: u16,
    pub shader_register: u32,
    pub register_space: u32,
    pub shader_visibility: super::root_signature::D3D12_SHADER_VISIBILITY,
    pub flags: D3D12_STATIC_SAMPLER_FLAGS,
}

impl D3D12_STATIC_SAMPLER_DESC1 {
    // Validates static sampler configuration against Direct3D 12 specifications.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.max_anisotropy > 16 {
            return Err("Max anisotropy cannot exceed 16");
        }
        if matches!(self.filter, D3D12_FILTER::Anisotropic | D3D12_FILTER::ComparisonAnisotropic)
            && (self.max_anisotropy < 1 || self.max_anisotropy > 16)
        {
            return Err("Anisotropic filtering requires max_anisotropy between 1 and 16");
        }
        if self.min_lod_fixed > self.max_lod_fixed {
            return Err("min_lod cannot exceed max_lod");
        }
        Ok(())
    }

    // Compiles static sampler into 128-bit hardware sampler descriptor table entry.
    pub fn compile_hardware_descriptor(&self) -> CompiledHardwareSampler {
        let w0 = (self.filter as u32 & 0xff)
            | ((self.address_u as u32 & 0x7) << 8)
            | ((self.address_v as u32 & 0x7) << 11)
            | ((self.address_w as u32 & 0x7) << 14)
            | ((self.max_anisotropy.min(16) & 0x1f) << 17);

        let w1 = (self.mip_lod_bias_fixed as u16 as u32)
            | ((self.comparison_func as u32 & 0xf) << 16)
            | ((self.border_color as u32 & 0xf) << 20)
            | ((self.flags.0 & 0xf) << 24);

        let w2 = (self.min_lod_fixed as u32) | ((self.max_lod_fixed as u32) << 16);
        let w3 = self.shader_register | (self.register_space << 16);

        CompiledHardwareSampler {
            words: [w0, w1, w2, w3],
        }
    }
}

// Compiled 128-bit hardware sampler descriptor word representation.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledHardwareSampler {
    pub words: [u32; 4],
}

// Specification descriptor for Root Signature version 1.2 with extended static samplers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3D12_ROOT_SIGNATURE_DESC2 {
    pub parameters: Vec<D3D12_ROOT_PARAMETER1_2>,
    pub static_samplers: Vec<D3D12_STATIC_SAMPLER_DESC1>,
    pub flags: D3D12_ROOT_SIGNATURE_FLAGS_V1_2,
}

impl D3D12_ROOT_SIGNATURE_DESC2 {
    // Constructs an empty root signature 1.2 descriptor with flags.
    pub fn new(flags: D3D12_ROOT_SIGNATURE_FLAGS_V1_2) -> Self {
        Self {
            parameters: Vec::new(),
            static_samplers: Vec::new(),
            flags,
        }
    }

    // Appends a root parameter slot.
    pub fn add_parameter(&mut self, param: D3D12_ROOT_PARAMETER1_2) {
        self.parameters.push(param);
    }

    // Appends an extended static sampler.
    pub fn add_static_sampler(&mut self, sampler: D3D12_STATIC_SAMPLER_DESC1) {
        self.static_samplers.push(sampler);
    }
}

// Import raytracing alignments from dispatch_rays module to avoid ambiguous glob re-exports.
use super::dispatch_rays::{
    D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT,
    D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT,
    D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES,
};

// Maximum allowed byte size of local root arguments in a shader record (4 KiB).
pub const D3D12_MAX_LOCAL_ROOT_ARGUMENTS_SIZE_BYTES: usize = 4096;

// Local root record packing and layout for raytracing shader binding tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRootRecordLayout {
    pub shader_id: [u8; D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES],
    pub local_root_arguments: Vec<u8>,
}

impl LocalRootRecordLayout {
    // Creates a new local root shader record.
    pub fn new(shader_id: [u8; 32], local_args: &[u8]) -> Result<Self, &'static str> {
        if local_args.len() > D3D12_MAX_LOCAL_ROOT_ARGUMENTS_SIZE_BYTES {
            return Err("Local root arguments exceed maximum 4096 bytes");
        }
        Ok(Self {
            shader_id,
            local_root_arguments: local_args.to_vec(),
        })
    }

    // Calculates record byte size aligned to 32 bytes.
    #[inline(always)]
    pub fn packed_record_size(&self) -> usize {
        let raw = D3D12_SHADER_IDENTIFIER_SIZE_IN_BYTES + self.local_root_arguments.len();
        (raw + ((D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT as usize) - 1))
            & !((D3D12_RAYTRACING_SHADER_RECORD_BYTE_ALIGNMENT as usize) - 1)
    }

    // Serializes shader record into target memory buffer.
    pub fn serialize(&self, buffer: &mut [u8]) -> Result<usize, &'static str> {
        let stride = self.packed_record_size();
        if buffer.len() < stride {
            return Err("Destination buffer too small for packed shader record");
        }
        buffer[..32].copy_from_slice(&self.shader_id);
        buffer[32..32 + self.local_root_arguments.len()]
            .copy_from_slice(&self.local_root_arguments);
        // Zero pad remainder
        for b in &mut buffer[32 + self.local_root_arguments.len()..stride] {
            *b = 0;
        }
        Ok(stride)
    }
}

// Section layout description for a shader binding table range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShaderTableSectionInfo {
    pub buffer_offset: u64,
    pub size_in_bytes: u64,
    pub stride_in_bytes: u64,
    pub record_count: usize,
}

// Compiled layout of a multi-section shader binding table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShaderTableLayout {
    pub raygen: ShaderTableSectionInfo,
    pub miss: ShaderTableSectionInfo,
    pub hit_group: ShaderTableSectionInfo,
    pub callable: ShaderTableSectionInfo,
    pub total_sbt_bytes: u64,
}

// Helper builder to compile and pack shader binding table sections.
#[derive(Debug, Clone, Default)]
pub struct LocalRootShaderTableBuilder {
    raygen_records: Vec<LocalRootRecordLayout>,
    miss_records: Vec<LocalRootRecordLayout>,
    hit_group_records: Vec<LocalRootRecordLayout>,
    callable_records: Vec<LocalRootRecordLayout>,
}

impl LocalRootShaderTableBuilder {
    // Creates a new empty shader table builder.
    pub fn new() -> Self {
        Self::default()
    }

    // Adds a RayGen shader record.
    pub fn add_raygen(&mut self, record: LocalRootRecordLayout) {
        self.raygen_records.push(record);
    }

    // Adds a Miss shader record.
    pub fn add_miss(&mut self, record: LocalRootRecordLayout) {
        self.miss_records.push(record);
    }

    // Adds a HitGroup shader record.
    pub fn add_hit_group(&mut self, record: LocalRootRecordLayout) {
        self.hit_group_records.push(record);
    }

    // Adds a Callable shader record.
    pub fn add_callable(&mut self, record: LocalRootRecordLayout) {
        self.callable_records.push(record);
    }

    // Calculates section stride and total size.
    fn calculate_section_info(records: &[LocalRootRecordLayout], current_offset: &mut u64) -> ShaderTableSectionInfo {
        if records.is_empty() {
            return ShaderTableSectionInfo::default();
        }

        // Section base must align to 64 bytes
        *current_offset = (*current_offset + D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT - 1) & !(D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT - 1);
        let base_offset = *current_offset;

        // Max record stride in section, aligned to 32 bytes
        let mut max_stride = 32usize;
        for r in records {
            max_stride = max_stride.max(r.packed_record_size());
        }

        let section_size = (max_stride * records.len()) as u64;
        *current_offset = current_offset.saturating_add(section_size);

        ShaderTableSectionInfo {
            buffer_offset: base_offset,
            size_in_bytes: section_size,
            stride_in_bytes: max_stride as u64,
            record_count: records.len(),
        }
    }

    // Compiles table layout calculating offsets and strides.
    pub fn build_layout(&self) -> ShaderTableLayout {
        let mut offset = 0u64;
        let raygen = Self::calculate_section_info(&self.raygen_records, &mut offset);
        let miss = Self::calculate_section_info(&self.miss_records, &mut offset);
        let hit_group = Self::calculate_section_info(&self.hit_group_records, &mut offset);
        let callable = Self::calculate_section_info(&self.callable_records, &mut offset);

        ShaderTableLayout {
            raygen,
            miss,
            hit_group,
            callable,
            total_sbt_bytes: (offset + D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT - 1) & !(D3D12_RAYTRACING_SHADER_TABLE_BYTE_ALIGNMENT - 1),
        }
    }
}

// Work graph autonomous node record packing and offset descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkGraphNodeRecord {
    pub node_id: u32,
    pub node_type: u8,
    pub flags: u8,
    pub payload: Vec<u8>,
    pub local_root_arguments: Vec<u8>,
}

impl WorkGraphNodeRecord {
    // Creates a new work graph node record.
    pub fn new(node_id: u32, node_type: u8, flags: u8, payload: &[u8], local_args: &[u8]) -> Self {
        Self {
            node_id,
            node_type,
            flags,
            payload: payload.to_vec(),
            local_root_arguments: local_args.to_vec(),
        }
    }

    // Calculates aligned record stride (32-byte alignment).
    #[inline(always)]
    pub fn compute_record_stride(&self) -> usize {
        let raw = 8 + self.payload.len() + self.local_root_arguments.len();
        (raw + 31) & !31
    }

    // Packs node record into buffer.
    pub fn pack_record(&self, out: &mut [u8]) -> Result<usize, &'static str> {
        let stride = self.compute_record_stride();
        if out.len() < stride {
            return Err("Buffer too small for work graph node record");
        }
        out[0..4].copy_from_slice(&self.node_id.to_le_bytes());
        out[4] = self.node_type;
        out[5] = self.flags;
        out[6..8].copy_from_slice(&(self.payload.len() as u16).to_le_bytes());

        let payload_end = 8 + self.payload.len();
        out[8..payload_end].copy_from_slice(&self.payload);

        let args_end = payload_end + self.local_root_arguments.len();
        out[payload_end..args_end].copy_from_slice(&self.local_root_arguments);

        // Zero out padding
        for b in &mut out[args_end..stride] {
            *b = 0;
        }
        Ok(stride)
    }
}

// Maximum allowable root signature size in 32-bit DWORDs (256 bytes).
pub const D3D12_ROOT_SIGNATURE_MAX_COST_DWORDS: u32 = 64;

// Maximum allowable static samplers in a single root signature.
pub const D3D12_MAX_STATIC_SAMPLERS: usize = 128;

// Compiled hardware layout for a Direct3D 12 Root Signature Version 1.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRootSignature12 {
    pub version: D3D_ROOT_SIGNATURE_VERSION,
    pub flags: D3D12_ROOT_SIGNATURE_FLAGS_V1_2,
    pub total_dwords: u32,
    pub parameter_offsets: Vec<u32>,
    pub parameter_dwords: Vec<u32>,
    pub compiled_samplers: Vec<CompiledHardwareSampler>,
    pub is_bindless_resources: bool,
    pub is_bindless_samplers: bool,
    pub is_local: bool,
}

impl CompiledRootSignature12 {
    // Returns the total number of parameter slots in the compiled signature.
    #[inline(always)]
    pub fn num_parameters(&self) -> usize {
        self.parameter_offsets.len()
    }

    // Returns the root DWORD offset for the given parameter slot.
    #[inline(always)]
    pub fn get_parameter_offset(&self, slot: usize) -> Option<u32> {
        self.parameter_offsets.get(slot).copied()
    }

    // Returns the DWORD footprint for the given parameter slot.
    #[inline(always)]
    pub fn get_parameter_dwords(&self, slot: usize) -> Option<u32> {
        self.parameter_dwords.get(slot).copied()
    }
}

// Compiler engine converting Root Signature 1.2 descriptors to hardware execution states.
pub struct RootSignature12Compiler;

impl RootSignature12Compiler {
    // Compiles and validates Root Signature 1.2 descriptor against Agility SDK rules.
    pub fn compile(desc: &D3D12_ROOT_SIGNATURE_DESC2) -> Result<CompiledRootSignature12, &'static str> {
        if desc.static_samplers.len() > D3D12_MAX_STATIC_SAMPLERS {
            return Err("Static samplers count exceeds hardware maximum 128");
        }

        let mut compiled_samplers = Vec::with_capacity(desc.static_samplers.len());
        for sampler in &desc.static_samplers {
            sampler.validate()?;
            compiled_samplers.push(sampler.compile_hardware_descriptor());
        }

        let mut total_dwords = 0u32;
        let mut parameter_offsets = Vec::with_capacity(desc.parameters.len());
        let mut parameter_dwords = Vec::with_capacity(desc.parameters.len());

        for param in &desc.parameters {
            parameter_offsets.push(total_dwords);

            let dwords = match &param.payload {
                RootParameter12Payload::DescriptorTable(table) => {
                    for range in &table.descriptor_ranges {
                        if range.num_descriptors == 0 {
                            return Err("Descriptor range cannot contain 0 descriptors");
                        }
                    }
                    // Descriptor table handle occupies 1 DWORD (32-bit offset)
                    1
                }
                RootParameter12Payload::Constants32Bit(c) => {
                    if c.num_32bit_values == 0 {
                        return Err("Root constants count must be greater than 0");
                    }
                    c.num_32bit_values
                }
                RootParameter12Payload::Cbv(_)
                | RootParameter12Payload::Srv(_)
                | RootParameter12Payload::Uav(_) => {
                    // Direct 64-bit GPU virtual address occupies 2 DWORDs
                    2
                }
            };

            parameter_dwords.push(dwords);
            total_dwords = total_dwords.saturating_add(dwords);

            if total_dwords > D3D12_ROOT_SIGNATURE_MAX_COST_DWORDS {
                return Err("Root signature exceeds maximum 64 DWORD hardware budget");
            }
        }

        let is_bindless_resources = desc.flags.is_bindless_resources_enabled();
        let is_bindless_samplers = desc.flags.is_bindless_samplers_enabled();
        let is_local = desc.flags.is_local_root_signature();

        // Agility SDK validation: Local root signatures cannot define static samplers
        if is_local && !desc.static_samplers.is_empty() {
            return Err("Local root signatures cannot contain static samplers");
        }

        Ok(CompiledRootSignature12 {
            version: D3D_ROOT_SIGNATURE_VERSION::Version_1_2,
            flags: desc.flags,
            total_dwords,
            parameter_offsets,
            parameter_dwords,
            compiled_samplers,
            is_bindless_resources,
            is_bindless_samplers,
            is_local,
        })
    }
}
// Comprehensive self-test suite for Direct3D 12 Root Signature 1.2 & Local Shader Records.
pub fn test_root_signature_v1_2_subsystem() -> bool {
    // 1. Verify Root Signature 1.2 bindless direct indexing flags and compilation
    let flags = D3D12_ROOT_SIGNATURE_FLAGS_V1_2::CBV_SRV_UAV_HEAP_DIRECTLY_INDEXED
        .union(D3D12_ROOT_SIGNATURE_FLAGS_V1_2::SAMPLER_HEAP_DIRECTLY_INDEXED);

    if !flags.is_bindless_resources_enabled() || !flags.is_bindless_samplers_enabled() {
        return false;
    }

    let mut desc = D3D12_ROOT_SIGNATURE_DESC2::new(flags);

    // Param 0: 4 DWORD inline constants (push constants)
    desc.add_parameter(D3D12_ROOT_PARAMETER1_2::constants(
        super::root_signature::D3D12_SHADER_VISIBILITY::All,
        0,
        0,
        4,
    ));

    // Param 1: Root CBV descriptor (2 DWORDs)
    desc.add_parameter(D3D12_ROOT_PARAMETER1_2::cbv(
        super::root_signature::D3D12_SHADER_VISIBILITY::All,
        1,
        0,
        D3D12_ROOT_DESCRIPTOR_FLAGS_V1_2::DATA_STATIC,
    ));

    // Extended static sampler: Anisotropic x16 with border color
    let sampler = D3D12_STATIC_SAMPLER_DESC1 {
        filter: D3D12_FILTER::Anisotropic,
        address_u: D3D12_TEXTURE_ADDRESS_MODE::Clamp,
        address_v: D3D12_TEXTURE_ADDRESS_MODE::Clamp,
        address_w: D3D12_TEXTURE_ADDRESS_MODE::Clamp,
        mip_lod_bias_fixed: 0,
        max_anisotropy: 16,
        comparison_func: super::pso::D3D12_COMPARISON_FUNC::Always,
        border_color: D3D12_STATIC_BORDER_COLOR::OpaqueBlack,
        min_lod_fixed: 0,
        max_lod_fixed: 16 << 8,
        shader_register: 0,
        register_space: 0,
        shader_visibility: super::root_signature::D3D12_SHADER_VISIBILITY::Pixel,
        flags: D3D12_STATIC_SAMPLER_FLAGS::NONE,
    };
    desc.add_static_sampler(sampler);

    let compiled = match RootSignature12Compiler::compile(&desc) {
        Ok(c) => c,
        Err(_) => return false,
    };

    if compiled.total_dwords != 6 || compiled.num_parameters() != 2 {
        return false;
    }
    if compiled.get_parameter_offset(0) != Some(0) || compiled.get_parameter_offset(1) != Some(4) {
        return false;
    }
    if !compiled.is_bindless_resources || !compiled.is_bindless_samplers {
        return false;
    }

    // Verify invalid sampler rejection (anisotropy > 16)
    let invalid_sampler = D3D12_STATIC_SAMPLER_DESC1 {
        max_anisotropy: 32,
        ..sampler
    };
    if invalid_sampler.validate().is_ok() {
        return false;
    }

    // 2. Verify Local Root Signature Shader Record Table packing and alignment
    let shader_id_raygen = [0x11u8; 32];
    let shader_id_hit = [0x22u8; 32];

    let mut raygen_args = [0u8; 16];
    raygen_args[0..4].copy_from_slice(&0xDEADBEEFu32.to_le_bytes());

    let raygen_rec = match LocalRootRecordLayout::new(shader_id_raygen, &raygen_args) {
        Ok(r) => r,
        Err(_) => return false,
    };
    // 32 ID + 16 args = 48 bytes -> aligns to 64 bytes (multiple of 32)
    if raygen_rec.packed_record_size() != 64 {
        return false;
    }

    let mut sbt_buf = [0u8; 64];
    if raygen_rec.serialize(&mut sbt_buf).is_err() {
        return false;
    }
    if sbt_buf[0..32] != shader_id_raygen || sbt_buf[32..36] != 0xDEADBEEFu32.to_le_bytes() {
        return false;
    }

    // Test LocalRootShaderTableBuilder
    let mut sbt_builder = LocalRootShaderTableBuilder::new();
    sbt_builder.add_raygen(raygen_rec);

    let hit_args = [0xAAu8; 24]; // 32 + 24 = 56 -> aligns to 64
    let hit_rec = LocalRootRecordLayout::new(shader_id_hit, &hit_args).unwrap();
    sbt_builder.add_hit_group(hit_rec);

    let sbt_layout = sbt_builder.build_layout();
    if sbt_layout.raygen.record_count != 1 || sbt_layout.raygen.stride_in_bytes != 64 {
        return false;
    }
    if sbt_layout.hit_group.record_count != 1 || sbt_layout.hit_group.stride_in_bytes != 64 {
        return false;
    }
    // Each section base offset must be 64-byte aligned
    if (sbt_layout.raygen.buffer_offset % 64) != 0 || (sbt_layout.hit_group.buffer_offset % 64) != 0 {
        return false;
    }

    // 3. Verify Work Graph autonomous node record packing
    let node_rec = WorkGraphNodeRecord::new(
        101,
        0, // Broadcasting
        0x1,
        &[1, 2, 3, 4],
        &[0x55, 0x66],
    );
    // 8 header + 4 payload + 2 args = 14 bytes -> aligns to 32 bytes
    let stride = node_rec.compute_record_stride();
    if stride != 32 {
        return false;
    }

    let mut node_buf = [0u8; 32];
    if node_rec.pack_record(&mut node_buf).is_err() {
        return false;
    }
    if &node_buf[0..4] != &101u32.to_le_bytes() || node_buf[4] != 0 || node_buf[5] != 0x1 {
        return false;
    }

    crate::serial_write_str("[KNT-D3D12-ROOT-SIG-1-2-PASS] Direct3D 12 root signature 1.2 & local shader record table verified\r\n");
    true
}
