//! Win32 Access Tokens, Security Descriptors & Privilege Mask Governor.
//!
//! Implements Windows NT access control models, security identifiers (SIDs),
//! access control lists (ACLs/ACEs), security descriptors, access tokens,
//! and process/mod integrity level isolation for gaming runtimes.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_imports, dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{
    win32_set_last_error, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE,
    ERROR_INVALID_PARAMETER, ERROR_SUCCESS, INVALID_HANDLE_VALUE,
};

pub use crate::pe::peb_teb::ERROR_INSUFFICIENT_BUFFER;
pub const ERROR_NO_TOKEN: u32 = 1008;
pub const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;
pub use crate::pe::synch::HANDLE;
pub use crate::pe::threadpool::{BOOL, FALSE, TRUE};

pub type DWORD = u32;
pub type WORD = u16;
pub type BYTE = u8;
pub type ACCESS_MASK = u32;
pub type REGSAM = u32;

// Standard and generic access rights
pub const DELETE: ACCESS_MASK = 0x0001_0000;
pub const READ_CONTROL: ACCESS_MASK = 0x0002_0000;
pub const WRITE_DAC: ACCESS_MASK = 0x0004_0000;
pub const WRITE_OWNER: ACCESS_MASK = 0x0008_0000;
pub const SYNCHRONIZE: ACCESS_MASK = 0x0010_0000;

pub const STANDARD_RIGHTS_REQUIRED: ACCESS_MASK = 0x000F_0000;
pub const STANDARD_RIGHTS_READ: ACCESS_MASK = READ_CONTROL;
pub const STANDARD_RIGHTS_WRITE: ACCESS_MASK = READ_CONTROL;
pub const STANDARD_RIGHTS_EXECUTE: ACCESS_MASK = READ_CONTROL;
pub const STANDARD_RIGHTS_ALL: ACCESS_MASK = 0x001F_0000;

pub const SPECIFIC_RIGHTS_ALL: ACCESS_MASK = 0x0000_FFFF;

pub use crate::pe::file::{GENERIC_ALL, GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE};

// Token specific access rights
pub const TOKEN_ASSIGN_PRIMARY: ACCESS_MASK = 0x0001;
pub const TOKEN_DUPLICATE: ACCESS_MASK = 0x0002;
pub const TOKEN_IMPERSONATE: ACCESS_MASK = 0x0004;
pub const TOKEN_QUERY: ACCESS_MASK = 0x0008;
pub const TOKEN_QUERY_SOURCE: ACCESS_MASK = 0x0010;
pub const TOKEN_ADJUST_PRIVILEGES: ACCESS_MASK = 0x0020;
pub const TOKEN_ADJUST_GROUPS: ACCESS_MASK = 0x0040;
pub const TOKEN_ADJUST_DEFAULT: ACCESS_MASK = 0x0080;
pub const TOKEN_ADJUST_SESSIONID: ACCESS_MASK = 0x0100;
pub const TOKEN_ALL_ACCESS: ACCESS_MASK = STANDARD_RIGHTS_REQUIRED | 0x01FF;
pub const TOKEN_READ: ACCESS_MASK = STANDARD_RIGHTS_READ | TOKEN_QUERY;
pub const TOKEN_WRITE: ACCESS_MASK = STANDARD_RIGHTS_WRITE | TOKEN_ADJUST_PRIVILEGES | TOKEN_ADJUST_GROUPS | TOKEN_ADJUST_DEFAULT;
pub const TOKEN_EXECUTE: ACCESS_MASK = STANDARD_RIGHTS_EXECUTE;

// SID Authority identifiers
pub const SECURITY_NULL_SID_AUTHORITY: [BYTE; 6] = [0, 0, 0, 0, 0, 0];
pub const SECURITY_WORLD_SID_AUTHORITY: [BYTE; 6] = [0, 0, 0, 0, 0, 1];
pub const SECURITY_LOCAL_SID_AUTHORITY: [BYTE; 6] = [0, 0, 0, 0, 0, 2];
pub const SECURITY_CREATOR_SID_AUTHORITY: [BYTE; 6] = [0, 0, 0, 0, 0, 3];
pub const SECURITY_NON_UNIQUE_AUTHORITY: [BYTE; 6] = [0, 0, 0, 0, 0, 4];
pub const SECURITY_NT_AUTHORITY: [BYTE; 6] = [0, 0, 0, 0, 0, 5];
pub const SECURITY_MANDATORY_LABEL_AUTHORITY: [BYTE; 6] = [0, 0, 0, 0, 0, 16];

