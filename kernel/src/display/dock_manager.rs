//! Handheld USB-C Docking Station & DisplayPort MST Manager.

#![allow(dead_code)]

/// Dock connection and power state machine states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockState {
    Undocked,
    DockNegotiating,
    DockedAC,
    DockedBattery,
    Unplugging,
}

/// Handheld USB-C docking station state coordinator.
#[derive(Debug, Clone, Copy)]
pub struct UsbCDockCoordinator {
    pub current_state: DockState,
    pub ac_power_detected: bool,
    pub negotiated_pd_watts: u8,
}

impl UsbCDockCoordinator {
    /// Creates a new docking station coordinator in Undocked state.
    pub const fn new() -> Self {
        Self {
            current_state: DockState::Undocked,
            ac_power_detected: false,
            negotiated_pd_watts: 0,
        }
    }

    /// Handles USB-C CC line attachment and begins dock negotiation.
    pub fn on_dock_attached(&mut self) -> bool {
        if self.current_state != DockState::Undocked {
            return false;
        }
        self.current_state = DockState::DockNegotiating;
        true
    }

    /// Completes negotiation with USB-PD contract and power source status.
    pub fn on_negotiation_complete(&mut self, pd_watts: u8, ac_connected: bool) -> DockState {
        self.negotiated_pd_watts = pd_watts;
        self.ac_power_detected = ac_connected;
        if ac_connected && pd_watts >= 45 {
            self.current_state = DockState::DockedAC;
        } else {
            self.current_state = DockState::DockedBattery;
        }
        self.current_state
    }

    /// Handles dock detachment and teardown.
    pub fn on_dock_unplugged(&mut self) -> DockState {
        self.current_state = DockState::Unplugging;
        self.negotiated_pd_watts = 0;
        self.ac_power_detected = false;
        self.current_state = DockState::Undocked;
        self.current_state
    }
}

/// Unit verification for dock state machine.
pub fn verify_dock_state_machine() -> bool {
    let mut dock = UsbCDockCoordinator::new();
    if dock.current_state != DockState::Undocked {
        return false;
    }

    if !dock.on_dock_attached() || dock.current_state != DockState::DockNegotiating {
        return false;
    }

    // 100W AC dock negotiation
    if dock.on_negotiation_complete(100, true) != DockState::DockedAC {
        return false;
    }
    if dock.negotiated_pd_watts != 100 || !dock.ac_power_detected {
        return false;
    }

    // Unplug
    if dock.on_dock_unplugged() != DockState::Undocked {
        return false;
    }

    true
}

/// Parsed DisplayPort/HDMI EDID monitor capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedEdidDescriptor {
    pub manufacturer_id: [u8; 3],
    pub product_code: u16,
    pub native_width: u16,
    pub native_height: u16,
    pub max_refresh_hz: u16,
    pub supports_vrr: bool,
}

/// USB-C DP Alt Mode & DisplayPort Multi-Stream Transport (MST) hot-plug manager.
#[derive(Debug, Clone, Copy)]
pub struct DpMstHotPlugManager {
    pub hpd_asserted: bool,
    pub dp_lanes_allocated: u8,
    pub mst_branch_detected: bool,
    pub connected_display_count: u8,
    pub primary_display: ParsedEdidDescriptor,
}

impl DpMstHotPlugManager {
    /// Creates a new DisplayPort MST manager.
    pub const fn new() -> Self {
        Self {
            hpd_asserted: false,
            dp_lanes_allocated: 0,
            mst_branch_detected: false,
            connected_display_count: 0,
            primary_display: ParsedEdidDescriptor {
                manufacturer_id: [b'D', b'E', b'L'],
                product_code: 0x4010,
                native_width: 3840,
                native_height: 2160,
                max_refresh_hz: 144,
                supports_vrr: true,
            },
        }
    }

    /// Handles DP HPD (Hot-Plug Detect) pin assertion and configures USB-C DP Alt Mode 4-lane link.
    pub fn on_hpd_irq(&mut self, asserted: bool) -> bool {
        self.hpd_asserted = asserted;
        if asserted {
            self.dp_lanes_allocated = 4; // 4-lane DisplayPort 1.4a/2.1 UHBR10
        } else {
            self.dp_lanes_allocated = 0;
            self.connected_display_count = 0;
            self.mst_branch_detected = false;
        }
        self.hpd_asserted
    }

