//! Менеджер подключений (connect): профили Wi-Fi/сети с шифрованным хранением.
//!
//! Честная архитектура для QEMU/железа без Wi-Fi-радио:
//! - Скан PCI находит Ethernet (02/00) и беспроводные (02/80) адаптеры.
//!   Без радио `scan` так и говорит; профили и подключение работают поверх
//!   проводного e1000 (DHCP) — каркас готов к WPA, когда появится радио.
//! - PSK хранится НЕ открытым текстом: AES-128-CBC, ключ = PBKDF2(HMAC-SHA256,
//!   пароль = MAC-адрес машины, соль на профиль, 1000 итераций) — как в WPA2.
//! - Файл: /etc/net.conf (MAGIC + записи). Последнее подключение помечается.

use crate::driver::uart;

pub const MAX_PROFILES: usize = 8;
pub const MAX_SSID: usize = 32;
pub const MAX_PSK: usize = 63;
const ENC_MAX: usize = 64; // PSK 63 + PKCS#7 -> 64
const CONF_PATH: &[u8] = b"/etc/net.conf";
const MAGIC: &[u8; 8] = b"DBSNET01";

#[derive(Clone, Copy)]
pub struct Profile {
    pub ssid: [u8; MAX_SSID],
    pub ssid_len: u8,
    pub enc_len: u16, // 0 = открытая сеть (без PSK)
    pub salt: [u8; 16],
    pub iv: [u8; 16],
    pub enc: [u8; ENC_MAX],
}

impl Profile {
    const fn empty() -> Self {
        Profile {
            ssid: [0; MAX_SSID], ssid_len: 0, enc_len: 0,
            salt: [0; 16], iv: [0; 16], enc: [0; ENC_MAX],
        }
    }
    pub fn ssid_slice(&self) -> &[u8] { &self.ssid[..self.ssid_len as usize] }
    pub fn secured(&self) -> bool { self.enc_len != 0 }
}

static mut PROFILES: [Profile; MAX_PROFILES] = [Profile::empty(); MAX_PROFILES];
static mut NPROF: usize = 0;
static mut LAST: u8 = 0xFF; // индекс последнего подключения

fn up(s: &str) { uart::write_str(s); }

// ── Ключ устройства ────────────────────────────────────────────────

/// Ключ шифрования хранилища: PBKDF2(MAC машины, соль профиля).
fn device_key(salt: &[u8; 16], out: &mut [u8; 16]) {
    let mac = crate::driver::net::mac();
    let mut dk = [0u8; 32];
    crate::crypto::pbkdf2(&mac, salt, 1000, &mut dk);
    out.copy_from_slice(&dk[..16]);
}

/// Псевдослучайные соль/IV из таймера (не секрет, нужна уникальность).
fn fresh_rand(out: &mut [u8]) {
    let mut t = crate::timer::ticks().wrapping_add(crate::timer::millis().wrapping_mul(0x9E3779B9));
    for i in 0..out.len() {
        // xorshift32
        t ^= t << 13;
        t ^= t >> 17;
        t ^= t << 5;
        out[i] = (t ^ (t >> 11) ^ (crate::memory::free_count() as u64)) as u8;
        t = t.wrapping_add(0x2545F491);
    }
}

// ── Профили ────────────────────────────────────────────────────────

pub fn count() -> usize { unsafe { NPROF } }

pub fn get(idx: usize) -> Option<Profile> {
    unsafe {
        if idx < NPROF { Some(PROFILES[idx]) } else { None }
    }
}

pub fn find(ssid: &[u8]) -> Option<usize> {
    unsafe {
        for i in 0..NPROF {
            if PROFILES[i].ssid_len as usize == ssid.len()
                && PROFILES[i].ssid[..ssid.len()] == *ssid {
                return Some(i);
            }
        }
        None
    }
}

