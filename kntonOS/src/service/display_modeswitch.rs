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


// ============================================================================
// Feature 3: Dynamic Color Space Selection & HDR10 Metadata Routing
// ============================================================================

/// Display color gamut and dynamic range standard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    /// Standard sRGB gamut (standard PC SDR).
    SrgbStandard,
    /// ITU-R BT.709 HDTV color space.
    Rec709Bt709,
    /// DCI-P3 cinema wide color gamut.
    DciP3WideColor,
    /// ITU-R BT.2020 Wide Color Gamut with HDR10 (ST 2084 PQ).
    Rec2020Hdr10,
}

/// Color bit depth per channel (bpc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    Bpc8 = 8,
    Bpc10 = 10,
    Bpc12 = 12,
}

/// CTA-861.3 HDR10 static mastering metadata descriptor.
#[derive(Debug, Clone, Copy)]
pub struct Hdr10StaticMetadata {
    /// Mastering display maximum luminance in nits (cd/m^2).
    pub max_luminance_nits: u32,
    /// Mastering display minimum luminance in milli-nits (1000 = 1.0 nit).
    pub min_luminance_milli_nits: u32,
    /// Maximum Content Light Level (MaxCLL) in nits.
    pub max_cll_nits: u32,
    /// Maximum Frame-Average Light Level (MaxFALL) in nits.
    pub max_fall_nits: u32,
    /// Whether HDR10 PQ transfer function is currently engaged in display controller.
    pub is_hdr_active: bool,
}

impl Hdr10StaticMetadata {
    pub const STEAMDECK_OLED_HDR: Self = Self {
        max_luminance_nits: 1000,
        min_luminance_milli_nits: 1, // 0.001 nit true OLED black
        max_cll_nits: 1000,
        max_fall_nits: 400,
        is_hdr_active: true,
    };

    pub const SDR_DEFAULT: Self = Self {
        max_luminance_nits: 400,
        min_luminance_milli_nits: 200,
        max_cll_nits: 400,
        max_fall_nits: 200,
        is_hdr_active: false,
    };
}

/// Dynamic color space and HDR10 metadata routing manager.
pub struct ColorSpaceManager {
    active_colorspace: AtomicU32, // ColorSpace as u32
    active_bpc: AtomicU32,        // ColorDepth as u32
    hdr_active: AtomicBool,
    max_lum_nits: AtomicU32,
}

impl ColorSpaceManager {
    pub const fn new() -> Self {
        Self {
            active_colorspace: AtomicU32::new(ColorSpace::SrgbStandard as u32),
            active_bpc: AtomicU32::new(ColorDepth::Bpc8 as u32),
            hdr_active: AtomicBool::new(false),
            max_lum_nits: AtomicU32::new(400),
        }
    }

    /// Selects optimal color space and depth based on game request and monitor EDID capability.
    pub fn select_color_space(
        &self,
        requested_space: ColorSpace,
        monitor: &EdidMonitorInfo,
    ) -> (ColorSpace, ColorDepth) {
        let (resolved_space, bpc, is_hdr) = match requested_space {
            ColorSpace::Rec2020Hdr10 => {
                if monitor.supports_hdr10 {
                    (ColorSpace::Rec2020Hdr10, ColorDepth::Bpc10, true)
                } else {
                    // Fall back to sRGB if monitor does not support HDR10
                    (ColorSpace::SrgbStandard, ColorDepth::Bpc8, false)
                }
            }
            ColorSpace::DciP3WideColor => (ColorSpace::DciP3WideColor, ColorDepth::Bpc10, false),
            ColorSpace::Rec709Bt709 => (ColorSpace::Rec709Bt709, ColorDepth::Bpc8, false),
            ColorSpace::SrgbStandard => (ColorSpace::SrgbStandard, ColorDepth::Bpc8, false),
        };

        self.active_colorspace.store(resolved_space as u32, Ordering::Release);
        self.active_bpc.store(bpc as u32, Ordering::Release);
        self.hdr_active.store(is_hdr, Ordering::Release);

        if is_hdr {
            self.max_lum_nits.store(1000, Ordering::Release);
        } else {
            self.max_lum_nits.store(400, Ordering::Release);
        }

        (resolved_space, bpc)
    }

