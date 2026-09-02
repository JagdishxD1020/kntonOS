//! Direct3D 12 User-Mode Driver (UMD) Command Streamer & Swapchain Service.
//! Win32 DXGI swapchain, flip-model presentation, and userland command submission bridge.

extern crate alloc;
use alloc::vec::Vec;
use crate::display::color::Color;

/// Standard DXGI surface pixel formats used by Direct3D 12 applications.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxgiFormat {
    Unknown = 0,
    R32G32B32A32Float = 2,
    R16G16B16A16Float = 10,
    R8G8B8A8Unorm = 28,
    B8G8R8A8Unorm = 87,
    R10G10B10A2Unorm = 24,
}

impl DxgiFormat {
    /// Returns the bytes-per-pixel for the specified surface format.
    pub const fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::R32G32B32A32Float => 16,
            Self::R16G16B16A16Float => 8,
            Self::R8G8B8A8Unorm | Self::B8G8R8A8Unorm | Self::R10G10B10A2Unorm => 4,
            Self::Unknown => 0,
        }
    }
}

/// DXGI swapchain presentation effects controlling backbuffer management.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxgiSwapEffect {
    Discard = 0,
    Sequential = 1,
    FlipSequential = 3,
    FlipDiscard = 4,
}

/// Multi-sample anti-aliasing (MSAA) parameters for swapchain surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DxgiSampleDesc {
    pub count: u32,
    pub quality: u32,
}

impl Default for DxgiSampleDesc {
    fn default() -> Self {
        Self { count: 1, quality: 0 }
    }
}

/// Surface usage bitflags specifying access permissions for swapchain buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DxgiUsage(pub u32);

impl DxgiUsage {
    pub const RENDER_TARGET_OUTPUT: u32 = 1 << 5;
    pub const BACK_BUFFER: u32 = 1 << 6;
    pub const SHARED: u32 = 1 << 7;
    pub const READ_ONLY: u32 = 1 << 8;
}

/// Configuration descriptor for Direct3D 12 / DXGI swapchain creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DXGISwapChainDesc {
    pub width: u32,
    pub height: u32,
    pub format: DxgiFormat,
    pub buffer_count: u32,
    pub swap_effect: DxgiSwapEffect,
    pub sample_desc: DxgiSampleDesc,
    pub buffer_usage: u32,
    pub flags: u32,
}

impl Default for DXGISwapChainDesc {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            format: DxgiFormat::R8G8B8A8Unorm,
            buffer_count: 2,
            swap_effect: DxgiSwapEffect::FlipDiscard,
            sample_desc: DxgiSampleDesc::default(),
            buffer_usage: DxgiUsage::RENDER_TARGET_OUTPUT,
            flags: 0,
        }
    }
}

/// Swapchain presentation modes defining pacing and vertical blank synchronization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DXGIPresentationMode {
    /// Immediate uncapped presentation prioritizing lowest input latency.
    Mailbox,
    /// Vertical-sync locked presentation synchronized with display vblank.
    Fifo,
}

/// Individual backbuffer surface backing swapchain presentation.
#[derive(Debug, Clone)]
pub struct BackbufferSurface {
    pub buffer_index: u32,
    pub width: u32,
    pub height: u32,
    pub pitch_pixels: usize,
    pub pixels: Vec<Color>,
    pub fence_sync_value: u64,
}

impl BackbufferSurface {
    /// Allocates an empty backbuffer surface filled with transparent pixels.
    pub fn new(buffer_index: u32, width: u32, height: u32) -> Self {
        let total_pixels = (width as usize) * (height as usize);
        Self {
            buffer_index,
            width,
            height,
            pitch_pixels: width as usize,
            pixels: alloc::vec![Color::BLACK; total_pixels],
            fence_sync_value: 0,
        }
    }

    /// Clears the entire surface to a uniform solid color.
    pub fn clear(&mut self, color: Color) {
        for p in &mut self.pixels {
            *p = color;
        }
    }

    /// Sets an individual pixel color with boundary checks.
    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x < self.width as usize && y < self.height as usize {
            let idx = y * self.pitch_pixels + x;
            self.pixels[idx] = color;
        }
    }
}