// Well-known Relative IDs (RIDs)
pub const SECURITY_NULL_RID: DWORD = 0x0000_0000;
pub const SECURITY_WORLD_RID: DWORD = 0x0000_0000;
pub const SECURITY_LOCAL_RID: DWORD = 0x0000_0000;
pub const SECURITY_BUILTIN_DOMAIN_RID: DWORD = 0x0000_0020;

// NT Authority RIDs
pub const SECURITY_LOCAL_SYSTEM_RID: DWORD = 0x0000_0012;
pub const SECURITY_LOCAL_SERVICE_RID: DWORD = 0x0000_0013;
pub const SECURITY_NETWORK_SERVICE_RID: DWORD = 0x0000_0014;
pub const SECURITY_BUILTIN_ADMINISTRATORS_RID: DWORD = 0x0000_0220;
pub const SECURITY_BUILTIN_USERS_RID: DWORD = 0x0000_0221;
pub const SECURITY_BUILTIN_GUESTS_RID: DWORD = 0x0000_0222;

// Mandatory integrity label RIDs
pub const SECURITY_MANDATORY_UNTRUSTED_RID: DWORD = 0x0000_0000;
pub const SECURITY_MANDATORY_LOW_RID: DWORD = 0x0000_1000;
pub const SECURITY_MANDATORY_MEDIUM_RID: DWORD = 0x0000_2000;
pub const SECURITY_MANDATORY_MEDIUM_PLUS_RID: DWORD = 0x0000_2100;
pub const SECURITY_MANDATORY_HIGH_RID: DWORD = 0x0000_3000;
pub const SECURITY_MANDATORY_SYSTEM_RID: DWORD = 0x0000_4000;
pub const SECURITY_MANDATORY_PROTECTED_PROCESS_RID: DWORD = 0x0000_5000;

// ACE Types
pub const ACCESS_ALLOWED_ACE_TYPE: BYTE = 0x00;
pub const ACCESS_DENIED_ACE_TYPE: BYTE = 0x01;
pub const SYSTEM_AUDIT_ACE_TYPE: BYTE = 0x02;
pub const SYSTEM_ALARM_ACE_TYPE: BYTE = 0x03;
pub const SYSTEM_MANDATORY_LABEL_ACE_TYPE: BYTE = 0x11;

// ACE Flags
pub const OBJECT_INHERIT_ACE: BYTE = 0x01;
pub const CONTAINER_INHERIT_ACE: BYTE = 0x02;
pub const NO_PROPAGATE_INHERIT_ACE: BYTE = 0x04;
pub const INHERIT_ONLY_ACE: BYTE = 0x08;
pub const INHERITED_ACE: BYTE = 0x10;
pub const SUCCESSFUL_ACCESS_ACE_FLAG: BYTE = 0x40;
pub const FAILED_ACCESS_ACE_FLAG: BYTE = 0x80;

// Security Descriptor Control
pub const SE_OWNER_DEFAULTED: WORD = 0x0001;
pub const SE_GROUP_DEFAULTED: WORD = 0x0002;
pub const SE_DACL_PRESENT: WORD = 0x0004;
pub const SE_DACL_DEFAULTED: WORD = 0x0008;
pub const SE_SACL_PRESENT: WORD = 0x0010;
pub const SE_SACL_DEFAULTED: WORD = 0x0020;
pub const SE_DACL_AUTO_INHERIT_REQ: WORD = 0x0100;
pub const SE_SACL_AUTO_INHERIT_REQ: WORD = 0x0200;
pub const SE_DACL_AUTO_INHERITED: WORD = 0x0400;
pub const SE_SACL_AUTO_INHERITED: WORD = 0x0800;
pub const SE_DACL_PROTECTED: WORD = 0x1000;
pub const SE_SACL_PROTECTED: WORD = 0x2000;
pub const SE_SELF_RELATIVE: WORD = 0x8000;

pub const SID_REVISION: BYTE = 1;
pub const ACL_REVISION: BYTE = 2;
pub const SECURITY_DESCRIPTOR_REVISION: BYTE = 1;

/// Windows NT Security Identifier (SID).
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SID {
    pub revision: BYTE,
    pub sub_authority_count: BYTE,
    pub identifier_authority: [BYTE; 6],
    pub sub_authorities: Vec<DWORD>,
}

