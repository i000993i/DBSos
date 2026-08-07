/// DNS — минимальный резолвер A-записей поверх UDP:53.
///
/// Формирует query (header + QNAME + QTYPE=A + QCLASS=IN), отправляет на
/// DNS-сервер из `net::dns_server()` (после DHCP — 10.0.2.3 у QEMU slirp),
/// ждёт ответ и разбирает секцию ответов. Возвращает IPv4.
///
/// Всё блокирующее: качает `net::poll()` до ответа/таймаута.

use super::net;
use super::uart;
use super::udp;

pub const DNS_PORT: u16 = 53;

const DNS_ID: u16 = 0x1234;
const DNS_MAX: usize = 512;

fn be16(v: u16) -> [u8; 2] {
    [(v >> 8) as u8, v as u8]
}

/// Закодировать имя в формат DNS-labels ("a.b" -> "\x01a\x01b\x00").
fn encode_name(name: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut pos = 0usize;
    let mut label = 0usize;
    let mut i = 0usize;
    while i <= name.len() {
        let done = i == name.len();
        let c = if done { 0 } else { name[i] };
        if done || c == b'.' {
            if pos + 1 + label > out.len() {
                return None;
            }
            out[pos] = label as u8;
            if label == 0 && !done {
                return None; // двойная точка
            }
            pos += 1;
            out[pos - 1..pos - 1 + label]
                .copy_from_slice(&name[i - label..i]);
            pos += label;
            label = 0;
            if done {
                out[pos] = 0;
                pos += 1;
                return Some(pos);
            }
        } else {
            label += 1;
        }
        i += 1;
    }
    None
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

fn rd16(pkt: &[u8], off: usize) -> u16 {
    ((pkt[off] as u16) << 8) | pkt[off + 1] as u16
}

fn read_qname_len(pkt: &[u8], mut off: usize) -> Option<usize> {
    // Возвращает длину имени в "жирном" виде (с учётом возможных compression-указателей).
    let mut jumps = 0usize;
    let mut total = 0usize;
    loop {
        if off >= pkt.len() {
            return None;
        }
        let len = pkt[off];
        if len == 0 {
            if jumps == 0 {
                total += 1;
            }
            return Some(total);
        } else if len & 0xC0 == 0xC0 {
            if jumps == 0 {
                total += 2;
            }
            jumps += 1;
            if jumps > 16 {
                return None;
            }
            return Some(total);
        } else {
            if jumps == 0 {
                total += 1 + len as usize + 1;
            }
            off += 1 + len as usize;
            if off >= pkt.len() {
                return None;
            }
            if jumps > 16 {
                return None;
            }
        }
    }
}

/// Резолв имени в IPv4 через наш DNS-сервер.
/// `buf` — буфер для QNAME (нужен потому что encode_name пишет во временный массив).
pub fn resolve(name: &[u8], timeout_ms: u64) -> Option<[u8; 4]> {
    let mut qname = [0u8; 255];
    let qname_len = encode_name(name, &mut qname)?;

    let dns_ip = net::dns_server();
    if dns_ip == [0, 0, 0, 0] {
        uart::write_str("[DNS] no DNS server configured\r\n");
        return None;
    }

    let mut query = [0u8; DNS_MAX];
    // DNS header (12 bytes)
    query[0] = (DNS_ID >> 8) as u8;
    query[1] = DNS_ID as u8;
    query[2] = 0x01; // flags: RD
    query[3] = 0x00;
    query[4] = 0x00; query[5] = 0x01; // QDCOUNT = 1
    // ANCOUNT/NSCOUNT/ARCOUNT = 0
    query[6..12].fill(0);
    let mut pos = 12;
    query[pos..pos + qname_len].copy_from_slice(&qname[..qname_len]);
    pos += qname_len;
    query[pos..pos + 2].copy_from_slice(&be16(1)); // QTYPE = A
    pos += 2;
    query[pos..pos + 2].copy_from_slice(&be16(1)); // QCLASS = IN
    pos += 2;

    let src_port: u16 = 0xC000 | ((net::mac()[4] as u16) << 8) | net::mac()[5] as u16;

    uart::write_str("[DNS] query ");
    let s = core::str::from_utf8(name).unwrap_or("?");
    uart::write_str(s);
    uart::write_str(" -> ");
    uart_ip(&dns_ip);
    uart::write_str("\r\n");

    udp::rx_clear();
    if !udp::send(net::our_ip(), dns_ip, src_port, DNS_PORT, &query[..pos]) {
        uart::write_str("[DNS] send failed\r\n");
        return None;
    }

    let deadline = crate::timer::millis() + timeout_ms;
    while crate::timer::millis() < deadline {
        net::poll();
        if !udp::rx_pending() {
            continue;
        }
        // Ответ должен прийти с порта 53 (наш src_port может отличаться, slirp эхом).
        let pl = udp::rx_payload();
        if pl.len() < 12 {
            continue;
        }
        let id = rd16(pl, 0);
        if id != DNS_ID {
            continue;
        }
        let ancount = rd16(pl, 6);
        if ancount == 0 {
            continue;
        }
        // Пропускаем question-секцию
        let mut off = 12usize;
        if let Some(qlen) = read_qname_len(pl, off) {
            off += qlen;
            off += 4; // QTYPE + QCLASS
        } else {
            continue;
        }
        // Разбираем ответы (A-записи)
        for _ in 0..ancount {
            if off + 10 > pl.len() {
                break;
            }
            let _ = read_qname_len(pl, off).unwrap_or(0);
            // name может быть compression-указателем (2 байта) или полным именем;
            // переходим на начало RR.
            let t = pl[off];
            let name_skip = if t & 0xC0 == 0xC0 { 2 } else { read_qname_len(pl, off).unwrap_or(0) };
            let mut rr = off + name_skip;
            if rr + 10 > pl.len() {
                break;
            }
            let rtype = rd16(pl, rr);
            let rdlen = rd16(pl, rr + 8) as usize;
            rr += 10;
            if rtype == 1 && rdlen == 4 && rr + 4 <= pl.len() {
                let mut ip = [0u8; 4];
                ip.copy_from_slice(&pl[rr..rr + 4]);
                return Some(ip);
            }
            off = rr + rdlen;
        }
    }
    uart::write_str("[DNS] timeout\r\n");
    None
}
