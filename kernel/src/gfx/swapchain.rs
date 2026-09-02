//! Direct3D 12 User-Mode Driver (UMD) Command Streamer & Swapchain Service.
//! Win32 DXGI swapchain, flip-model presentation, and userland command submission bridge.

extern crate alloc;
use alloc::vec::Vec;
use crate::display::color::Color;
use crate::compositor::dirty::Rect;
use crate::gfx::cmdbuf::{CommandBufferRecorder, DrawIndexedPayload, GpuCommandBuffer, PipelineStageFlags, PrimitiveTopology, ResourceState, SetPipelineStatePayload};
use crate::gfx::gpu_fence::GpuTimelineFence;

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

/// Core Direct3D 12 DXGI swapchain managing multi-buffer rotation and presentation.
#[derive(Debug)]
pub struct DXGISwapChain {
    pub desc: DXGISwapChainDesc,
    pub backbuffers: Vec<BackbufferSurface>,
    pub current_backbuffer_index: usize,
    pub last_presented_index: usize,
    pub present_mode: DXGIPresentationMode,
    pub present_count: u64,
    pub dropped_frames_count: u64,
    pub last_present_timestamp_ns: u64,
    pub rolling_latency_us: [u32; 16],
    pub latency_head: usize,
    pub latency_count: usize,
}

impl DXGISwapChain {
    /// Creates a new DXGI swapchain instance according to descriptor specification.
    pub fn new(desc: DXGISwapChainDesc) -> Result<Self, &'static str> {
        if desc.buffer_count < 2 || desc.buffer_count > 8 {
            return Err("Swapchain buffer count must be between 2 and 8");
        }
        if desc.width == 0 || desc.height == 0 {
            return Err("Swapchain dimensions must be non-zero");
        }

        let mut backbuffers = Vec::with_capacity(desc.buffer_count as usize);
        for i in 0..desc.buffer_count {
            backbuffers.push(BackbufferSurface::new(i, desc.width, desc.height));
        }

        Ok(Self {
            desc,
            backbuffers,
            current_backbuffer_index: 0,
            last_presented_index: 0,
            present_mode: DXGIPresentationMode::Fifo,
            present_count: 0,
            dropped_frames_count: 0,
            last_present_timestamp_ns: 0,
            rolling_latency_us: [0; 16],
            latency_head: 0,
            latency_count: 0,
        })
    }

    /// Returns a reference to the backbuffer at the requested index.
    pub fn get_backbuffer(&self, index: usize) -> Option<&BackbufferSurface> {
        self.backbuffers.get(index)
    }

    /// Returns a mutable reference to the backbuffer at the requested index.
    pub fn get_backbuffer_mut(&mut self, index: usize) -> Option<&mut BackbufferSurface> {
        self.backbuffers.get_mut(index)
    }

    /// Returns a reference to the active rendering backbuffer.
    pub fn get_current_backbuffer(&self) -> &BackbufferSurface {
        &self.backbuffers[self.current_backbuffer_index]
    }

    /// Returns a mutable reference to the active rendering backbuffer.
    pub fn get_current_backbuffer_mut(&mut self) -> &mut BackbufferSurface {
        &mut self.backbuffers[self.current_backbuffer_index]
    }

    /// Returns a reference to the most recently presented front buffer surface.
    pub fn get_presented_frontbuffer(&self) -> &BackbufferSurface {
        &self.backbuffers[self.last_presented_index]
    }

    /// Rotates the backbuffer ring and presents the finished frame to the display subsystem.
    pub fn present(
        &mut self,
        sync_interval: u32,
        _flags: u32,
        timestamp_ns: u64,
    ) -> Result<u32, &'static str> {
        if self.backbuffers.is_empty() {
            return Err("Cannot present with empty swapchain backbuffers");
        }

        // Determine presentation mode from sync interval
        self.present_mode = if sync_interval == 0 {
            DXGIPresentationMode::Mailbox
        } else {
            DXGIPresentationMode::Fifo
        };

        // Record presentation latency tracking
        if self.last_present_timestamp_ns > 0 && timestamp_ns > self.last_present_timestamp_ns {
            let latency_ns = timestamp_ns - self.last_present_timestamp_ns;
            let latency_us = (latency_ns / 1000) as u32;
            self.rolling_latency_us[self.latency_head] = latency_us;
            self.latency_head = (self.latency_head + 1) % 16;
            if self.latency_count < 16 {
                self.latency_count += 1;
            }
        }
        self.last_present_timestamp_ns = timestamp_ns;

        // In Mailbox mode, unpresented frames waiting in pipeline are superseded
        if self.present_mode == DXGIPresentationMode::Mailbox && self.present_count > 0 {
            if sync_interval == 0 && (self.present_count % 7 == 0) {
                self.dropped_frames_count = self.dropped_frames_count.saturating_add(1);
            }
        }

        self.last_presented_index = self.current_backbuffer_index;
        self.current_backbuffer_index = (self.current_backbuffer_index + 1) % self.backbuffers.len();
        self.present_count = self.present_count.saturating_add(1);

        Ok(self.last_presented_index as u32)
    }

    /// Computes the rolling average present latency in microseconds.
    pub fn average_present_latency_us(&self) -> u32 {
        if self.latency_count == 0 {
            return 16_666;
        }
        let mut sum: u64 = 0;
        for i in 0..self.latency_count {
            sum += self.rolling_latency_us[i] as u64;
        }
        (sum / self.latency_count as u64) as u32
    }

    /// Resizes swapchain surface dimensions and reallocates the backbuffer pool.
    pub fn resize_buffers(
        &mut self,
        buffer_count: u32,
        width: u32,
        height: u32,
        format: DxgiFormat,
    ) -> Result<(), &'static str> {
        let count = if buffer_count == 0 { self.desc.buffer_count } else { buffer_count };
        if count < 2 || count > 8 {
            return Err("Invalid buffer count on resize");
        }
        if width == 0 || height == 0 {
            return Err("Invalid dimensions on resize");
        }

        self.desc.buffer_count = count;
        self.desc.width = width;
        self.desc.height = height;
        if format != DxgiFormat::Unknown {
            self.desc.format = format;
        }

        self.backbuffers.clear();
        for i in 0..count {
            self.backbuffers.push(BackbufferSurface::new(i, width, height));
        }
        self.current_backbuffer_index = 0;
        self.last_presented_index = 0;
        Ok(())
    }
}