impl SID {
    pub fn new(authority: [BYTE; 6], sub_authorities: &[DWORD]) -> Self {
        Self {
            revision: SID_REVISION,
            sub_authority_count: sub_authorities.len() as BYTE,
            identifier_authority: authority,
            sub_authorities: sub_authorities.to_vec(),
        }
    }

    pub fn world_sid() -> Self {
        Self::new(SECURITY_WORLD_SID_AUTHORITY, &[SECURITY_WORLD_RID])
    }

    pub fn local_system_sid() -> Self {
        Self::new(SECURITY_NT_AUTHORITY, &[SECURITY_LOCAL_SYSTEM_RID])
    }

    pub fn administrators_sid() -> Self {
        Self::new(
            SECURITY_NT_AUTHORITY,
            &[SECURITY_BUILTIN_DOMAIN_RID, SECURITY_BUILTIN_ADMINISTRATORS_RID],
        )
    }

    pub fn users_sid() -> Self {
        Self::new(
            SECURITY_NT_AUTHORITY,
            &[SECURITY_BUILTIN_DOMAIN_RID, SECURITY_BUILTIN_USERS_RID],
        )
    }

    pub fn integrity_sid(rid: DWORD) -> Self {
        Self::new(SECURITY_MANDATORY_LABEL_AUTHORITY, &[rid])
    }

    pub fn to_string_sid(&self) -> String {
        let mut s = String::from("S-1-");
        let mut auth_val: u64 = 0;
        for &b in &self.identifier_authority {
            auth_val = (auth_val << 8) | (b as u64);
        }
        use alloc::format;
        s.push_str(&format!("{}", auth_val));
        for &sub in &self.sub_authorities {
            s.push_str(&format!("-{}", sub));
        }
        s
    }
}

/// Access Control Entry (ACE).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ACE {
    pub ace_type: BYTE,
    pub ace_flags: BYTE,
    pub mask: ACCESS_MASK,
    pub sid: SID,
}

/// Access Control List (ACL).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ACL {
    pub revision: BYTE,
    pub aces: Vec<ACE>,
}

impl ACL {
    pub fn new() -> Self {
        Self {
            revision: ACL_REVISION,
            aces: Vec::new(),
        }
    }

    pub fn add_allowed_ace(&mut self, mask: ACCESS_MASK, sid: SID) {
        self.aces.push(ACE {
            ace_type: ACCESS_ALLOWED_ACE_TYPE,
            ace_flags: 0,
            mask,
            sid,
        });
    }

    pub fn add_denied_ace(&mut self, mask: ACCESS_MASK, sid: SID) {
        self.aces.push(ACE {
            ace_type: ACCESS_DENIED_ACE_TYPE,
            ace_flags: 0,
            mask,
            sid,
        });
    }
}

/// Security Descriptor encapsulating Owner, Group, DACL, and SACL.
#[derive(Clone, Debug)]
pub struct SecurityDescriptor {
    pub revision: BYTE,
    pub control: WORD,
    pub owner: Option<SID>,
    pub group: Option<SID>,
    pub dacl: Option<ACL>,
    pub sacl: Option<ACL>,
}

impl Default for SecurityDescriptor {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityDescriptor {
    pub fn new() -> Self {
        Self {
            revision: SECURITY_DESCRIPTOR_REVISION,
            control: 0,
            owner: None,
            group: None,
            dacl: None,
            sacl: None,
        }
    }

    pub fn set_owner(&mut self, owner: SID, defaulted: bool) {
        self.owner = Some(owner);
        if defaulted {
            self.control |= SE_OWNER_DEFAULTED;
        } else {
            self.control &= !SE_OWNER_DEFAULTED;
        }
    }

    pub fn set_dacl(&mut self, dacl: Option<ACL>, defaulted: bool) {
        if let Some(d) = dacl {
            self.dacl = Some(d);
            self.control |= SE_DACL_PRESENT;
            if defaulted {
                self.control |= SE_DACL_DEFAULTED;
            } else {
                self.control &= !SE_DACL_DEFAULTED;
            }
        } else {
            self.dacl = None;
            self.control &= !SE_DACL_PRESENT;
        }
    }