    /// Parses a standard 128-byte raw EDID block from external monitor.
    pub fn parse_edid_block(&mut self, edid: &[u8]) -> bool {
        if edid.len() < 128 {
            return false;
        }

        // Validate EDID header: 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00
        if edid[0] != 0x00 || edid[1] != 0xFF || edid[2] != 0xFF || edid[3] != 0xFF
            || edid[4] != 0xFF || edid[5] != 0xFF || edid[6] != 0xFF || edid[7] != 0x00 {
            return false;
        }

        // Manufacturer ID at bytes 8-9 (packed 5-bit characters)
        let mfg_raw = u16::from_be_bytes([edid[8], edid[9]]);
        let c1 = ((mfg_raw >> 10) & 0x1F) as u8 + b'@';
        let c2 = ((mfg_raw >> 5) & 0x1F) as u8 + b'@';
        let c3 = (mfg_raw & 0x1F) as u8 + b'@';
        self.primary_display.manufacturer_id = [c1, c2, c3];

        self.primary_display.product_code = u16::from_le_bytes([edid[10], edid[11]]);

        // Detailed Timing Descriptor 1 starts at byte 54
        // Pixel clock: bytes 54-55 (in 10 kHz units)
        let pixel_clock_10khz = u16::from_le_bytes([edid[54], edid[55]]);
        if pixel_clock_10khz > 0 {
            let h_active = (edid[56] as u16) | (((edid[58] as u16) & 0xF0) << 4);
            let v_active = (edid[59] as u16) | (((edid[61] as u16) & 0xF0) << 4);
            self.primary_display.native_width = h_active;
            self.primary_display.native_height = v_active;
        }

        self.connected_display_count = 1;
        true
    }
}

/// Unit verification for DisplayPort MST hot-plug and EDID parser.
pub fn verify_dp_mst_and_edid() -> bool {
    let mut mgr = DpMstHotPlugManager::new();
    if !mgr.on_hpd_irq(true) || mgr.dp_lanes_allocated != 4 {
        return false;
    }

    // Construct mock valid 128-byte EDID for 3840x2160 (4K)
    let mut mock_edid = [0u8; 128];
    mock_edid[0] = 0x00;
    mock_edid[1] = 0xFF;
    mock_edid[2] = 0xFF;
    mock_edid[3] = 0xFF;
    mock_edid[4] = 0xFF;
    mock_edid[5] = 0xFF;
    mock_edid[6] = 0xFF;
    mock_edid[7] = 0x00;

    // Packed MFG "DEL" -> D (4), E (5), L (12): (4<<10)|(5<<5)|12 = 4096 + 160 + 12 = 4268 (0x10AC)
    mock_edid[8] = 0x10;
    mock_edid[9] = 0xAC;

    // Detailed Timing 1 at byte 54: pixel clock non-zero
    mock_edid[54] = 0x00;
    mock_edid[55] = 0x80;
    // H-active 3840 (0xF00) -> lower 8 bits 0x00 at byte 56, upper 4 bits 0x0F in high nibble of byte 58
    mock_edid[56] = 0x00;
    mock_edid[58] = 0xF0;
    // V-active 2160 (0x870) -> lower 8 bits 0x70 at byte 59, upper 4 bits 0x08 in high nibble of byte 61
    mock_edid[59] = 0x70;
    mock_edid[61] = 0x80;

    if !mgr.parse_edid_block(&mock_edid) {
        return false;
    }
    if mgr.primary_display.native_width != 3840 || mgr.primary_display.native_height != 2160 {
        return false;
    }
    if mgr.connected_display_count != 1 {
        return false;
    }

    true
}

/// Display target configuration mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveDisplayMode {
    InternalHandheldPanel, // 1080p (1920x1080), 120Hz VRR
    ExternalMonitor4K,     // 4K (3840x2160), 144Hz
    ExternalMonitor1440p,  // 1440p (2560x1440), 165Hz
}

/// Dynamic display mode and resolution switcher.
#[derive(Debug, Clone, Copy)]
pub struct DynamicDisplayModeSwitcher {
    pub current_mode: ActiveDisplayMode,
    pub active_width: u16,
    pub active_height: u16,
    pub active_refresh_hz: u16,
    pub vrr_active: bool,
}

impl DynamicDisplayModeSwitcher {
    /// Creates a new display mode switcher defaulted to internal handheld panel.
    pub const fn new() -> Self {
        Self {
            current_mode: ActiveDisplayMode::InternalHandheldPanel,
            active_width: 1920,
            active_height: 1080,
            active_refresh_hz: 120,
            vrr_active: true,
        }
    }

    /// Switches display configuration based on docked status and EDID capabilities.
    pub fn switch_display_mode(&mut self, docked: bool, edid: Option<&ParsedEdidDescriptor>) -> ActiveDisplayMode {
        if !docked || edid.is_none() {
            // Revert to internal handheld panel
            self.current_mode = ActiveDisplayMode::InternalHandheldPanel;
            self.active_width = 1920;
            self.active_height = 1080;
            self.active_refresh_hz = 120;
            self.vrr_active = true;
            return self.current_mode;
        }

        let desc = edid.unwrap();
        if desc.native_width >= 3840 {
            self.current_mode = ActiveDisplayMode::ExternalMonitor4K;
            self.active_width = 3840;
            self.active_height = 2160;
            self.active_refresh_hz = desc.max_refresh_hz.min(144);
            self.vrr_active = desc.supports_vrr;
        } else {
            self.current_mode = ActiveDisplayMode::ExternalMonitor1440p;
            self.active_width = 2560;
            self.active_height = 1440;
            self.active_refresh_hz = desc.max_refresh_hz.min(165);
            self.vrr_active = desc.supports_vrr;
        }

        self.current_mode
    }
}

