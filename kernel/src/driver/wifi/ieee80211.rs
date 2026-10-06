#![allow(dead_code, unused_variables, unused_imports)]
// IEEE 802.11 frame structures — management, control, data.

use crate::driver::uart;

// ── Frame Control field (2 bytes) ──────────────────────────────────

pub const FC_TYPE_MGMT: u8 = 0;
pub const FC_TYPE_CTRL: u8 = 1;
pub const FC_TYPE_DATA: u8 = 2;

// Management subtypes
pub const FC_SUBTYPE_ASSOC_REQ: u8 = 0;
pub const FC_SUBTYPE_ASSOC_RESP: u8 = 1;
pub const FC_SUBTYPE_REASSOC_REQ: u8 = 2;
pub const FC_SUBTYPE_REASSOC_RESP: u8 = 3;
pub const FC_SUBTYPE_PROBE_REQ: u8 = 4;
pub const FC_SUBTYPE_PROBE_RESP: u8 = 5;
pub const FC_SUBTYPE_BEACON: u8 = 8;
pub const FC_SUBTYPE_AUTH: u8 = 11;
pub const FC_SUBTYPE_DEAUTH: u8 = 12;
pub const FC_SUBTYPE_ACTION: u8 = 13;

// Control subtypes
pub const FC_SUBTYPE_RTS: u8 = 11;
pub const FC_SUBTYPE_CTS: u8 = 12;
pub const FC_SUBTYPE_ACK: u8 = 13;

// Data subtypes
pub const FC_SUBTYPE_DATA_CF: u8 = 0;
pub const FC_SUBTYPE_QOS_DATA: u8 = 8;

// ── Constants ──────────────────────────────────────────────────────

pub const BROADCAST: [u8; 6] = [0xFF; 6];
pub const MAX_80211_FRAME: usize = 2346;
pub const EAPOL_ETHER_TYPE: u16 = 0x888E;
pub const STATUS_SUCCESS: u16 = 0;

// ── Element IDs ────────────────────────────────────────────────────

pub const EI_SSID: u8 = 0;
pub const EI_SUPPORTED_RATES: u8 = 1;
pub const EI_CHANNEL: u8 = 3;
pub const EI_RSN: u8 = 48;

// ── Scan result (returned by beacon/probe parsing) ─────────────────

#[derive(Clone, Copy)]
pub struct ScanResult {
    pub bssid: [u8; 6],
    pub ssid: [u8; 32],
    pub ssid_len: u8,
    pub channel: u8,
    pub rssi: i8,
    pub capability: u16,
    pub has_rsn: bool,
    pub wpa2_psk: bool,
}

impl ScanResult {
    pub const fn empty() -> Self {
        ScanResult {
            bssid: [0; 6], ssid: [0; 32], ssid_len: 0,
            channel: 0, rssi: 0, capability: 0, has_rsn: false, wpa2_psk: false,
        }
    }
    pub fn ssid_str(&self) -> &str {
        core::str::from_utf8(&self.ssid[..self.ssid_len as usize]).unwrap_or("")
    }
}

// ── Information Element ────────────────────────────────────────────

pub struct Ie {
    pub id: u8,
    pub data: [u8; 255],
    pub len: usize,
}

pub fn parse_ies(frame: &[u8], offset: usize, ies: &mut [Ie]) -> usize {
    let mut pos = offset;
    let mut count = 0;
    while pos + 2 <= frame.len() && count < ies.len() {
        let eid = frame[pos];
        let elen = frame[pos + 1] as usize;
        if pos + 2 + elen > frame.len() { break; }
        ies[count].id = eid;
        ies[count].len = elen;
        ies[count].data[..elen].copy_from_slice(&frame[pos + 2..pos + 2 + elen]);
        count += 1;
        pos += 2 + elen;
    }
    count
}

// ── IE finders (operate on raw bytes) ──────────────────────────────

pub fn find_ssid(ie_data: &[u8]) -> Option<(&[u8], usize)> {
    let mut pos = 0;
    while pos + 2 <= ie_data.len() {
        let eid = ie_data[pos];
        let elen = ie_data[pos + 1] as usize;
        if pos + 2 + elen > ie_data.len() { break; }
        if eid == EI_SSID {
            return Some((&ie_data[pos + 2..pos + 2 + elen], elen));
        }
        pos += 2 + elen;
    }
    None
}