    pub fn is_hdr_active(&self) -> bool {
        self.hdr_active.load(Ordering::Acquire)
    }

    pub fn get_active_colorspace(&self) -> ColorSpace {
        match self.active_colorspace.load(Ordering::Acquire) {
            1 => ColorSpace::Rec709Bt709,
            2 => ColorSpace::DciP3WideColor,
            3 => ColorSpace::Rec2020Hdr10,
            _ => ColorSpace::SrgbStandard,
        }
    }

    pub fn get_active_bpc(&self) -> ColorDepth {
        match self.active_bpc.load(Ordering::Acquire) {
            10 => ColorDepth::Bpc10,
            12 => ColorDepth::Bpc12,
            _ => ColorDepth::Bpc8,
        }
    }

    pub fn get_hdr_metadata(&self) -> Hdr10StaticMetadata {
        if self.is_hdr_active() {
            Hdr10StaticMetadata {
                max_luminance_nits: self.max_lum_nits.load(Ordering::Acquire),
                min_luminance_milli_nits: 1,
                max_cll_nits: 1000,
                max_fall_nits: 400,
                is_hdr_active: true,
            }
        } else {
            Hdr10StaticMetadata::SDR_DEFAULT
        }
    }
}

pub static COLOR_MANAGER: ColorSpaceManager = ColorSpaceManager::new();


// ============================================================================
// Feature 4: Variable Refresh Rate (VRR) & Display Clock Pacing Governor
// ============================================================================

/// Variable Refresh Rate (VRR) operational state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VrrStatus {
    /// VRR disabled (fixed refresh rate).
    Disabled,
    /// VRR actively tracking game frame rate within dynamic window.
    ActiveTracking,
    /// Game FPS below VRR window; Low Frame-Rate Compensation (LFC) active.
    LfcEngaged,
}

/// Variable refresh rate governor and frame-time pacing supervisor.
pub struct VrrPacingGovernor {
    vrr_enabled: AtomicBool,
    vrr_min_hz: AtomicU32,
    vrr_max_hz: AtomicU32,
    target_refresh_cap_hz: AtomicU32,
    effective_refresh_hz: AtomicU32,
    lfc_multiplier: AtomicU32,
    status: AtomicU32, // VrrStatus as u32
}

impl VrrPacingGovernor {
    pub const fn new() -> Self {
        Self {
            vrr_enabled: AtomicBool::new(false),
            vrr_min_hz: AtomicU32::new(40),
            vrr_max_hz: AtomicU32::new(120),
            target_refresh_cap_hz: AtomicU32::new(60),
            effective_refresh_hz: AtomicU32::new(60),
            lfc_multiplier: AtomicU32::new(1),
            status: AtomicU32::new(VrrStatus::Disabled as u32),
        }
    }

    /// Configures VRR capabilities from monitor EDID.
    pub fn configure_monitor_vrr(&self, monitor: &EdidMonitorInfo) {
        if monitor.native_mode.is_vrr_capable {
            self.vrr_min_hz.store(monitor.min_refresh_hz, Ordering::Release);
            self.vrr_max_hz.store(monitor.max_refresh_hz, Ordering::Release);
            self.vrr_enabled.store(true, Ordering::Release);
        } else {
            self.vrr_enabled.store(false, Ordering::Release);
        }
    }

    pub fn set_vrr_range(&self, min_hz: u32, max_hz: u32) {
        self.vrr_min_hz.store(min_hz, Ordering::Release);
        self.vrr_max_hz.store(max_hz, Ordering::Release);
        self.vrr_enabled.store(true, Ordering::Release);
    }

    pub fn set_vrr_enabled(&self, enabled: bool) {
        self.vrr_enabled.store(enabled, Ordering::Release);
    }

    pub fn set_target_refresh_cap(&self, cap_hz: u32) {
        self.target_refresh_cap_hz.store(cap_hz, Ordering::Release);
    }

