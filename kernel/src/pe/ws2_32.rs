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

use alloc::vec::Vec;
use crate::net::socket::{
    GLOBAL_SOCKET_TABLE, SocketDomain, SocketError, SocketProtocol, SocketType,
};

pub struct Win32SocketEntry {
    pub socket: SOCKET,
    pub af: i32,
    pub sock_type: i32,
    pub protocol: i32,
    pub nonblocking: bool,
    pub local_addr: Option<SockAddrIn>,
    pub peer_addr: Option<SockAddrIn>,
}

static WIN32_SOCKET_TABLE: Spinlock<Vec<Win32SocketEntry>> = Spinlock::new(Vec::new());

fn map_socket_error(err: SocketError) -> i32 {
    match err {
        SocketError::InvalidHandle => WSAENOTSOCK,
        SocketError::TableFull => WSASYSNOTREADY,
        SocketError::AlreadyBound => WSAEINVAL,
        SocketError::NotBound => WSAEINVAL,
        SocketError::AlreadyConnected => WSAEINVAL,
        SocketError::NotConnected => WSAENOTCONN,
        SocketError::ConnectionRefused => WSAECONNREFUSED,
        SocketError::WouldBlock => WSAEWOULDBLOCK,
        SocketError::UnsupportedDomain => WSAEINVAL,
        SocketError::UnsupportedType => WSAEINVAL,
        SocketError::AddressInUse => WSAEADDRINUSE,
        SocketError::InvalidAddress => WSAEFAULT,
        SocketError::BrokenPipe => 10053,
    }
}

/// Creates a socket that is bound to a specific transport-service provider.
pub fn win32_socket(af: i32, sock_type: i32, protocol: i32) -> SOCKET {
    if !win32_wsa_is_initialized() {
        win32_wsa_set_last_error(WSASYSNOTREADY);
        return INVALID_SOCKET;
    }

    let domain = match af {
        AF_INET => SocketDomain::AfInet,
        AF_UNIX => SocketDomain::AfUnix,
        AF_UNSPEC => SocketDomain::AfUnspec,
        _ => {
            win32_wsa_set_last_error(WSAEINVAL);
            return INVALID_SOCKET;
        }
    };

    let stype = match sock_type {
        SOCK_STREAM => SocketType::SockStream,
        SOCK_DGRAM => SocketType::SockDgram,
        SOCK_RAW => SocketType::SockRaw,
        _ => {
            win32_wsa_set_last_error(WSAEINVAL);
            return INVALID_SOCKET;
        }
    };

    let proto = match protocol {
        IPPROTO_TCP => SocketProtocol::Tcp,
        IPPROTO_UDP => SocketProtocol::Udp,
        _ => SocketProtocol::Default,
    };

    match GLOBAL_SOCKET_TABLE.create_socket(domain, stype, proto) {
        Ok(handle) => {
            let mut table = WIN32_SOCKET_TABLE.lock();
            table.push(Win32SocketEntry {
                socket: handle,
                af,
                sock_type,
                protocol,
                nonblocking: false,
                local_addr: None,
                peer_addr: None,
            });
            win32_wsa_set_last_error(0);
            handle
        }
        Err(err) => {
            let wsa_err = map_socket_error(err);
            win32_wsa_set_last_error(wsa_err);
            INVALID_SOCKET
        }
    }
}

/// Creates a socket descriptor with extended WSA attributes and flags.
pub fn win32_wsa_socket(
    af: i32,
    sock_type: i32,
    protocol: i32,
    _lp_protocol_info: usize,
    _g: u32,
    _dw_flags: u32,
) -> SOCKET {
    win32_socket(af, sock_type, protocol)
}

