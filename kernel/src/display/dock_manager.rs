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