/// Добавить/обновить профиль. psk пустой = открытая сеть.
pub fn add(ssid: &[u8], psk: &[u8]) -> bool {
    if ssid.is_empty() || ssid.len() > MAX_SSID || psk.len() > MAX_PSK {
        return false;
    }
    let slot = match find(ssid) {
        Some(i) => i,
        None => {
            unsafe {
                if NPROF >= MAX_PROFILES { return false; }
                let s = NPROF;
                NPROF += 1;
                s
            }
        }
    };
    let mut p = Profile::empty();
    p.ssid[..ssid.len()].copy_from_slice(ssid);
    p.ssid_len = ssid.len() as u8;
    if !psk.is_empty() {
        fresh_rand(&mut p.salt);
        fresh_rand(&mut p.iv);
        let mut key = [0u8; 16];
        device_key(&p.salt, &mut key);
        let mut enc = [0u8; ENC_MAX];
        match crate::crypto::aes_cbc_encrypt(&key, &p.iv, psk, &mut enc) {
            Some(el) => {
                p.enc.copy_from_slice(&enc);
                p.enc_len = el as u16;
            }
            None => return false,
        }
        // затереть ключ в стеке
        for b in key.iter_mut() { *b = 0; }
    }
    unsafe { PROFILES[slot] = p; }
    save()
}

/// Расшифровать PSK профиля в out. Возвращает длину.
pub fn reveal(idx: usize, out: &mut [u8; MAX_PSK]) -> Option<usize> {
    let p = get(idx)?;
    if !p.secured() { return Some(0); }
    let mut key = [0u8; 16];
    device_key(&p.salt, &mut key);
    let mut tmp = [0u8; ENC_MAX];
    let r = crate::crypto::aes_cbc_decrypt(&key, &p.iv, &p.enc[..p.enc_len as usize], &mut tmp);
    for b in key.iter_mut() { *b = 0; }
    let n = r?;
    if n > MAX_PSK { return None; }
    out[..n].copy_from_slice(&tmp[..n]);
    for b in tmp.iter_mut() { *b = 0; }
    Some(n)
}

pub fn forget(ssid: &[u8]) -> bool {
    unsafe {
        let i = match find(ssid) {
            Some(i) => i,
            None => return false,
        };
        // затереть секрет перед удалением
        for b in PROFILES[i].enc.iter_mut() { *b = 0; }
        for j in i + 1..NPROF {
            PROFILES[j - 1] = PROFILES[j];
        }
        NPROF -= 1;
        PROFILES[NPROF] = Profile::empty();
        if LAST as usize >= NPROF { LAST = 0xFF; }
        save()
    }
}

// ── Персистентность ────────────────────────────────────────────────

fn save() -> bool {
    // Формат: MAGIC(8) count(1) last(1) [ssid_len(1) ssid(32) enc_len(2LE) salt(16) iv(16) enc(64)]*
    let mut buf = [0u8; 8 + 2 + MAX_PROFILES * (1 + 32 + 2 + 16 + 16 + 64)];
    buf[..8].copy_from_slice(MAGIC);
    unsafe {
        buf[8] = NPROF as u8;
        buf[9] = LAST;
        let mut o = 10;
        for i in 0..NPROF {
            let p = &PROFILES[i];
            buf[o] = p.ssid_len; o += 1;
            buf[o..o + 32].copy_from_slice(&p.ssid); o += 32;
            buf[o] = (p.enc_len & 0xFF) as u8;
            buf[o + 1] = (p.enc_len >> 8) as u8; o += 2;
            buf[o..o + 16].copy_from_slice(&p.salt); o += 16;
            buf[o..o + 16].copy_from_slice(&p.iv); o += 16;
            buf[o..o + ENC_MAX].copy_from_slice(&p.enc); o += ENC_MAX;
        }
        let total = o;
        let _ = crate::vfs::unlink(CONF_PATH);
        let fd = crate::vfs::open(CONF_PATH, 0x100 | 0x200 | 1); // CREAT|TRUNC|WRONLY
        if fd < 0 { return false; }
        let n = crate::vfs::write(fd, &buf[..total]);
        crate::vfs::close(fd);
        n == total as i64
    }
}