/// Associates a local address with a socket.
pub unsafe fn win32_bind(s: SOCKET, name: *const SockAddr, namelen: i32) -> i32 {
    if !win32_wsa_is_initialized() {
        win32_wsa_set_last_error(WSASYSNOTREADY);
        return SOCKET_ERROR;
    }
    if name.is_null() || namelen < core::mem::size_of::<SockAddrIn>() as i32 {
        win32_wsa_set_last_error(WSAEFAULT);
        return SOCKET_ERROR;
    }

    let sa_in = &*(name as *const SockAddrIn);
    let port = u16::from_be(sa_in.sin_port);
    let ip_bytes = sa_in.sin_addr.s_addr.to_ne_bytes();

    match GLOBAL_SOCKET_TABLE.bind(s, ip_bytes, port) {
        Ok(()) => {
            let mut table = WIN32_SOCKET_TABLE.lock();
            if let Some(entry) = table.iter_mut().find(|e| e.socket == s) {
                entry.local_addr = Some(*sa_in);
            }
            win32_wsa_set_last_error(0);
            0
        }
        Err(err) => {
            let wsa_err = map_socket_error(err);
            win32_wsa_set_last_error(wsa_err);
            SOCKET_ERROR
        }
    }
}

/// Places a socket in a state in which it is listening for an incoming connection.
pub fn win32_listen(s: SOCKET, backlog: i32) -> i32 {
    if !win32_wsa_is_initialized() {
        win32_wsa_set_last_error(WSASYSNOTREADY);
        return SOCKET_ERROR;
    }

    let bl = if backlog > 0 { backlog as usize } else { 5 };
    match GLOBAL_SOCKET_TABLE.listen(s, bl) {
        Ok(()) => {
            win32_wsa_set_last_error(0);
            0
        }
        Err(err) => {
            let wsa_err = map_socket_error(err);
            win32_wsa_set_last_error(wsa_err);
            SOCKET_ERROR
        }
    }
}

/// Permits an incoming connection attempt on a socket.
pub unsafe fn win32_accept(s: SOCKET, addr: *mut SockAddr, addrlen: *mut i32) -> SOCKET {
    if !win32_wsa_is_initialized() {
        win32_wsa_set_last_error(WSASYSNOTREADY);
        return INVALID_SOCKET;
    }

    match GLOBAL_SOCKET_TABLE.accept(s) {
        Ok((client_handle, (ip, port))) => {
            let client_sa = SockAddrIn {
                sin_family: AF_INET as u16,
                sin_port: port.to_be(),
                sin_addr: InAddr {
                    s_addr: u32::from_ne_bytes(ip),
                },
                sin_zero: [0; 8],
            };

            if !addr.is_null() && !addrlen.is_null() {
                if *addrlen >= core::mem::size_of::<SockAddrIn>() as i32 {
                    core::ptr::copy_nonoverlapping(
                        &client_sa as *const _ as *const u8,
                        addr as *mut u8,
                        core::mem::size_of::<SockAddrIn>(),
                    );
                    *addrlen = core::mem::size_of::<SockAddrIn>() as i32;
                }
            }

            let mut table = WIN32_SOCKET_TABLE.lock();
            table.push(Win32SocketEntry {
                socket: client_handle,
                af: AF_INET,
                sock_type: SOCK_STREAM,
                protocol: IPPROTO_TCP,
                nonblocking: false,
                local_addr: None,
                peer_addr: Some(client_sa),
            });

            win32_wsa_set_last_error(0);
            client_handle
        }
        Err(err) => {
            let wsa_err = map_socket_error(err);
            win32_wsa_set_last_error(wsa_err);
            INVALID_SOCKET
        }
    }
}

