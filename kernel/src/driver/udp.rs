/// UDP/IPv4 — минимальная поддержка для DHCP и DNS.
///
/// Слои выше (`crate::driver::dhcp`, `crate::driver::dns`) шлют датаграммы
/// через `send()`, а входящие UDP-пакеты из `net::poll()` попадают в `input()`.
/// Один входящий буфер, т.к. DHCP и DNS используются по очереди (блокирующе).
///
/// Проводной порядок байтов — big-endian.

use super::net;

pub const UDP_HDR: usize = 8;
pub const UDP_BUF: usize = 1514;

// --- Последняя принятая UDP-датаграмма ---
static mut RX_VALID: bool = false;
static mut RX_SRC_IP: [u8; 4] = [0; 4];
static mut RX_SRC_MAC: [u8; 6] = [0; 6];
static mut RX_SPORT: u16 = 0;
static mut RX_DPORT: u16 = 0;
static mut RX_LEN: usize = 0;
static mut RX_BUF: [u8; UDP_BUF] = [0; UDP_BUF];

/// Отправить UDP-датаграмму.
/// MAC резолвится через ARP (или broadcast для 255.255.255.255).
pub fn send(src_ip: [u8; 4], dst_ip: [u8; 4], src_port: u16, dst_port: u16, payload: &[u8]) -> bool {
    let bcast: [u8; 4] = [255, 255, 255, 255];
    let dst_mac = if dst_ip == bcast {
        [0xFF; 6]
    } else {
        net::resolve(dst_ip, 2000).unwrap_or([0xFF; 6])
    };
    net::send_udp(src_ip, dst_mac, dst_ip, src_port, dst_port, payload)
}

/// Входящий IP-пакет с protocol=17 из `net::poll()`.
/// Копирует датаграмму в единственный RX-слот.
pub fn input(data: &[u8], src_ip: [u8; 4], src_mac: [u8; 6]) {
    if data.len() < 34 + UDP_HDR {
        return;
    }
    let udp = &data[34..34 + UDP_HDR];
    let sport = ((udp[0] as u16) << 8) | udp[1] as u16;
    let dport = ((udp[2] as u16) << 8) | udp[3] as u16;
    let ulen = ((udp[4] as u16) << 8) | udp[5] as u16;
    let plen = (ulen as usize).saturating_sub(UDP_HDR).min(UDP_BUF);
    unsafe {
        RX_VALID = true;
        RX_SRC_IP = src_ip;
        RX_SRC_MAC = src_mac;
        RX_SPORT = sport;
        RX_DPORT = dport;
        RX_LEN = plen;
        if !data[42..].is_empty() {
            let n = plen.min(data.len().saturating_sub(42));
            RX_BUF[..n].copy_from_slice(&data[42..42 + n]);
        }
    }
}

/// Сбросить RX-слот (перед новым запросом).
pub fn rx_clear() {
    unsafe { RX_VALID = false; }
}

/// Показывать ли текущий RX-слот как валидный (то есть была ли датаграмма).
pub fn rx_pending() -> bool {
    unsafe { RX_VALID }
}

/// Данные текущей датаграммы (после `rx_pending()`).
pub fn rx_payload() -> &'static [u8] {
    unsafe { &RX_BUF[..RX_LEN] }
}

pub fn rx_src_ip() -> [u8; 4] {
    unsafe { RX_SRC_IP }
}

pub fn rx_src_mac() -> [u8; 6] {
    unsafe { RX_SRC_MAC }
}

pub fn rx_sport() -> u16 {
    unsafe { RX_SPORT }
}

pub fn rx_dport() -> u16 {
    unsafe { RX_DPORT }
}