    /// Evaluates game render FPS and paces display refresh timing with LFC.
    pub fn pace_frame(&self, current_fps: u32) -> VrrStatus {
        if !self.vrr_enabled.load(Ordering::Acquire) {
            let cap = self.target_refresh_cap_hz.load(Ordering::Acquire);
            self.effective_refresh_hz.store(cap, Ordering::Release);
            self.lfc_multiplier.store(1, Ordering::Release);
            self.status.store(VrrStatus::Disabled as u32, Ordering::Release);
            return VrrStatus::Disabled;
        }

        let min_hz = self.vrr_min_hz.load(Ordering::Acquire);
        let max_hz = self.vrr_max_hz.load(Ordering::Acquire);
        let cap_hz = self.target_refresh_cap_hz.load(Ordering::Acquire);
        let upper_limit = max_hz.min(cap_hz);

        if current_fps >= min_hz {
            // Direct VRR window: match display refresh to game FPS
            let paced_hz = current_fps.min(upper_limit);
            self.effective_refresh_hz.store(paced_hz, Ordering::Release);
            self.lfc_multiplier.store(1, Ordering::Release);
            self.status.store(VrrStatus::ActiveTracking as u32, Ordering::Release);
            VrrStatus::ActiveTracking
        } else {
            // Below VRR floor -> Engage Low Frame-Rate Compensation (LFC)
            // Multiplier doubles or triples frames to fit inside [min_hz..max_hz]
            let mult = if current_fps > 0 {
                let m = (min_hz + current_fps - 1) / current_fps;
                m.max(2)
            } else {
                2
            };
            let lfc_hz = (current_fps * mult).min(upper_limit).max(min_hz);

            self.effective_refresh_hz.store(lfc_hz, Ordering::Release);
            self.lfc_multiplier.store(mult, Ordering::Release);
            self.status.store(VrrStatus::LfcEngaged as u32, Ordering::Release);
            VrrStatus::LfcEngaged
        }
    }

    pub fn get_effective_refresh_hz(&self) -> u32 {
        self.effective_refresh_hz.load(Ordering::Acquire)
    }

    pub fn get_lfc_multiplier(&self) -> u32 {
        self.lfc_multiplier.load(Ordering::Acquire)
    }

    pub fn get_status(&self) -> VrrStatus {
        match self.status.load(Ordering::Acquire) {
            1 => VrrStatus::ActiveTracking,
            2 => VrrStatus::LfcEngaged,
            _ => VrrStatus::Disabled,
        }
    }

    pub fn is_vrr_active(&self) -> bool {
        self.get_status() != VrrStatus::Disabled
    }
}

pub static VRR_GOVERNOR: VrrPacingGovernor = VrrPacingGovernor::new();


// ============================================================================
// Verification Self-Test Suite & Subsystem Sentinel
// ============================================================================

