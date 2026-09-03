//! Win32 WinSock2 (ws2_32.dll) Game Multiplayer Network Socket Emulation Layer.
//! Implements Windows socket descriptors, WSA lifecycle, socket management, and network I/O.

use crate::memory::spinlock::Spinlock;

pub type SOCKET = usize;
pub const INVALID_SOCKET: SOCKET = usize::MAX;
pub const SOCKET_ERROR: i32 = -1;

pub const AF_UNSPEC: i32 = 0;
pub const AF_UNIX: i32 = 1;
pub const AF_INET: i32 = 2;
pub const AF_INET6: i32 = 23;

pub const SOCK_STREAM: i32 = 1;
pub const SOCK_DGRAM: i32 = 2;
pub const SOCK_RAW: i32 = 3;

pub const IPPROTO_IP: i32 = 0;
pub const IPPROTO_TCP: i32 = 6;
pub const IPPROTO_UDP: i32 = 17;

pub const WSASYSNOTREADY: i32 = 10091;
pub const WSAVERNOTSUPPORTED: i32 = 10092;
pub const WSAEINVAL: i32 = 10022;
pub const WSAEWOULDBLOCK: i32 = 10035;
pub const WSAEINPROGRESS: i32 = 10036;
pub const WSAEFAULT: i32 = 10014;
pub const WSAENOTSOCK: i32 = 10038;
pub const WSAECONNREFUSED: i32 = 10061;
pub const WSAEADDRINUSE: i32 = 10048;
pub const WSAENOTCONN: i32 = 10057;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WsaData {
    pub w_version: u16,
    pub w_high_version: u16,
    pub sz_description: [u8; 257],
    pub sz_system_status: [u8; 129],
    pub i_max_sockets: u16,
    pub i_max_udpdg: u16,
    pub lp_vendor_info: usize,
}

impl Default for WsaData {
    fn default() -> Self {
        Self {
            w_version: 0x0202,
            w_high_version: 0x0202,
            sz_description: [0; 257],
            sz_system_status: [0; 129],
            i_max_sockets: 256,
            i_max_udpdg: 65507,
            lp_vendor_info: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InAddr {
    pub s_addr: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SockAddrIn {
    pub sin_family: u16,
    pub sin_port: u16,
    pub sin_addr: InAddr,
    pub sin_zero: [u8; 8],
}

impl Default for SockAddrIn {
    fn default() -> Self {
        Self {
            sin_family: AF_INET as u16,
            sin_port: 0,
            sin_addr: InAddr { s_addr: 0 },
            sin_zero: [0; 8],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SockAddr {
    pub sa_family: u16,
    pub sa_data: [u8; 14],
}

impl Default for SockAddr {
    fn default() -> Self {
        Self {
            sa_family: AF_INET as u16,
            sa_data: [0; 14],
        }
    }
}

static WSA_REF_COUNT: Spinlock<u32> = Spinlock::new(0);
static WSA_LAST_ERROR: Spinlock<i32> = Spinlock::new(0);

/// Retrieves the error status for the last Windows Sockets operation that failed.
pub fn win32_wsa_get_last_error() -> i32 {
    *WSA_LAST_ERROR.lock()
}

/// Sets the error code that can be retrieved by the WSAGetLastError function.
pub fn win32_wsa_set_last_error(error: i32) {
    *WSA_LAST_ERROR.lock() = error;
}

/// Initiates use of the Winsock DLL by a process.
pub unsafe fn win32_wsa_startup(version_requested: u16, lp_wsa_data: *mut WsaData) -> i32 {
    if lp_wsa_data.is_null() {
        win32_wsa_set_last_error(WSAEFAULT);
        return WSAEFAULT;
    }

    let low_byte = (version_requested & 0xFF) as u8;
    let high_byte = ((version_requested >> 8) & 0xFF) as u8;
    if low_byte < 1 || (low_byte == 1 && high_byte < 1) {
        win32_wsa_set_last_error(WSAVERNOTSUPPORTED);
        return WSAVERNOTSUPPORTED;
    }

    let mut data = WsaData::default();
    let desc = b"KntonOS WinSock 2.0 Emulation Layer\0";
    data.sz_description[..desc.len()].copy_from_slice(desc);
    let status = b"Running\0";
    data.sz_system_status[..status.len()].copy_from_slice(status);

    *lp_wsa_data = data;

    let mut ref_count = WSA_REF_COUNT.lock();
    *ref_count = ref_count.saturating_add(1);
    win32_wsa_set_last_error(0);
    0
}

/// Terminates use of the Winsock DLL by a process.
pub fn win32_wsa_cleanup() -> i32 {
    let mut ref_count = WSA_REF_COUNT.lock();
    if *ref_count == 0 {
        win32_wsa_set_last_error(WSASYSNOTREADY);
        return SOCKET_ERROR;
    }
    *ref_count = ref_count.saturating_sub(1);
    0
}

/// Determines whether Winsock is currently initialized.
pub fn win32_wsa_is_initialized() -> bool {
    *WSA_REF_COUNT.lock() > 0
}

/// Converts a u16 from host to TCP/IP network byte order (big-endian).
pub const fn win32_htons(hostshort: u16) -> u16 {
    hostshort.to_be()
}

/// Converts a u16 from TCP/IP network byte order to host byte order.
pub const fn win32_ntohs(netshort: u16) -> u16 {
    u16::from_be(netshort)
}

/// Converts a u32 from host to TCP/IP network byte order (big-endian).
pub const fn win32_htonl(hostlong: u32) -> u32 {
    hostlong.to_be()
}

/// Converts a u32 from TCP/IP network byte order to host byte order.
pub const fn win32_ntohl(netlong: u32) -> u32 {
    u32::from_be(netlong)
}

/// Converts an IPv4 dotted-decimal string into an in_addr structure address.
pub fn win32_inet_addr(cp: &str) -> u32 {
    let mut octets = [0u8; 4];
    let mut idx = 0;
    let mut current: u32 = 0;
    let mut has_digits = false;

    for b in cp.bytes() {
        if b >= b'0' && b <= b'9' {
            current = current * 10 + (b - b'0') as u32;
            if current > 255 {
                return 0xFFFF_FFFF;
            }
            has_digits = true;
        } else if b == b'.' {
            if !has_digits || idx >= 3 {
                return 0xFFFF_FFFF;
            }
            octets[idx] = current as u8;
            idx += 1;
            current = 0;
            has_digits = false;
        } else if b == 0 {
            break;
        } else {
            return 0xFFFF_FFFF;
        }
    }

    if !has_digits || idx != 3 {
        return 0xFFFF_FFFF;
    }
    octets[3] = current as u8;

    u32::from_ne_bytes(octets)
}
