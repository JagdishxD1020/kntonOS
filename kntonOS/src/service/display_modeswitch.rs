//! SteamDeck External Monitor Hotplug & Multi-Resolution Display Modeswitching Manager (TASK-KNT-338).
//!
//! Provides USB-C DP Alt-mode and HDMI hotplug detection, EDID/DisplayID timing parsing,
//! seamless DRM atomic modesetting, dynamic color space/HDR10 selection, and VRR frame pacing.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

// ============================================================================
// Feature 1: Hotplug Event Listener & EDID Timing Block Parser
// ============================================================================

/// Display connector interface type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayConnectorType {
    /// Internal eDP 1.4b panel (1280x800 native).
    InternalEdp,
    /// USB-C DisplayPort 1.4 Alt-Mode (supporting HBR3 DSC).
    UsbCDpAltMode,
    /// External HDMI 2.0 / 2.1 via Dock or adapter.
    HdmiExternal,
}

/// Hotplug state event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotplugEvent {
    Connected,
    Disconnected,
    LinkTrained,
    HpdPulse,
}

/// Video mode timing specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisplayModeTiming {
    pub width: u32,
    pub height: u32,
    pub refresh_rate_hz: u32,
    pub pixel_clock_khz: u32,
    pub h_front_porch: u32,
    pub h_sync_width: u32,
    pub h_back_porch: u32,
    pub v_front_porch: u32,
    pub v_sync_width: u32,
    pub v_back_porch: u32,
    pub is_interlaced: bool,
    pub is_vrr_capable: bool,
}

impl DisplayModeTiming {
    pub const fn new_progressive(width: u32, height: u32, refresh_rate_hz: u32) -> Self {
        Self {
            width,
            height,
            refresh_rate_hz,
            pixel_clock_khz: (width * height * refresh_rate_hz) / 1000,
            h_front_porch: 48,
            h_sync_width: 32,
            h_back_porch: 80,
            v_front_porch: 3,
            v_sync_width: 5,
            v_back_porch: 15,
            is_interlaced: false,
            is_vrr_capable: false,
        }
    }
}

/// Parsed EDID monitor descriptor.
#[derive(Debug, Clone, Copy)]
pub struct EdidMonitorInfo {
    pub manufacturer_id: [u8; 3],
    pub product_code: u16,
    pub serial_number: u32,
    pub native_mode: DisplayModeTiming,
    pub max_refresh_hz: u32,
    pub min_refresh_hz: u32, // VRR range floor
    pub has_cta_extension: bool,
    pub supports_hdr10: bool,
    pub is_valid: bool,
}

impl EdidMonitorInfo {
    pub const fn empty() -> Self {
        Self {
            manufacturer_id: [0; 3],
            product_code: 0,
            serial_number: 0,
            native_mode: DisplayModeTiming::new_progressive(1280, 800, 60),
            max_refresh_hz: 60,
            min_refresh_hz: 60,
            has_cta_extension: false,
            supports_hdr10: false,
            is_valid: false,
        }
    }
}

/// 128-byte EDID block parser.
pub struct EdidParser;

impl EdidParser {
    pub const EDID_HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

    /// Validates and parses raw 128-byte EDID binary payload.
    pub fn parse(edid_bytes: &[u8]) -> Result<EdidMonitorInfo, &'static str> {
        if edid_bytes.len() < 128 {
            return Err("EDID payload under 128 bytes");
        }

        // 1. Verify EDID magic header
        let mut i = 0;
        while i < 8 {
            if edid_bytes[i] != Self::EDID_HEADER[i] {
                return Err("Invalid EDID header magic signature");
            }
            i += 1;
        }

        // 2. Verify checksum (sum of all 128 bytes % 256 == 0)
        let mut sum: u8 = 0;
        let mut idx = 0;
        while idx < 128 {
            sum = sum.wrapping_add(edid_bytes[idx]);
            idx += 1;
        }
        if sum != 0 {
            return Err("EDID checksum mismatch");
        }

