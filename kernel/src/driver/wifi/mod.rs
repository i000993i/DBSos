//! Wi-Fi driver framework: 802.11 MLME + ath9k + WPA2-PSK supplicant.
//!
//! Architecture:
//! - ieee80211: 802.11 frame structures (management/control/data)
//! - ath9k: Atheros AR92xx/AR93xx PCI WiFi driver (MMIO + DMA)
//! - supplicant: WPA2-PSK 4-way handshake (EAPOL)
//!
//! Supported:
//! - PCI detection of Atheros WiFi cards (AR9285/AR9287/AR9300/etc.)
//! - Passive + active scanning (probe request/beacon parsing)
//! - Open system authentication
//! - Association with RSN (WPA2)
//! - WPA2-PSK 4-way handshake (PMK->PTK, GTK install)
//! - Encrypted network profile storage (AES-CBC)

pub mod ieee80211;
pub mod ath9k;
pub mod supplicant;

use crate::driver::uart;

// ── State machine ──────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum WifiState {
    NoRadio,
    Disconnected,
    Scanning,
    Authenticating,
    Associating,
    Associated,
    Handshaking,  // WPA2 4-way handshake in progress
    Connected,    // Fully connected with IP
}

static mut STATE: WifiState = WifiState::Disconnected;
static mut RADIO_COUNT: u32 = 0;

// ── Scan results ───────────────────────────────────────────────────

const MAX_SCAN_RESULTS: usize = 16;

static mut SCAN_RESULTS: [ieee80211::ScanResult; MAX_SCAN_RESULTS] = [ieee80211::ScanResult::empty(); MAX_SCAN_RESULTS];
static mut SCAN_COUNT: usize = 0;

// ── Current association ────────────────────────────────────────────

static mut CUR_SSID: [u8; 32] = [0; 32];
static mut CUR_SSID_LEN: usize = 0;
static mut CUR_BSSID: [u8; 6] = [0; 6];
static mut CUR_CHANNEL: u16 = 0;

fn up(s: &str) { uart::write_str(s); }

fn dec(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut b = [0u8; 20]; let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putchar(b[i]); }
}

// ── Public API ─────────────────────────────────────────────────────

pub fn state() -> WifiState { unsafe { STATE } }

pub fn state_name() -> &'static str {
    unsafe {
        match STATE {
            WifiState::NoRadio => "no-radio",
            WifiState::Disconnected => "disconnected",
            WifiState::Scanning => "scanning",
            WifiState::Authenticating => "authenticating",
            WifiState::Associating => "associating",
            WifiState::Associated => "associated",
            WifiState::Handshaking => "handshaking",
            WifiState::Connected => "connected",
        }
    }
}

pub fn has_radio() -> bool { unsafe { RADIO_COUNT > 0 } }

pub fn scan_result_count() -> usize { unsafe { SCAN_COUNT } }

pub fn get_scan_result(idx: usize) -> Option<ieee80211::ScanResult> {
    unsafe {
        if idx < SCAN_COUNT { Some(SCAN_RESULTS[idx]) } else { None }
    }
}

pub fn assoc_ssid() -> &'static [u8] {
    unsafe { &CUR_SSID[..CUR_SSID_LEN] }
}

pub fn assoc_bssid() -> &'static [u8; 6] {
    unsafe { &CUR_BSSID }
}

pub fn pmk(ssid: &[u8], passphrase: &[u8], out: &mut [u8; 32]) -> bool {
    if ssid.is_empty() || ssid.len() > 32 || passphrase.len() < 8 || passphrase.len() > 63 {
        return false;
    }
    crate::crypto::pbkdf2(passphrase, ssid, 4096, out);
    true
}

// ── PCI scan (lightweight) ─────────────────────────────────────────

