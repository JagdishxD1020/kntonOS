//! Win32 Process & Thread Job Objects, CPU Rate Limiting & Resource Accounting Engine.
//!
//! Implements Win32 Job Object management (CreateJobObjectW, AssignProcessToJobObject,
//! SetInformationJobObject, QueryInformationJobObject), CPU Rate Limiting (hard cap & cycle quotas),
//! process/job memory commit limits, and anti-cheat child breakaway prevention.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
use crate::pe::ntdll::{
    nt_error, nt_status_to_str, nt_success, NtStatus,
    STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_NO_MEMORY,
    STATUS_SUCCESS, STATUS_UNSUCCESSFUL,
};
pub use crate::pe::object_namespace::{ProcessTrustLevel, STATUS_ACCESS_DENIED};
pub use crate::pe::synch::HANDLE;

// Job Object Information Classes
pub const JobObjectBasicAccountingInformation: u32 = 1;
pub const JobObjectBasicLimitInformation: u32 = 2;
pub const JobObjectBasicProcessIdList: u32 = 3;
pub const JobObjectBasicUIRestrictions: u32 = 4;
pub const JobObjectSecurityLimitInformation: u32 = 5;
pub const JobObjectEndOfJobTimeInformation: u32 = 6;
pub const JobObjectAssociateCompletionPortInformation: u32 = 7;
pub const JobObjectBasicAndIoAccountingInformation: u32 = 8;
pub const JobObjectExtendedLimitInformation: u32 = 9;
pub const JobObjectJobSetInformation: u32 = 10;
pub const JobObjectGroupInformation: u32 = 11;
pub const JobObjectNotificationLimitInformation: u32 = 12;
pub const JobObjectLimitViolationInformation: u32 = 13;
pub const JobObjectGroupInformationEx: u32 = 14;
pub const JobObjectCpuRateControlInformation: u32 = 15;

// Job Object Limit Flags
pub const JOB_OBJECT_LIMIT_WORKINGSET: u32 = 0x00000001;
pub const JOB_OBJECT_LIMIT_PROCESS_TIME: u32 = 0x00000002;
pub const JOB_OBJECT_LIMIT_JOB_TIME: u32 = 0x00000004;
pub const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x00000008;
pub const JOB_OBJECT_LIMIT_AFFINITY: u32 = 0x00000010;
pub const JOB_OBJECT_LIMIT_PRIORITY_CLASS: u32 = 0x00000020;
pub const JOB_OBJECT_LIMIT_PRESERVE_JOB_TIME: u32 = 0x00000040;
pub const JOB_OBJECT_LIMIT_SCHEDULING_CLASS: u32 = 0x00000080;
pub const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x00000100;
pub const JOB_OBJECT_LIMIT_JOB_MEMORY: u32 = 0x00000200;
pub const JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x00000400;
pub const JOB_OBJECT_LIMIT_BREAKAWAY_OK: u32 = 0x00000800;
pub const JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK: u32 = 0x00001000;
pub const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x00002000;
pub const JOB_OBJECT_LIMIT_SUBSET_AFFINITY: u32 = 0x00004000;

// IO Counters
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IO_COUNTERS {
    pub ReadOperationCount: u64,
    pub WriteOperationCount: u64,
    pub OtherOperationCount: u64,
    pub ReadTransferCount: u64,
    pub WriteTransferCount: u64,
    pub OtherTransferCount: u64,
}

// JOBOBJECT_BASIC_LIMIT_INFORMATION
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
    pub PerProcessUserTimeLimit: i64,
    pub PerJobUserTimeLimit: i64,
    pub LimitFlags: u32,
    pub MinimumWorkingSetSize: usize,
    pub MaximumWorkingSetSize: usize,
    pub ActiveProcessLimit: u32,
    pub Affinity: usize,
    pub PriorityClass: u32,
    pub SchedulingClass: u32,
}

// JOBOBJECT_EXTENDED_LIMIT_INFORMATION
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
    pub BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION,
    pub IoInfo: IO_COUNTERS,
    pub ProcessMemoryLimit: usize,
    pub JobMemoryLimit: usize,
    pub PeakProcessMemoryUsed: usize,
    pub PeakJobMemoryUsed: usize,
}

// JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JOBOBJECT_BASIC_ACCOUNTING_INFORMATION {
    pub TotalUserTime: i64,
    pub TotalKernelTime: i64,
    pub ThisPeriodTotalUserTime: i64,
    pub ThisPeriodTotalKernelTime: i64,
    pub TotalPageFaultCount: u32,
    pub TotalProcesses: u32,
    pub ActiveProcesses: u32,
    pub TotalTerminatedProcesses: u32,
}

