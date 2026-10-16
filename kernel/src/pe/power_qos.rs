//! Win32 Process & Thread Power / Thermal Throttling Quality-of-Service (QoS) Policy Engine.
//!
//! Implements Windows NT Power Throttling APIs (SetProcessInformation / SetThreadInformation),
//! QoS execution levels (EcoQoS / HighQoS), gaming-aware foreground/background scheduler routing,
//! heterogenous P-core / E-core affinity governance, and thermal pressure prioritization.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_SUCCESS,
};
use crate::pe::ntdll::{
    nt_error, nt_status_to_str, nt_success, NtStatus,
    STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_NO_MEMORY,
    STATUS_SUCCESS, STATUS_UNSUCCESSFUL,
};
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;

pub const ProcessPowerThrottling: u32 = 4;
pub const ThreadPowerThrottling: u32 = 1;

pub const PROCESS_POWER_THROTTLING_CURRENT_VERSION: u32 = 1;
pub const THREAD_POWER_THROTTLING_CURRENT_VERSION: u32 = 1;

pub const PROCESS_POWER_THROTTLING_EXECUTION_SPEED: u32 = 0x0000_0001;
pub const PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION: u32 = 0x0000_0002;
pub const PROCESS_POWER_THROTTLING_VALID_FLAGS: u32 =
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION;

pub const THREAD_POWER_THROTTLING_EXECUTION_SPEED: u32 = 0x0000_0001;
pub const THREAD_POWER_THROTTLING_VALID_FLAGS: u32 = THREAD_POWER_THROTTLING_EXECUTION_SPEED;

/// Process power throttling configuration state matching Win32 PROCESS_POWER_THROTTLING_STATE.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PROCESS_POWER_THROTTLING_STATE {
    pub Version: u32,
    pub ControlMask: u32,
    pub StateMask: u32,
}

pub const TIMER_RESOLUTION_DEFAULT_100NS: u32 = 156250; // 15.625 ms standard tick
pub const TIMER_RESOLUTION_HIGH_PRECISION_100NS: u32 = 5000; // 0.5 ms precision
pub const TIMER_RESOLUTION_COALESCED_ECO_100NS: u32 = 300000; // 30.0 ms coalesced tick

pub const HIGH_QOS_MAX_FREQ_MHZ: u32 = 4800;
pub const ECO_QOS_MAX_FREQ_MHZ: u32 = 1600;

/// Quality-of-Service execution efficiency tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QosExecutionLevel {
    EcoQoS,
    HighQoS,
}

/// Dynamic timer resolution and coalescing configuration for a process or thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerResolutionPolicy {
    pub requested_resolution_100ns: u32,
    pub effective_resolution_100ns: u32,
    pub timer_coalescing_active: bool,
}

/// Thread power throttling configuration state matching Win32 THREAD_POWER_THROTTLING_STATE.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct THREAD_POWER_THROTTLING_STATE {
    pub Version: u32,
    pub ControlMask: u32,
    pub StateMask: u32,
}

/// Process-level QoS and power governance descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessPowerPolicy {
    pub process_id: u64,
    pub throttling_state: PROCESS_POWER_THROTTLING_STATE,
    pub is_foreground: bool,
    pub execution_speed_throttled: bool,
    pub ignore_timer_resolution: bool,
}

impl ProcessPowerPolicy {
    pub fn new(process_id: u64) -> Self {
        Self {
            process_id,
            throttling_state: PROCESS_POWER_THROTTLING_STATE {
                Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: 0,
                StateMask: 0,
            },
            is_foreground: true,
            execution_speed_throttled: false,
            ignore_timer_resolution: false,
        }
    }
}


pub const P_CORE_AFFINITY_MASK: u64 = 0b0011; // Cores 0 and 1
pub const E_CORE_AFFINITY_MASK: u64 = 0b1100; // Cores 2 and 3
pub const ALL_CORES_AFFINITY_MASK: u64 = 0b1111; // Cores 0..3

/// Functional role of a thread within a gaming runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamingThreadRole {
    RenderMain,
    PhysicsSim,
    AudioRealtime,
    InputProcessing,
    WorkerTask,
    TelemetryWorker,
    UpdateDownloader,
    BackgroundIdle,
}

