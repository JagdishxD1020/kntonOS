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
