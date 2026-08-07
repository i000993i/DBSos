/// DHCP/BOOTP — минимальный клиент поверх UDP.
///
/// Протокол: DISCOVER (broadcast 0.0.0.0:68 -> 255.255.255.255:67),
/// получаем OFFER, шлём REQUEST, получаем ACK и применяем конфигурацию:
/// IP, маску, gateway, DNS. Работает с DHCP-сервером QEMU slirp (10.0.2.2).
///
/// Всё блокирующее: `run()` сам качает `net::poll()` до ответа/таймаута.

use super::net;
use super::uart;
use super::udp;

pub const DHCP_SERVER_PORT: u16 = 67;
pub const DHCP_CLIENT_PORT: u16 = 68;

// --- BOOTP field offsets (проводной порядок, big-endian) ---
const F_OP: usize = 0;
const F_XID: usize = 4;
const F_FLAGS: usize = 10;
const F_YIADDR: usize = 16;
const F_CHADDR: usize = 28;
const F_MAGIC: usize = 236;
const F_OPT: usize = 240;
const BOOTP_LEN: usize = 300;

// --- DHCP message types (option 53) ---
const MT_DISCOVER: u8 = 1;
const MT_OFFER: u8 = 2;
const MT_REQUEST: u8 = 3;
const MT_ACK: u8 = 5;

// --- options ---
const O_MSG_TYPE: u8 = 53;
const O_REQ_LIST: u8 = 55;
const O_CLIENT_ID: u8 = 61;
const O_REQ_IP: u8 = 50;
const O_SRV_ID: u8 = 54;
const O_MASK: u8 = 1;
const O_ROUTER: u8 = 3;
const O_DNS: u8 = 6;
const O_END: u8 = 255;

const MAGIC_COOKIE: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

static mut XID: u32 = 0x5100_0000;

fn next_xid() -> u32 {
    unsafe { XID = XID.wrapping_add(0x0001_0001); XID }
}

fn bcast_ip() -> [u8; 4] {
    [255, 255, 255, 255]
}

fn add_opt(pkt: &mut [u8; BOOTP_LEN], pos: &mut usize, code: u8, data: &[u8]) {
    if *pos + 2 + data.len() >= BOOTP_LEN {
        return;
    }
    pkt[*pos] = code;
    *pos += 1;
    pkt[*pos] = data.len() as u8;
    *pos += 1;
    pkt[*pos..*pos + data.len()].copy_from_slice(data);
    *pos += data.len();
}

/// Собрать базовый BOOTP-заголовок (request, наш MAC, cookie).
fn build_header(pkt: &mut [u8; BOOTP_LEN], xid: u32) {
    pkt.fill(0);
    pkt[F_OP] = 1; // request
    pkt[1] = 1; // htype = ethernet
    pkt[2] = 6; // hlen
    pkt[F_XID..F_XID + 4].copy_from_slice(&xid.to_be_bytes());
    pkt[F_FLAGS] = 0x80; pkt[F_FLAGS + 1] = 0x00; // broadcast
    let mac = net::mac();
    pkt[F_CHADDR..F_CHADDR + 6].copy_from_slice(&mac);
    pkt[F_MAGIC..F_MAGIC + 4].copy_from_slice(&MAGIC_COOKIE);
}

/// Отправить DISCOVER.
fn send_discover(xid: u32) {
    let mut pkt = [0u8; BOOTP_LEN];
    build_header(&mut pkt, xid);
    let mut pos = F_OPT;
    add_opt(&mut pkt, &mut pos, O_MSG_TYPE, &[MT_DISCOVER]);
    add_opt(&mut pkt, &mut pos, O_REQ_LIST, &[O_MASK, O_ROUTER, O_DNS]);
    let mut cid = [0u8; 7];
    cid[0] = 1;
    cid[1..7].copy_from_slice(&net::mac());
    add_opt(&mut pkt, &mut pos, O_CLIENT_ID, &cid);
    pkt[pos] = O_END;
    udp::send([0, 0, 0, 0], bcast_ip(), DHCP_CLIENT_PORT, DHCP_SERVER_PORT, &pkt);
}

/// Отправить REQUEST для выбранного IP у указанного сервера.
fn send_request(xid: u32, requested_ip: [u8; 4], server_id: [u8; 4]) {
    let mut pkt = [0u8; BOOTP_LEN];
    build_header(&mut pkt, xid);
    let mut pos = F_OPT;
    add_opt(&mut pkt, &mut pos, O_MSG_TYPE, &[MT_REQUEST]);
    add_opt(&mut pkt, &mut pos, O_REQ_IP, &requested_ip);
    add_opt(&mut pkt, &mut pos, O_SRV_ID, &server_id);
    add_opt(&mut pkt, &mut pos, O_REQ_LIST, &[O_MASK, O_ROUTER, O_DNS]);
    let mut cid = [0u8; 7];
    cid[0] = 1;
    cid[1..7].copy_from_slice(&net::mac());
    add_opt(&mut pkt, &mut pos, O_CLIENT_ID, &cid);
    pkt[pos] = O_END;
    udp::send([0, 0, 0, 0], bcast_ip(), DHCP_CLIENT_PORT, DHCP_SERVER_PORT, &pkt);
}