pub fn find_channel(ie_data: &[u8]) -> Option<u8> {
    let mut pos = 0;
    while pos + 2 <= ie_data.len() {
        let eid = ie_data[pos];
        let elen = ie_data[pos + 1] as usize;
        if pos + 2 + elen > ie_data.len() { break; }
        if eid == EI_CHANNEL && elen >= 1 {
            return Some(ie_data[pos + 2]);
        }
        pos += 2 + elen;
    }
    None
}

pub fn ds_channel(ie_data: &[u8]) -> Option<u8> { find_channel(ie_data) }

pub fn find_rsn(ie_data: &[u8]) -> Option<RsnInfo> {
    let mut pos = 0;
    while pos + 2 <= ie_data.len() {
        let eid = ie_data[pos];
        let elen = ie_data[pos + 1] as usize;
        if pos + 2 + elen > ie_data.len() { break; }
        if eid == EI_RSN && elen >= 2 {
            let d = &ie_data[pos + 2..pos + 2 + elen];
            let version = (d[0] as u16) | ((d[1] as u16) << 8);
            if version != 1 { return None; }
            return Some(RsnInfo { version });
        }
        pos += 2 + elen;
    }
    None
}

pub struct RsnInfo {
    pub version: u16,
}

pub fn rsn_is_wpa2_psk(_rsn: &RsnInfo) -> bool {
    true // simplified: assume WPA2-PSK if RSN present
}

// ── Frame Control ──────────────────────────────────────────────────

pub fn make_fc(to_ds: bool, from_ds: bool, more_frag: bool, retry: bool,
               power_mgmt: bool, subtype: u8, ftype: u8) -> u16 {
    let mut fc = 0u16;
    if to_ds { fc |= 1 << 8; }
    if from_ds { fc |= 1 << 9; }
    if more_frag { fc |= 1 << 10; }
    if retry { fc |= 1 << 11; }
    if power_mgmt { fc |= 1 << 12; }
    fc |= ((subtype as u16) & 0xF) << 4;
    fc |= ((ftype as u16) & 0x3) << 2;
    fc
}

pub fn fc_type(fc: u16) -> u8 { ((fc >> 2) & 0x3) as u8 }
pub fn fc_subtype(fc: u16) -> u8 { ((fc >> 4) & 0xF) as u8 }

// ── Builder functions ──────────────────────────────────────────────

pub fn build_probe_request(ssid: &[u8], body: &mut [u8]) -> usize {
    let mut pos = 0;
    body[pos] = EI_SSID; pos += 1;
    body[pos] = ssid.len() as u8; pos += 1;
    if !ssid.is_empty() {
        body[pos..pos + ssid.len()].copy_from_slice(ssid);
        pos += ssid.len();
    }
    body[pos] = EI_SUPPORTED_RATES; pos += 1;
    body[pos] = 8; pos += 1;
    body[pos..pos + 8].copy_from_slice(&[0x82, 0x84, 0x8B, 0x96, 0x0C, 0x12, 0x18, 0x24]);
    pos += 8;
    pos
}

pub fn build_auth_open(seq: u16, body: &mut [u8]) -> usize {
    body[0] = 0; body[1] = 0; // algo = Open System
    body[2] = (seq & 0xFF) as u8;
    body[3] = ((seq >> 8) & 0xFF) as u8;
    4
}

pub fn build_assoc_request(ssid: &[u8], rsn_ie: Option<&[u8]>, body: &mut [u8]) -> usize {
    let mut pos = 0;
    // Capability info
    body[pos] = 0x01; pos += 1;
    body[pos] = 0x04; pos += 1; // ESS + Short Preamble + Short Slot
    body[pos] = 0x00; pos += 1;
    body[pos] = 0x00; pos += 1;
    // Listen interval
    body[pos] = 0x01; pos += 1;
    body[pos] = 0x00; pos += 1;
    // SSID
    body[pos] = EI_SSID; pos += 1;
    body[pos] = ssid.len() as u8; pos += 1;
    body[pos..pos + ssid.len()].copy_from_slice(ssid);
    pos += ssid.len();
    // Supported rates
    body[pos] = EI_SUPPORTED_RATES; pos += 1;
    body[pos] = 8; pos += 1;
    body[pos..pos + 8].copy_from_slice(&[0x82, 0x84, 0x8B, 0x96, 0x0C, 0x12, 0x18, 0x24]);
    pos += 8;
    // DS Parameter Set (channel)
    body[pos] = EI_CHANNEL; pos += 1;
    body[pos] = 1; pos += 1;
    body[pos] = 6; pos += 1;
    // RSN IE
    if let Some(rsn) = rsn_ie {
        let rlen = rsn.len().min(255);
        body[pos..pos + rlen].copy_from_slice(&rsn[..rlen]);
        pos += rlen;
    }
    pos
}

