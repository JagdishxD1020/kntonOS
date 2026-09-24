//! Handheld eGPU Hot-Plug & PCIe Bus Rebalancing Governor.

#![allow(dead_code)]

/// Standard Hot-Plug Controller (SHPC) state machine for handheld eGPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgpuHotplugState {
    /// No external GPU connected to USB4/Oculink port.
    Disconnected,
    /// Cable detected; protocol negotiation and PCIe tunnel training active.
    TunnelNegotiating,
    /// High-speed PCIe Gen4 x4 tunnel established and link trained.
    LinkReadyGen4,
    /// MMIO BARs allocated and eGPU active as primary graphics renderer.
    ActivePrimary,
    /// Sudden physical disconnect detected while active; safety fallback engaged.
    SurpriseRemoval,
}

/// PCIe Link Generation and Lane Width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcieLinkSpeed {
    None,
    Gen3x4,
    Gen4x4,
}

/// Hot-plug eGPU controller tracking connection state and PCIe link negotiation.
#[derive(Debug, Clone)]
pub struct EgpuController {
    pub state: EgpuHotplugState,
    pub link_speed: PcieLinkSpeed,
    pub link_width: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub state_transition_count: u32,
}

impl EgpuController {
    pub const fn new() -> Self {
        Self {
            state: EgpuHotplugState::Disconnected,
            link_speed: PcieLinkSpeed::None,
            link_width: 0,
            vendor_id: 0,
            device_id: 0,
            state_transition_count: 0,
        }
    }

    /// Handles physical cable insertion interrupt event.
    pub fn on_cable_insert(&mut self) -> EgpuHotplugState {
        if self.state == EgpuHotplugState::Disconnected {
            self.state = EgpuHotplugState::TunnelNegotiating;
            self.state_transition_count += 1;
        }
        self.state
    }

    /// Completes PCIe tunnel training and establishes high-speed link.
    pub fn complete_tunnel_training(&mut self, vendor_id: u16, device_id: u16) -> Result<PcieLinkSpeed, &'static str> {
        if self.state != EgpuHotplugState::TunnelNegotiating {
            return Err("Not in tunnel negotiating state");
        }

        self.vendor_id = vendor_id;
        self.device_id = device_id;
        self.link_speed = PcieLinkSpeed::Gen4x4;
        self.link_width = 4;
        self.state = EgpuHotplugState::LinkReadyGen4;
        self.state_transition_count += 1;
        Ok(self.link_speed)
    }

    /// Transitions to active primary renderer once BARs are mapped.
    pub fn mark_active_primary(&mut self) -> Result<(), &'static str> {
        if self.state != EgpuHotplugState::LinkReadyGen4 {
            return Err("Link not ready");
        }
        self.state = EgpuHotplugState::ActivePrimary;
        self.state_transition_count += 1;
        Ok(())
    }

    /// Triggers sudden surprise removal safety sequence.
    pub fn on_surprise_removal(&mut self) -> EgpuHotplugState {
        self.state = EgpuHotplugState::SurpriseRemoval;
        self.link_speed = PcieLinkSpeed::None;
        self.link_width = 0;
        self.state_transition_count += 1;
        self.state
    }

    /// Resets state machine back to Disconnected after teardown.
    pub fn complete_disconnect(&mut self) {
        self.state = EgpuHotplugState::Disconnected;
        self.vendor_id = 0;
        self.device_id = 0;
        self.state_transition_count += 1;
    }
}

/// Unit test verifying eGPU hot-plug controller state machine and link negotiation.
pub fn verify_egpu_state_machine() -> bool {
    let mut egpu = EgpuController::new();
    if egpu.state != EgpuHotplugState::Disconnected {
        return false;
    }

    // 1. Cable insertion -> TunnelNegotiating
    let s1 = egpu.on_cable_insert();
    if s1 != EgpuHotplugState::TunnelNegotiating {
        return false;
    }

    // 2. Tunnel training -> LinkReadyGen4 (e.g. AMD Radeon RX 7800 XT / NVIDIA RTX 4070)
    let res = egpu.complete_tunnel_training(0x10DE, 0x2782);
    if res != Ok(PcieLinkSpeed::Gen4x4) || egpu.state != EgpuHotplugState::LinkReadyGen4 {
        return false;
    }
    if egpu.link_width != 4 {
        return false;
    }

    // 3. Mark active primary
    if egpu.mark_active_primary().is_err() || egpu.state != EgpuHotplugState::ActivePrimary {
        return false;
    }

    // 4. Sudden disconnect -> SurpriseRemoval
    let s_drop = egpu.on_surprise_removal();
    if s_drop != EgpuHotplugState::SurpriseRemoval || egpu.link_speed != PcieLinkSpeed::None {
        return false;
    }

    // 5. Complete teardown
    egpu.complete_disconnect();
    if egpu.state != EgpuHotplugState::Disconnected {
        return false;
    }

    true
}