/// Загрузить профили при старте (ошибки = просто пустой список).
pub fn load() {
    unsafe { NPROF = 0; LAST = 0xFF; }
    let fd = crate::vfs::open(CONF_PATH, 0);
    if fd < 0 { return; }
    let mut buf = [0u8; 8 + 2 + MAX_PROFILES * (1 + 32 + 2 + 16 + 16 + 64)];
    let n = crate::vfs::read(fd, &mut buf);
    crate::vfs::close(fd);
    if n < 10 || buf[..8] != *MAGIC { return; }
    let cnt = (buf[8] as usize).min(MAX_PROFILES);
    let mut o = 10;
    unsafe {
        LAST = buf[9];
        for i in 0..cnt {
            if o + 1 + 32 + 2 + 16 + 16 + ENC_MAX > buf.len() { break; }
            let mut p = Profile::empty();
            p.ssid_len = buf[o].min(MAX_SSID as u8); o += 1;
            p.ssid.copy_from_slice(&buf[o..o + 32]); o += 32;
            p.enc_len = (buf[o] as u16) | ((buf[o + 1] as u16) << 8); o += 2;
            if p.enc_len as usize > ENC_MAX { break; }
            p.salt.copy_from_slice(&buf[o..o + 16]); o += 16;
            p.iv.copy_from_slice(&buf[o..o + 16]); o += 16;
            p.enc.copy_from_slice(&buf[o..o + ENC_MAX]); o += ENC_MAX;
            PROFILES[i] = p;
        }
        NPROF = cnt;
        if LAST as usize >= NPROF { LAST = 0xFF; }
    }
    up("[netman] profiles loaded\r\n");
}

// ── Скан адаптеров ─────────────────────────────────────────────────

/// (ethernet, wireless): честный скан PCI. Без радио wifi=0.
pub fn scan_adapters() -> (u32, u32) {
    let mut eth = 0u32;
    let mut wifi = 0u32;
    for dev in 0..32u8 {
        for func in 0..8u8 {
            let vendor = crate::driver::pci::read16(0, dev, func, 0x00);
            if vendor == 0xFFFF {
                if func == 0 { break; }
                continue;
            }
            let reg = crate::driver::pci::read32(0, dev, func, 0x08);
            let class = (reg >> 24) as u8;
            let subclass = ((reg >> 16) & 0xFF) as u8;
            if class == 0x02 && subclass == 0x00 { eth += 1; }
            if class == 0x02 && subclass == 0x80 { wifi += 1; }
            let htype = ((crate::driver::pci::read32(0, dev, func, 0x0C) >> 16) & 0xFF) as u8;
            if func == 0 && htype & 0x80 == 0 { break; }
        }
    }
    (eth, wifi)
}

// ── Подключение ────────────────────────────────────────────────────

/// Подключиться к профилю: пометить last + DHCP на проводном интерфейсе.
/// WPA-хендшейк требует радио — без него честно работаем поверх Ethernet.
pub fn connect(idx: usize) -> bool {
    let p = match get(idx) {
        Some(p) => p,
        None => return false,
    };
    unsafe { LAST = idx as u8; }
    save();
    up("[netman] join '");
    uart::write_str(core::str::from_utf8(p.ssid_slice()).unwrap_or("?"));
    up("' ");
    if p.secured() {
        up("(WPA-PSK stored encrypted; no radio -> wired DHCP)\r\n");
    } else {
        up("(open)\r\n");
    }
    if crate::driver::dhcp::run(5000) {
        up("[netman] DHCP OK\r\n");
        true
    } else {
        up("[netman] DHCP failed\r\n");
        false
    }
}

pub fn last() -> Option<usize> {
    unsafe {
        if LAST != 0xFF && (LAST as usize) < NPROF {
            Some(LAST as usize)
        } else {
            None
        }
    }
}