impl GamingThreadRole {
    pub fn is_latency_critical(&self) -> bool {
        matches!(self, Self::AudioRealtime | Self::InputProcessing)
    }

    pub fn is_foreground_performance(&self) -> bool {
        matches!(self, Self::RenderMain | Self::PhysicsSim | Self::AudioRealtime | Self::InputProcessing)
    }

    pub fn is_background_throttled(&self) -> bool {
        matches!(self, Self::TelemetryWorker | Self::UpdateDownloader | Self::BackgroundIdle)
    }
}

/// Architectural CPU core class in heterogenous processors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuCoreClass {
    PerformanceCore,
    EfficiencyCore,
}

/// Computed scheduling placement and frequency profile for a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadSchedulingProfile {
    pub core_affinity_mask: u64,
    pub primary_core_class: CpuCoreClass,
    pub target_freq_mhz: u32,
    pub qos_level: QosExecutionLevel,
    pub is_immune_to_throttling: bool,
}

/// Thread-level QoS and power governance descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadPowerPolicy {
    pub thread_id: u64,
    pub process_id: u64,
    pub throttling_state: THREAD_POWER_THROTTLING_STATE,
    pub role: GamingThreadRole,
    pub execution_speed_throttled: bool,
}

impl ThreadPowerPolicy {
    pub fn new(thread_id: u64, process_id: u64) -> Self {
        Self {
            thread_id,
            process_id,
            throttling_state: THREAD_POWER_THROTTLING_STATE {
                Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: 0,
                StateMask: 0,
            },
            role: GamingThreadRole::WorkerTask,
            execution_speed_throttled: false,
        }
    }
}


/// Operating thermal zones based on SoC junction temperature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalPressureZone {
    Nominal,           // < 75 C
    Elevated,          // 75 C - 85 C
    Critical,          // 85 C - 95 C
    ThermalSaturation, // > 95 C (TJMax throttle zone)
}

impl ThermalPressureZone {
    pub fn from_temperature(temp_celsius: u32) -> Self {
        if temp_celsius > 95 {
            ThermalPressureZone::ThermalSaturation
        } else if temp_celsius >= 85 {
            ThermalPressureZone::Critical
        } else if temp_celsius >= 75 {
            ThermalPressureZone::Elevated
        } else {
            ThermalPressureZone::Nominal
        }
    }
}

/// Dynamic thermal governor tracking temperature and enforcing prioritized degradation.
#[derive(Debug)]
pub struct ThermalGovernorState {
    pub current_temperature_celsius: u32,
    pub current_zone: ThermalPressureZone,
    pub throttling_events_count: AtomicU32,
    pub audio_input_preserved_count: AtomicU32,
    pub render_throttled_count: AtomicU32,
    pub background_quenched_count: AtomicU32,
}

impl ThermalGovernorState {
    pub fn new() -> Self {
        Self {
            current_temperature_celsius: 45,
            current_zone: ThermalPressureZone::Nominal,
            throttling_events_count: AtomicU32::new(0),
            audio_input_preserved_count: AtomicU32::new(0),
            render_throttled_count: AtomicU32::new(0),
            background_quenched_count: AtomicU32::new(0),
        }
    }

    pub fn set_temperature(&mut self, temp_celsius: u32) -> ThermalPressureZone {
        self.current_temperature_celsius = temp_celsius;
        let new_zone = ThermalPressureZone::from_temperature(temp_celsius);
        if new_zone != self.current_zone && new_zone != ThermalPressureZone::Nominal {
            self.throttling_events_count.fetch_add(1, Ordering::SeqCst);
        }
        self.current_zone = new_zone;
        new_zone
    }
}

/// Central Power & Thermal Throttling Quality-of-Service Governor.
pub struct PowerQosManager {
    pub process_policies: BTreeMap<u64, ProcessPowerPolicy>,
    pub thread_policies: BTreeMap<u64, ThreadPowerPolicy>,
    pub thermal_state: ThermalGovernorState,
}

impl PowerQosManager {
    pub fn new() -> Self {
        Self {
            process_policies: BTreeMap::new(),
            thread_policies: BTreeMap::new(),
            thermal_state: ThermalGovernorState::new(),
        }
    }

