//! Direct3D 12 Graphics & Compute Pipeline State Object (PSO) Engine.
//! Provides pipeline descriptors, state validation, and layout compilation.

#![allow(non_camel_case_types, non_snake_case, dead_code)]

use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::crypto::Sha256;

// Append aligned element constant for automatic stride offset calculation
pub const D3D12_APPEND_ALIGNED_ELEMENT: u32 = 0xFFFF_FFFF;

// Color write enable bitflags
pub const D3D12_COLOR_WRITE_ENABLE_RED: u8 = 1;
pub const D3D12_COLOR_WRITE_ENABLE_GREEN: u8 = 2;
pub const D3D12_COLOR_WRITE_ENABLE_BLUE: u8 = 4;
pub const D3D12_COLOR_WRITE_ENABLE_ALPHA: u8 = 8;
pub const D3D12_COLOR_WRITE_ENABLE_ALL: u8 = 0x0F;

// FourCC magic identifiers for shader bytecode containers
pub const DXBC_MAGIC: u32 = 0x4342_5844;
pub const LLVM_BC_MAGIC: u32 = 0xDEC0_4342;
pub const PSO_CACHE_MAGIC: u32 = 0x4F53_504B;

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

// Shader container format classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderContainerType {
    Dxbc,
    Dxil,
}

// Metadata for an individual chunk within a DXBC container
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderChunkInfo {
    pub fourcc: [u8; 4],
    pub offset: u32,
    pub size: u32,
}

// Parsed and validated DXBC/DXIL container header
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DxbcContainerHeader {
    pub checksum: [u8; 16],
    pub version: u32,
    pub total_size: u32,
    pub chunk_count: u32,
    pub chunks: Vec<ShaderChunkInfo>,
    pub container_type: ShaderContainerType,
}

// Validates DXBC or DXIL bytecode container structure and chunk offsets
pub fn parse_and_validate_shader_bytecode(bytecode: &[u8]) -> Result<DxbcContainerHeader, PsoError> {
    if bytecode.len() < 32 {
        return Err(PsoError::BytecodeTooSmall);
    }

    let magic = u32::from_le_bytes([bytecode[0], bytecode[1], bytecode[2], bytecode[3]]);
    if magic == LLVM_BC_MAGIC {
        return Ok(DxbcContainerHeader {
            checksum: [0u8; 16],
            version: 1,
            total_size: bytecode.len() as u32,
            chunk_count: 1,
            chunks: alloc::vec![ShaderChunkInfo {
                fourcc: *b"DXIL",
                offset: 0,
                size: bytecode.len() as u32,
            }],
            container_type: ShaderContainerType::Dxil,
        });
    }

    if magic != DXBC_MAGIC {
        return Err(PsoError::InvalidContainerMagic);
    }

    let mut checksum = [0u8; 16];
    checksum.copy_from_slice(&bytecode[4..20]);

    let version = u32::from_le_bytes([bytecode[20], bytecode[21], bytecode[22], bytecode[23]]);
    let total_size = u32::from_le_bytes([bytecode[24], bytecode[25], bytecode[26], bytecode[27]]);
    let chunk_count = u32::from_le_bytes([bytecode[28], bytecode[29], bytecode[30], bytecode[31]]);

    if total_size as usize != bytecode.len() {
        return Err(PsoError::InvalidShaderBytecode);
    }

    let offset_table_end = 32 + (chunk_count as usize * 4);
    if offset_table_end > bytecode.len() {
        return Err(PsoError::BytecodeTooSmall);
    }

    let mut chunks = Vec::with_capacity(chunk_count as usize);
    let mut container_type = ShaderContainerType::Dxbc;

    for i in 0..chunk_count as usize {
        let entry_pos = 32 + (i * 4);
        let chunk_offset = u32::from_le_bytes([
            bytecode[entry_pos],
            bytecode[entry_pos + 1],
            bytecode[entry_pos + 2],
            bytecode[entry_pos + 3],
        ]) as usize;

        if chunk_offset + 8 > bytecode.len() {
            return Err(PsoError::BytecodeTooSmall);
        }

        let mut fourcc = [0u8; 4];
        fourcc.copy_from_slice(&bytecode[chunk_offset..chunk_offset + 4]);

        let chunk_size = u32::from_le_bytes([
            bytecode[chunk_offset + 4],
            bytecode[chunk_offset + 5],
            bytecode[chunk_offset + 6],
            bytecode[chunk_offset + 7],
        ]);

        if chunk_offset + 8 + (chunk_size as usize) > bytecode.len() {
            return Err(PsoError::InvalidShaderBytecode);
        }

        if &fourcc == b"DXIL" || &fourcc == b"ILDB" {
            container_type = ShaderContainerType::Dxil;
        }

        chunks.push(ShaderChunkInfo {
            fourcc,
            offset: chunk_offset as u32,
            size: chunk_size,
        });
    }

    Ok(DxbcContainerHeader {
        checksum,
        version,
        total_size,
        chunk_count,
        chunks,
        container_type,
    })
}