        // 3. Extract Manufacturer ID (compressed 3x 5-bit ASCII at bytes 8-9)
        let mfg_raw = ((edid_bytes[8] as u16) << 8) | (edid_bytes[9] as u16);
        let c1 = (((mfg_raw >> 10) & 0x1F) as u8) + b'@';
        let c2 = (((mfg_raw >> 5) & 0x1F) as u8) + b'@';
        let c3 = ((mfg_raw & 0x1F) as u8) + b'@';

        let product_code = (edid_bytes[10] as u16) | ((edid_bytes[11] as u16) << 8);
        let serial = (edid_bytes[12] as u32)
            | ((edid_bytes[13] as u32) << 8)
            | ((edid_bytes[14] as u32) << 16)
            | ((edid_bytes[15] as u32) << 24);

        // 4. Parse Detailed Timing Descriptor (DTD) at offset 54 (byte 54-71)
        // Pixel clock in 10 kHz units
        let pixel_clock_10khz = (edid_bytes[54] as u32) | ((edid_bytes[55] as u32) << 8);
        let pixel_clock_khz = pixel_clock_10khz * 10;

        let h_active = (edid_bytes[56] as u32) | (((edid_bytes[58] as u32) >> 4) << 8);
        let v_active = (edid_bytes[59] as u32) | (((edid_bytes[61] as u32) >> 4) << 8);

        let h_blank = (edid_bytes[57] as u32) | (((edid_bytes[58] as u32) & 0x0F) << 8);
        let v_blank = (edid_bytes[60] as u32) | (((edid_bytes[61] as u32) & 0x0F) << 8);

        let h_total = h_active + h_blank;
        let v_total = v_active + v_blank;

        let refresh_hz = if h_total > 0 && v_total > 0 && pixel_clock_khz > 0 {
            let total_pixels = h_total * v_total;
            ((pixel_clock_khz as u64 * 1000) / total_pixels as u64) as u32
        } else {
            60
        };

        let extension_count = edid_bytes[126];
        let has_cta = extension_count > 0;

        let native_mode = DisplayModeTiming {
            width: if h_active > 0 { h_active } else { 1280 },
            height: if v_active > 0 { v_active } else { 800 },
            refresh_rate_hz: refresh_hz,
            pixel_clock_khz,
            h_front_porch: 48,
            h_sync_width: 32,
            h_back_porch: 80,
            v_front_porch: 3,
            v_sync_width: 5,
            v_back_porch: 15,
            is_interlaced: false,
            is_vrr_capable: refresh_hz >= 120 || has_cta,
        };

        Ok(EdidMonitorInfo {
            manufacturer_id: [c1, c2, c3],
            product_code,
            serial_number: serial,
            native_mode,
            max_refresh_hz: refresh_hz.max(60),
            min_refresh_hz: if native_mode.is_vrr_capable { 40 } else { refresh_hz },
            has_cta_extension: has_cta,
            supports_hdr10: has_cta,
            is_valid: true,
        })
    }
}

/// External display hotplug listener.
pub struct HotplugListener {
    is_external_connected: AtomicBool,
    active_connector: AtomicU32, // DisplayConnectorType as u32
    hotplug_counter: AtomicU32,
    last_event_ts: AtomicU64,
}

impl HotplugListener {
    pub const fn new() -> Self {
        Self {
            is_external_connected: AtomicBool::new(false),
            active_connector: AtomicU32::new(DisplayConnectorType::InternalEdp as u32),
            hotplug_counter: AtomicU32::new(0),
            last_event_ts: AtomicU64::new(0),
        }
    }