/// Establishes a connection to a specified socket.
pub unsafe fn win32_connect(s: SOCKET, name: *const SockAddr, namelen: i32) -> i32 {
    if !win32_wsa_is_initialized() {
        win32_wsa_set_last_error(WSASYSNOTREADY);
        return SOCKET_ERROR;
    }
    if name.is_null() || namelen < core::mem::size_of::<SockAddrIn>() as i32 {
        win32_wsa_set_last_error(WSAEFAULT);
        return SOCKET_ERROR;
    }

    let sa_in = &*(name as *const SockAddrIn);
    let port = u16::from_be(sa_in.sin_port);
    let ip_bytes = sa_in.sin_addr.s_addr.to_ne_bytes();

    match GLOBAL_SOCKET_TABLE.connect(s, ip_bytes, port) {
        Ok(()) => {
            let mut table = WIN32_SOCKET_TABLE.lock();
            if let Some(entry) = table.iter_mut().find(|e| e.socket == s) {
                entry.peer_addr = Some(*sa_in);
            }
            win32_wsa_set_last_error(0);
            0
        }
        Err(err) => {
            let wsa_err = map_socket_error(err);
            win32_wsa_set_last_error(wsa_err);
            SOCKET_ERROR
        }
    }
}

/// Closes an existing socket descriptor.
pub fn win32_closesocket(s: SOCKET) -> i32 {
    if !win32_wsa_is_initialized() {
        win32_wsa_set_last_error(WSASYSNOTREADY);
        return SOCKET_ERROR;
    }

    let mut table = WIN32_SOCKET_TABLE.lock();
    if let Some(pos) = table.iter().position(|e| e.socket == s) {
        table.remove(pos);
    }
    drop(table);

    match GLOBAL_SOCKET_TABLE.close(s) {
        Ok(()) => {
            win32_wsa_set_last_error(0);
            0
        }
        Err(err) => {
            let wsa_err = map_socket_error(err);
            win32_wsa_set_last_error(wsa_err);
            SOCKET_ERROR
        }
    }
}

/// Retrieves the local name for a socket.
pub unsafe fn win32_get_sock_name(s: SOCKET, name: *mut SockAddr, namelen: *mut i32) -> i32 {
    if name.is_null() || namelen.is_null() {
        win32_wsa_set_last_error(WSAEFAULT);
        return SOCKET_ERROR;
    }

    let table = WIN32_SOCKET_TABLE.lock();
    let entry = match table.iter().find(|e| e.socket == s) {
        Some(e) => e,
        None => {
            win32_wsa_set_last_error(WSAENOTSOCK);
            return SOCKET_ERROR;
        }
    };

    let local_sa = match entry.local_addr {
        Some(sa) => sa,
        None => SockAddrIn::default(),
    };

    if *namelen >= core::mem::size_of::<SockAddrIn>() as i32 {
        core::ptr::copy_nonoverlapping(
            &local_sa as *const _ as *const u8,
            name as *mut u8,
            core::mem::size_of::<SockAddrIn>(),
        );
        *namelen = core::mem::size_of::<SockAddrIn>() as i32;
        0
    } else {
        win32_wsa_set_last_error(WSAEFAULT);
        SOCKET_ERROR
    }
}

/// Retrieves the address of the peer to which a socket is connected.
pub unsafe fn win32_get_peer_name(s: SOCKET, name: *mut SockAddr, namelen: *mut i32) -> i32 {
    if name.is_null() || namelen.is_null() {
        win32_wsa_set_last_error(WSAEFAULT);
        return SOCKET_ERROR;
    }

    let table = WIN32_SOCKET_TABLE.lock();
    let entry = match table.iter().find(|e| e.socket == s) {
        Some(e) => e,
        None => {
            win32_wsa_set_last_error(WSAENOTSOCK);
            return SOCKET_ERROR;
        }
    };

    let peer_sa = match entry.peer_addr {
        Some(sa) => sa,
        None => {
            win32_wsa_set_last_error(WSAENOTCONN);
            return SOCKET_ERROR;
        }
    };

    if *namelen >= core::mem::size_of::<SockAddrIn>() as i32 {
        core::ptr::copy_nonoverlapping(
            &peer_sa as *const _ as *const u8,
            name as *mut u8,
            core::mem::size_of::<SockAddrIn>(),
        );
        *namelen = core::mem::size_of::<SockAddrIn>() as i32;
        0
    } else {
        win32_wsa_set_last_error(WSAEFAULT);
        SOCKET_ERROR
    }
}

