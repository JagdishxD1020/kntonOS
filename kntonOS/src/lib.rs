//! # KntonOS Userspace Operating System & Gaming Environment
//!
//! Autonomous userspace environment providing session management, background system services,
//! low-latency IPC ports, user-mode runtime callgates, and handheld game mode shell.

#![no_std]

pub mod ipc;
pub mod runtime;
pub mod service;
pub mod session;
pub mod shell;

pub use ipc::verify_ipc_dispatcher_subsystem;
pub use runtime::verify_usermode_allocator_subsystem;
pub use service::verify_display_daemon_subsystem;
pub use service::verify_gamepad_mapper_subsystem;
pub use service::verify_storage_daemon_subsystem;
pub use service::verify_perf_overlay_subsystem;
pub use service::verify_thermal_governor_subsystem;
pub use service::verify_dock_daemon_subsystem;
pub use service::verify_backlight_daemon_subsystem;
pub use service::verify_motion_daemon_subsystem;
pub use service::verify_haptics_subsystem;
pub use service::verify_battery_manager_subsystem;
pub use service::verify_speaker_dsp_subsystem;
pub use service::verify_sd_indexer_subsystem;

/// Verification sentinel proving full userspace OS subsystem readiness.
pub fn verify_knton_os_userspace() -> bool {
    runtime::verify_user_runtime()
        && runtime::verify_usermode_allocator_subsystem()
        && session::verify_session_manager()
        && service::verify_service_manager()
        && service::verify_device_daemon_subsystem()
        && service::verify_power_daemon_subsystem()
        && service::verify_audio_daemon_subsystem()
        && service::verify_display_daemon_subsystem()
        && service::verify_gamepad_mapper_subsystem()
        && service::verify_network_daemon_subsystem()
        && service::verify_storage_daemon_subsystem()
        && service::verify_bluetooth_daemon_subsystem()
        && service::verify_perf_overlay_subsystem()
        && service::verify_dock_daemon_subsystem()
        && service::verify_thermal_governor_subsystem()
        && service::verify_backlight_daemon_subsystem()
        && service::verify_motion_daemon_subsystem()
        && service::verify_haptics_subsystem()
        && service::verify_battery_manager_subsystem()
        && service::verify_speaker_dsp_subsystem()
        && service::verify_sd_indexer_subsystem()
        && ipc::verify_ipc_ports()
        && ipc::verify_ipc_dispatcher_subsystem()
        && shell::verify_game_mode_shell()
}
