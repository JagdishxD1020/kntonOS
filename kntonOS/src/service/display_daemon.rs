//! Userspace Display Compositor & Direct Scanout Manager (knton.svc.display).
//!
//! Provides KMS-style surface leases, zero-latency direct scanout bypass for full-screen
//! games, hardware overlay plane allocation, dynamic refresh rate switching (DRRS),
//! and VSync synchronization telemetry.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const MAX_SURFACES: usize = 16;
pub const DISPLAY_DEFAULT_WIDTH: u32 = 1280;
pub const DISPLAY_DEFAULT_HEIGHT: u32 = 800;

/// Display subsystem error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayError {
    SurfaceNotFound,
    SurfaceAlreadyExists,
    PermissionDenied,
    LeaseUnavailable,
    InvalidParameters,
    TableFull,
    UnsupportedFormat,
    ScanoutBusy,
}

/// Pixel format classification for framebuffer surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceFormat {
    Bgra8Unorm,
    Rgba8Unorm,
    Rgb10A2Unorm,
    Fp16,
}

/// Unique handle identifying a registered display surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SurfaceId(pub u32);

/// Framebuffer surface descriptor managed by the compositor.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceDescriptor {
    pub surface_id: SurfaceId,
    pub owner_pid: u32,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub format: SurfaceFormat,
    pub buffer_address: u64,
    pub is_fullscreen: bool,
    pub is_leased: bool,
}

impl SurfaceDescriptor {
    pub const fn empty() -> Self {
        Self {
            surface_id: SurfaceId(0),
            owner_pid: 0,
            width: 0,
            height: 0,
            stride_bytes: 0,
            format: SurfaceFormat::Bgra8Unorm,
            buffer_address: 0,
            is_fullscreen: false,
            is_leased: false,
        }
    }
}

/// Surface lease state representing exclusive scanout access granted to a client.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceLeaseState {
    pub lease_id: u32,
    pub surface_id: SurfaceId,
    pub grantee_pid: u32,
    pub is_active: bool,
    pub bypass_enabled: bool,
}

impl SurfaceLeaseState {
    pub const fn empty() -> Self {
        Self {
            lease_id: 0,
            surface_id: SurfaceId(0),
            grantee_pid: 0,
            is_active: false,
            bypass_enabled: false,
        }
    }
}

struct LeaseManagerInner {
    surfaces: [SurfaceDescriptor; MAX_SURFACES],
    surface_count: usize,
    active_lease: SurfaceLeaseState,
    next_lease_id: u32,
}

/// Surface lease manager coordinating exclusive framebuffer scanout leases.
pub struct SurfaceLeaseManager {
    inner: UnsafeCell<LeaseManagerInner>,
    lock: AtomicBool,
}

unsafe impl Sync for SurfaceLeaseManager {}
unsafe impl Send for SurfaceLeaseManager {}

impl SurfaceLeaseManager {
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(LeaseManagerInner {
                surfaces: [const { SurfaceDescriptor::empty() }; MAX_SURFACES],
                surface_count: 0,
                active_lease: SurfaceLeaseState::empty(),
                next_lease_id: 1,
            }),
            lock: AtomicBool::new(false),
        }
    }

    fn acquire(&self) {
        while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
    }

    fn release(&self) {
        self.lock.store(false, Ordering::Release);
    }

    /// Registers a new userland or compositor surface.
    pub fn register_surface(&self, desc: SurfaceDescriptor) -> Result<(), DisplayError> {
        if desc.surface_id.0 == 0 || desc.width == 0 || desc.height == 0 || desc.buffer_address == 0 {
            return Err(DisplayError::InvalidParameters);
        }

        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };

        for i in 0..MAX_SURFACES {
            if inner.surfaces[i].surface_id == desc.surface_id {
                self.release();
                return Err(DisplayError::SurfaceAlreadyExists);
            }
        }

        for i in 0..MAX_SURFACES {
            if inner.surfaces[i].surface_id.0 == 0 {
                inner.surfaces[i] = desc;
                inner.surface_count += 1;
                self.release();
                return Ok(());
            }
        }

        self.release();
        Err(DisplayError::TableFull)
    }

    /// Requests an exclusive surface lease for direct scanout bypass.
    pub fn request_lease(&self, surface_id: SurfaceId, caller_pid: u32) -> Result<u32, DisplayError> {
        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };

        if inner.active_lease.is_active {
            self.release();
            return Err(DisplayError::LeaseUnavailable);
        }

        let mut found_idx = None;
        for i in 0..MAX_SURFACES {
            if inner.surfaces[i].surface_id == surface_id {
                if inner.surfaces[i].owner_pid != caller_pid {
                    self.release();
                    return Err(DisplayError::PermissionDenied);
                }
                found_idx = Some(i);
                break;
            }
        }

        let idx = match found_idx {
            Some(i) => i,
            None => {
                self.release();
                return Err(DisplayError::SurfaceNotFound);
            }
        };

        let lease_id = inner.next_lease_id;
        inner.next_lease_id = inner.next_lease_id.wrapping_add(1).max(1);

        inner.surfaces[idx].is_leased = true;
        inner.active_lease = SurfaceLeaseState {
            lease_id,
            surface_id,
            grantee_pid: caller_pid,
            is_active: true,
            bypass_enabled: inner.surfaces[idx].is_fullscreen,
        };

        self.release();
        Ok(lease_id)
    }

    /// Releases an existing scanout lease.
    pub fn release_lease(&self, lease_id: u32, caller_pid: u32) -> Result<(), DisplayError> {
        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };

        if !inner.active_lease.is_active || inner.active_lease.lease_id != lease_id {
            self.release();
            return Err(DisplayError::LeaseUnavailable);
        }

        if inner.active_lease.grantee_pid != caller_pid {
            self.release();
            return Err(DisplayError::PermissionDenied);
        }

        let surface_id = inner.active_lease.surface_id;
        for i in 0..MAX_SURFACES {
            if inner.surfaces[i].surface_id == surface_id {
                inner.surfaces[i].is_leased = false;
                break;
            }
        }

        inner.active_lease = SurfaceLeaseState::empty();
        self.release();
        Ok(())
    }

    /// Revokes active lease unconditionally (e.g. on shell quick menu overlay trigger).
    pub fn revoke_lease(&self) -> bool {
        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };

        if !inner.active_lease.is_active {
            self.release();
            return false;
        }

        let surface_id = inner.active_lease.surface_id;
        for i in 0..MAX_SURFACES {
            if inner.surfaces[i].surface_id == surface_id {
                inner.surfaces[i].is_leased = false;
                break;
            }
        }

        inner.active_lease = SurfaceLeaseState::empty();
        self.release();
        true
    }

    /// Returns current lease state snapshot.
    pub fn get_active_lease(&self) -> SurfaceLeaseState {
        self.acquire();
        let inner = unsafe { &*self.inner.get() };
        let state = inner.active_lease;
        self.release();
        state
    }
}