    /// Handles an incoming HPD interrupt from USB-C or HDMI port.
    pub fn handle_hpd(&self, connector: DisplayConnectorType, event: HotplugEvent, timestamp: u64) {
        match event {
            HotplugEvent::Connected | HotplugEvent::LinkTrained => {
                self.is_external_connected.store(true, Ordering::Release);
                self.active_connector.store(connector as u32, Ordering::Release);
            }
            HotplugEvent::Disconnected => {
                self.is_external_connected.store(false, Ordering::Release);
                self.active_connector.store(DisplayConnectorType::InternalEdp as u32, Ordering::Release);
            }
            HotplugEvent::HpdPulse => {}
        }
        self.hotplug_counter.fetch_add(1, Ordering::Relaxed);
        self.last_event_ts.store(timestamp, Ordering::Release);
    }

    pub fn is_external_active(&self) -> bool {
        self.is_external_connected.load(Ordering::Acquire)
    }

    pub fn get_active_connector(&self) -> DisplayConnectorType {
        match self.active_connector.load(Ordering::Acquire) {
            1 => DisplayConnectorType::UsbCDpAltMode,
            2 => DisplayConnectorType::HdmiExternal,
            _ => DisplayConnectorType::InternalEdp,
        }
    }
}

pub static HOTPLUG_LISTENER: HotplugListener = HotplugListener::new();


// ============================================================================
// Feature 2: Seamless DRM Atomic Modesetting & Resolution Scaler
// ============================================================================

/// Display topology layout mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayTopologyMode {
    /// Internal eDP panel only (handheld mode).
    InternalOnly,
    /// External display is primary (docked console mode, internal panel blanked).
    ExternalPrimary,
    /// Mirror internal screen to external monitor.
    ClonedMirror,
    /// Extended multi-monitor workspace.
    DualExtended,
}

/// Scaling filter used when game resolution differs from physical panel resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayScalingFilter {
    /// Integer pixel doubling (sharp retro / pixel art).
    NearestInteger,
    /// Smooth bilinear filtering.
    Bilinear,
    /// AMD FidelityFX Super Resolution edge-preserving upscaler.
    FsrUpscale,
    /// Aspect ratio preservation with letterboxing/pillarboxing.
    FitPreserveAspect,
}

/// DRM atomic commit flags.
pub const DRM_MODE_ATOMIC_TEST_ONLY: u32 = 0x01;
pub const DRM_MODE_ATOMIC_NONBLOCK: u32 = 0x02;
pub const DRM_MODE_ATOMIC_ALLOW_MODESET: u32 = 0x04;

/// DRM atomic modesetting commit state descriptor.
#[derive(Debug, Clone, Copy)]
pub struct DrmAtomicCommitState {
    pub crtc_id: u32,
    pub connector_id: u32,
    pub plane_id: u32,
    pub active_timing: DisplayModeTiming,
    pub scaling_filter: DisplayScalingFilter,
    pub is_enabled: bool,
}

/// DRM atomic modeswitch manager.
pub struct DrmModesetManager {
    topology: AtomicU32, // DisplayTopologyMode as u32
    internal_w: AtomicU32,
    internal_h: AtomicU32,
    internal_hz: AtomicU32,
    external_w: AtomicU32,
    external_h: AtomicU32,
    external_hz: AtomicU32,
    scaling_filter: AtomicU32, // DisplayScalingFilter as u32
    modeset_sequence_id: AtomicU64,
    is_switching: AtomicBool,
}

impl DrmModesetManager {
    pub const fn new() -> Self {
        Self {
            topology: AtomicU32::new(DisplayTopologyMode::InternalOnly as u32),
            internal_w: AtomicU32::new(1280),
            internal_h: AtomicU32::new(800),
            internal_hz: AtomicU32::new(60),
            external_w: AtomicU32::new(1920),
            external_h: AtomicU32::new(1080),
            external_hz: AtomicU32::new(60),
            scaling_filter: AtomicU32::new(DisplayScalingFilter::FsrUpscale as u32),
            modeset_sequence_id: AtomicU64::new(0),
            is_switching: AtomicBool::new(false),
        }
    }