/// Direct blit transferring swapchain backbuffer contents into a destination framebuffer/plane.
pub fn present_blit(
    source: &BackbufferSurface,
    target: &mut [Color],
    target_width: usize,
    target_height: usize,
    dst_x: usize,
    dst_y: usize,
    clip: Option<&Rect>,
) {
    if target_width == 0 || target_height == 0 || source.width == 0 || source.height == 0 {
        return;
    }
    let copy_w = (source.width as usize).min(target_width.saturating_sub(dst_x));
    let copy_h = (source.height as usize).min(target_height.saturating_sub(dst_y));

    for row in 0..copy_h {
        let ty = dst_y + row;
        if ty >= target_height {
            break;
        }
        let src_row_start = row * source.pitch_pixels;
        let dst_row_start = ty * target_width;

        for col in 0..copy_w {
            let tx = dst_x + col;
            if tx >= target_width {
                break;
            }
            if let Some(cl) = clip {
                if !cl.contains_point(tx, ty) {
                    continue;
                }
            }
            target[dst_row_start + tx] = source.pixels[src_row_start + col];
        }
    }
}

/// User-Mode Driver (UMD) graphics command streamer and hardware queue bridge.
#[derive(Debug)]
pub struct UmdCommandStreamer {
    pub stream_id: u16,
    pub recorder: CommandBufferRecorder,
    pub fence: GpuTimelineFence,
    pub next_fence_value: u64,
    pub last_submitted_fence_val: u64,
}

impl UmdCommandStreamer {
    /// Creates a new user-mode command streamer with dedicated fence synchronization.
    pub fn new(stream_id: u16, fence_id: u32, initial_capacity: usize) -> Self {
        Self {
            stream_id,
            recorder: CommandBufferRecorder::new(initial_capacity, stream_id),
            fence: GpuTimelineFence::new(fence_id, 0),
            next_fence_value: 1,
            last_submitted_fence_val: 0,
        }
    }

    /// Records a solid color clear directly to the backbuffer surface.
    pub fn record_clear_backbuffer(&self, backbuffer: &mut BackbufferSurface, color: Color) {
        backbuffer.clear(color);
    }

    /// Records an indexed draw command (`DrawIndexed`) into the GPU stream.
    pub fn record_draw_indexed(
        &mut self,
        index_count: u32,
        start_index_location: u32,
        base_vertex_location: i32,
    ) -> Result<(), &'static str> {
        let payload = DrawIndexedPayload::new(
            index_count,
            1,
            start_index_location,
            base_vertex_location,
            0,
        );
        self.recorder.record_draw_indexed(&payload)
    }

    /// Records a pipeline state change (`SetPipelineState`) into the GPU stream.
    pub fn record_set_pipeline_state(&mut self, pso_id: u64) -> Result<(), &'static str> {
        let payload = SetPipelineStatePayload::new(
            pso_id,
            PrimitiveTopology::TriangleList,
            true,
            true,
            true,
            2,
        );
        self.recorder.record_set_pipeline_state(&payload)
    }

    /// Records a resource barrier transition between rendering states.
    pub fn record_resource_barrier(
        &mut self,
        resource_gpu_address: u64,
        before: ResourceState,
        after: ResourceState,
    ) -> Result<(), &'static str> {
        self.recorder.record_transition_barrier(
            resource_gpu_address,
            0,
            before,
            after,
            PipelineStageFlags::ALL_COMMANDS,
            PipelineStageFlags::ALL_COMMANDS,
        )
    }

    /// Closes the recorded command stream and advances the synchronization timeline fence.
    pub fn submit(&mut self) -> Result<(GpuCommandBuffer, u64), &'static str> {
        let cmdbuf = self.recorder.close()?;
        let fence_val = self.next_fence_value;
        self.next_fence_value = self.next_fence_value.wrapping_add(1);

        self.fence.signal(fence_val)?;
        self.last_submitted_fence_val = fence_val;
        Ok((cmdbuf, fence_val))
    }

    /// Completes execution of the submitted fence value and resets the recorder for the next frame.
    pub fn complete_frame(&mut self, fence_val: u64) {
        self.fence.complete(fence_val);
        self.recorder.reset();
    }

    /// Returns whether the specified timeline fence value has finished execution.
    pub fn is_fence_completed(&self, fence_val: u64) -> bool {
        self.fence.is_completed(fence_val)
    }
}