/// Direct KMS-style scanout bypass controller for zero-latency presentation.
pub struct ScanoutBypassController {
    direct_scanout_active: AtomicBool,
    current_scanout_addr: AtomicU64,
    page_flip_counter: AtomicU64,
}

impl ScanoutBypassController {
    pub const fn new() -> Self {
        Self {
            direct_scanout_active: AtomicBool::new(false),
            current_scanout_addr: AtomicU64::new(0),
            page_flip_counter: AtomicU64::new(0),
        }
    }

    /// Checks if a leased surface is eligible for direct KMS scanout bypass.
    pub fn can_bypass(&self, lease: &SurfaceLeaseState) -> bool {
        lease.is_active && lease.bypass_enabled
    }

    /// Activates hardware scanout bypass for full-screen game surface.
    pub fn activate_bypass(&self, surface: &SurfaceDescriptor) -> Result<(), DisplayError> {
        if !surface.is_fullscreen || surface.buffer_address == 0 {
            return Err(DisplayError::InvalidParameters);
        }
        self.current_scanout_addr.store(surface.buffer_address, Ordering::SeqCst);
        self.direct_scanout_active.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Deactivates bypass, falling back to full compositor multi-plane blending.
    pub fn deactivate_bypass(&self) {
        self.direct_scanout_active.store(false, Ordering::SeqCst);
    }

    /// Performs an atomic direct scanout page flip without compositor intervention.
    pub fn queue_direct_flip(&self, buffer_address: u64) -> Result<u64, DisplayError> {
        if !self.direct_scanout_active.load(Ordering::Acquire) {
            return Err(DisplayError::ScanoutBusy);
        }
        if buffer_address == 0 {
            return Err(DisplayError::InvalidParameters);
        }

        self.current_scanout_addr.store(buffer_address, Ordering::SeqCst);
        let flip_seq = self.page_flip_counter.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(flip_seq)
    }

    pub fn is_bypass_active(&self) -> bool {
        self.direct_scanout_active.load(Ordering::Acquire)
    }

    pub fn current_scanout_address(&self) -> u64 {
        self.current_scanout_addr.load(Ordering::Acquire)
    }
}

pub static SURFACE_LEASE_MGR: SurfaceLeaseManager = SurfaceLeaseManager::new();
pub static SCANOUT_BYPASS: ScanoutBypassController = ScanoutBypassController::new();

pub const MAX_OVERLAY_PLANES: usize = 3;

/// Hardware overlay plane classification according to z-ordering and exclusivity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayPlaneType {
    PrimaryGame = 0,
    SystemDialog = 1,
    HudOverlay = 2,
}

/// Hardware overlay plane configuration descriptor.
#[derive(Debug, Clone, Copy)]
pub struct OverlayPlaneDescriptor {
    pub plane_type: OverlayPlaneType,
    pub plane_id: u8,
    pub is_enabled: bool,
    pub surface_id: SurfaceId,
    pub viewport_x: i32,
    pub viewport_y: i32,
    pub viewport_w: u32,
    pub viewport_h: u32,
    pub alpha_pct: u8,
    pub z_order: u8,
}

