//! Direct3D 12 Graphics & Compute Pipeline State Object (PSO) Engine.
//! Provides pipeline descriptors, state validation, and layout compilation.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use alloc::vec::Vec;

// Append aligned element constant for automatic stride offset calculation
pub const D3D12_APPEND_ALIGNED_ELEMENT: u32 = 0xFFFF_FFFF;

// Color write enable bitflags
pub const D3D12_COLOR_WRITE_ENABLE_RED: u8 = 1;
pub const D3D12_COLOR_WRITE_ENABLE_GREEN: u8 = 2;
pub const D3D12_COLOR_WRITE_ENABLE_BLUE: u8 = 4;
pub const D3D12_COLOR_WRITE_ENABLE_ALPHA: u8 = 8;
pub const D3D12_COLOR_WRITE_ENABLE_ALL: u8 = 0x0F;

// Direct3D 12 primitive topology types for pipeline state classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_PRIMITIVE_TOPOLOGY_TYPE {
    Undefined = 0,
    Point = 1,
    Line = 2,
    Triangle = 3,
    Patch = 4,
}

// Rasterizer fill mode enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_FILL_MODE {
    Wireframe = 2,
    Solid = 3,
}

// Rasterizer face culling mode enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_CULL_MODE {
    None = 1,
    Front = 2,
    Back = 3,
}

// Direct3D 12 rasterizer configuration descriptor
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct D3D12_RASTERIZER_DESC {
    pub fill_mode: D3D12_FILL_MODE,
    pub cull_mode: D3D12_CULL_MODE,
    pub front_counter_clockwise: bool,
    pub depth_bias: i32,
    pub depth_bias_clamp: f32,
    pub slope_scaled_depth_bias: f32,
    pub depth_clip_enable: bool,
    pub multisample_enable: bool,
    pub antialiased_line_enable: bool,
    pub forced_sample_count: u32,
    pub conservative_rasterization: bool,
}

impl Default for D3D12_RASTERIZER_DESC {
    fn default() -> Self {
        Self {
            fill_mode: D3D12_FILL_MODE::Solid,
            cull_mode: D3D12_CULL_MODE::Back,
            front_counter_clockwise: false,
            depth_bias: 0,
            depth_bias_clamp: 0.0,
            slope_scaled_depth_bias: 0.0,
            depth_clip_enable: true,
            multisample_enable: false,
            antialiased_line_enable: false,
            forced_sample_count: 0,
            conservative_rasterization: false,
        }
    }
}

// Color and alpha blend factor options
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_BLEND {
    Zero = 1,
    One = 2,
    SrcColor = 3,
    InvSrcColor = 4,
    SrcAlpha = 5,
    InvSrcAlpha = 6,
    DestAlpha = 7,
    InvDestAlpha = 8,
    DestColor = 9,
    InvDestColor = 10,
    SrcAlphaSat = 11,
    BlendFactor = 14,
    InvBlendFactor = 15,
    Src1Color = 16,
    InvSrc1Color = 17,
    Src1Alpha = 18,
    InvSrc1Alpha = 19,
}

// Blend arithmetic operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_BLEND_OP {
    Add = 1,
    Subtract = 2,
    RevSubtract = 3,
    Min = 4,
    Max = 5,
}

// Output merger bitwise logic operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_LOGIC_OP {
    Clear = 0,
    Set = 1,
    Copy = 2,
    CopyInverted = 3,
    Noop = 4,
    Invert = 5,
    And = 6,
    Nand = 7,
    Or = 8,
    Nor = 9,
    Xor = 10,
    Equiv = 11,
    AndReverse = 12,
    AndInverted = 13,
    OrReverse = 14,
    OrInverted = 15,
}

// Blend settings for an individual render target slot
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct D3D12_RENDER_TARGET_BLEND_DESC {
    pub blend_enable: bool,
    pub logic_op_enable: bool,
    pub src_blend: D3D12_BLEND,
    pub dest_blend: D3D12_BLEND,
    pub blend_op: D3D12_BLEND_OP,
    pub src_blend_alpha: D3D12_BLEND,
    pub dest_blend_alpha: D3D12_BLEND,
    pub blend_op_alpha: D3D12_BLEND_OP,
    pub logic_op: D3D12_LOGIC_OP,
    pub render_target_write_mask: u8,
}

