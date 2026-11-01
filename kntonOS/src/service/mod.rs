//! Background system service supervisor and device hardware daemon.

pub mod device_daemon;
pub use device_daemon::*;

pub mod power_daemon;
pub use power_daemon::*;

pub mod audio_daemon;
pub use audio_daemon::*;

pub mod display_daemon;
pub use display_daemon::*;

pub mod gamepad_mapper;
pub use gamepad_mapper::*;

pub mod net_daemon;
pub use net_daemon::*;

pub mod storage_daemon;
pub use storage_daemon::*;

pub mod bluetooth_daemon;
pub use bluetooth_daemon::*;

pub mod perf_overlay;
pub use perf_overlay::*;

pub mod dock_daemon;
pub use dock_daemon::*;

pub mod thermal_governor;
pub use thermal_governor::*;

pub mod backlight_daemon;
pub use backlight_daemon::*;

pub mod motion_daemon;
pub use motion_daemon::*;

pub mod haptics_daemon;
pub use haptics_daemon::*;

pub mod sd_indexer;
pub use sd_indexer::*;

/// Service operational state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Stopped,
    Starting,
    Running,
    Failed,
}

/// Service definition descriptor.
#[derive(Debug, Clone, Copy)]
pub struct SystemService {
    pub service_id: u16,
    pub state: ServiceState,
    pub restart_count: u32,
    pub auto_restart: bool,
}

impl SystemService {
    pub const fn new(service_id: u16, auto_restart: bool) -> Self {
        Self {
            service_id,
            state: ServiceState::Stopped,
            restart_count: 0,
            auto_restart,
        }
    }

    pub fn start(&mut self) {
        self.state = ServiceState::Running;
    }

    pub fn handle_crash(&mut self) -> bool {
        self.state = ServiceState::Failed;
        if self.auto_restart && self.restart_count < 5 {
            self.restart_count += 1;
            self.state = ServiceState::Running;
            true
        } else {
            false
        }
    }
}

/// Verification test for service manager.
pub fn verify_service_manager() -> bool {
    let mut svc = SystemService::new(1, true);
    svc.start();
    if svc.state != ServiceState::Running {
        return false;
    }

    let recovered = svc.handle_crash();
    if !recovered || svc.state != ServiceState::Running || svc.restart_count != 1 {
        return false;
    }

    true
}