// Builds a synthetic DXBC container with an arbitrary chunk payload
pub fn create_test_dxbc_container(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let chunk_size = payload.len() as u32;
    let total_size = 32 + 4 + 8 + chunk_size;
    let mut out = Vec::with_capacity(total_size as usize);

    out.extend_from_slice(b"DXBC");
    out.extend_from_slice(&[0x11; 16]);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&total_size.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&36u32.to_le_bytes());
    out.extend_from_slice(fourcc);
    out.extend_from_slice(&chunk_size.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

// 256-bit cryptographic pipeline state fingerprint
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PsoHash(pub [u8; 32]);

// Computes deterministic SHA-256 fingerprint for graphics pipeline state
pub fn compute_graphics_pso_hash(desc: &D3D12_GRAPHICS_PIPELINE_STATE_DESC) -> PsoHash {
    let mut hasher = Sha256::new();
    hasher.update(&desc.root_signature_id.to_le_bytes());
    hasher.update(&(desc.vs.len() as u32).to_le_bytes());
    hasher.update(&desc.vs.bytecode);
    hasher.update(&(desc.ps.len() as u32).to_le_bytes());
    hasher.update(&desc.ps.bytecode);
    hasher.update(&(desc.ds.len() as u32).to_le_bytes());
    hasher.update(&desc.ds.bytecode);
    hasher.update(&(desc.hs.len() as u32).to_le_bytes());
    hasher.update(&desc.hs.bytecode);
    hasher.update(&(desc.gs.len() as u32).to_le_bytes());
    hasher.update(&desc.gs.bytecode);

    hasher.update(&[
        desc.blend_state.alpha_to_coverage_enable as u8,
        desc.blend_state.independent_blend_enable as u8,
    ]);
    for rt in &desc.blend_state.render_target {
        hasher.update(&[rt.blend_enable as u8, rt.logic_op_enable as u8, rt.render_target_write_mask]);
        hasher.update(&(rt.src_blend as u32).to_le_bytes());
        hasher.update(&(rt.dest_blend as u32).to_le_bytes());
        hasher.update(&(rt.blend_op as u32).to_le_bytes());
        hasher.update(&(rt.src_blend_alpha as u32).to_le_bytes());
        hasher.update(&(rt.dest_blend_alpha as u32).to_le_bytes());
        hasher.update(&(rt.blend_op_alpha as u32).to_le_bytes());
        hasher.update(&(rt.logic_op as u32).to_le_bytes());
    }

    hasher.update(&desc.sample_mask.to_le_bytes());
    hasher.update(&(desc.rasterizer_state.fill_mode as u32).to_le_bytes());
    hasher.update(&(desc.rasterizer_state.cull_mode as u32).to_le_bytes());
    hasher.update(&[
        desc.rasterizer_state.front_counter_clockwise as u8,
        desc.rasterizer_state.depth_clip_enable as u8,
        desc.rasterizer_state.multisample_enable as u8,
        desc.rasterizer_state.antialiased_line_enable as u8,
        desc.rasterizer_state.conservative_rasterization as u8,
    ]);
    hasher.update(&desc.rasterizer_state.depth_bias.to_le_bytes());
    hasher.update(&desc.rasterizer_state.depth_bias_clamp.to_bits().to_le_bytes());
    hasher.update(&desc.rasterizer_state.slope_scaled_depth_bias.to_bits().to_le_bytes());
    hasher.update(&desc.rasterizer_state.forced_sample_count.to_le_bytes());

    hasher.update(&[
        desc.depth_stencil_state.depth_enable as u8,
        desc.depth_stencil_state.stencil_enable as u8,
        desc.depth_stencil_state.stencil_read_mask,
        desc.depth_stencil_state.stencil_write_mask,
    ]);
    hasher.update(&(desc.depth_stencil_state.depth_write_mask as u32).to_le_bytes());
    hasher.update(&(desc.depth_stencil_state.depth_func as u32).to_le_bytes());
    hasher.update(&(desc.depth_stencil_state.front_face.stencil_fail_op as u32).to_le_bytes());
    hasher.update(&(desc.depth_stencil_state.front_face.stencil_depth_fail_op as u32).to_le_bytes());
    hasher.update(&(desc.depth_stencil_state.front_face.stencil_pass_op as u32).to_le_bytes());
    hasher.update(&(desc.depth_stencil_state.front_face.stencil_func as u32).to_le_bytes());
    hasher.update(&(desc.depth_stencil_state.back_face.stencil_fail_op as u32).to_le_bytes());
    hasher.update(&(desc.depth_stencil_state.back_face.stencil_depth_fail_op as u32).to_le_bytes());
    hasher.update(&(desc.depth_stencil_state.back_face.stencil_pass_op as u32).to_le_bytes());
    hasher.update(&(desc.depth_stencil_state.back_face.stencil_func as u32).to_le_bytes());

    hasher.update(&(desc.input_layout.elements.len() as u32).to_le_bytes());
    for elem in &desc.input_layout.elements {
        hasher.update(&elem.semantic_name);
        hasher.update(&elem.semantic_index.to_le_bytes());
        hasher.update(&(elem.format as u32).to_le_bytes());
        hasher.update(&elem.input_slot.to_le_bytes());
        hasher.update(&elem.aligned_byte_offset.to_le_bytes());
        hasher.update(&(elem.input_slot_class as u32).to_le_bytes());
        hasher.update(&elem.instance_data_step_rate.to_le_bytes());
    }

    hasher.update(&desc.ib_strip_cut_value.to_le_bytes());
    hasher.update(&(desc.primitive_topology_type as u32).to_le_bytes());
    hasher.update(&desc.num_render_targets.to_le_bytes());
    for fmt in &desc.rtv_formats {
        hasher.update(&(*fmt as u32).to_le_bytes());
    }
    hasher.update(&(desc.dsv_format as u32).to_le_bytes());
    hasher.update(&desc.sample_desc.count.to_le_bytes());
    hasher.update(&desc.sample_desc.quality.to_le_bytes());
    hasher.update(&desc.node_mask.to_le_bytes());
    hasher.update(&desc.flags.0.to_le_bytes());

    PsoHash(hasher.finalize())
}

// Computes deterministic SHA-256 fingerprint for compute pipeline state
pub fn compute_compute_pso_hash(desc: &D3D12_COMPUTE_PIPELINE_STATE_DESC) -> PsoHash {
    let mut hasher = Sha256::new();
    hasher.update(&desc.root_signature_id.to_le_bytes());
    hasher.update(&(desc.cs.len() as u32).to_le_bytes());
    hasher.update(&desc.cs.bytecode);
    hasher.update(&desc.node_mask.to_le_bytes());
    hasher.update(&desc.flags.0.to_le_bytes());
    PsoHash(hasher.finalize())
}

// Compiled hardware graphics pipeline state object
#[derive(Debug, Clone)]
pub struct CompiledGraphicsPipelineState {
    pub hash: PsoHash,
    pub desc: D3D12_GRAPHICS_PIPELINE_STATE_DESC,
    pub vs_header: DxbcContainerHeader,
    pub ps_header: Option<DxbcContainerHeader>,
    pub hw_pipeline_id: u64,
}

// Compiled hardware compute pipeline state object
#[derive(Debug, Clone)]
pub struct CompiledComputePipelineState {
    pub hash: PsoHash,
    pub desc: D3D12_COMPUTE_PIPELINE_STATE_DESC,
    pub cs_header: DxbcContainerHeader,
    pub hw_pipeline_id: u64,
}

// Centralized PSO memory cache and deduplication store
pub struct PsoCache {
    graphics_psos: Vec<Arc<CompiledGraphicsPipelineState>>,
    compute_psos: Vec<Arc<CompiledComputePipelineState>>,
    cache_hits: u64,
    cache_misses: u64,
    next_pipeline_id: u64,
}

impl PsoCache {
    // Construct empty pipeline state object cache
    pub fn new() -> Self {
        Self {
            graphics_psos: Vec::new(),
            compute_psos: Vec::new(),
            cache_hits: 0,
            cache_misses: 0,
            next_pipeline_id: 1,
        }
    }

    // Lookup existing compiled graphics pipeline by hash
    pub fn find_graphics(&self, hash: &PsoHash) -> Option<Arc<CompiledGraphicsPipelineState>> {
        self.graphics_psos.iter().find(|pso| pso.hash == *hash).cloned()
    }

    // Lookup existing compiled compute pipeline by hash
    pub fn find_compute(&self, hash: &PsoHash) -> Option<Arc<CompiledComputePipelineState>> {
        self.compute_psos.iter().find(|pso| pso.hash == *hash).cloned()
    }

    // Retrieve or compile graphics pipeline state object with deduplication
    pub fn get_or_compile_graphics(
        &mut self,
        desc: D3D12_GRAPHICS_PIPELINE_STATE_DESC,
    ) -> Result<Arc<CompiledGraphicsPipelineState>, PsoError> {
        desc.validate()?;
        let hash = compute_graphics_pso_hash(&desc);

        if let Some(existing) = self.find_graphics(&hash) {
            self.cache_hits += 1;
            return Ok(existing);
        }

        self.cache_misses += 1;
        let vs_header = parse_and_validate_shader_bytecode(&desc.vs.bytecode)?;
        let ps_header = if !desc.ps.is_empty() {
            Some(parse_and_validate_shader_bytecode(&desc.ps.bytecode)?)
        } else {
            None
        };

        let hw_pipeline_id = self.next_pipeline_id;
        self.next_pipeline_id += 1;

        let compiled = Arc::new(CompiledGraphicsPipelineState {
            hash,
            desc,
            vs_header,
            ps_header,
            hw_pipeline_id,
        });

        self.graphics_psos.push(Arc::clone(&compiled));
        Ok(compiled)
    }

    // Retrieve or compile compute pipeline state object with deduplication
    pub fn get_or_compile_compute(
        &mut self,
        desc: D3D12_COMPUTE_PIPELINE_STATE_DESC,
    ) -> Result<Arc<CompiledComputePipelineState>, PsoError> {
        desc.validate()?;
        let hash = compute_compute_pso_hash(&desc);

        if let Some(existing) = self.find_compute(&hash) {
            self.cache_hits += 1;
            return Ok(existing);
        }

        self.cache_misses += 1;
        let cs_header = parse_and_validate_shader_bytecode(&desc.cs.bytecode)?;

        let hw_pipeline_id = self.next_pipeline_id;
        self.next_pipeline_id += 1;

        let compiled = Arc::new(CompiledComputePipelineState {
            hash,
            desc,
            cs_header,
            hw_pipeline_id,
        });

        self.compute_psos.push(Arc::clone(&compiled));
        Ok(compiled)
    }

    // Telemetry query for cache performance
    pub fn stats(&self) -> (u64, u64, usize) {
        (
            self.cache_hits,
            self.cache_misses,
            self.graphics_psos.len() + self.compute_psos.len(),
        )
    }

    // Export cache index binary blob for quick serialization
    pub fn export_blob(&self) -> Vec<u8> {
        let total_count = (self.graphics_psos.len() + self.compute_psos.len()) as u32;
        let mut blob = Vec::with_capacity(8 + (total_count as usize * 40));
        blob.extend_from_slice(&PSO_CACHE_MAGIC.to_le_bytes());
        blob.extend_from_slice(&(self.graphics_psos.len() as u32).to_le_bytes());
        for pso in &self.graphics_psos {
            blob.extend_from_slice(&pso.hash.0);
            blob.extend_from_slice(&pso.hw_pipeline_id.to_le_bytes());
        }
        blob.extend_from_slice(&(self.compute_psos.len() as u32).to_le_bytes());
        for pso in &self.compute_psos {
            blob.extend_from_slice(&pso.hash.0);
            blob.extend_from_slice(&pso.hw_pipeline_id.to_le_bytes());
        }
        blob
    }

    // Purge all compiled pipelines from memory store
    pub fn clear(&mut self) {
        self.graphics_psos.clear();
        self.compute_psos.clear();
        self.cache_hits = 0;
        self.cache_misses = 0;
    }
}