    /// Evaluates if a client SID set satisfies the requested access mask against the DACL.
    pub fn evaluate_access(&self, client_sids: &[SID], requested_access: ACCESS_MASK) -> bool {
        // If no DACL is present, access is granted implicitly (NULL DACL)
        if (self.control & SE_DACL_PRESENT) == 0 || self.dacl.is_none() {
            return true;
        }

        let dacl = self.dacl.as_ref().unwrap();
        // An empty DACL explicitly denies all access
        if dacl.aces.is_empty() {
            return false;
        }

        let mut remaining_access = requested_access;

        for ace in &dacl.aces {
            let sid_match = client_sids.iter().any(|s| s == &ace.sid);
            if !sid_match {
                continue;
            }

            if ace.ace_type == ACCESS_DENIED_ACE_TYPE {
                if (remaining_access & ace.mask) != 0 {
                    return false;
                }
            } else if ace.ace_type == ACCESS_ALLOWED_ACE_TYPE {
                remaining_access &= !ace.mask;
                if remaining_access == 0 {
                    return true;
                }
            }
        }

        remaining_access == 0
    }
}

// Privilege attributes
pub const SE_PRIVILEGE_ENABLED_BY_DEFAULT: DWORD = 0x0000_0001;
pub const SE_PRIVILEGE_ENABLED: DWORD = 0x0000_0002;
pub const SE_PRIVILEGE_REMOVED: DWORD = 0x0000_0004;
pub const SE_PRIVILEGE_USED_FOR_ACCESS: DWORD = 0x8000_0000;

// Token Information Classes
pub const TokenUser: DWORD = 1;
pub const TokenGroups: DWORD = 2;
pub const TokenPrivileges: DWORD = 3;
pub const TokenOwner: DWORD = 4;
pub const TokenPrimaryGroup: DWORD = 5;
pub const TokenDefaultDacl: DWORD = 6;
pub const TokenSource: DWORD = 7;
pub const TokenType: DWORD = 8;
pub const TokenImpersonationLevel: DWORD = 9;
pub const TokenStatistics: DWORD = 10;
pub const TokenRestrictedSids: DWORD = 11;
pub const TokenSessionId: DWORD = 12;
pub const TokenGroupsAndPrivileges: DWORD = 13;
pub const TokenSessionReference: DWORD = 14;
pub const TokenSandBoxInert: DWORD = 15;
pub const TokenAuditPolicy: DWORD = 16;
pub const TokenOrigin: DWORD = 17;
pub const TokenElevationType: DWORD = 18;
pub const TokenLinkedToken: DWORD = 19;
pub const TokenElevation: DWORD = 20;
pub const TokenHasRestrictions: DWORD = 21;
pub const TokenAccessInformation: DWORD = 22;
pub const TokenVirtualizationAllowed: DWORD = 23;
pub const TokenVirtualizationEnabled: DWORD = 24;
pub const TokenIntegrityLevel: DWORD = 25;
pub const TokenUIAccess: DWORD = 26;
pub const TokenMandatoryPolicy: DWORD = 27;
pub const TokenLogonSid: DWORD = 28;

// Token Elevation Types
pub const TokenElevationTypeDefault: DWORD = 1;
pub const TokenElevationTypeFull: DWORD = 2;
pub const TokenElevationTypeLimited: DWORD = 3;

// Token Types
pub const TokenPrimary: DWORD = 1;
pub const TokenImpersonation: DWORD = 2;

// Security Impersonation Levels
pub const SecurityAnonymous: DWORD = 0;
pub const SecurityIdentification: DWORD = 1;
pub const SecurityImpersonation: DWORD = 2;
pub const SecurityDelegation: DWORD = 3;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LUID {
    pub LowPart: DWORD,
    pub HighPart: i32,
}

impl LUID {
    pub const fn new(low: DWORD, high: i32) -> Self {
        Self {
            LowPart: low,
            HighPart: high,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LUID_AND_ATTRIBUTES {
    pub Luid: LUID,
    pub Attributes: DWORD,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TOKEN_ELEVATION {
    pub TokenIsElevated: DWORD,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TOKEN_ELEVATION_TYPE {
    pub ElevationType: DWORD,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TOKEN_MANDATORY_LABEL {
    pub Label: SID_AND_ATTRIBUTES,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SID_AND_ATTRIBUTES {
    pub Sid: *mut c_void,
    pub Attributes: DWORD,
}

/// In-memory representation of a Win32 Access Token.
#[derive(Clone, Debug)]
pub struct AccessTokenInner {
    pub id: HANDLE,
    pub token_type: DWORD,
    pub impersonation_level: DWORD,
    pub user_sid: SID,
    pub group_sids: Vec<SID>,
    pub privileges: BTreeMap<u64, DWORD>,
    pub integrity_level: DWORD,
    pub is_elevated: bool,
    pub default_dacl: Option<ACL>,
}

impl AccessTokenInner {
    pub fn new_default_user(id: HANDLE) -> Self {
        let mut privileges = BTreeMap::new();
        // Give basic default privileges
        privileges.insert(SE_CHANGE_NOTIFY_PRIVILEGE_LUID, SE_PRIVILEGE_ENABLED_BY_DEFAULT | SE_PRIVILEGE_ENABLED);

        Self {
            id,
            token_type: TokenPrimary,
            impersonation_level: SecurityImpersonation,
            user_sid: SID::users_sid(),
            group_sids: alloc::vec![SID::users_sid(), SID::world_sid()],
            privileges,
            integrity_level: SECURITY_MANDATORY_MEDIUM_RID,
            is_elevated: false,
            default_dacl: None,
        }
    }

    pub fn new_admin(id: HANDLE) -> Self {
        let mut privileges = BTreeMap::new();
        privileges.insert(SE_CHANGE_NOTIFY_PRIVILEGE_LUID, SE_PRIVILEGE_ENABLED_BY_DEFAULT | SE_PRIVILEGE_ENABLED);
        privileges.insert(SE_DEBUG_PRIVILEGE_LUID, 0); // Disabled by default, can be enabled
        privileges.insert(SE_LOCK_MEMORY_PRIVILEGE_LUID, 0);

        Self {
            id,
            token_type: TokenPrimary,
            impersonation_level: SecurityImpersonation,
            user_sid: SID::administrators_sid(),
            group_sids: alloc::vec![SID::administrators_sid(), SID::users_sid(), SID::world_sid()],
            privileges,
            integrity_level: SECURITY_MANDATORY_HIGH_RID,
            is_elevated: true,
            default_dacl: None,
        }
    }

    pub fn new_system(id: HANDLE) -> Self {
        let mut privileges = BTreeMap::new();
        privileges.insert(SE_CHANGE_NOTIFY_PRIVILEGE_LUID, SE_PRIVILEGE_ENABLED_BY_DEFAULT | SE_PRIVILEGE_ENABLED);
        privileges.insert(SE_DEBUG_PRIVILEGE_LUID, SE_PRIVILEGE_ENABLED);
        privileges.insert(SE_LOCK_MEMORY_PRIVILEGE_LUID, SE_PRIVILEGE_ENABLED);

        Self {
            id,
            token_type: TokenPrimary,
            impersonation_level: SecurityImpersonation,
            user_sid: SID::local_system_sid(),
            group_sids: alloc::vec![SID::local_system_sid(), SID::administrators_sid(), SID::world_sid()],
            privileges,
            integrity_level: SECURITY_MANDATORY_SYSTEM_RID,
            is_elevated: true,
            default_dacl: None,
        }
    }
}

pub const SE_CHANGE_NOTIFY_PRIVILEGE_LUID: u64 = 23;
pub const SE_DEBUG_PRIVILEGE_LUID: u64 = 20;
pub const SE_LOCK_MEMORY_PRIVILEGE_LUID: u64 = 4;

static NEXT_TOKEN_ID: AtomicU64 = AtomicU64::new(0x2000);
static ACCESS_TOKENS: Spinlock<BTreeMap<HANDLE, AccessTokenInner>> = Spinlock::new(BTreeMap::new());
static PROCESS_TOKENS: Spinlock<BTreeMap<u32, HANDLE>> = Spinlock::new(BTreeMap::new());
static THREAD_TOKENS: Spinlock<BTreeMap<u32, HANDLE>> = Spinlock::new(BTreeMap::new());

fn luid_to_u64(luid: LUID) -> u64 {
    ((luid.HighPart as u64) << 32) | (luid.LowPart as u64)
}

fn u64_to_luid(val: u64) -> LUID {
    LUID {
        LowPart: val as DWORD,
        HighPart: (val >> 32) as i32,
    }
}

/// Helper to get current execution process ID.
fn get_current_process_id() -> u32 {
    let teb = crate::pe::peb_teb::win32_get_current_teb();
    if !teb.is_null() {
        let pid = unsafe { (*teb).client_id.unique_process as u32 };
        if pid != 0 {
            return pid;
        }
    }
    1
}

/// Helper to get current execution thread ID.
fn get_current_thread_id() -> u32 {
    let teb = crate::pe::peb_teb::win32_get_current_teb();
    if !teb.is_null() {
        let tid = unsafe { (*teb).client_id.unique_thread as u32 };
        if tid != 0 {
            return tid;
        }
    }
    1
}

/// Opens the access token associated with a process.
pub unsafe extern "system" fn OpenProcessToken(
    ProcessHandle: HANDLE,
    _DesiredAccess: ACCESS_MASK,
    TokenHandle: *mut HANDLE,
) -> BOOL {
    if TokenHandle.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let pid = if ProcessHandle == 0 || ProcessHandle == (INVALID_HANDLE_VALUE as HANDLE) || ProcessHandle == 0xFFFFFFFF {
        get_current_process_id()
    } else {
        ProcessHandle as u32
    };

    let mut proc_tokens = PROCESS_TOKENS.lock();
    let token_id = if let Some(&tid) = proc_tokens.get(&pid) {
        tid
    } else {
        let new_id = NEXT_TOKEN_ID.fetch_add(1, Ordering::SeqCst) as HANDLE;
        let token = AccessTokenInner::new_admin(new_id);
        ACCESS_TOKENS.lock().insert(new_id, token);
        proc_tokens.insert(pid, new_id);
        new_id
    };

    *TokenHandle = token_id;
    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

/// Opens the access token associated with a thread.
pub unsafe extern "system" fn OpenThreadToken(
    ThreadHandle: HANDLE,
    DesiredAccess: ACCESS_MASK,
    _OpenAsSelf: BOOL,
    TokenHandle: *mut HANDLE,
) -> BOOL {
    if TokenHandle.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let tid = if ThreadHandle == 0 || ThreadHandle == (INVALID_HANDLE_VALUE as HANDLE) || ThreadHandle == 0xFFFFFFFE {
        get_current_thread_id()
    } else {
        ThreadHandle as u32
    };

    let thread_tokens = THREAD_TOKENS.lock();
    if let Some(&token_id) = thread_tokens.get(&tid) {
        *TokenHandle = token_id;
        win32_set_last_error(ERROR_SUCCESS);
        TRUE
    } else {
        // Fallback to process token if no thread impersonation token is set
        drop(thread_tokens);
        OpenProcessToken(0, DesiredAccess, TokenHandle)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TOKEN_PRIVILEGES {
    pub PrivilegeCount: DWORD,
    pub Privileges: [LUID_AND_ATTRIBUTES; 1],
}

/// Retrieves specified type of information about an access token.
pub unsafe extern "system" fn GetTokenInformation(
    TokenHandle: HANDLE,
    TokenInformationClass: DWORD,
    TokenInformation: *mut c_void,
    TokenInformationLength: DWORD,
    ReturnLength: *mut DWORD,
) -> BOOL {
    let tokens = ACCESS_TOKENS.lock();
    let token = match tokens.get(&TokenHandle) {
        Some(t) => t,
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }
    };

    match TokenInformationClass {
        TokenElevation => {
            let req_len = core::mem::size_of::<TOKEN_ELEVATION>() as DWORD;
            if !ReturnLength.is_null() {
                *ReturnLength = req_len;
            }
            if TokenInformation.is_null() || TokenInformationLength < req_len {
                win32_set_last_error(ERROR_INSUFFICIENT_BUFFER);
                return FALSE;
            }
            let elev_ptr = TokenInformation as *mut TOKEN_ELEVATION;
            (*elev_ptr).TokenIsElevated = if token.is_elevated { 1 } else { 0 };
            win32_set_last_error(ERROR_SUCCESS);
            TRUE
        }
        TokenElevationType => {
            let req_len = core::mem::size_of::<TOKEN_ELEVATION_TYPE>() as DWORD;
            if !ReturnLength.is_null() {
                *ReturnLength = req_len;
            }
            if TokenInformation.is_null() || TokenInformationLength < req_len {
                win32_set_last_error(ERROR_INSUFFICIENT_BUFFER);
                return FALSE;
            }
            let type_ptr = TokenInformation as *mut TOKEN_ELEVATION_TYPE;
            (*type_ptr).ElevationType = if token.is_elevated {
                TokenElevationTypeFull
            } else {
                TokenElevationTypeLimited
            };
            win32_set_last_error(ERROR_SUCCESS);
            TRUE
        }
        TokenIntegrityLevel => {
            let req_len = core::mem::size_of::<TOKEN_MANDATORY_LABEL>() as DWORD;
            if !ReturnLength.is_null() {
                *ReturnLength = req_len;
            }
            if TokenInformation.is_null() || TokenInformationLength < req_len {
                win32_set_last_error(ERROR_INSUFFICIENT_BUFFER);
                return FALSE;
            }
            let label_ptr = TokenInformation as *mut TOKEN_MANDATORY_LABEL;
            (*label_ptr).Label.Attributes = token.integrity_level;
            (*label_ptr).Label.Sid = core::ptr::null_mut();
            win32_set_last_error(ERROR_SUCCESS);
            TRUE
        }
        TokenPrivileges => {
            let priv_count = token.privileges.len() as DWORD;
            let req_len = 4 + priv_count * (core::mem::size_of::<LUID_AND_ATTRIBUTES>() as DWORD);
            if !ReturnLength.is_null() {
                *ReturnLength = req_len;
            }
            if TokenInformation.is_null() || TokenInformationLength < req_len {
                win32_set_last_error(ERROR_INSUFFICIENT_BUFFER);
                return FALSE;
            }
            let count_ptr = TokenInformation as *mut DWORD;
            *count_ptr = priv_count;
            let array_ptr = (TokenInformation as usize + 4) as *mut LUID_AND_ATTRIBUTES;
            let mut idx = 0;
            for (&luid_val, &attrs) in &token.privileges {
                let item_ptr = array_ptr.add(idx);
                (*item_ptr).Luid = u64_to_luid(luid_val);
                (*item_ptr).Attributes = attrs;
                idx += 1;
            }
            win32_set_last_error(ERROR_SUCCESS);
            TRUE
        }
        _ => {
            // Default success for generic queries with size
            let req_len = 4;
            if !ReturnLength.is_null() {
                *ReturnLength = req_len;
            }
            if TokenInformation.is_null() || TokenInformationLength < req_len {
                win32_set_last_error(ERROR_INSUFFICIENT_BUFFER);
                return FALSE;
            }
            let out_ptr = TokenInformation as *mut DWORD;
            *out_ptr = 1;
            win32_set_last_error(ERROR_SUCCESS);
            TRUE
        }
    }
}

/// Enables or disables privileges in the specified access token.
pub unsafe extern "system" fn AdjustTokenPrivileges(
    TokenHandle: HANDLE,
    DisableAllPrivileges: BOOL,
    NewState: *const TOKEN_PRIVILEGES,
    BufferLength: DWORD,
    PreviousState: *mut TOKEN_PRIVILEGES,
    ReturnLength: *mut DWORD,
) -> BOOL {
    let mut tokens = ACCESS_TOKENS.lock();
    let token = match tokens.get_mut(&TokenHandle) {
        Some(t) => t,
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }
    };

    if DisableAllPrivileges == TRUE {
        for attrs in token.privileges.values_mut() {
            *attrs &= !SE_PRIVILEGE_ENABLED;
        }
        win32_set_last_error(ERROR_SUCCESS);
        return TRUE;
    }

    if NewState.is_null() {
        win32_set_last_error(ERROR_SUCCESS);
        return TRUE;
    }

    let priv_count = (*NewState).PrivilegeCount;
    let array_ptr = &(*NewState).Privileges[0] as *const LUID_AND_ATTRIBUTES;
    let mut all_assigned = true;

    for i in 0..priv_count {
        let la = *array_ptr.add(i as usize);
        let key = luid_to_u64(la.Luid);
        if let Some(existing_attrs) = token.privileges.get_mut(&key) {
            if !PreviousState.is_null() && BufferLength >= core::mem::size_of::<TOKEN_PRIVILEGES>() as DWORD {
                (*PreviousState).PrivilegeCount = 1;
                (*PreviousState).Privileges[0] = LUID_AND_ATTRIBUTES {
                    Luid: la.Luid,
                    Attributes: *existing_attrs,
                };
            }
            *existing_attrs = la.Attributes;
        } else {
            // New privilege being enabled on token
            token.privileges.insert(key, la.Attributes);
            all_assigned = false;
        }
    }

    if !ReturnLength.is_null() {
        *ReturnLength = core::mem::size_of::<TOKEN_PRIVILEGES>() as DWORD;
    }

    if all_assigned {
        win32_set_last_error(ERROR_SUCCESS);
    } else {
        win32_set_last_error(ERROR_NOT_ALL_ASSIGNED);
    }
    TRUE
}

/// Duplicates an existing access token.
pub unsafe extern "system" fn DuplicateTokenEx(
    ExistingTokenHandle: HANDLE,
    _DesiredAccess: ACCESS_MASK,
    _TokenAttributes: *const c_void,
    ImpersonationLevel: DWORD,
    TokenTypeVal: DWORD,
    DuplicateTokenHandle: *mut HANDLE,
) -> BOOL {
    if DuplicateTokenHandle.is_null() {
        win32_set_last_error(ERROR_INVALID_PARAMETER);
        return FALSE;
    }

    let tokens = ACCESS_TOKENS.lock();
    let existing = match tokens.get(&ExistingTokenHandle) {
        Some(t) => t.clone(),
        None => {
            win32_set_last_error(ERROR_INVALID_HANDLE);
            return FALSE;
        }
    };
    drop(tokens);

    let new_id = NEXT_TOKEN_ID.fetch_add(1, Ordering::SeqCst) as HANDLE;
    let mut new_token = existing;
    new_token.id = new_id;
    new_token.token_type = TokenTypeVal;
    new_token.impersonation_level = ImpersonationLevel;

    ACCESS_TOKENS.lock().insert(new_id, new_token);
    *DuplicateTokenHandle = new_id;
    win32_set_last_error(ERROR_SUCCESS);
    TRUE
}

pub const SE_DEBUG_NAME: &str = "SeDebugPrivilege";
pub const SE_LOCK_MEMORY_NAME: &str = "SeLockMemoryPrivilege";
pub const SE_CHANGE_NOTIFY_NAME: &str = "SeChangeNotifyPrivilege";
pub const SE_INCREASE_QUOTA_NAME: &str = "SeIncreaseQuotaPrivilege";
pub const SE_ASSIGNPRIMARYTOKEN_NAME: &str = "SeAssignPrimaryTokenPrivilege";

/// Maps a well-known privilege name to its corresponding 64-bit LUID identifier.
pub fn win32_lookup_privilege_value(name: &str) -> Option<LUID> {
    match name {
        SE_DEBUG_NAME => Some(u64_to_luid(SE_DEBUG_PRIVILEGE_LUID)),
        SE_LOCK_MEMORY_NAME => Some(u64_to_luid(SE_LOCK_MEMORY_PRIVILEGE_LUID)),
        SE_CHANGE_NOTIFY_NAME => Some(u64_to_luid(SE_CHANGE_NOTIFY_PRIVILEGE_LUID)),
        _ => None,
    }
}

/// Verifies whether an access token possesses an actively enabled privilege.
pub fn win32_check_token_privilege(token_handle: HANDLE, privilege_luid: u64) -> bool {
    let tokens = ACCESS_TOKENS.lock();
    if let Some(token) = tokens.get(&token_handle) {
        if let Some(&attrs) = token.privileges.get(&privilege_luid) {
            return (attrs & SE_PRIVILEGE_ENABLED) != 0;
        }
    }
    false
}

/// Gates cross-process inspection based on anti-cheat integrity levels and SeDebugPrivilege.
pub fn win32_verify_anti_cheat_cross_process_access(
    caller_token: HANDLE,
    target_token: HANDLE,
    desired_access: ACCESS_MASK,
) -> bool {
    let tokens = ACCESS_TOKENS.lock();
    let caller = match tokens.get(&caller_token) {
        Some(t) => t,
        None => return false,
    };
    let target = match tokens.get(&target_token) {
        Some(t) => t,
        None => return false,
    };

    // Caller integrity level must be greater than or equal to target integrity level
    if caller.integrity_level < target.integrity_level {
        return false;
    }

    // High or System level callers with SeDebugPrivilege can access lower or equal targets
    if caller.integrity_level >= SECURITY_MANDATORY_HIGH_RID {
        if let Some(&attrs) = caller.privileges.get(&SE_DEBUG_PRIVILEGE_LUID) {
            if (attrs & SE_PRIVILEGE_ENABLED) != 0 {
                return true;
            }
        }
    }

    // Otherwise apply default standard access masking
    if (desired_access & (GENERIC_WRITE | WRITE_DAC | WRITE_OWNER)) != 0 {
        caller.integrity_level > target.integrity_level
    } else {
        true
    }
}

/// Gates large page and locked physical memory allocation for game performance runtimes.
pub fn win32_verify_lock_memory_privilege(caller_token: HANDLE) -> bool {
    win32_check_token_privilege(caller_token, SE_LOCK_MEMORY_PRIVILEGE_LUID)
}