    /// Tests whether the proposed resolution and pixel clock are supported by AMD Van Gogh DCN.
    pub fn test_modeset(&self, timing: &DisplayModeTiming) -> bool {
        // Max pixel clock on SteamDeck DCN 3.0.1 is ~1.2 GHz (supports 4K 120Hz via DSC)
        if timing.pixel_clock_khz > 1_200_000 {
            return false;
        }
        // Maximum resolution bounds
        if timing.width > 3840 || timing.height > 2160 || timing.width < 640 || timing.height < 480 {
            return false;
        }
        // Maximum refresh rate bounds
        if timing.refresh_rate_hz < 30 || timing.refresh_rate_hz > 240 {
            return false;
        }
        true
    }

    /// Performs a seamless atomic modeset transition to target display topology and timing.
    pub fn commit_atomic_modeset(
        &self,
        target_topology: DisplayTopologyMode,
        target_timing: DisplayModeTiming,
        flags: u32,
    ) -> Result<u64, &'static str> {
        if !self.test_modeset(&target_timing) {
            return Err("Target mode exceeds display engine capabilities");
        }

        if (flags & DRM_MODE_ATOMIC_TEST_ONLY) != 0 {
            return Ok(0); // Test passed
        }

        self.is_switching.store(true, Ordering::Release);

        match target_topology {
            DisplayTopologyMode::InternalOnly => {
                self.internal_w.store(target_timing.width, Ordering::Release);
                self.internal_h.store(target_timing.height, Ordering::Release);
                self.internal_hz.store(target_timing.refresh_rate_hz, Ordering::Release);
            }
            DisplayTopologyMode::ExternalPrimary | DisplayTopologyMode::ClonedMirror | DisplayTopologyMode::DualExtended => {
                self.external_w.store(target_timing.width, Ordering::Release);
                self.external_h.store(target_timing.height, Ordering::Release);
                self.external_hz.store(target_timing.refresh_rate_hz, Ordering::Release);
            }
        }

        self.topology.store(target_topology as u32, Ordering::Release);
        let seq = self.modeset_sequence_id.fetch_add(1, Ordering::AcqRel) + 1;
        self.is_switching.store(false, Ordering::Release);
        Ok(seq)
    }

    pub fn get_topology(&self) -> DisplayTopologyMode {
        match self.topology.load(Ordering::Acquire) {
            1 => DisplayTopologyMode::ExternalPrimary,
            2 => DisplayTopologyMode::ClonedMirror,
            3 => DisplayTopologyMode::DualExtended,
            _ => DisplayTopologyMode::InternalOnly,
        }
    }

    pub fn get_active_timing(&self, connector: DisplayConnectorType) -> DisplayModeTiming {
        match connector {
            DisplayConnectorType::InternalEdp => DisplayModeTiming::new_progressive(
                self.internal_w.load(Ordering::Acquire),
                self.internal_h.load(Ordering::Acquire),
                self.internal_hz.load(Ordering::Acquire),
            ),
            _ => DisplayModeTiming::new_progressive(
                self.external_w.load(Ordering::Acquire),
                self.external_h.load(Ordering::Acquire),
                self.external_hz.load(Ordering::Acquire),
            ),
        }
    }

    pub fn set_scaling_filter(&self, filter: DisplayScalingFilter) {
        self.scaling_filter.store(filter as u32, Ordering::Release);
    }

    pub fn get_scaling_filter(&self) -> DisplayScalingFilter {
        match self.scaling_filter.load(Ordering::Acquire) {
            0 => DisplayScalingFilter::NearestInteger,
            1 => DisplayScalingFilter::Bilinear,
            2 => DisplayScalingFilter::FsrUpscale,
            _ => DisplayScalingFilter::FitPreserveAspect,
        }
    }
}

pub static MODESET_MANAGER: DrmModesetManager = DrmModesetManager::new();
