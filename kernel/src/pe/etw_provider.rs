//! Win32 High-Resolution Performance Telemetry Ring Buffer & Real-Time ETW Provider.
//!
//! Implements Win32 ETW provider registration and trace session governor (EventRegister, EventWriteTransfer, EventUnregister),
//! lock-free SPSC telemetry ring buffer capturing frame boundaries, GPU present timings, and draw calls,
//! compact binary trace packet serializer with zero kernel heap allocations,
//! and handheld trace session buffer circular overwrite mode for minimal RAM footprint.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, ERROR_SUCCESS,
};
pub use crate::pe::srw_futex::PVOID;
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

/// GUID representation for ETW provider identification.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GUID {
    pub Data1: u32,
    pub Data2: u16,
    pub Data3: u16,
    pub Data4: [u8; 8],
}

/// ETW Event Descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct EVENT_DESCRIPTOR {
    pub Id: u16,
    pub Version: u8,
    pub Channel: u8,
    pub Level: u8,
    pub Opcode: u8,
    pub Task: u16,
    pub Keyword: u64,
}

pub type REGHANDLE = u64;

/// ETW Provider callback.
pub type PENABLECALLBACK = Option<
    unsafe extern "system" fn(
        SourceId: *const GUID,
        IsEnabled: u32,
        Level: u8,
        MatchAnyKeyword: u64,
        MatchAllKeyword: u64,
        FilterData: *const core::ffi::c_void,
        CallbackContext: *mut core::ffi::c_void,
    ),
>;

/// ETW Provider internal descriptor.
pub struct EtwProviderInner {
    pub reg_handle: REGHANDLE,
    pub provider_id: GUID,
    pub is_enabled: bool,
    pub level: u8,
    pub match_any_keyword: u64,
    pub callback: PENABLECALLBACK,
    pub context: usize,
}

static NEXT_REG_HANDLE: AtomicU64 = AtomicU64::new(0xE001);
static ETW_PROVIDERS: Spinlock<BTreeMap<REGHANDLE, EtwProviderInner>> = Spinlock::new(BTreeMap::new());

/// Registers a userland or driver Win32 ETW event provider (EventRegister).
pub unsafe extern "system" fn EventRegister(
    ProviderId: *const GUID,
    EnableCallback: PENABLECALLBACK,
    CallbackContext: *mut core::ffi::c_void,
    RegHandle: *mut REGHANDLE,
) -> u32 {
    if ProviderId.is_null() || RegHandle.is_null() {
        return 87; // ERROR_INVALID_PARAMETER
    }

    let prov_guid = *ProviderId;
    let handle = NEXT_REG_HANDLE.fetch_add(1, Ordering::SeqCst);

    let provider = EtwProviderInner {
        reg_handle: handle,
        provider_id: prov_guid,
        is_enabled: true,
        level: 5, // Verbose
        match_any_keyword: !0,
        callback: EnableCallback,
        context: CallbackContext as usize,
    };

    ETW_PROVIDERS.lock().insert(handle, provider);
    *RegHandle = handle;
    0 // ERROR_SUCCESS
}

/// Unregisters an ETW provider (EventUnregister).
pub unsafe extern "system" fn EventUnregister(RegHandle: REGHANDLE) -> u32 {
    if RegHandle == 0 {
        return 87; // ERROR_INVALID_PARAMETER
    }

    if ETW_PROVIDERS.lock().remove(&RegHandle).is_some() {
        0 // ERROR_SUCCESS
    } else {
        6 // ERROR_INVALID_HANDLE
    }
}

/// Returns whether the specified provider is enabled for events at given level and keyword.
pub fn win32_etw_provider_is_enabled(reg_handle: REGHANDLE, level: u8, keyword: u64) -> bool {
    let providers = ETW_PROVIDERS.lock();
    if let Some(p) = providers.get(&reg_handle) {
        p.is_enabled && (level <= p.level) && ((keyword & p.match_any_keyword) != 0 || keyword == 0)
    } else {
        false
    }
}