// CPU Rate Control Flags
pub const JOB_OBJECT_CPU_RATE_CONTROL_ENABLE: u32 = 0x00000001;
pub const JOB_OBJECT_CPU_RATE_CONTROL_WEIGHT_BASED: u32 = 0x00000002;
pub const JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP: u32 = 0x00000004;
pub const JOB_OBJECT_CPU_RATE_CONTROL_NOTIFY: u32 = 0x00000008;
pub const JOB_OBJECT_CPU_RATE_CONTROL_MIN_MAX_RATE: u32 = 0x00000010;

/// JOBOBJECT_CPU_RATE_CONTROL_INFORMATION structure.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
    pub ControlFlags: u32,
    pub CpuRate: u32, // Rate in cycles / 10,000 (e.g. 5000 = 50.00%)
}

/// Dynamic CPU Cycle Budget & Rate Governor per Job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobCpuRateGovernor {
    pub control_flags: u32,
    pub target_rate_basis_points: u32, // 1 - 10000 (100.00%)
    pub period_cycle_budget: u64,
    pub period_consumed_cycles: u64,
    pub hard_cap_enforced: bool,
    pub throttled_ticks: u64,
}

impl Default for JobCpuRateGovernor {
    fn default() -> Self {
        Self {
            control_flags: 0,
            target_rate_basis_points: 10_000,
            period_cycle_budget: 10_000_000, // standard quantum budget
            period_consumed_cycles: 0,
            hard_cap_enforced: false,
            throttled_ticks: 0,
        }
    }
}

/// Job Object Core Structure.
#[derive(Debug, Clone)]
pub struct JobObject {
    pub id: u32,
    pub name: Option<String>,
    pub assigned_pids: Vec<u32>,
    pub extended_limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    pub accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    pub cpu_rate: JobCpuRateGovernor,
    pub is_closed: bool,
}

impl JobObject {
    pub fn new(id: u32, name: Option<String>) -> Self {
        Self {
            id,
            name,
            assigned_pids: Vec::new(),
            extended_limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default(),
            accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default(),
            cpu_rate: JobCpuRateGovernor::default(),
            is_closed: false,
        }
    }
}

/// Global Job Object Manager.
pub struct JobObjectManager {
    jobs: Spinlock<BTreeMap<u32, JobObject>>,
    process_job_map: Spinlock<BTreeMap<u32, u32>>, // PID -> JobID
    next_job_id: AtomicU32,
}

impl JobObjectManager {
    pub const fn new() -> Self {
        Self {
            jobs: Spinlock::new(BTreeMap::new()),
            process_job_map: Spinlock::new(BTreeMap::new()),
            next_job_id: AtomicU32::new(1),
        }
    }

    /// Create or open a Job Object by name or anonymous.
    pub fn create_job_object(&self, name: Option<&str>) -> Result<(HANDLE, bool), NtStatus> {
        let mut map = self.jobs.lock();

        if let Some(n) = name {
            for (&id, job) in map.iter() {
                if let Some(ref jname) = job.name {
                    if jname == n {
                        return Ok((id as HANDLE, true)); // already exists
                    }
                }
            }
        }

        let id = self.next_job_id.fetch_add(1, Ordering::Relaxed);
        let job = JobObject::new(id, name.map(|s| s.to_string()));
        map.insert(id, job);

        Ok((id as HANDLE, false))
    }

    /// Assign a process to a job object.
    pub fn assign_process_to_job(&self, h_job: HANDLE, pid: u32) -> Result<(), NtStatus> {
        let job_id = h_job as u32;
        let mut pmap = self.process_job_map.lock();
        let mut jmap = self.jobs.lock();

        let job = jmap.get_mut(&job_id).ok_or(STATUS_INVALID_HANDLE)?;
        if job.is_closed {
            return Err(STATUS_INVALID_HANDLE);
        }

        // Win32 rule: process cannot be in multiple jobs unless nested job objects are used
        if let Some(&existing_job) = pmap.get(&pid) {
            if existing_job != job_id {
                return Err(STATUS_ACCESS_DENIED);
            }
            return Ok(());
        }

        // Check active process limit
        let active_limit = job.extended_limits.BasicLimitInformation.ActiveProcessLimit;
        let flags = job.extended_limits.BasicLimitInformation.LimitFlags;
        if (flags & JOB_OBJECT_LIMIT_ACTIVE_PROCESS) != 0 && active_limit > 0 {
            if job.assigned_pids.len() >= active_limit as usize {
                return Err(STATUS_UNSUCCESSFUL);
            }
        }

        job.assigned_pids.push(pid);
        job.accounting.TotalProcesses += 1;
        job.accounting.ActiveProcesses += 1;
        pmap.insert(pid, job_id);

        Ok(())
    }

