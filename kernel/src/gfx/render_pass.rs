//! Direct3D 12 Render Passes & Subpass Optimization Engine.
//! Provides render pass descriptors, tile memory cache optimization, and cross-command-list split passes.

#![allow(dead_code)]

extern crate alloc;
use alloc::vec::Vec;

// Classification of render pass beginning access actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE {
    #[default]
    Discard = 0,
    Preserve = 1,
    Clear = 2,
    NoAccess = 3,
}

// Classification of render pass ending access actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum D3D12_RENDER_PASS_ENDING_ACCESS_TYPE {
    #[default]
    Discard = 0,
    Preserve = 1,
    Resolve = 2,
    NoAccess = 3,
}

// Render pass flags controlling UAV writes and cross-command-list pass splitting
pub const D3D12_RENDER_PASS_FLAG_NONE: u32 = 0x0;
pub const D3D12_RENDER_PASS_FLAG_ALLOW_UAV_WRITES: u32 = 0x1;
pub const D3D12_RENDER_PASS_FLAG_SUSPENDING_PASS: u32 = 0x2;
pub const D3D12_RENDER_PASS_FLAG_RESUMING_PASS: u32 = 0x4;

// Clear color parameters for BeginningAccess::Clear
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_RENDER_PASS_BEGINNING_ACCESS_CLEAR_PARAMETERS {
    pub color: [f32; 4],
    pub depth: f32,
    pub stencil: u8,
}

// Beginning access descriptor configuring load/clear behavior at pass start
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_RENDER_PASS_BEGINNING_ACCESS {
    pub access_type: D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE,
    pub clear_parameters: D3D12_RENDER_PASS_BEGINNING_ACCESS_CLEAR_PARAMETERS,
}

impl D3D12_RENDER_PASS_BEGINNING_ACCESS {
    // Construct Discard beginning access
    pub const fn discard() -> Self {
        Self {
            access_type: D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE::Discard,
            clear_parameters: D3D12_RENDER_PASS_BEGINNING_ACCESS_CLEAR_PARAMETERS {
                color: [0.0; 4],
                depth: 1.0,
                stencil: 0,
            },
        }
    }

    // Construct Preserve beginning access (load previous contents from VRAM / tile cache)
    pub const fn preserve() -> Self {
        Self {
            access_type: D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE::Preserve,
            clear_parameters: D3D12_RENDER_PASS_BEGINNING_ACCESS_CLEAR_PARAMETERS {
                color: [0.0; 4],
                depth: 1.0,
                stencil: 0,
            },
        }
    }

    // Construct Clear beginning access with RGBA color
    pub const fn clear_color(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            access_type: D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE::Clear,
            clear_parameters: D3D12_RENDER_PASS_BEGINNING_ACCESS_CLEAR_PARAMETERS {
                color: [r, g, b, a],
                depth: 1.0,
                stencil: 0,
            },
        }
    }

    // Construct Clear beginning access for depth/stencil
    pub const fn clear_depth_stencil(depth: f32, stencil: u8) -> Self {
        Self {
            access_type: D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE::Clear,
            clear_parameters: D3D12_RENDER_PASS_BEGINNING_ACCESS_CLEAR_PARAMETERS {
                color: [0.0; 4],
                depth,
                stencil,
            },
        }
    }
}

// Subresource rect and coordinate parameters for MSAA resolve operations
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_RENDER_PASS_ENDING_ACCESS_RESOLVE_SUBRESOURCE_PARAMETERS {
    pub src_subresource: u32,
    pub dst_subresource: u32,
    pub dst_x: u32,
    pub dst_y: u32,
    pub src_rect: [u32; 4], // [left, top, right, bottom]
}

// Multisample resolve configuration at pass completion
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub struct D3D12_RENDER_PASS_ENDING_ACCESS_RESOLVE_PARAMETERS {
    pub src_resource_ptr: u64,
    pub dst_resource_ptr: u64,
    pub resolve_mode: u32, // 0: Min, 1: Max, 2: Average, 3: Decolorize
    pub format: u32,
    pub subresources: Vec<D3D12_RENDER_PASS_ENDING_ACCESS_RESOLVE_SUBRESOURCE_PARAMETERS>,
}