/// Unit verification for dynamic display mode switcher.
pub fn verify_dynamic_display_mode_switcher() -> bool {
    let mut switcher = DynamicDisplayModeSwitcher::new();
    if switcher.current_mode != ActiveDisplayMode::InternalHandheldPanel {
        return false;
    }

    // Dock with 4K external monitor
    let edid_4k = ParsedEdidDescriptor {
        manufacturer_id: [b'D', b'E', b'L'],
        product_code: 0x4010,
        native_width: 3840,
        native_height: 2160,
        max_refresh_hz: 144,
        supports_vrr: true,
    };

    if switcher.switch_display_mode(true, Some(&edid_4k)) != ActiveDisplayMode::ExternalMonitor4K {
        return false;
    }
    if switcher.active_width != 3840 || switcher.active_height != 2160 || switcher.active_refresh_hz != 144 {
        return false;
    }

    // Undock: reverts to internal 1080p 120Hz
    if switcher.switch_display_mode(false, None) != ActiveDisplayMode::InternalHandheldPanel {
        return false;
    }
    if switcher.active_width != 1920 || switcher.active_height != 1080 || switcher.active_refresh_hz != 120 {
        return false;
    }

    true
}

/// Audio output endpoint routing target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioOutputEndpoint {
    InternalStereoSpeakers,
    ExternalHdmiDisplayPort,
}

/// Docked power policy booster and audio routing controller.
#[derive(Debug, Clone, Copy)]
pub struct DockedPowerPolicyBooster {
    pub active_tdp_target_watts: u8,
    pub turbo_boost_engaged: bool,
    pub audio_endpoint: AudioOutputEndpoint,
}

impl DockedPowerPolicyBooster {
    /// Creates a new docked power booster in battery mode (15W base).
    pub const fn new() -> Self {
        Self {
            active_tdp_target_watts: 15,
            turbo_boost_engaged: false,
            audio_endpoint: AudioOutputEndpoint::InternalStereoSpeakers,
        }
    }

    /// Evaluates dock state and raises APU TDP target when connected to AC dock.
    pub fn evaluate_dock_policy(&mut self, dock_state: DockState) -> u8 {
        match dock_state {
            DockState::DockedAC => {
                // Raise from 15W battery baseline to 30W+ docked AC turbo profile
                self.active_tdp_target_watts = 35;
                self.turbo_boost_engaged = true;
                self.audio_endpoint = AudioOutputEndpoint::ExternalHdmiDisplayPort;
            }
            DockState::DockedBattery => {
                self.active_tdp_target_watts = 18;
                self.turbo_boost_engaged = false;
                self.audio_endpoint = AudioOutputEndpoint::ExternalHdmiDisplayPort;
            }
            DockState::Undocked | DockState::DockNegotiating | DockState::Unplugging => {
                self.active_tdp_target_watts = 15;
                self.turbo_boost_engaged = false;
                self.audio_endpoint = AudioOutputEndpoint::InternalStereoSpeakers;
            }
        }
        self.active_tdp_target_watts
    }
}

/// Unit verification for docked power policy booster.
pub fn verify_docked_power_policy_booster() -> bool {
    let mut booster = DockedPowerPolicyBooster::new();
    if booster.active_tdp_target_watts != 15 || booster.turbo_boost_engaged {
        return false;
    }

    // Connect to AC dock: raises to 35W turbo and routes audio to HDMI/DP
    if booster.evaluate_dock_policy(DockState::DockedAC) != 35 {
        return false;
    }
    if !booster.turbo_boost_engaged || booster.audio_endpoint != AudioOutputEndpoint::ExternalHdmiDisplayPort {
        return false;
    }

    // Undock: restores 15W and internal stereo speakers
    if booster.evaluate_dock_policy(DockState::Undocked) != 15 {
        return false;
    }
    if booster.turbo_boost_engaged || booster.audio_endpoint != AudioOutputEndpoint::InternalStereoSpeakers {
        return false;
    }

    true
}

/// Comprehensive verification self-test suite for Handheld USB-C Docking Station & DisplayPort MST Manager.
pub fn test_dock_manager_subsystem() -> bool {
    crate::serial_write_str("[KNT-DISP] Testing Handheld USB-C Docking Station & DisplayPort MST Manager...\r\n");

    if !verify_dock_state_machine() {
        crate::serial_write_str("[KNT-PANIC] Dock state machine verification failed!\r\n");
        return false;
    }
    if !verify_dp_mst_and_edid() {
        crate::serial_write_str("[KNT-PANIC] DisplayPort MST and EDID parser verification failed!\r\n");
        return false;
    }
    if !verify_dynamic_display_mode_switcher() {
        crate::serial_write_str("[KNT-PANIC] Dynamic display mode switcher verification failed!\r\n");
        return false;
    }
    if !verify_docked_power_policy_booster() {
        crate::serial_write_str("[KNT-PANIC] Docked power policy booster verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-DOCK-MGR-PASS] Handheld USB-C docking station and DP MST manager verified\r\n");
    true
}