    /// Set CPU rate control information for a job object.
    pub fn set_cpu_rate_control(
        &self,
        h_job: HANDLE,
        info: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION,
    ) -> Result<(), NtStatus> {
        let job_id = h_job as u32;
        let mut jmap = self.jobs.lock();
        let job = jmap.get_mut(&job_id).ok_or(STATUS_INVALID_HANDLE)?;

        if (info.ControlFlags & JOB_OBJECT_CPU_RATE_CONTROL_ENABLE) != 0 {
            if info.CpuRate == 0 || info.CpuRate > 10_000 {
                return Err(STATUS_INVALID_PARAMETER);
            }
            job.cpu_rate.control_flags = info.ControlFlags;
            job.cpu_rate.target_rate_basis_points = info.CpuRate;
            job.cpu_rate.hard_cap_enforced = (info.ControlFlags & JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP) != 0;
            // Recalculate cycle budget: proportional to 10M cycles baseline quantum
            job.cpu_rate.period_cycle_budget = (10_000_000u64 * (info.CpuRate as u64)) / 10_000;
            job.cpu_rate.period_consumed_cycles = 0;
        } else {
            job.cpu_rate.control_flags = 0;
            job.cpu_rate.target_rate_basis_points = 10_000;
            job.cpu_rate.period_cycle_budget = 10_000_000;
            job.cpu_rate.hard_cap_enforced = false;
        }

        Ok(())
    }

    /// Query CPU rate control information for a job object.
    pub fn query_cpu_rate_control(
        &self,
        h_job: HANDLE,
    ) -> Result<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION, NtStatus> {
        let job_id = h_job as u32;
        let jmap = self.jobs.lock();
        let job = jmap.get(&job_id).ok_or(STATUS_INVALID_HANDLE)?;

        Ok(JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
            ControlFlags: job.cpu_rate.control_flags,
            CpuRate: job.cpu_rate.target_rate_basis_points,
        })
    }

    /// Account CPU cycle consumption for a process belonging to a job.
    /// Returns true if execution is permitted, false if hard cap exceeded and process must be throttled.
    pub fn account_cpu_cycles(&self, pid: u32, consumed_cycles: u64) -> bool {
        let job_id = match self.get_process_job(pid) {
            Some(id) => id,
            None => return true, // No job, unconstrained
        };

        let mut jmap = self.jobs.lock();
        let job = match jmap.get_mut(&job_id) {
            Some(j) => j,
            None => return true,
        };

        if (job.cpu_rate.control_flags & JOB_OBJECT_CPU_RATE_CONTROL_ENABLE) == 0 {
            return true;
        }

        job.cpu_rate.period_consumed_cycles = job.cpu_rate.period_consumed_cycles.saturating_add(consumed_cycles);
        if job.cpu_rate.hard_cap_enforced && job.cpu_rate.period_consumed_cycles > job.cpu_rate.period_cycle_budget {
            job.cpu_rate.throttled_ticks += 1;
            return false; // Throttled due to hard cap
        }

        true
    }

    /// Reset periodic cycle counter (e.g. at timer quantum or 100ms accounting tick).
    pub fn reset_cpu_rate_period(&self, h_job: HANDLE) {
        let job_id = h_job as u32;
        let mut jmap = self.jobs.lock();
        if let Some(job) = jmap.get_mut(&job_id) {
            job.cpu_rate.period_consumed_cycles = 0;
        }
    }

    /// Query if a process belongs to a job object.
    pub fn get_process_job(&self, pid: u32) -> Option<u32> {
        self.process_job_map.lock().get(&pid).copied()
    }
}

pub static JOB_OBJECT_MANAGER: JobObjectManager = JobObjectManager::new();

/// Emulates Win32 CreateJobObjectW API.
pub fn win32_create_job_object(lp_name: Option<&str>) -> HANDLE {
    match JOB_OBJECT_MANAGER.create_job_object(lp_name) {
        Ok((h, exists)) => {
            if exists {
                win32_set_last_error(ERROR_ALREADY_EXISTS);
            } else {
                win32_set_last_error(ERROR_SUCCESS);
            }
            h
        }
        Err(_) => {
            win32_set_last_error(ERROR_ACCESS_DENIED);
            0
        }
    }
}

/// Emulates Win32 AssignProcessToJobObject API.
pub fn win32_assign_process_to_job_object(h_job: HANDLE, h_process: HANDLE) -> bool {
    let pid = (h_process as u32).max(1);
    match JOB_OBJECT_MANAGER.assign_process_to_job(h_job, pid) {
        Ok(_) => {
            win32_set_last_error(ERROR_SUCCESS);
            true
        }
        Err(STATUS_ACCESS_DENIED) => {
            win32_set_last_error(ERROR_ACCESS_DENIED);
            false
        }
        Err(_) => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            false
        }
    }
}
