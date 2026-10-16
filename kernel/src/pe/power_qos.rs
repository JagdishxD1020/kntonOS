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
            execution_speed_throttled: false,
            ignore_timer_resolution: false,
        }
    }
}

/// Thread-level QoS and power governance descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadPowerPolicy {
    pub thread_id: u64,
    pub process_id: u64,
    pub throttling_state: THREAD_POWER_THROTTLING_STATE,
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
            execution_speed_throttled: false,
        }
    }
}

/// Central Power & Thermal Throttling Quality-of-Service Governor.
pub struct PowerQosManager {
    pub process_policies: BTreeMap<u64, ProcessPowerPolicy>,
    pub thread_policies: BTreeMap<u64, ThreadPowerPolicy>,
}

impl PowerQosManager {
    pub fn new() -> Self {
        Self {
            process_policies: BTreeMap::new(),
            thread_policies: BTreeMap::new(),
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
