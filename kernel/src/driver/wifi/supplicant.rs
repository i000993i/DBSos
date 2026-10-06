#![allow(dead_code, unused_variables, non_upper_case_globals)]
// WPA2-PSK 4-Way Handshake (EAPOL) supplicant.

use crate::driver::uart;

// ── WPA2 Key constants ─────────────────────────────────────────────

const WPA2_KCK_LEN: usize = 16;
const WPA2_KEK_LEN: usize = 16;
const WPA2_TK_LEN: usize = 16;

// ── Supplicant state ───────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Wpa2State {
    Disconnected,
    WaitingMsg1,
    WaitingMsg3,
    HandshakeDone,
}

static mut STATE: Wpa2State = Wpa2State::Disconnected;
static mut ANONCE: [u8; 32] = [0u8; 32];
static mut SNONCE: [u8; 32] = [0u8; 32];
static mut PMK: [u8; 32] = [0u8; 32];
static mut PTK: [u8; 48] = [0u8; 48];
static mut GTK: [u8; 32] = [0u8; 32];
static mut KCK_VALID: bool = false;
static mut SA: [u8; 6] = [0u8; 6];
static mut AA: [u8; 6] = [0u8; 6];
static mut REPLAY_COUNTER: u64 = 0;

pub fn state() -> Wpa2State { unsafe { STATE } }

pub fn set_pmk(pmk: &[u8]) {
    let len = pmk.len().min(32);
    unsafe { PMK[..len].copy_from_slice(&pmk[..len]); }
    up("[wpa2] PMK set\r\n");
}

pub fn init() {
    unsafe {
        STATE = Wpa2State::Disconnected;
        KCK_VALID = false;
        REPLAY_COUNTER = 0;
        SNONCE = [0u8; 32];
        PTK = [0u8; 48];
        GTK = [0u8; 32];
    }
    up("[wpa2] supplicant reset\r\n");
}

pub fn start(ssid: &[u8], ap: &[u8; 6]) {
    unsafe {
        SA = super::ath9k::mac();
        AA = *ap;
        generate_snonce();
        STATE = Wpa2State::WaitingMsg1;
    }
    up("[wpa2] handshake started for '");
    for &c in ssid { uart::putchar(c); }
    up("'\r\n");
}

pub fn handle_msg1(msg: &[u8], _sta: &[u8; 6], _ssid: &[u8]) {
    unsafe {
        if STATE != Wpa2State::WaitingMsg1 { return; }
        if msg.len() < 45 { return; }

        ANONCE.copy_from_slice(&msg[13..45]);

        derive_ptk();
        REPLAY_COUNTER += 1;

        let mut mic_out = [0u8; 16];
        compute_mic(&mut mic_out);

        STATE = Wpa2State::WaitingMsg3;
        up("[wpa2] Msg1 handled, sending Msg2\r\n");

        let mut eapol_buf = [0u8; 256];
        let len = super::ieee80211::build_eapol_msg2(&SA, &AA, &SNONCE, &mic_out, &mut eapol_buf);
        super::send_eapol_frame(&eapol_buf[..len]);
    }
}

pub fn handle_msg3(msg: &[u8], _sta: &[u8; 6]) {
    unsafe {
        if STATE != Wpa2State::WaitingMsg3 { return; }
        if msg.len() < 45 { return; }

        STATE = Wpa2State::HandshakeDone;
        KCK_VALID = true;

        up("[wpa2] Msg3 handled, handshake done!\r\n");

        let mut eapol_buf = [0u8; 256];
        let len = super::ieee80211::build_eapol_msg4(&SA, &AA, &mut eapol_buf);
        super::send_eapol_frame(&eapol_buf[..len]);
    }
}

pub fn is_completed() -> bool {
    unsafe { STATE == Wpa2State::HandshakeDone }
}

pub fn reset() {
    init();
}

fn generate_snonce() {
    unsafe {
        let tsc = super::ath9k::current_channel() as u64;
        SNONCE[0] = (tsc & 0xFF) as u8;
        SNONCE[1] = ((tsc >> 8) & 0xFF) as u8;
        for i in 2..32 { SNONCE[i] = (i as u8).wrapping_add(0x42); }
    }
}

pub fn derive_ptk() {
    unsafe {
        let mut ctx = [0u8; 128];
        let mut pos = 0;

        let label = b"Pairwise key expansion";
        ctx[pos..pos + label.len()].copy_from_slice(label);
        pos += label.len();

        if SA < AA {
            ctx[pos..pos + 6].copy_from_slice(&SA); pos += 6;
            ctx[pos..pos + 6].copy_from_slice(&AA); pos += 6;
        } else {
            ctx[pos..pos + 6].copy_from_slice(&AA); pos += 6;
            ctx[pos..pos + 6].copy_from_slice(&SA); pos += 6;
        }

        if SNONCE < ANONCE {
            ctx[pos..pos + 32].copy_from_slice(&SNONCE); pos += 32;
            ctx[pos..pos + 32].copy_from_slice(&ANONCE); pos += 32;
        } else {
            ctx[pos..pos + 32].copy_from_slice(&ANONCE); pos += 32;
            ctx[pos..pos + 32].copy_from_slice(&SNONCE); pos += 32;
        }

        super::ieee80211::prf_sha1(&PMK, &ctx[..pos], &mut PTK, 48);
    }
}

fn compute_mic(mic_out: &mut [u8; 16]) {
    unsafe {
        if !KCK_VALID { mic_out.fill(0); return; }
        mic_out.fill(0);
    }
}

pub fn gtk() -> &'static [u8] {
    unsafe { &GTK }
}

fn up(s: &str) { uart::write_str(s); }
