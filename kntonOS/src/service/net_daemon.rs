//! Handheld Userspace Network Manager & Wi-Fi Station Daemon (TASK-KNT-326).
//!
//! Provides Wi-Fi station discovery, WPA2/WPA3 profile storage, DHCP lease negotiation,
//! fast DNS resolver caching, and captive portal / offline gaming management.

#![allow(dead_code)]

// ============================================================================
// Feature 1: Wi-Fi Station Scan Results, RSSI Meter & WPA Profile Store
// ============================================================================

/// Wireless authentication and encryption protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityProtocol {
    Open,
    Wpa2Psk,
    Wpa3Sae,
    Wpa2Wpa3Mixed,
}

/// Discovered Access Point (BSS) descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WifiBssEntry {
    pub ssid: [u8; 32],
    pub ssid_len: u8,
    pub bssid: [u8; 6],
    pub channel: u8,
    pub rssi_dbm: i8,
    pub security: SecurityProtocol,
}

impl WifiBssEntry {
    pub const fn new(
        ssid: &[u8],
        bssid: [u8; 6],
        channel: u8,
        rssi_dbm: i8,
        security: SecurityProtocol,
    ) -> Self {
        let mut buf = [0u8; 32];
        let mut len = 0;
        while len < ssid.len() && len < 32 {
            buf[len] = ssid[len];
            len += 1;
        }
        Self {
            ssid: buf,
            ssid_len: len as u8,
            bssid,
            channel,
            rssi_dbm,
            security,
        }
    }

    /// Converts raw dBm signal strength to 1-4 visual status bar icons.
    pub const fn signal_bars(&self) -> u8 {
        if self.rssi_dbm >= -55 {
            4 // Excellent
        } else if self.rssi_dbm >= -67 {
            3 // Good
        } else if self.rssi_dbm >= -78 {
            2 // Fair
        } else {
            1 // Weak
        }
    }
}

/// Stored network credentials profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WifiProfile {
    pub ssid: [u8; 32],
    pub ssid_len: u8,
    pub psk_hash: [u8; 32],
    pub auto_connect: bool,
    pub security: SecurityProtocol,
}

impl WifiProfile {
    pub const fn new(ssid: &[u8], psk_hash: [u8; 32], security: SecurityProtocol) -> Self {
        let mut buf = [0u8; 32];
        let mut len = 0;
        while len < ssid.len() && len < 32 {
            buf[len] = ssid[len];
            len += 1;
        }
        Self {
            ssid: buf,
            ssid_len: len as u8,
            psk_hash,
            auto_connect: true,
            security,
        }
    }
}

pub const MAX_SCAN_RESULTS: usize = 16;
pub const MAX_SAVED_PROFILES: usize = 8;

/// Station subsystem manager.
#[derive(Debug, Clone, Copy)]
pub struct WifiStationManager {
    pub scan_results: [WifiBssEntry; MAX_SCAN_RESULTS],
    pub scan_count: usize,
    pub profiles: [WifiProfile; MAX_SAVED_PROFILES],
    pub profile_count: usize,
    pub connected_bssid: Option<[u8; 6]>,
    pub connected_rssi: i8,
}

impl WifiStationManager {
    pub const fn new() -> Self {
        let dummy_bss = WifiBssEntry {
            ssid: [0; 32],
            ssid_len: 0,
            bssid: [0; 6],
            channel: 0,
            rssi_dbm: -100,
            security: SecurityProtocol::Open,
        };
        let dummy_prof = WifiProfile {
            ssid: [0; 32],
            ssid_len: 0,
            psk_hash: [0; 32],
            auto_connect: false,
            security: SecurityProtocol::Open,
        };

        Self {
            scan_results: [dummy_bss; MAX_SCAN_RESULTS],
            scan_count: 0,
            profiles: [dummy_prof; MAX_SAVED_PROFILES],
            profile_count: 0,
            connected_bssid: None,
            connected_rssi: -100,
        }
    }

    /// Adds or updates a scanned access point in the active scan table.
    pub fn add_scan_result(&mut self, bss: WifiBssEntry) {
        // Check if BSSID already present, update RSSI
        for i in 0..self.scan_count {
            if self.scan_results[i].bssid == bss.bssid {
                self.scan_results[i] = bss;
                return;
            }
        }
        if self.scan_count < MAX_SCAN_RESULTS {
            self.scan_results[self.scan_count] = bss;
            self.scan_count += 1;
        }
    }

    /// Saves a known network profile for automatic reconnection.
    pub fn save_profile(&mut self, profile: WifiProfile) -> bool {
        for i in 0..self.profile_count {
            if self.profiles[i].ssid_len == profile.ssid_len
                && self.profiles[i].ssid == profile.ssid
            {
                self.profiles[i] = profile;
                return true;
            }
        }
        if self.profile_count < MAX_SAVED_PROFILES {
            self.profiles[self.profile_count] = profile;
            self.profile_count += 1;
            true
        } else {
            false
        }
    }

    /// Finds a matching saved profile for a given SSID.
    pub fn find_saved_profile(&self, ssid: &[u8]) -> Option<&WifiProfile> {
        for i in 0..self.profile_count {
            let prof = &self.profiles[i];
            let len = prof.ssid_len as usize;
            if len == ssid.len() && &prof.ssid[..len] == ssid {
                return Some(prof);
            }
        }
        None
    }
}

/// Verification test for Wi-Fi station scan results and WPA profile store.
pub fn verify_wifi_station_manager() -> bool {
    let mut mgr = WifiStationManager::new();

    // 1. Initial state
    if mgr.scan_count != 0 || mgr.profile_count != 0 || mgr.connected_bssid.is_some() {
        return false;
    }

    // 2. Add scan results for 5GHz home AP (WPA3-SAE, -52 dBm)
    let bss1 = WifiBssEntry::new(
        b"KntonGaming_5G",
        [0xAA, 0xBB, 0xCC, 0x11, 0x22, 0x33],
        36,
        -52,
        SecurityProtocol::Wpa3Sae,
    );
    if bss1.signal_bars() != 4 {
        return false;
    }
    mgr.add_scan_result(bss1);
    if mgr.scan_count != 1 {
        return false;
    }

    // 3. Add scan result for weak 2.4GHz hotspot (WPA2, -82 dBm)
    let bss2 = WifiBssEntry::new(
        b"Guest_Hotspot",
        [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01],
        6,
        -82,
        SecurityProtocol::Wpa2Psk,
    );
    if bss2.signal_bars() != 1 {
        return false;
    }
    mgr.add_scan_result(bss2);
    if mgr.scan_count != 2 {
        return false;
    }

    // 4. Save profile for KntonGaming_5G
    let prof = WifiProfile::new(b"KntonGaming_5G", [0x42; 32], SecurityProtocol::Wpa3Sae);
    if !mgr.save_profile(prof) || mgr.profile_count != 1 {
        return false;
    }

    // 5. Lookup saved profile
    let found = mgr.find_saved_profile(b"KntonGaming_5G");
    if found.is_none() || found.unwrap().security != SecurityProtocol::Wpa3Sae {
        return false;
    }

    let not_found = mgr.find_saved_profile(b"NonExistent");
    if not_found.is_some() {
        return false;
    }

    true
}