/// Quick PCI scan for wireless radios. Returns count.
pub fn scan() -> u32 {
    let mut n = 0u32;
    for dev in 0..32u8 {
        for func in 0..8u8 {
            let vendor = crate::driver::pci::read16(0, dev, func, 0x00);
            if vendor == 0xFFFF {
                if func == 0 { break; }
                continue;
            }
            let reg = crate::driver::pci::read32(0, dev, func, 0x08);
            if (reg >> 24) as u8 == 0x02 && ((reg >> 16) & 0xFF) as u8 == 0x80 {
                n += 1;
                let device = crate::driver::pci::read16(0, dev, func, 0x02);
                up("[wifi] radio: vendor=0x");
                let hv = |v: u16| {
                    let h = b"0123456789ABCDEF";
                    uart::putchar(h[((v >> 12) & 0xF) as usize]);
                    uart::putchar(h[((v >> 8) & 0xF) as usize]);
                    uart::putchar(h[((v >> 4) & 0xF) as usize]);
                    uart::putchar(h[(v & 0xF) as usize]);
                };
                hv(vendor);
                up(" dev=0x");
                hv(device);
                up("\r\n");
            }
            let htype = ((crate::driver::pci::read32(0, dev, func, 0x0C) >> 16) & 0xFF) as u8;
            if func == 0 && htype & 0x80 == 0 { break; }
        }
    }
    unsafe {
        RADIO_COUNT = n;
        STATE = if n == 0 { WifiState::NoRadio } else { WifiState::Disconnected };
    }
    n
}

// ── Full initialization (ath9k + scanning) ─────────────────────────

/// Initialize WiFi hardware (ath9k driver) and detect radios.
pub fn init() -> bool {
    up("[wifi] initializing...\r\n");
    let n = scan();
    if n == 0 {
        up("[wifi] no WiFi hardware found\r\n");
        return false;
    }
    // Try to init ath9k driver
    if ath9k::hw_init() {
        up("[wifi] ath9k driver initialized\r\n");
        unsafe { STATE = WifiState::Disconnected; }
        true
    } else {
        up("[wifi] ath9k init failed\r\n");
        false
    }
}

// ── Scanning ───────────────────────────────────────────────────────

/// Perform a WiFi scan. Returns number of networks found.
pub fn scan_wifi() -> u32 {
    unsafe {
        if RADIO_COUNT == 0 { return 0; }
        STATE = WifiState::Scanning;
        SCAN_COUNT = 0;
    }

    let count = if ath9k::is_initialized() {
        ath9k::hw_scan()
    } else {
        0
    };

    unsafe {
        SCAN_COUNT = count;
        STATE = if count > 0 { WifiState::Disconnected } else { WifiState::Disconnected };
    }
    count as u32
}

/// Called by ath9k driver when a beacon/probe response is received.
pub fn handle_scan_response(data: &[u8]) {
    if data.len() < 24 { return; }

    let fc = (data[0] as u16) | ((data[1] as u16) << 8);
    let subtype = ieee80211::fc_subtype(fc);
    let _ = subtype;

    // Extract BSSID from Address 3
    let bssid = {
        if data.len() < 22 { return; }
        let mut b = [0u8; 6];
        b.copy_from_slice(&data[16..22]);
        b
    };

    // Parse body (after 24-byte header)
    let body = if data.len() > 24 { &data[24..] } else { return; };

    // Skip fixed fields for beacon/probe response (Timestamp 8 + BeaconInterval 2 + Capability 2 = 12 bytes)
    let ie_data = if body.len() > 12 { &body[12..] } else { return; };

    let ssid = match ieee80211::find_ssid(ie_data) {
        Some(s) => s,
        None => return,
    };

    // Skip hidden SSIDs
    if ssid.0.is_empty() { return; }

    let rsn = ieee80211::find_rsn(ie_data);
    let wpa2_psk = rsn.as_ref().map(|r| ieee80211::rsn_is_wpa2_psk(r)).unwrap_or(false);
    let capability = if data.len() > 22 {
        (data[20] as u16) | ((data[21] as u16) << 8)
    } else { 0 };
    let channel = ieee80211::ds_channel(ie_data).unwrap_or(0);

    unsafe {
        if SCAN_COUNT >= MAX_SCAN_RESULTS { return; }

        // Check for duplicate (same BSSID)
        for i in 0..SCAN_COUNT {
            if SCAN_RESULTS[i].bssid == bssid {
                // Update RSSI if better
                return;
            }
        }

        let mut result = ieee80211::ScanResult::empty();
        result.bssid = bssid;
        let sl = ssid.0.len().min(32);
        result.ssid[..sl].copy_from_slice(&ssid.0[..sl]);
        result.ssid_len = sl as u8;
        result.channel = channel;
        result.rssi = -50; // placeholder — real RSSI from PHY register
        result.capability = capability;
        result.has_rsn = rsn.is_some();
        result.wpa2_psk = wpa2_psk;

        SCAN_RESULTS[SCAN_COUNT] = result;
        SCAN_COUNT += 1;
    }
}