impl Default for D3D12_RENDER_TARGET_BLEND_DESC {
    fn default() -> Self {
        Self {
            blend_enable: false,
            logic_op_enable: false,
            src_blend: D3D12_BLEND::One,
            dest_blend: D3D12_BLEND::Zero,
            blend_op: D3D12_BLEND_OP::Add,
            src_blend_alpha: D3D12_BLEND::One,
            dest_blend_alpha: D3D12_BLEND::Zero,
            blend_op_alpha: D3D12_BLEND_OP::Add,
            logic_op: D3D12_LOGIC_OP::Noop,
            render_target_write_mask: D3D12_COLOR_WRITE_ENABLE_ALL,
        }
    }
}

// Global output merger blend state descriptor across all 8 render targets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct D3D12_BLEND_DESC {
    pub alpha_to_coverage_enable: bool,
    pub independent_blend_enable: bool,
    pub render_target: [D3D12_RENDER_TARGET_BLEND_DESC; 8],
}

impl Default for D3D12_BLEND_DESC {
    fn default() -> Self {
        Self {
            alpha_to_coverage_enable: false,
            independent_blend_enable: false,
            render_target: [D3D12_RENDER_TARGET_BLEND_DESC::default(); 8],
        }
    }
}

// Depth write mask flag
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_DEPTH_WRITE_MASK {
    Zero = 0,
    All = 1,
}

// Comparison functions for depth and stencil tests
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_COMPARISON_FUNC {
    Never = 1,
    Less = 2,
    Equal = 3,
    LessEqual = 4,
    Greater = 5,
    NotEqual = 6,
    GreaterEqual = 7,
    Always = 8,
}

// Stencil operational modifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_STENCIL_OP {
    Keep = 1,
    Zero = 2,
    Replace = 3,
    IncrSat = 4,
    DecrSat = 5,
    Invert = 6,
    Incr = 7,
    Decr = 8,
}

// Stencil operations for front or back faces
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct D3D12_DEPTH_STENCILOP_DESC {
    pub stencil_fail_op: D3D12_STENCIL_OP,
    pub stencil_depth_fail_op: D3D12_STENCIL_OP,
    pub stencil_pass_op: D3D12_STENCIL_OP,
    pub stencil_func: D3D12_COMPARISON_FUNC,
}

impl Default for D3D12_DEPTH_STENCILOP_DESC {
    fn default() -> Self {
        Self {
            stencil_fail_op: D3D12_STENCIL_OP::Keep,
            stencil_depth_fail_op: D3D12_STENCIL_OP::Keep,
            stencil_pass_op: D3D12_STENCIL_OP::Keep,
            stencil_func: D3D12_COMPARISON_FUNC::Always,
        }
    }
}

// Depth-stencil testing configuration descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct D3D12_DEPTH_STENCIL_DESC {
    pub depth_enable: bool,
    pub depth_write_mask: D3D12_DEPTH_WRITE_MASK,
    pub depth_func: D3D12_COMPARISON_FUNC,
    pub stencil_enable: bool,
    pub stencil_read_mask: u8,
    pub stencil_write_mask: u8,
    pub front_face: D3D12_DEPTH_STENCILOP_DESC,
    pub back_face: D3D12_DEPTH_STENCILOP_DESC,
}

impl Default for D3D12_DEPTH_STENCIL_DESC {
    fn default() -> Self {
        Self {
            depth_enable: true,
            depth_write_mask: D3D12_DEPTH_WRITE_MASK::All,
            depth_func: D3D12_COMPARISON_FUNC::Less,
            stencil_enable: false,
            stencil_read_mask: 0xFF,
            stencil_write_mask: 0xFF,
            front_face: D3D12_DEPTH_STENCILOP_DESC::default(),
            back_face: D3D12_DEPTH_STENCILOP_DESC::default(),
        }
    }
}