    /// Sets process power throttling QoS state.
    pub fn set_process_power_throttling(
        &mut self,
        process_id: u64,
        state: PROCESS_POWER_THROTTLING_STATE,
    ) -> Result<(), NtStatus> {
        if state.Version != PROCESS_POWER_THROTTLING_CURRENT_VERSION {
            return Err(STATUS_INVALID_PARAMETER);
        }

        if (state.ControlMask & !PROCESS_POWER_THROTTLING_VALID_FLAGS) != 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }

        let policy = self
            .process_policies
            .entry(process_id)
            .or_insert_with(|| ProcessPowerPolicy::new(process_id));

        if (state.ControlMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED) != 0 {
            policy.execution_speed_throttled =
                (state.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED) != 0;
        }

        if (state.ControlMask & PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION) != 0 {
            policy.ignore_timer_resolution =
                (state.StateMask & PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION) != 0;
        }

        policy.throttling_state = state;
        Ok(())
    }

    /// Retrieves current process power throttling QoS state.
    pub fn get_process_power_throttling(
        &self,
        process_id: u64,
    ) -> Result<PROCESS_POWER_THROTTLING_STATE, NtStatus> {
        let policy = self
            .process_policies
            .get(&process_id)
            .ok_or(STATUS_INVALID_HANDLE)?;

        Ok(policy.throttling_state)
    }

    /// Sets thread power throttling QoS state.
    pub fn set_thread_power_throttling(
        &mut self,
        thread_id: u64,
        process_id: u64,
        state: THREAD_POWER_THROTTLING_STATE,
    ) -> Result<(), NtStatus> {
        if state.Version != THREAD_POWER_THROTTLING_CURRENT_VERSION {
            return Err(STATUS_INVALID_PARAMETER);
        }

        if (state.ControlMask & !THREAD_POWER_THROTTLING_VALID_FLAGS) != 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }

        let policy = self
            .thread_policies
            .entry(thread_id)
            .or_insert_with(|| ThreadPowerPolicy::new(thread_id, process_id));

        if (state.ControlMask & THREAD_POWER_THROTTLING_EXECUTION_SPEED) != 0 {
            policy.execution_speed_throttled =
                (state.StateMask & THREAD_POWER_THROTTLING_EXECUTION_SPEED) != 0;
        }

        policy.throttling_state = state;
        Ok(())
    }

    /// Retrieves current thread power throttling QoS state.
    pub fn get_thread_power_throttling(
        &self,
        thread_id: u64,
    ) -> Result<THREAD_POWER_THROTTLING_STATE, NtStatus> {
        let policy = self
            .thread_policies
            .get(&thread_id)
            .ok_or(STATUS_INVALID_HANDLE)?;

        Ok(policy.throttling_state)
    }


    /// Determines the effective QoS execution level (EcoQoS vs HighQoS) for a thread.
    /// Thread-level policy overrides process-level policy.
    pub fn determine_thread_qos_level(&self, thread_id: u64) -> QosExecutionLevel {
        if let Some(thread_policy) = self.thread_policies.get(&thread_id) {
            if thread_policy.execution_speed_throttled {
                return QosExecutionLevel::EcoQoS;
            }
            // If thread specifically configured as not throttled
            if (thread_policy.throttling_state.ControlMask & THREAD_POWER_THROTTLING_EXECUTION_SPEED) != 0 {
                return QosExecutionLevel::HighQoS;
            }
            // Check parent process policy
            if let Some(proc_policy) = self.process_policies.get(&thread_policy.process_id) {
                if proc_policy.execution_speed_throttled {
                    return QosExecutionLevel::EcoQoS;
                }
            }
        }
        QosExecutionLevel::HighQoS
    }

    /// Calculates the effective timer resolution according to PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION.
    pub fn calculate_effective_timer_resolution(
        &self,
        thread_id: u64,
        requested_100ns: u32,
    ) -> TimerResolutionPolicy {
        let qos = self.determine_thread_qos_level(thread_id);
        let ignore_timer = match qos {
            QosExecutionLevel::EcoQoS => true,
            QosExecutionLevel::HighQoS => false,
        };

        if ignore_timer {
            // EcoQoS timer coalescing active: enforce standard or relaxed tick rate
            let effective = if requested_100ns < TIMER_RESOLUTION_DEFAULT_100NS {
                TIMER_RESOLUTION_DEFAULT_100NS
            } else {
                requested_100ns
            };

            TimerResolutionPolicy {
                requested_resolution_100ns: requested_100ns,
                effective_resolution_100ns: effective,
                timer_coalescing_active: true,
            }
        } else {
            // HighQoS: respect high-precision timer down to 0.5 ms
            let effective = requested_100ns.max(TIMER_RESOLUTION_HIGH_PRECISION_100NS);
            TimerResolutionPolicy {
                requested_resolution_100ns: requested_100ns,
                effective_resolution_100ns: effective,
                timer_coalescing_active: false,
            }
        }
    }

    /// Evaluates CPU maximum frequency cap based on active QoS level.
    pub fn get_effective_frequency_cap(&self, thread_id: u64) -> u32 {
        match self.determine_thread_qos_level(thread_id) {
            QosExecutionLevel::EcoQoS => ECO_QOS_MAX_FREQ_MHZ,
            QosExecutionLevel::HighQoS => HIGH_QOS_MAX_FREQ_MHZ,
        }
    }


    /// Sets the gaming runtime functional role for a thread.
    pub fn set_thread_role(&mut self, thread_id: u64, process_id: u64, role: GamingThreadRole) {
        let policy = self
            .thread_policies
            .entry(thread_id)
            .or_insert_with(|| ThreadPowerPolicy::new(thread_id, process_id));
        policy.role = role;

        // Automatically assign EcoQoS if thread role is background throttled
        if role.is_background_throttled() {
            policy.execution_speed_throttled = true;
            policy.throttling_state.ControlMask |= THREAD_POWER_THROTTLING_EXECUTION_SPEED;
            policy.throttling_state.StateMask |= THREAD_POWER_THROTTLING_EXECUTION_SPEED;
        }
    }

    /// Updates process foreground / background window state.
    pub fn set_process_foreground_state(&mut self, process_id: u64, is_foreground: bool) {
        let policy = self
            .process_policies
            .entry(process_id)
            .or_insert_with(|| ProcessPowerPolicy::new(process_id));
        policy.is_foreground = is_foreground;

        // If moved to background, enable power throttling unless explicitly exempted
        if !is_foreground {
            policy.execution_speed_throttled = true;
            policy.ignore_timer_resolution = true;
        } else {
            policy.execution_speed_throttled = false;
            policy.ignore_timer_resolution = false;
        }
    }

    /// Evaluates core affinity and scheduling placement for a thread based on role and QoS.
    pub fn route_thread_scheduling(&self, thread_id: u64) -> ThreadSchedulingProfile {
        let (role, proc_id) = if let Some(tp) = self.thread_policies.get(&thread_id) {
            (tp.role, tp.process_id)
        } else {
            (GamingThreadRole::WorkerTask, 0)
        };

        let is_proc_fg = self
            .process_policies
            .get(&proc_id)
            .map(|p| p.is_foreground)
            .unwrap_or(true);

        // Latency-critical threads (Audio, Input) always get dedicated P-Core affinity and top frequency
        if role.is_latency_critical() {
            return ThreadSchedulingProfile {
                core_affinity_mask: P_CORE_AFFINITY_MASK,
                primary_core_class: CpuCoreClass::PerformanceCore,
                target_freq_mhz: HIGH_QOS_MAX_FREQ_MHZ,
                qos_level: QosExecutionLevel::HighQoS,
                is_immune_to_throttling: true,
            };
        }

        // Background / telemetry threads or background processes routed to E-Cores
        if role.is_background_throttled() || !is_proc_fg {
            return ThreadSchedulingProfile {
                core_affinity_mask: E_CORE_AFFINITY_MASK,
                primary_core_class: CpuCoreClass::EfficiencyCore,
                target_freq_mhz: ECO_QOS_MAX_FREQ_MHZ,
                qos_level: QosExecutionLevel::EcoQoS,
                is_immune_to_throttling: false,
            };
        }

        // Foreground Game Render & Physics threads routed to P-Cores
        if role.is_foreground_performance() {
            ThreadSchedulingProfile {
                core_affinity_mask: P_CORE_AFFINITY_MASK,
                primary_core_class: CpuCoreClass::PerformanceCore,
                target_freq_mhz: HIGH_QOS_MAX_FREQ_MHZ,
                qos_level: QosExecutionLevel::HighQoS,
                is_immune_to_throttling: false,
            }
        } else {
            // General worker tasks run across all cores with balanced QoS
            ThreadSchedulingProfile {
                core_affinity_mask: ALL_CORES_AFFINITY_MASK,
                primary_core_class: CpuCoreClass::PerformanceCore,
                target_freq_mhz: 3200,
                qos_level: QosExecutionLevel::HighQoS,
                is_immune_to_throttling: false,
            }
        }
    }


    /// Updates SoC thermal temperature reading and triggers dynamic degradation evaluation.
    pub fn update_soc_temperature(&mut self, temp_celsius: u32) -> ThermalPressureZone {
        self.thermal_state.set_temperature(temp_celsius)
    }

    /// Evaluates effective thread scheduling profile under active thermal pressure constraints.
    /// Under thermal saturation (>95°C), strictly preserves Audio/Input stability while degrading render/background.
    pub fn route_thread_thermal_scheduling(&self, thread_id: u64) -> ThreadSchedulingProfile {
        let mut profile = self.route_thread_scheduling(thread_id);
        let zone = self.thermal_state.current_zone;

        match zone {
            ThermalPressureZone::Nominal => profile,
            ThermalPressureZone::Elevated => {
                // Background threads throttled further down
                if !profile.is_immune_to_throttling && profile.primary_core_class == CpuCoreClass::EfficiencyCore {
                    profile.target_freq_mhz = 1200;
                    self.thermal_state.background_quenched_count.fetch_add(1, Ordering::SeqCst);
                }
                profile
            }
            ThermalPressureZone::Critical => {
                if !profile.is_immune_to_throttling {
                    if profile.primary_core_class == CpuCoreClass::EfficiencyCore {
                        profile.target_freq_mhz = 1000;
                        self.thermal_state.background_quenched_count.fetch_add(1, Ordering::SeqCst);
                    } else if profile.target_freq_mhz > 3600 {
                        profile.target_freq_mhz = 3600;
                        self.thermal_state.render_throttled_count.fetch_add(1, Ordering::SeqCst);
                    }
                }
                profile
            }
            ThermalPressureZone::ThermalSaturation => {
                // > 95°C Thermal Saturation (Emergency Handheld / Laptop Protection)
                if profile.is_immune_to_throttling {
                    // Audio and Input are strictly protected against frequency degradation and remain on P-Cores
                    self.thermal_state.audio_input_preserved_count.fetch_add(1, Ordering::SeqCst);
                    profile.target_freq_mhz = HIGH_QOS_MAX_FREQ_MHZ;
                    profile.core_affinity_mask = P_CORE_AFFINITY_MASK;
                } else if profile.primary_core_class == CpuCoreClass::PerformanceCore {
                    // Render and physics scaled down to 2800 MHz to shed thermal load
                    profile.target_freq_mhz = 2800;
                    self.thermal_state.render_throttled_count.fetch_add(1, Ordering::SeqCst);
                } else {
                    // All background tasks quenched to 800 MHz and restricted to E-Cores
                    profile.target_freq_mhz = 800;
                    profile.core_affinity_mask = E_CORE_AFFINITY_MASK;
                    self.thermal_state.background_quenched_count.fetch_add(1, Ordering::SeqCst);
                }
                profile
            }
        }
    }

    /// Queries thermal governor statistics: (current_zone, audio_preserved, render_throttled, background_quenched).
    pub fn get_thermal_governor_stats(&self) -> (ThermalPressureZone, u32, u32, u32) {
        (
            self.thermal_state.current_zone,
            self.thermal_state.audio_input_preserved_count.load(Ordering::SeqCst),
            self.thermal_state.render_throttled_count.load(Ordering::SeqCst),
            self.thermal_state.background_quenched_count.load(Ordering::SeqCst),
        )
    }

    /// Win32 SetProcessInformation dispatch wrapper for ProcessPowerThrottling.
    pub fn win32_set_process_information(
        &mut self,
        process_id: u64,
        info_class: u32,
        state: PROCESS_POWER_THROTTLING_STATE,
    ) -> u32 {
        if info_class != ProcessPowerThrottling {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }

        match self.set_process_power_throttling(process_id, state) {
            Ok(_) => 1,
            Err(_) => {
                win32_set_last_error(ERROR_INVALID_PARAMETER);
                0
            }
        }
    }

    /// Win32 SetThreadInformation dispatch wrapper for ThreadPowerThrottling.
    pub fn win32_set_thread_information(
        &mut self,
        thread_id: u64,
        process_id: u64,
        info_class: u32,
        state: THREAD_POWER_THROTTLING_STATE,
    ) -> u32 {
        if info_class != ThreadPowerThrottling {
            win32_set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }

        match self.set_thread_power_throttling(thread_id, process_id, state) {
            Ok(_) => 1,
            Err(_) => {
                win32_set_last_error(ERROR_INVALID_PARAMETER);
                0
            }
        }
    }
}