/// Comprehensive verification self-test for SteamDeck External Monitor Hotplug & Display Modeswitching.
pub fn verify_display_modeswitch_subsystem() -> bool {
    // 1. EDID Parsing & Hotplug Event Listener
    let mut edid = [0u8; 128];
    edid[0..8].copy_from_slice(&EdidParser::EDID_HEADER);
    // Manufacturer ID: "SAM" (0x4C2D)
    edid[8] = 0x4C;
    edid[9] = 0x2D;
    edid[10] = 0x01;
    edid[11] = 0x04;
    edid[12] = 0x78;
    edid[13] = 0x56;
    edid[14] = 0x34;
    edid[15] = 0x12;

    // DTD: 1920x1080 @ 60Hz (148.5 MHz pixel clock)
    edid[54] = 0x02;
    edid[55] = 0x3A;
    edid[56] = 0x80;
    edid[57] = 0x18;
    edid[58] = 0x71;
    edid[59] = 0x38;
    edid[60] = 0x2D;
    edid[61] = 0x40;

    // CTA-861 Extension present (HDR10 capable)
    edid[126] = 1;

    // Compute checksum
    let mut sum: u8 = 0;
    let mut i = 0;
    while i < 127 {
        sum = sum.wrapping_add(edid[i]);
        i += 1;
    }
    edid[127] = (256u16.wrapping_sub(sum as u16) % 256) as u8;

    let monitor_info = match EdidParser::parse(&edid) {
        Ok(info) => info,
        Err(_) => return false,
    };

    if !monitor_info.is_valid || monitor_info.manufacturer_id != *b"SAM" {
        return false;
    }
    if monitor_info.native_mode.width != 1920 || monitor_info.native_mode.height != 1080 {
        return false;
    }
    if monitor_info.native_mode.refresh_rate_hz != 60 || !monitor_info.supports_hdr10 {
        return false;
    }

    // Hotplug Event Listener test
    HOTPLUG_LISTENER.handle_hpd(DisplayConnectorType::UsbCDpAltMode, HotplugEvent::Connected, 1_000);
    if !HOTPLUG_LISTENER.is_external_active() {
        return false;
    }
    if HOTPLUG_LISTENER.get_active_connector() != DisplayConnectorType::UsbCDpAltMode {
        return false;
    }

    // 2. Seamless DRM Atomic Modesetting & Resolution Scaler
    let mode_4k_120 = DisplayModeTiming::new_progressive(3840, 2160, 120);

    // Test-only commit
    if MODESET_MANAGER.commit_atomic_modeset(
        DisplayTopologyMode::ExternalPrimary,
        mode_4k_120,
        DRM_MODE_ATOMIC_TEST_ONLY,
    ).is_err() {
        return false;
    }

    // Actual atomic modeset commit
    let seq = match MODESET_MANAGER.commit_atomic_modeset(
        DisplayTopologyMode::ExternalPrimary,
        mode_4k_120,
        DRM_MODE_ATOMIC_ALLOW_MODESET,
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if seq == 0 || MODESET_MANAGER.get_topology() != DisplayTopologyMode::ExternalPrimary {
        return false;
    }

    let active_ext = MODESET_MANAGER.get_active_timing(DisplayConnectorType::UsbCDpAltMode);
    if active_ext.width != 3840 || active_ext.height != 2160 || active_ext.refresh_rate_hz != 120 {
        return false;
    }

    MODESET_MANAGER.set_scaling_filter(DisplayScalingFilter::FsrUpscale);
    if MODESET_MANAGER.get_scaling_filter() != DisplayScalingFilter::FsrUpscale {
        return false;
    }

    // 3. Dynamic Color Space Selection & HDR10 Metadata Routing
    let (colorspace, depth) = COLOR_MANAGER.select_color_space(ColorSpace::Rec2020Hdr10, &monitor_info);
    if colorspace != ColorSpace::Rec2020Hdr10 || depth != ColorDepth::Bpc10 || !COLOR_MANAGER.is_hdr_active() {
        return false;
    }

    let hdr_meta = COLOR_MANAGER.get_hdr_metadata();
    if !hdr_meta.is_hdr_active || hdr_meta.max_luminance_nits != 1000 {
        return false;
    }

    // 4. Variable Refresh Rate (VRR) & Display Clock Pacing
    VRR_GOVERNOR.configure_monitor_vrr(&monitor_info);
    VRR_GOVERNOR.set_vrr_range(40, 120);
    VRR_GOVERNOR.set_target_refresh_cap(120);

    // Pace at 75 FPS within 40..120Hz VRR window -> ActiveTracking
    let vrr_stat = VRR_GOVERNOR.pace_frame(75);
    if vrr_stat != VrrStatus::ActiveTracking || VRR_GOVERNOR.get_effective_refresh_hz() != 75 {
        return false;
    }

    // Pace at 30 FPS below 40Hz VRR floor -> Low Frame-Rate Compensation (LFC 2x multiplier = 60Hz)
    let lfc_stat = VRR_GOVERNOR.pace_frame(30);
    if lfc_stat != VrrStatus::LfcEngaged || VRR_GOVERNOR.get_lfc_multiplier() != 2 || VRR_GOVERNOR.get_effective_refresh_hz() != 60 {
        return false;
    }

    // Clean baseline reset
    let mode_internal = DisplayModeTiming::new_progressive(1280, 800, 60);
    let _ = MODESET_MANAGER.commit_atomic_modeset(DisplayTopologyMode::InternalOnly, mode_internal, DRM_MODE_ATOMIC_ALLOW_MODESET);
    HOTPLUG_LISTENER.handle_hpd(DisplayConnectorType::InternalEdp, HotplugEvent::Disconnected, 2_000);
    COLOR_MANAGER.select_color_space(ColorSpace::SrgbStandard, &monitor_info);

    true
}