impl OverlayPlaneDescriptor {
    pub const fn new(plane_type: OverlayPlaneType) -> Self {
        let z = plane_type as u8;
        Self {
            plane_type,
            plane_id: z,
            is_enabled: false,
            surface_id: SurfaceId(0),
            viewport_x: 0,
            viewport_y: 0,
            viewport_w: DISPLAY_DEFAULT_WIDTH,
            viewport_h: DISPLAY_DEFAULT_HEIGHT,
            alpha_pct: 100,
            z_order: z,
        }
    }
}

/// Composition summary representing the combined multi-plane scanout state.
#[derive(Debug, Clone, Copy)]
pub struct LayerCompositionSummary {
    pub active_plane_mask: u8,
    pub top_layer_z: u8,
    pub total_active: u8,
    pub requires_blending: bool,
}

struct PlaneAllocatorInner {
    planes: [OverlayPlaneDescriptor; MAX_OVERLAY_PLANES],
}

/// Hardware overlay plane allocator managing dedicated scanout layers.
pub struct HardwareOverlayPlaneAllocator {
    inner: UnsafeCell<PlaneAllocatorInner>,
    lock: AtomicBool,
}

unsafe impl Sync for HardwareOverlayPlaneAllocator {}
unsafe impl Send for HardwareOverlayPlaneAllocator {}

impl HardwareOverlayPlaneAllocator {
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(PlaneAllocatorInner {
                planes: [
                    OverlayPlaneDescriptor::new(OverlayPlaneType::PrimaryGame),
                    OverlayPlaneDescriptor::new(OverlayPlaneType::SystemDialog),
                    OverlayPlaneDescriptor::new(OverlayPlaneType::HudOverlay),
                ],
            }),
            lock: AtomicBool::new(false),
        }
    }

    fn acquire(&self) {
        while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
    }

    fn release(&self) {
        self.lock.store(false, Ordering::Release);
    }

    /// Allocates or attaches a surface to the designated hardware overlay plane.
    pub fn allocate_plane(&self, plane_type: OverlayPlaneType, surface_id: SurfaceId) -> Result<u8, DisplayError> {
        if surface_id.0 == 0 {
            return Err(DisplayError::InvalidParameters);
        }

        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };
        let idx = plane_type as usize;

        inner.planes[idx].surface_id = surface_id;
        inner.planes[idx].is_enabled = true;
        let id = inner.planes[idx].plane_id;

        self.release();
        Ok(id)
    }

    /// Configures plane spatial positioning and transparency.
    pub fn configure_plane(&self, plane_type: OverlayPlaneType, x: i32, y: i32, w: u32, h: u32, alpha: u8) -> Result<(), DisplayError> {
        if w == 0 || h == 0 {
            return Err(DisplayError::InvalidParameters);
        }

        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };
        let idx = plane_type as usize;

        inner.planes[idx].viewport_x = x;
        inner.planes[idx].viewport_y = y;
        inner.planes[idx].viewport_w = w;
        inner.planes[idx].viewport_h = h;
        inner.planes[idx].alpha_pct = alpha.min(100);

        self.release();
        Ok(())
    }

    /// Disables scanout for the specified overlay plane.
    pub fn disable_plane(&self, plane_type: OverlayPlaneType) -> Result<(), DisplayError> {
        self.acquire();
        let inner = unsafe { &mut *self.inner.get() };
        let idx = plane_type as usize;

        inner.planes[idx].is_enabled = false;
        inner.planes[idx].surface_id = SurfaceId(0);

        self.release();
        Ok(())
    }

    /// Queries the descriptor for a plane.
    pub fn get_plane(&self, plane_type: OverlayPlaneType) -> OverlayPlaneDescriptor {
        self.acquire();
        let inner = unsafe { &*self.inner.get() };
        let plane = inner.planes[plane_type as usize];
        self.release();
        plane
    }

    /// Evaluates multi-layer composition state across all planes.
    pub fn compose_layers(&self) -> LayerCompositionSummary {
        self.acquire();
        let inner = unsafe { &*self.inner.get() };

        let mut mask = 0u8;
        let mut top_z = 0u8;
        let mut count = 0u8;
        let mut blending = false;

        for i in 0..MAX_OVERLAY_PLANES {
            if inner.planes[i].is_enabled {
                mask |= 1 << i;
                top_z = inner.planes[i].z_order;
                count += 1;
                if inner.planes[i].alpha_pct < 100 {
                    blending = true;
                }
            }
        }

        self.release();
        LayerCompositionSummary {
            active_plane_mask: mask,
            top_layer_z: top_z,
            total_active: count,
            requires_blending: blending || count > 1,
        }
    }
}

pub static OVERLAY_ALLOCATOR: HardwareOverlayPlaneAllocator = HardwareOverlayPlaneAllocator::new();