/// Global system-wide instance of PowerQosManager.
pub static GLOBAL_POWER_QOS_MANAGER: Spinlock<Option<PowerQosManager>> = Spinlock::new(None);

/// Initializes the Power QoS manager.
pub fn init_power_qos_manager() {
    let mut lock = GLOBAL_POWER_QOS_MANAGER.lock();
    if lock.is_none() {
        *lock = Some(PowerQosManager::new());
    }
}

/// Accessor for global Power QoS manager.
pub fn with_power_qos_manager<F, R>(f: F) -> R
where
    F: FnOnce(&mut PowerQosManager) -> R,
{
    init_power_qos_manager();
    let mut lock = GLOBAL_POWER_QOS_MANAGER.lock();
    f(lock.as_mut().unwrap())
}

/// Public self-test suite validating Win32 Process & Thread Power Throttling API,
/// QoS Execution Levels (EcoQoS/HighQoS), Gaming Core Affinity & Thermal Saturation Prioritization.
pub fn test_win32_power_qos_subsystem() -> bool {
    crate::serial_write_str("[KNT-TEST] Initializing Win32 Power & Thermal QoS self-test suite...\r\n");

    let mut mgr = PowerQosManager::new();

    // 1. Verify Win32 Power Throttling API Configuration
    let proc_state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    };

    if mgr.win32_set_process_information(500, ProcessPowerThrottling, proc_state) == 0 {
        crate::serial_write_str("[KNT-TEST-ERR] Failed to set process power throttling state\r\n");
        return false;
    }

    let read_state = match mgr.get_process_power_throttling(500) {
        Ok(s) => s,
        Err(_) => {
            crate::serial_write_str("[KNT-TEST-ERR] Failed to get process power throttling state\r\n");
            return false;
        }
    };

    if read_state.StateMask != proc_state.StateMask {
        crate::serial_write_str("[KNT-TEST-ERR] Process power throttling state mask mismatch\r\n");
        return false;
    }

    // Thread 1001 overrides process throttling (HighQoS)
    let thread_state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    if mgr.win32_set_thread_information(1001, 500, ThreadPowerThrottling, thread_state) == 0 {
        crate::serial_write_str("[KNT-TEST-ERR] Failed to set thread power throttling state\r\n");
        return false;
    }

    // Thread 1002 inherits process EcoQoS
    let _ = mgr.win32_set_thread_information(
        1002,
        500,
        ThreadPowerThrottling,
        THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: 0,
            StateMask: 0,
        },
    );

    if mgr.determine_thread_qos_level(1001) != QosExecutionLevel::HighQoS {
        crate::serial_write_str("[KNT-TEST-ERR] Thread 1001 did not override process EcoQoS\r\n");
        return false;
    }

    if mgr.determine_thread_qos_level(1002) != QosExecutionLevel::EcoQoS {
        crate::serial_write_str("[KNT-TEST-ERR] Thread 1002 did not inherit process EcoQoS\r\n");
        return false;
    }

    // 2. Verify Core Affinity & Heterogeneous Scheduler Routing
    mgr.set_thread_role(2001, 500, GamingThreadRole::RenderMain);
    mgr.set_thread_role(2002, 500, GamingThreadRole::AudioRealtime);
    mgr.set_thread_role(2003, 500, GamingThreadRole::TelemetryWorker);

    let route_render = mgr.route_thread_scheduling(2001);
    let route_audio = mgr.route_thread_scheduling(2002);
    let route_telem = mgr.route_thread_scheduling(2003);

    // RenderMain on P-Cores with HighQoS
    if route_render.core_affinity_mask != P_CORE_AFFINITY_MASK || route_render.qos_level != QosExecutionLevel::HighQoS {
        crate::serial_write_str("[KNT-TEST-ERR] Render thread not routed to P-Cores with HighQoS\r\n");
        return false;
    }

    // AudioRealtime on P-Cores with throttling immunity
    if !route_audio.is_immune_to_throttling || route_audio.core_affinity_mask != P_CORE_AFFINITY_MASK {
        crate::serial_write_str("[KNT-TEST-ERR] Audio thread not flagged as immune to throttling\r\n");
        return false;
    }

    // TelemetryWorker on E-Cores with EcoQoS
    if route_telem.core_affinity_mask != E_CORE_AFFINITY_MASK || route_telem.qos_level != QosExecutionLevel::EcoQoS {
        crate::serial_write_str("[KNT-TEST-ERR] Telemetry worker not throttled to E-Cores\r\n");
        return false;
    }

    // 3. Verify Background Timer Resolution Throttling & Coalescing
    let eco_timer = mgr.calculate_effective_timer_resolution(1002, 1000);
    if !eco_timer.timer_coalescing_active || eco_timer.effective_resolution_100ns < TIMER_RESOLUTION_DEFAULT_100NS {
        crate::serial_write_str("[KNT-TEST-ERR] EcoQoS timer was not coalesced to default power-saving resolution\r\n");
        return false;
    }

    let high_timer = mgr.calculate_effective_timer_resolution(1001, 5000);
    if high_timer.timer_coalescing_active || high_timer.effective_resolution_100ns != 5000 {
        crate::serial_write_str("[KNT-TEST-ERR] HighQoS high-precision timer was unexpectedly coalesced\r\n");
        return false;
    }

    // 4. Verify Thermal Saturation (>95°C) Dynamic Degradation Policy
    let zone = mgr.update_soc_temperature(98);
    if zone != ThermalPressureZone::ThermalSaturation {
        crate::serial_write_str("[KNT-TEST-ERR] 98°C reading did not enter ThermalSaturation zone\r\n");
        return false;
    }

    let thermal_audio = mgr.route_thread_thermal_scheduling(2002);
    let thermal_render = mgr.route_thread_thermal_scheduling(2001);
    let thermal_telem = mgr.route_thread_thermal_scheduling(2003);

    // Audio MUST be preserved at top speed on P-Cores
    if thermal_audio.target_freq_mhz != HIGH_QOS_MAX_FREQ_MHZ || thermal_audio.core_affinity_mask != P_CORE_AFFINITY_MASK {
        crate::serial_write_str("[KNT-TEST-ERR] Audio thread was improperly degraded under thermal saturation\r\n");
        return false;
    }

    // Render scaled down from 4800 to 2800 MHz
    if thermal_render.target_freq_mhz > 3000 {
        crate::serial_write_str("[KNT-TEST-ERR] Render thread frequency was not throttled under saturation\r\n");
        return false;
    }

    // Telemetry quenched to 800 MHz on E-Cores
    if thermal_telem.target_freq_mhz > 1000 || thermal_telem.core_affinity_mask != E_CORE_AFFINITY_MASK {
        crate::serial_write_str("[KNT-TEST-ERR] Background worker not deeply quenched under saturation\r\n");
        return false;
    }

    let (_, audio_cnt, render_cnt, telem_cnt) = mgr.get_thermal_governor_stats();
    if audio_cnt == 0 || render_cnt == 0 || telem_cnt == 0 {
        crate::serial_write_str("[KNT-TEST-ERR] Thermal governor event counters not updated\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-POWER-QOS-PASS] Win32 Process & Thread Power/Thermal QoS Engine verified\r\n");
    true
}