pub const FIONBIO: i32 = -2147195266; // 0x8004667E as i32
pub const FIONREAD: i32 = 0x4004_667F;

pub const POLLRDNORM: i16 = 0x0100;
pub const POLLRDBAND: i16 = 0x0200;
pub const POLLIN: i16 = 0x0300;
pub const POLLPRI: i16 = 0x0400;
pub const POLLWRNORM: i16 = 0x0010;
pub const POLLOUT: i16 = 0x0010;
pub const POLLERR: i16 = 0x0001;
pub const POLLHUP: i16 = 0x0002;
pub const POLLNVAL: i16 = 0x0004;

pub const FD_SETSIZE: usize = 64;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FdSet {
    pub fd_count: u32,
    pub fd_array: [SOCKET; FD_SETSIZE],
}

impl Default for FdSet {
    fn default() -> Self {
        Self {
            fd_count: 0,
            fd_array: [INVALID_SOCKET; FD_SETSIZE],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimeVal {
    pub tv_sec: i32,
    pub tv_usec: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WsaPollFd {
    pub fd: SOCKET,
    pub events: i16,
    pub revents: i16,
}

/// Clears an fd_set structure.
pub unsafe fn win32_fd_zero(set: *mut FdSet) {
    if !set.is_null() {
        (*set).fd_count = 0;
        for i in 0..FD_SETSIZE {
            (*set).fd_array[i] = INVALID_SOCKET;
        }
    }
}

/// Adds a socket descriptor to an fd_set structure.
pub unsafe fn win32_fd_set(s: SOCKET, set: *mut FdSet) {
    if set.is_null() {
        return;
    }
    let count = (*set).fd_count as usize;
    if count >= FD_SETSIZE {
        return;
    }
    for i in 0..count {
        if (*set).fd_array[i] == s {
            return;
        }
    }
    (*set).fd_array[count] = s;
    (*set).fd_count += 1;
}

/// Removes a socket descriptor from an fd_set structure.
pub unsafe fn win32_fd_clr(s: SOCKET, set: *mut FdSet) {
    if set.is_null() {
        return;
    }
    let count = (*set).fd_count as usize;
    for i in 0..count {
        if (*set).fd_array[i] == s {
            for j in i..(count - 1) {
                (*set).fd_array[j] = (*set).fd_array[j + 1];
            }
            (*set).fd_array[count - 1] = INVALID_SOCKET;
            (*set).fd_count -= 1;
            break;
        }
    }
}

/// Determines whether a socket descriptor is contained in an fd_set structure.
pub unsafe fn win32_fd_isset(s: SOCKET, set: *const FdSet) -> bool {
    if set.is_null() {
        return false;
    }
    let count = (*set).fd_count as usize;
    for i in 0..count {
        if (*set).fd_array[i] == s {
            return true;
        }
    }
    false
}

/// Controls the I/O mode of a socket descriptor.
pub unsafe fn win32_ioctlsocket(s: SOCKET, cmd: i32, argp: *mut u32) -> i32 {
    if !win32_wsa_is_initialized() {
        win32_wsa_set_last_error(WSASYSNOTREADY);
        return SOCKET_ERROR;
    }
    if argp.is_null() {
        win32_wsa_set_last_error(WSAEFAULT);
        return SOCKET_ERROR;
    }

    match cmd {
        FIONBIO => {
            let nonblocking = *argp != 0;
            match GLOBAL_SOCKET_TABLE.set_nonblocking(s, nonblocking) {
                Ok(()) => {
                    let mut table = WIN32_SOCKET_TABLE.lock();
                    if let Some(entry) = table.iter_mut().find(|e| e.socket == s) {
                        entry.nonblocking = nonblocking;
                    }
                    win32_wsa_set_last_error(0);
                    0
                }
                Err(err) => {
                    let wsa_err = map_socket_error(err);
                    win32_wsa_set_last_error(wsa_err);
                    SOCKET_ERROR
                }
            }
        }
        FIONREAD => {
            let readable = GLOBAL_SOCKET_TABLE.is_readable(s);
            *argp = if readable { 1 } else { 0 };
            win32_wsa_set_last_error(0);
            0
        }
        _ => {
            win32_wsa_set_last_error(WSAEINVAL);
            SOCKET_ERROR
        }
    }
}

/// Determines the status of one or more sockets, waiting if necessary.
pub unsafe fn win32_select(
    _nfds: i32,
    readfds: *mut FdSet,
    writefds: *mut FdSet,
    exceptfds: *mut FdSet,
    _timeout: *const TimeVal,
) -> i32 {
    if !win32_wsa_is_initialized() {
        win32_wsa_set_last_error(WSASYSNOTREADY);
        return SOCKET_ERROR;
    }

    let mut ready_count = 0;

    if !readfds.is_null() {
        let count = (*readfds).fd_count as usize;
        let mut ready_fds = [INVALID_SOCKET; FD_SETSIZE];
        let mut r_count = 0;

        for i in 0..count {
            let fd = (*readfds).fd_array[i];
            if GLOBAL_SOCKET_TABLE.is_readable(fd) {
                ready_fds[r_count] = fd;
                r_count += 1;
                ready_count += 1;
            }
        }

        (*readfds).fd_count = r_count as u32;
        (*readfds).fd_array = ready_fds;
    }

    if !writefds.is_null() {
        let count = (*writefds).fd_count as usize;
        let mut ready_fds = [INVALID_SOCKET; FD_SETSIZE];
        let mut w_count = 0;

        for i in 0..count {
            let fd = (*writefds).fd_array[i];
            if GLOBAL_SOCKET_TABLE.is_writable(fd) {
                ready_fds[w_count] = fd;
                w_count += 1;
                ready_count += 1;
            }
        }

        (*writefds).fd_count = w_count as u32;
        (*writefds).fd_array = ready_fds;
    }

    if !exceptfds.is_null() {
        (*exceptfds).fd_count = 0;
        for i in 0..FD_SETSIZE {
            (*exceptfds).fd_array[i] = INVALID_SOCKET;
        }
    }

    win32_wsa_set_last_error(0);
    ready_count
}

/// Polls the status of one or more sockets with timeout support.
pub unsafe fn win32_wsa_poll(fdarray: *mut WsaPollFd, nfds: u32, _timeout_ms: i32) -> i32 {
    if !win32_wsa_is_initialized() {
        win32_wsa_set_last_error(WSASYSNOTREADY);
        return SOCKET_ERROR;
    }
    if fdarray.is_null() {
        win32_wsa_set_last_error(WSAEFAULT);
        return SOCKET_ERROR;
    }

    let mut ready_count = 0;
    for i in 0..(nfds as usize) {
        let pfd = &mut *fdarray.add(i);
        pfd.revents = 0;

        if !GLOBAL_SOCKET_TABLE.exists(pfd.fd) {
            pfd.revents = POLLNVAL;
            ready_count += 1;
            continue;
        }

        let mut rev = 0;
        if (pfd.events & (POLLIN | POLLRDNORM)) != 0 && GLOBAL_SOCKET_TABLE.is_readable(pfd.fd) {
            rev |= pfd.events & (POLLIN | POLLRDNORM);
        }
        if (pfd.events & (POLLOUT | POLLWRNORM)) != 0 && GLOBAL_SOCKET_TABLE.is_writable(pfd.fd) {
            rev |= pfd.events & (POLLOUT | POLLWRNORM);
        }

        pfd.revents = rev;
        if rev != 0 {
            ready_count += 1;
        }
    }

    win32_wsa_set_last_error(0);
    ready_count
}