// ── Authentication ─────────────────────────────────────────────────

/// Called by ath9k when an Authentication frame is received.
pub fn handle_auth_response(data: &[u8]) {
    if data.len() < 24 { return; }

    let body = &data[24..];
    if body.len() < 6 { return; }

    let _algo = (body[0] as u16) | ((body[1] as u16) << 8);
    let seq = (body[2] as u16) | ((body[3] as u16) << 8);
    let status = (body[4] as u16) | ((body[5] as u16) << 8);

    if status == ieee80211::STATUS_SUCCESS && seq == 2 {
        up("[wifi] auth success, sending assoc...\r\n");
        unsafe { STATE = WifiState::Associating; }
        // Send association request
        send_assoc_request();
    } else {
        up("[wifi] auth failed, status=");
        dec(status as u64);
        up("\r\n");
        unsafe { STATE = WifiState::Disconnected; }
    }
}

/// Called by ath9k when an Association Response is received.
pub fn handle_assoc_response(data: &[u8]) {
    if data.len() < 24 { return; }

    let body = &data[24..];
    if body.len() < 4 { return; }

    let capability = (body[0] as u16) | ((body[1] as u16) << 8);
    let status = (body[2] as u16) | ((body[3] as u16) << 8);
    let _aid = if body.len() >= 6 {
        (body[4] as u16) | ((body[5] as u16) << 8) & 0x3FFF
    } else { 0 };

    let _ = capability;

    if status == ieee80211::STATUS_SUCCESS {
        up("[wifi] associated! starting WPA2 handshake...\r\n");
        unsafe { STATE = WifiState::Handshaking; }
        // Start WPA2 4-way handshake
        let _sta_addr = ath9k::mac();
        let ssid = unsafe { &CUR_SSID[..CUR_SSID_LEN] };
        let bssid = unsafe { CUR_BSSID };
        supplicant::start(ssid, &bssid);
    } else {
        up("[wifi] assoc failed, status=");
        dec(status as u64);
        up("\r\n");
        unsafe { STATE = WifiState::Disconnected; }
    }
}

/// Called by ath9k when an EAPOL frame is received.
pub fn handle_eapol(data: &[u8]) {
    if data.len() < 8 { return; }

    // Check for EAPOL EtherType
    let ether_type = (data[6] as u16) << 8 | data[7] as u16;
    if ether_type != ieee80211::EAPOL_ETHER_TYPE { return; }

    let eapol_body = &data[8..];
    let sta_addr = ath9k::mac();

    match supplicant::state() {
        supplicant::Wpa2State::WaitingMsg1 => {
            // Msg1: AP sends ANonce
            let mut ssid_buf = [0u8; 32];
            let ssid_len = unsafe { CUR_SSID_LEN.min(32) };
            unsafe {
                ssid_buf[..ssid_len].copy_from_slice(&CUR_SSID[..ssid_len]);
            }
            let _ = supplicant::handle_msg1(eapol_body, &sta_addr, &ssid_buf[..ssid_len]);
        }
        supplicant::Wpa2State::WaitingMsg3 => {
            // Msg3: AP sends GTK + MIC
            supplicant::handle_msg3(eapol_body, &sta_addr);
            if supplicant::is_completed() {
                unsafe { STATE = WifiState::Associated; }
                up("[wifi] WPA2 handshake complete!\r\n");
                // Now run DHCP
                if crate::driver::dhcp::run(5000) {
                    up("[wifi] DHCP OK, connected!\r\n");
                    unsafe { STATE = WifiState::Connected; }
                } else {
                    up("[wifi] DHCP failed\r\n");
                    unsafe { STATE = WifiState::Disconnected; }
                }
            }
        }
        _ => {}
    }
}

