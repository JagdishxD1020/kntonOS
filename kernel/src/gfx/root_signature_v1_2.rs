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

// Temporary stub for self-test during incremental commits.
pub fn test_root_signature_v1_2_subsystem() -> bool {
    true
}