/// Прочитать option из DHCP-ответа. Возвращает копию значения.
fn get_opt(pkt: &[u8], code: u8, out: &mut [u8]) -> usize {
    if pkt.len() < F_OPT {
        return 0;
    }
    let mut pos = F_OPT;
    while pos + 1 < pkt.len() {
        let c = pkt[pos];
        if c == O_END {
            break;
        }
        let len = pkt[pos + 1] as usize;
        if pos + 2 + len > pkt.len() {
            break;
        }
        if c == code {
            let n = len.min(out.len());
            out[..n].copy_from_slice(&pkt[pos + 2..pos + 2 + n]);
            return n;
        }
        pos += 2 + len;
    }
    0
}

fn message_type(pkt: &[u8]) -> Option<u8> {
    let mut m = [0u8; 1];
    if get_opt(pkt, O_MSG_TYPE, &mut m) == 1 {
        Some(m[0])
    } else {
        None
    }
}

fn uart_ip(ip: &[u8; 4]) {
    for i in 0..4 {
        let mut v = ip[i] as u64;
        let mut d = [0u8; 3];
        let mut n = 0;
        while v > 0 {
            d[n] = b'0' + (v % 10) as u8;
            v /= 10;
            n += 1;
        }
        if n == 0 {
            uart::putchar(b'0');
        }
        while n > 0 {
            n -= 1;
            uart::putchar(d[n]);
        }
        if i < 3 {
            uart::putchar(b'.');
        }
    }
}

/// Получить DHCP-ответ (OFFER/ACK) для нашего xid.
/// Качает `net::poll()` до нужного пакета или таймаута.
fn wait_reply(xid: u32, timeout_ms: u64) -> Option<[u8; BOOTP_LEN]> {
    let deadline = crate::timer::millis() + timeout_ms;
    while crate::timer::millis() < deadline {
        net::poll();
        if !udp::rx_pending() {
            continue;
        }
        if udp::rx_dport() != DHCP_CLIENT_PORT {
            continue;
        }
        let pl = udp::rx_payload();
        if pl.len() < F_OPT {
            continue;
        }
        let pkt = pl;
        if pkt[F_OP] != 2 {
            continue;
        }
        let rx_xid = ((pkt[F_XID] as u32) << 24)
            | ((pkt[F_XID + 1] as u32) << 16)
            | ((pkt[F_XID + 2] as u32) << 8)
            | pkt[F_XID + 3] as u32;
        if rx_xid != xid {
            continue;
        }
        let mut buf = [0u8; BOOTP_LEN];
        let n = pl.len().min(BOOTP_LEN);
        buf[..n].copy_from_slice(&pl[..n]);
        return Some(buf);
    }
    None
}

/// Запустить полный цикл DHCP. Возвращает true при успехе и применяет конфиг.
pub fn run(timeout_ms: u64) -> bool {
    let xid = next_xid();
    uart::write_str("[DHCP] DISCOVER...\r\n");
    udp::rx_clear();
    send_discover(xid);

    // Ожидаем OFFER
    let offer = match wait_reply(xid, timeout_ms) {
        Some(o) => o,
        None => {
            uart::write_str("[DHCP] timeout waiting OFFER\r\n");
            return false;
        }
    };
    if message_type(&offer) != Some(MT_OFFER) {
        uart::write_str("[DHCP] expected OFFER, got other\r\n");
        return false;
    }
    let mut yiaddr = [0u8; 4];
    yiaddr.copy_from_slice(&offer[F_YIADDR..F_YIADDR + 4]);
    let mut server_id = [0u8; 4];
    get_opt(&offer, O_SRV_ID, &mut server_id);
    uart::write_str("[DHCP] OFFER ip=");
    uart_ip(&yiaddr);
    uart::write_str("\r\n");

    // REQUEST
    uart::write_str("[DHCP] REQUEST...\r\n");
    udp::rx_clear();
    send_request(xid, yiaddr, server_id);

    let ack = match wait_reply(xid, timeout_ms) {
        Some(a) => a,
        None => {
            uart::write_str("[DHCP] timeout waiting ACK\r\n");
            return false;
        }
    };
    if message_type(&ack) != Some(MT_ACK) {
        uart::write_str("[DHCP] expected ACK, got other\r\n");
        return false;
    }

    // Применяем конфигурацию
    let mut mask = [255, 255, 255, 0];
    let mut gw = server_id;
    let mut dns = [0u8; 4];
    get_opt(&ack, O_MASK, &mut mask);
    if get_opt(&ack, O_ROUTER, &mut gw) == 0 {
        gw = server_id;
    }
    get_opt(&ack, O_DNS, &mut dns);
    net::set_ip(yiaddr);
    net::set_netmask(mask);
    net::set_gateway(gw);
    if dns != [0, 0, 0, 0] {
        net::set_dns_server(dns);
    }

    uart::write_str("[DHCP] ACK! ip=");
    uart_ip(&yiaddr);
    uart::write_str(" gw=");
    uart_ip(&gw);
    uart::write_str(" mask=");
    uart_ip(&mask);
    uart::write_str(" dns=");
    uart_ip(&dns);
    uart::write_str("\r\n");
    true
}