// ── Association ────────────────────────────────────────────────────

/// Connect to a network by SSID (requires profile in netman).
pub fn associate(ssid: &[u8]) -> &'static str {
    unsafe {
        if RADIO_COUNT == 0 {
            STATE = WifiState::NoRadio;
            return "no-radio";
        }
    }

    // Find scan result for this SSID
    unsafe {
        let mut found = [0u8; 6];
        let mut found_ch = 0u16;
        for i in 0..SCAN_COUNT {
            if SCAN_RESULTS[i].ssid_len as usize == ssid.len()
                && SCAN_RESULTS[i].ssid[..ssid.len()] == *ssid
            {
                found = SCAN_RESULTS[i].bssid;
                found_ch = SCAN_RESULTS[i].channel as u16 * 5 + 2407; // channel to MHz
                if found_ch < 2412 { found_ch = 2412; }
                break;
            }
        }
        if found == [0u8; 6] {
            return "not-found"; // SSID not in scan results
        }
        CUR_BSSID = found;
        CUR_CHANNEL = found_ch;
        CUR_SSID[..ssid.len().min(32)].copy_from_slice(&ssid[..ssid.len().min(32)]);
        CUR_SSID_LEN = ssid.len().min(32);
    };

    // Set channel
    if unsafe { CUR_CHANNEL } > 0 {
        ath9k::hw_set_channel(unsafe { CUR_CHANNEL });
    }

    // Check if we have a profile (for WPA2-PSK)
    let has_profile = crate::netman::find(ssid).is_some();

    up("[wifi] associating with '");
    for &c in ssid { uart::putchar(c); }
    up("'...\r\n");

    unsafe { STATE = WifiState::Authenticating; }

    // Send authentication request (Open System)
    let sta_addr = ath9k::mac();
    let mut auth_body = [0u8; 6];
    let auth_len = ieee80211::build_auth_open(1, &mut auth_body);
    let bssid = unsafe { CUR_BSSID };
    ath9k::tx_mgmt_frame(
        ieee80211::FC_SUBTYPE_AUTH,
        &bssid,
        &sta_addr,
        &bssid,
        &auth_body[..auth_len],
    );

    if has_profile {
        "auth-sent"
    } else {
        "open-auth-sent"
    }
}

/// Send an Association Request frame.
fn send_assoc_request() {
    let sta_addr = ath9k::mac();
    let mut body = [0u8; 256];

    // Build RSN IE if we have a profile
    let mut rsn_ie = [0u8; 22];
    let rsn_len = unsafe {
        if CUR_SSID_LEN > 0 && crate::netman::find(&CUR_SSID[..CUR_SSID_LEN]).is_some() {
            ieee80211::build_rsn_ie_psk(&mut rsn_ie)
        } else { 0 }
    };

    let ssid_slice = unsafe { &CUR_SSID[..CUR_SSID_LEN] };
    let body_len = ieee80211::build_assoc_request(
        ssid_slice,
        if rsn_len > 0 { Some(&rsn_ie[..rsn_len]) } else { None },
        &mut body,
    );

    let bssid = unsafe { CUR_BSSID };
    ath9k::tx_mgmt_frame(
        ieee80211::FC_SUBTYPE_ASSOC_REQ,
        &bssid,
        &sta_addr,
        &bssid,
        &body[..body_len],
    );
    up("[wifi] assoc request sent\r\n");
}