pub fn build_rsn_ie_psk(buf: &mut [u8]) -> usize {
    if buf.len() < 22 { return 0; }
    buf[0] = EI_RSN;       // Element ID
    buf[1] = 20;            // Length
    buf[2] = 1; buf[3] = 0; // Version 1
    // Group cipher: CCMP (00-0F-AC-4)
    buf[4] = 0x00; buf[5] = 0x0F; buf[6] = 0xAC; buf[7] = 0x04;
    // Pairwise cipher suite count = 1
    buf[8] = 1; buf[9] = 0;
    // Pairwise cipher: CCMP (00-0F-AC-4)
    buf[10] = 0x00; buf[11] = 0x0F; buf[12] = 0xAC; buf[13] = 0x04;
    // AKM suite count = 1
    buf[14] = 1; buf[15] = 0;
    // AKM: PSK (00-0F-AC-2)
    buf[16] = 0x00; buf[17] = 0x0F; buf[18] = 0xAC; buf[19] = 0x02;
    // RSN capabilities
    buf[20] = 0x00; buf[21] = 0x00;
    22
}

// ── EAPOL helpers ──────────────────────────────────────────────────

pub fn eapol_key_len(data: &[u8]) -> Option<usize> {
    if data.len() < 4 { return None; }
    let len_field = ((data[1] as usize) << 8) | data[2] as usize;
    Some(len_field + 4)
}

pub fn parse_eapol_key(data: &[u8]) -> Option<EapolKey> {
    if data.len() < 99 { return None; }
    Some(EapolKey {
        key_info: (data[5] as u16) << 8 | data[6] as u16,
        key_len: ((data[7] as u16) << 8) | data[8] as u16,
        key_replay_counter: ((data[9] as u64) << 56) | ((data[10] as u64) << 48)
            | ((data[11] as u64) << 40) | ((data[12] as u64) << 32)
            | ((data[13] as u64) << 24) | ((data[14] as u64) << 16)
            | ((data[15] as u64) << 8) | data[16] as u64,
        key_mic: {
            let mut mic = [0u8; 16];
            let end = 93.min(data.len());
            let start = 77.min(data.len());
            let copy_len = end.saturating_sub(start);
            mic[..copy_len].copy_from_slice(&data[start..start + copy_len]);
            mic
        },
        key_data_len: ((data[97] as u16) << 8) | data[98] as u16,
    })
}

pub struct EapolKey {
    pub key_info: u16,
    pub key_len: u16,
    pub key_replay_counter: u64,
    pub key_mic: [u8; 16],
    pub key_data_len: u16,
}

// ── Crypto primitives ──────────────────────────────────────────────

pub fn prf_sha1(key: &[u8], ctx: &[u8], out: &mut [u8], len: usize) {
    let mut pos = 0;
    let mut counter = 0u8;
    while pos < len {
        let mut hmac = [0u8; 20];
        hmac_sha1(key, ctx, counter, &mut hmac);
        let copy_len = (len - pos).min(20);
        out[pos..pos + copy_len].copy_from_slice(&hmac[..copy_len]);
        pos += copy_len;
        counter += 1;
    }
}

pub fn hmac_sha1(key: &[u8], msg: &[u8], extra: u8, out: &mut [u8; 20]) {
    let mut k_ipad = [0x36u8; 64];
    let mut k_opad = [0x5Cu8; 64];

    for (i, &kb) in key.iter().enumerate() {
        if i >= 64 { break; }
        k_ipad[i] ^= kb;
        k_opad[i] ^= kb;
    }

    let mut inner_msg = [0u8; 256];
    inner_msg[0] = extra;
    let mlen = msg.len().min(254);
    inner_msg[1..1 + mlen].copy_from_slice(&msg[..mlen]);

    let mut sha = Sha1::new();
    sha.update(&k_ipad);
    sha.update(&inner_msg[..1 + mlen]);
    let inner = sha.finalize();

    let mut sha2 = Sha1::new();
    sha2.update(&k_opad);
    sha2.update(&inner);
    let result = sha2.finalize();
    out.copy_from_slice(&result);
}

struct Sha1 {
    state: [u32; 5],
    total: u64,
    buf: [u8; 64],
    buf_len: usize,
}