// DXGI surface and buffer data formats
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum DXGI_FORMAT {
    Unknown = 0,
    R32G32B32A32_FLOAT = 2,
    R32G32B32_FLOAT = 6,
    R16G16B16A16_FLOAT = 10,
    R32G32_FLOAT = 16,
    R8G8B8A8_UNORM = 28,
    B8G8R8A8_UNORM = 87,
    R32_FLOAT = 40,
    D24_UNORM_S8_UINT = 45,
    D16_UNORM = 55,
}

// Vertex input slot stepping classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum D3D12_INPUT_CLASSIFICATION {
    PerVertexData = 0,
    PerInstanceData = 1,
}

// Single vertex buffer attribute descriptor
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct D3D12_INPUT_ELEMENT_DESC {
    pub semantic_name: [u8; 16],
    pub semantic_index: u32,
    pub format: DXGI_FORMAT,
    pub input_slot: u32,
    pub aligned_byte_offset: u32,
    pub input_slot_class: D3D12_INPUT_CLASSIFICATION,
    pub instance_data_step_rate: u32,
}

impl D3D12_INPUT_ELEMENT_DESC {
    // Construct vertex input attribute from byte slice and parameters
    pub fn new(
        name: &[u8],
        index: u32,
        format: DXGI_FORMAT,
        slot: u32,
        offset: u32,
        slot_class: D3D12_INPUT_CLASSIFICATION,
        step_rate: u32,
    ) -> Self {
        let mut semantic_name = [0u8; 16];
        let copy_len = name.len().min(15);
        semantic_name[..copy_len].copy_from_slice(&name[..copy_len]);
        Self {
            semantic_name,
            semantic_index: index,
            format,
            input_slot: slot,
            aligned_byte_offset: offset,
            input_slot_class: slot_class,
            instance_data_step_rate: step_rate,
        }
    }
}

// Complete input assembler layout configuration
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct D3D12_INPUT_LAYOUT_DESC {
    pub elements: Vec<D3D12_INPUT_ELEMENT_DESC>,
}

// Raw compiled shader bytecode buffer holder
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct D3D12_SHADER_BYTECODE {
    pub bytecode: Vec<u8>,
}

impl D3D12_SHADER_BYTECODE {
    // Wrap bytecode slice into container
    pub fn new(bytes: &[u8]) -> Self {
        Self {
            bytecode: bytes.to_vec(),
        }
    }

    // Check if bytecode buffer is empty
    pub fn is_empty(&self) -> bool {
        self.bytecode.is_empty()
    }

    // Number of bytes in shader container
    pub fn len(&self) -> usize {
        self.bytecode.len()
    }
}

// Multisample antialiasing count and quality descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DXGI_SAMPLE_DESC {
    pub count: u32,
    pub quality: u32,
}

impl Default for DXGI_SAMPLE_DESC {
    fn default() -> Self {
        Self {
            count: 1,
            quality: 0,
        }
    }
}

// Pipeline compilation debug flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct D3D12_PIPELINE_STATE_FLAGS(pub u32);

impl D3D12_PIPELINE_STATE_FLAGS {
    pub const NONE: Self = Self(0);
    pub const TOOL_DEBUG: Self = Self(1);
}

// Direct3D 12 complete graphics pipeline state object descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct D3D12_GRAPHICS_PIPELINE_STATE_DESC {
    pub root_signature_id: u64,
    pub vs: D3D12_SHADER_BYTECODE,
    pub ps: D3D12_SHADER_BYTECODE,
    pub ds: D3D12_SHADER_BYTECODE,
    pub hs: D3D12_SHADER_BYTECODE,
    pub gs: D3D12_SHADER_BYTECODE,
    pub blend_state: D3D12_BLEND_DESC,
    pub sample_mask: u32,
    pub rasterizer_state: D3D12_RASTERIZER_DESC,
    pub depth_stencil_state: D3D12_DEPTH_STENCIL_DESC,
    pub input_layout: D3D12_INPUT_LAYOUT_DESC,
    pub ib_strip_cut_value: u32,
    pub primitive_topology_type: D3D12_PRIMITIVE_TOPOLOGY_TYPE,
    pub num_render_targets: u32,
    pub rtv_formats: [DXGI_FORMAT; 8],
    pub dsv_format: DXGI_FORMAT,
    pub sample_desc: DXGI_SAMPLE_DESC,
    pub node_mask: u32,
    pub flags: D3D12_PIPELINE_STATE_FLAGS,
}