/// Send an EAPOL frame (for WPA2 handshake).
pub fn send_eapol_frame(frame: &[u8]) {
    // Wrap in802.11 data frame and send
    let sta_addr = ath9k::mac();
    let mut data_frame = [0u8; ieee80211::MAX_80211_FRAME];
    let mut pos = 0;

    // Frame Control: data frame, to_ds=1 (STA->AP)
    let fc = ieee80211::make_fc(true, false, false, false, false,
                                  ieee80211::FC_SUBTYPE_QOS_DATA, ieee80211::FC_TYPE_DATA);
    data_frame[pos] = (fc & 0xFF) as u8; pos += 1;
    data_frame[pos] = (fc >> 8) as u8; pos += 1;

    // Duration
    data_frame[pos] = 0; data_frame[pos + 1] = 0; pos += 2;

    let bssid = unsafe { CUR_BSSID };
    // Addr1 = BSSID (AP)
    data_frame[pos..pos + 6].copy_from_slice(&bssid); pos += 6;
    // Addr2 = Source (STA)
    data_frame[pos..pos + 6].copy_from_slice(&sta_addr); pos += 6;
    // Addr3 = BSSID (to_ds=1, so addr3 = destination = BSSID)
    data_frame[pos..pos + 6].copy_from_slice(&bssid); pos += 6;

    // Sequence Control
    data_frame[pos] = 0; data_frame[pos + 1] = 0; pos += 2;

    // QoS Control (0)
    data_frame[pos] = 0; data_frame[pos + 1] = 0; pos += 2;

    // LLC/SNAP header for EAPOL
    data_frame[pos] = 0xAA; data_frame[pos + 1] = 0xAA; data_frame[pos + 2] = 0x03;
    data_frame[pos + 3] = 0x00; data_frame[pos + 4] = 0x00; data_frame[pos + 5] = 0x00;
    data_frame[pos] = (ieee80211::EAPOL_ETHER_TYPE >> 8) as u8; pos += 1;
    data_frame[pos] = (ieee80211::EAPOL_ETHER_TYPE & 0xFF) as u8; pos += 1;
    // Wait, the LLC/SNAP is 6 bytes, then EtherType 2 bytes = 8 bytes
    // Let me redo this properly
    pos -= 8; // back up

    // LLC/SNAP: AA AA 03 00 00 00
    data_frame[pos..pos + 6].copy_from_slice(&[0xAA, 0xAA, 0x03, 0x00, 0x00, 0x00]);
    pos += 6;
    // EtherType
    data_frame[pos] = (ieee80211::EAPOL_ETHER_TYPE >> 8) as u8; pos += 1;
    data_frame[pos] = (ieee80211::EAPOL_ETHER_TYPE & 0xFF) as u8; pos += 1;

    // EAPOL body
    let body_len = frame.len().min(ieee80211::MAX_80211_FRAME - pos);
    data_frame[pos..pos + body_len].copy_from_slice(&frame[..body_len]);
    pos += body_len;

    ath9k::tx_frame(&data_frame[..pos]);
}

// ── Get PMK for current profile ────────────────────────────────────

pub fn get_current_pmk(out: &mut [u8; 32]) -> bool {
    unsafe {
        if CUR_SSID_LEN == 0 { return false; }
        let idx = match crate::netman::find(&CUR_SSID[..CUR_SSID_LEN]) {
            Some(i) => i,
            None => return false,
        };
        let mut psk = [0u8; 63];
        match crate::netman::reveal(idx, &mut psk) {
            Some(len) => {
                if len == 0 { return false; } // open network, no PMK
                crate::crypto::pbkdf2(&psk[..len], &CUR_SSID[..CUR_SSID_LEN], 4096, out);
                psk.fill(0);
                true
            }
            None => false,
        }
    }
}

// ── Disconnect ─────────────────────────────────────────────────────

pub fn disconnect() {
    unsafe {
        STATE = WifiState::Disconnected;
        CUR_SSID_LEN = 0;
        CUR_BSSID = [0; 6];
    }
    supplicant::reset();
    up("[wifi] disconnected\r\n");
}

// ── Driver trait ───────────────────────────────────────────────────

pub struct WifiDriver;

impl super::traits::Driver for WifiDriver {
    fn name(&self) -> &'static str { "Wi-Fi (ath9k + WPA2)" }
    fn device_type(&self) -> super::traits::DeviceType { super::traits::DeviceType::Legacy }
    fn init(&self) -> super::traits::DriverStatus {
        scan();
        super::traits::DriverStatus::Ok
    }
}