// Ending access descriptor configuring store/resolve/discard behavior at pass completion
#[derive(Debug, Clone, PartialEq, Default)]
#[allow(non_camel_case_types)]
pub struct D3D12_RENDER_PASS_ENDING_ACCESS {
    pub access_type: D3D12_RENDER_PASS_ENDING_ACCESS_TYPE,
    pub resolve_parameters: Option<D3D12_RENDER_PASS_ENDING_ACCESS_RESOLVE_PARAMETERS>,
}

impl D3D12_RENDER_PASS_ENDING_ACCESS {
    // Construct Discard ending access
    pub const fn discard() -> Self {
        Self {
            access_type: D3D12_RENDER_PASS_ENDING_ACCESS_TYPE::Discard,
            resolve_parameters: None,
        }
    }

    // Construct Preserve ending access (write back to memory)
    pub const fn preserve() -> Self {
        Self {
            access_type: D3D12_RENDER_PASS_ENDING_ACCESS_TYPE::Preserve,
            resolve_parameters: None,
        }
    }

    // Construct Resolve ending access for multisample resolve
    pub fn resolve(params: D3D12_RENDER_PASS_ENDING_ACCESS_RESOLVE_PARAMETERS) -> Self {
        Self {
            access_type: D3D12_RENDER_PASS_ENDING_ACCESS_TYPE::Resolve,
            resolve_parameters: Some(params),
        }
    }
}

// Render target attachment descriptor for a render pass
#[derive(Debug, Clone, PartialEq)]
#[allow(non_camel_case_types)]
pub struct D3D12_RENDER_PASS_RENDER_TARGET_DESC {
    pub cpu_descriptor_handle: u64,
    pub beginning_access: D3D12_RENDER_PASS_BEGINNING_ACCESS,
    pub ending_access: D3D12_RENDER_PASS_ENDING_ACCESS,
}

impl D3D12_RENDER_PASS_RENDER_TARGET_DESC {
    // Construct new render target descriptor
    pub fn new(
        cpu_handle: u64,
        beginning_access: D3D12_RENDER_PASS_BEGINNING_ACCESS,
        ending_access: D3D12_RENDER_PASS_ENDING_ACCESS,
    ) -> Self {
        Self {
            cpu_descriptor_handle: cpu_handle,
            beginning_access,
            ending_access,
        }
    }
}

// Depth/stencil attachment descriptor for a render pass
#[derive(Debug, Clone, PartialEq)]
#[allow(non_camel_case_types)]
pub struct D3D12_RENDER_PASS_DEPTH_STENCIL_DESC {
    pub cpu_descriptor_handle: u64,
    pub depth_beginning_access: D3D12_RENDER_PASS_BEGINNING_ACCESS,
    pub stencil_beginning_access: D3D12_RENDER_PASS_BEGINNING_ACCESS,
    pub depth_ending_access: D3D12_RENDER_PASS_ENDING_ACCESS,
    pub stencil_ending_access: D3D12_RENDER_PASS_ENDING_ACCESS,
}

impl D3D12_RENDER_PASS_DEPTH_STENCIL_DESC {
    // Construct new depth/stencil attachment descriptor
    pub fn new(
        cpu_handle: u64,
        depth_beg: D3D12_RENDER_PASS_BEGINNING_ACCESS,
        stencil_beg: D3D12_RENDER_PASS_BEGINNING_ACCESS,
        depth_end: D3D12_RENDER_PASS_ENDING_ACCESS,
        stencil_end: D3D12_RENDER_PASS_ENDING_ACCESS,
    ) -> Self {
        Self {
            cpu_descriptor_handle: cpu_handle,
            depth_beginning_access: depth_beg,
            stencil_beginning_access: stencil_beg,
            depth_ending_access: depth_end,
            stencil_ending_access: stencil_end,
        }
    }
}