impl Default for D3D12_GRAPHICS_PIPELINE_STATE_DESC {
    fn default() -> Self {
        Self {
            root_signature_id: 0,
            vs: D3D12_SHADER_BYTECODE::default(),
            ps: D3D12_SHADER_BYTECODE::default(),
            ds: D3D12_SHADER_BYTECODE::default(),
            hs: D3D12_SHADER_BYTECODE::default(),
            gs: D3D12_SHADER_BYTECODE::default(),
            blend_state: D3D12_BLEND_DESC::default(),
            sample_mask: 0xFFFF_FFFF,
            rasterizer_state: D3D12_RASTERIZER_DESC::default(),
            depth_stencil_state: D3D12_DEPTH_STENCIL_DESC::default(),
            input_layout: D3D12_INPUT_LAYOUT_DESC::default(),
            ib_strip_cut_value: 0,
            primitive_topology_type: D3D12_PRIMITIVE_TOPOLOGY_TYPE::Triangle,
            num_render_targets: 1,
            rtv_formats: [
                DXGI_FORMAT::R8G8B8A8_UNORM,
                DXGI_FORMAT::Unknown,
                DXGI_FORMAT::Unknown,
                DXGI_FORMAT::Unknown,
                DXGI_FORMAT::Unknown,
                DXGI_FORMAT::Unknown,
                DXGI_FORMAT::Unknown,
                DXGI_FORMAT::Unknown,
            ],
            dsv_format: DXGI_FORMAT::Unknown,
            sample_desc: DXGI_SAMPLE_DESC::default(),
            node_mask: 0,
            flags: D3D12_PIPELINE_STATE_FLAGS::NONE,
        }
    }
}

// Direct3D 12 compute pipeline state object descriptor
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct D3D12_COMPUTE_PIPELINE_STATE_DESC {
    pub root_signature_id: u64,
    pub cs: D3D12_SHADER_BYTECODE,
    pub node_mask: u32,
    pub flags: D3D12_PIPELINE_STATE_FLAGS,
}

impl Default for D3D12_COMPUTE_PIPELINE_STATE_DESC {
    fn default() -> Self {
        Self {
            root_signature_id: 0,
            cs: D3D12_SHADER_BYTECODE::default(),
            node_mask: 0,
            flags: D3D12_PIPELINE_STATE_FLAGS::NONE,
        }
    }
}

// Errors produced during PSO validation and compilation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsoError {
    MissingRootSignature,
    MissingVertexShader,
    MissingComputeShader,
    InvalidRenderTargetCount,
    InvalidPrimitiveTopology,
    InvalidSampleCount,
    InvalidShaderBytecode,
    InvalidContainerMagic,
    BytecodeTooSmall,
    CompilationFailed,
}

impl D3D12_GRAPHICS_PIPELINE_STATE_DESC {
    // Validate graphics pipeline state descriptor integrity
    pub fn validate(&self) -> Result<(), PsoError> {
        if self.vs.is_empty() {
            return Err(PsoError::MissingVertexShader);
        }
        if self.num_render_targets > 8 {
            return Err(PsoError::InvalidRenderTargetCount);
        }
        if self.primitive_topology_type == D3D12_PRIMITIVE_TOPOLOGY_TYPE::Undefined {
            return Err(PsoError::InvalidPrimitiveTopology);
        }
        if self.sample_desc.count == 0 {
            return Err(PsoError::InvalidSampleCount);
        }
        Ok(())
    }
}

impl D3D12_COMPUTE_PIPELINE_STATE_DESC {
    // Validate compute pipeline state descriptor integrity
    pub fn validate(&self) -> Result<(), PsoError> {
        if self.cs.is_empty() {
            return Err(PsoError::MissingComputeShader);
        }
        Ok(())
    }
}