impl Sha1 {
    fn new() -> Self {
        Sha1 {
            state: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0],
            total: 0,
            buf: [0u8; 64],
            buf_len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        let mut i = 0;
        self.total += data.len() as u64;
        while i < data.len() {
            let remaining = 64 - self.buf_len;
            let take = remaining.min(data.len() - i);
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[i..i + take]);
            self.buf_len += take;
            i += take;
            if self.buf_len == 64 {
                let block = self.buf;
                self.process_block(&block);
                self.buf_len = 0;
            }
        }
    }

    fn process_block(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = ((block[i * 4] as u32) << 24)
                | ((block[i * 4 + 1] as u32) << 16)
                | ((block[i * 4 + 2] as u32) << 8)
                | (block[i * 4 + 3] as u32);
        }
        for i in 16..80 {
            let x = w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16];
            w[i] = x.rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = self.state;
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
    }

    fn finalize(mut self) -> [u8; 20] {
        let total_bits = self.total * 8;
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 56 {
            while self.buf_len < 64 {
                self.buf_len += 1;
            }
            let block = self.buf;
            self.process_block(&block);
            self.buf_len = 0;
            self.buf = [0u8; 64];
        }
        while self.buf_len < 56 {
            self.buf[self.buf_len] = 0;
            self.buf_len += 1;
        }
        self.buf[56] = (total_bits >> 56) as u8;
        self.buf[57] = (total_bits >> 48) as u8;
        self.buf[58] = (total_bits >> 40) as u8;
        self.buf[59] = (total_bits >> 32) as u8;
        self.buf[60] = (total_bits >> 24) as u8;
        self.buf[61] = (total_bits >> 16) as u8;
        self.buf[62] = (total_bits >> 8) as u8;
        self.buf[63] = total_bits as u8;
        let block = self.buf;
        self.process_block(&block);

        let mut out = [0u8; 20];
        for i in 0..5 {
            out[i * 4] = (self.state[i] >> 24) as u8;
            out[i * 4 + 1] = (self.state[i] >> 16) as u8;
            out[i * 4 + 2] = (self.state[i] >> 8) as u8;
            out[i * 4 + 3] = self.state[i] as u8;
        }
        out
    }
}

// ── Scratch buffer for EAPOL message construction ──────────────────

pub fn build_eapol_msg2(
    sta_addr: &[u8; 6],
    ap_addr: &[u8; 6],
    snonce: &[u8; 32],
    mic: &[u8; 16],
    out: &mut [u8],
) -> usize {
    let mut pos = 0;
    // EAPOL header
    out[pos] = 0x01; pos += 1; // version 1
    out[pos] = 0x03; pos += 1; // type = EAPOL-Key
    out[pos] = 0; out[pos + 1] = 0; pos += 2; // length (filled later)
    // Key info: Pairwise, MIC set
    let key_info: u16 = 0x010A;
    out[pos] = (key_info >> 8) as u8; pos += 1;
    out[pos] = (key_info & 0xFF) as u8; pos += 1;
    // Key length
    out[pos] = 0; out[pos + 1] = 16; pos += 2;
    // Key replay counter (8 bytes)
    out[pos] = 1; pos += 8;
    // Key nonce (32 bytes)
    out[pos..pos + 32].copy_from_slice(snonce);
    pos += 32;
    // Key MIC (16 bytes)
    out[pos..pos + 16].copy_from_slice(mic);
    pos += 16;
    // Key data length
    out[pos] = 0; out[pos + 1] = 0; pos += 2;
    // Update EAPOL length
    let eapol_len = pos - 4;
    out[2] = (eapol_len >> 8) as u8;
    out[3] = (eapol_len & 0xFF) as u8;
    pos
}

pub fn build_eapol_msg4(
    sta_addr: &[u8; 6],
    ap_addr: &[u8; 6],
    out: &mut [u8],
) -> usize {
    let mut pos = 0;
    out[pos] = 0x01; pos += 1; // version 1
    out[pos] = 0x03; pos += 1; // type = EAPOL-Key
    out[pos] = 0; out[pos + 1] = 0; pos += 2; // length
    // Key info: Pairwise, ACK
    let key_info: u16 = 0x030A;
    out[pos] = (key_info >> 8) as u8; pos += 1;
    out[pos] = (key_info & 0xFF) as u8; pos += 1;
    out[pos] = 0; out[pos + 1] = 0; pos += 2; // key length
    pos += 8; // replay counter
    pos += 32; // key nonce
    pos += 16; // key MIC
    out[pos] = 0; out[pos + 1] = 0; pos += 2; // key data len
    let eapol_len = pos - 4;
    out[2] = (eapol_len >> 8) as u8;
    out[3] = (eapol_len & 0xFF) as u8;
    pos
}
