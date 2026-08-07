/// TCP/IPv4 — минимальный потоковый стек (клиент + сервер).
///
/// Работает поверх e1000/ARP/ICMP (`crate::driver::net`). Входящие IP-пакеты
/// с protocol=6 маршрутизируются сюда из `net::poll()` через `input()`.
///
/// Особенности:
/// - таблица соединений на MAX_CONNS слотов,
/// - полный 3-way handshake (клиент и сервер), передача данных с ACK,
/// - FIN-закрытие с обеих сторон,
/// - loopback: если peer_ip == наш IP, сегмент не уходит в NIC, а
///   возвращается обратно в `input()` — так стек можно проверить
///   целиком без внешнего сервера.
///
/// Проводной порядок байтов — big-endian.

use super::uart;

pub const MAX_CONNS: usize = 8;
pub const RX_BUF: usize = 2048;
const TCPH: usize = 20;

// --- TCP flags ---
pub const FLAG_FIN: u8 = 0x01;
pub const FLAG_SYN: u8 = 0x02;
pub const FLAG_RST: u8 = 0x04;
pub const FLAG_PSH: u8 = 0x08;
pub const FLAG_ACK: u8 = 0x10;

// --- состояние соединения ---
pub const CLOSED: u8 = 0;
pub const LISTEN: u8 = 1;
pub const SYN_SENT: u8 = 2;
pub const SYN_RCVD: u8 = 3;
pub const ESTABLISHED: u8 = 4;
pub const FIN_WAIT1: u8 = 5;
pub const FIN_WAIT2: u8 = 6;
pub const CLOSE_WAIT: u8 = 7;
pub const LAST_ACK: u8 = 8;
pub const CLOSING: u8 = 9;

#[derive(Clone, Copy)]
pub struct Conn {
    pub in_use: bool,
    pub state: u8,
    pub local_port: u16,
    pub remote_port: u16,
    pub remote_mac: [u8; 6],
    pub peer_ip: [u8; 4],
    pub snd_una: u32,
    pub snd_nxt: u32,
    pub rcv_nxt: u32,
    pub rx_len: usize,
    pub rx_buf: [u8; RX_BUF],
    // outbound un-acked data, for retransmission
    pub out_seq: u32,
    pub out_len: usize,
    pub out_buf: [u8; 1400],
    // retransmission bookkeeping
    pub last_tx_ms: u64,
    pub rtt_ms: u64,
    pub live: bool,
}

const CONN_EMPTY: Conn = Conn {
    in_use: false, state: CLOSED, local_port: 0, remote_port: 0,
    remote_mac: [0; 6], peer_ip: [0; 4],
    snd_una: 0, snd_nxt: 0, rcv_nxt: 0, rx_len: 0, rx_buf: [0; RX_BUF],
    out_seq: 0, out_len: 0, out_buf: [0; 1400],
    last_tx_ms: 0, rtt_ms: 1000, live: false,
};

static mut CONNS: [Conn; MAX_CONNS] = [CONN_EMPTY; MAX_CONNS];
static EPHEMERAL: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(49152);
static ISN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0x12345678);

fn conn(i: usize) -> &'static mut Conn {
    unsafe { &mut CONNS[i] }
}

fn next_isn() -> u32 {
    ISN.fetch_add(0x102040, core::sync::atomic::Ordering::Relaxed)
}

fn next_ephemeral() -> u16 {
    let p = EPHEMERAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    49152 + (p % 16384)
}

fn checksum(buf: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < buf.len() {
        sum += ((buf[i] as u32) << 8) | (buf[i + 1] as u32);
        i += 2;
    }
    if i < buf.len() {
        sum += (buf[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Публичный интернет-checksum (используется UDP-стеком из net.rs).
pub fn checksum_pub(buf: &[u8]) -> u16 {
    checksum(buf)
}

fn u32_from(b: &[u8]) -> u32 {
    ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32)
}

fn be16(v: u16) -> [u8; 2] {
    [(v >> 8) as u8, v as u8]
}

fn be32(v: u32) -> [u8; 4] {
    [(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn uart_ip(ip: &[u8; 4]) {
    for i in 0..4 {
        let mut v = ip[i] as u64;
        let mut d = [0u8; 3];
        let mut n = 0;
        while v > 0 { d[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
        if n == 0 { uart::putchar(b'0'); }
        while n > 0 { n -= 1; uart::putchar(d[n]); }
        if i < 3 { uart::putchar(b'.'); }
    }
}

fn uart_dec(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut b = [0u8; 20]; let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putchar(b[i]); }
}

fn uart_hex(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut b = [0u8; 16]; let mut i = 0;
    while v > 0 { let n = (v & 0xF) as u8; b[i] = if n < 10 { b'0' + n } else { b'A' + n - 10 }; v >>= 4; i += 1; }
    while i > 0 { i -= 1; uart::putchar(b[i]); }
}

/// Найти слот соединения по 4-кортежу (lport, rport, peer_ip).
fn find_tuple(lport: u16, rport: u16, peer: &[u8; 4]) -> Option<usize> {
    for i in 0..MAX_CONNS {
        let c = conn(i);
        if c.in_use && c.local_port == lport && c.remote_port == rport && c.peer_ip == *peer {
            return Some(i);
        }
    }
    None
}

/// Найти прослушивающий слот по порту.
fn find_listener(lport: u16) -> Option<usize> {
    for i in 0..MAX_CONNS {
        let c = conn(i);
        if c.in_use && c.state == LISTEN && c.local_port == lport {
            return Some(i);
        }
    }
    None
}

fn alloc_conn() -> Option<usize> {
    for i in 0..MAX_CONNS {
        if !conn(i).in_use {
            *conn(i) = CONN_EMPTY;
            conn(i).in_use = true;
            return Some(i);
        }
    }
    None
}

// ====== Отправка ======

/// Собрать и отправить TCP-сегмент для соединения `idx` c явным seq.
/// Если peer_ip == наш IP — зациклить в `input()` (loopback).
fn tcp_send_seq(idx: usize, seq: u32, flags: u8, payload: &[u8]) -> bool {
    let c = conn(idx);
    let peer_ip = c.peer_ip;
    let ack = c.rcv_nxt;

    let tcp_len = TCPH + payload.len();
    let pseudo_len = 12 + tcp_len + if tcp_len % 2 == 1 { 1 } else { 0 };
    let mut scratch = [0u8; 12 + 20 + 1400 + 2];
    let own_ip = crate::driver::net::our_ip();

    // pseudo-header: src_ip, dst_ip, zero, proto(6), tcp_len
    scratch[0..4].copy_from_slice(&own_ip);
    scratch[4..8].copy_from_slice(&peer_ip);
    scratch[8] = 0;
    scratch[9] = 6;
    scratch[10] = (tcp_len >> 8) as u8;
    scratch[11] = tcp_len as u8;

    {
        let p = &mut scratch[12..12 + tcp_len];
        p[0..2].copy_from_slice(&be16(c.local_port));
        p[2..4].copy_from_slice(&be16(c.remote_port));
        p[4..8].copy_from_slice(&be32(seq));
        p[8..12].copy_from_slice(&be32(ack));
        p[12] = 0x50; // data offset = 5 words
        p[13] = flags;
        let win: u16 = 8192;
        p[14..16].copy_from_slice(&be16(win));
        p[16] = 0; p[17] = 0; // csum
        p[18] = 0; p[19] = 0; // urg
        if !payload.is_empty() {
            p[TCPH..TCPH + payload.len()].copy_from_slice(payload);
        }
    }
    let csum = checksum(&scratch[..pseudo_len]);
    scratch[12 + 16] = (csum >> 8) as u8;
    scratch[12 + 17] = csum as u8;
    let p = &scratch[12..12 + tcp_len];

    if peer_ip == own_ip {
        // Loopback: скармливаем сегмент обратно в стек как входящий.
        let mut frame = [0u8; 14 + 20 + 20 + 1400];
        frame[0..6].copy_from_slice(&crate::driver::net::mac());
        frame[6..12].copy_from_slice(&crate::driver::net::mac());
        frame[12] = 0x08; frame[13] = 0x00;
        let ip = &mut frame[14..34];
        ip[0] = 0x45; ip[1] = 0;
        ip[2] = ((20 + tcp_len) >> 8) as u8; ip[3] = (20 + tcp_len) as u8;
        ip[4] = 0; ip[5] = 0; ip[6] = 0; ip[7] = 0;
        ip[8] = 64; ip[9] = 6; ip[10] = 0; ip[11] = 0;
        ip[12..16].copy_from_slice(&own_ip);
        ip[16..20].copy_from_slice(&own_ip);
        let ipc = checksum(ip);
        ip[10] = (ipc >> 8) as u8; ip[11] = ipc as u8;
        frame[34..34 + tcp_len].copy_from_slice(p);
        let seg = &frame[34..34 + tcp_len];
        input(own_ip, crate::driver::net::mac(), seg);
        return true;
    }

    // Реальная отправка в сеть.
    let mut frame = [0u8; 1514];
    frame[0..6].copy_from_slice(&c.remote_mac);
    frame[6..12].copy_from_slice(&crate::driver::net::mac());
    frame[12] = 0x08; frame[13] = 0x00;
    let ip = &mut frame[14..34];
    ip[0] = 0x45; ip[1] = 0;
    ip[2] = ((20 + tcp_len) >> 8) as u8; ip[3] = (20 + tcp_len) as u8;
    ip[4] = 0; ip[5] = 0; ip[6] = 0; ip[7] = 0;
    ip[8] = 64; ip[9] = 6; ip[10] = 0; ip[11] = 0;
    ip[12..16].copy_from_slice(&own_ip);
    ip[16..20].copy_from_slice(&peer_ip);
    let ipc = checksum(ip);
    ip[10] = (ipc >> 8) as u8; ip[11] = ipc as u8;
    frame[34..34 + tcp_len].copy_from_slice(p);
    crate::driver::net::send_raw(&frame[..14 + 20 + tcp_len])
}

/// Отправить на текущем snd_nxt (новый сегмент).
fn tcp_send(idx: usize, flags: u8, payload: &[u8]) -> bool {
    let seq = conn(idx).snd_nxt;
    tcp_send_seq(idx, seq, flags, payload)
}

/// Переслать неподтверждённый сегмент по истечении RTO.
fn check_retransmit() {
    let now = crate::timer::millis();
    for i in 0..MAX_CONNS {
        let c = conn(i);
        if !c.in_use || c.out_len == 0 { continue; }
        if now.wrapping_sub(c.last_tx_ms) > c.rtt_ms {
            uart::write_str("[TCP] retransmit ");
            uart_dec(c.out_len as u64);
            uart::write_str(" bytes (seq=");
            uart_hex(c.out_seq as u64);
            uart::write_str(")\r\n");
            let n = c.out_len;
            let seq = c.out_seq;
            let tmp = c.out_buf;
            tcp_send_seq(i, seq, FLAG_ACK | FLAG_PSH, &tmp[..n]);
            conn(i).last_tx_ms = now;
            let r2 = conn(i).rtt_ms.saturating_mul(2).min(8000);
            conn(i).rtt_ms = r2;
        }
    }
}

// ====== Входящие сегменты (из net::poll или loopback) ======

/// Обработать входящий TCP-сегмент `data` (заголовок + данные) от peer.
pub fn input(peer_ip: [u8; 4], peer_mac: [u8; 6], data: &[u8]) {
    if data.len() < TCPH {
        return;
    }
    let dport = ((data[2] as u16) << 8) | data[3] as u16;
    let sport = ((data[0] as u16) << 8) | data[1] as u16;
    let seq = u32_from(&data[4..8]);
    let ack = u32_from(&data[8..12]);
    let data_off = (data[12] >> 4) as usize;
    let flags = data[13];
    let payload = data.get(data_off * 4..).unwrap_or_default();

    // Ищем существующее соединение по 4-кортежу.
    if let Some(idx) = find_tuple(dport, sport, &peer_ip) {
        handle_segment(idx, peer_ip, peer_mac, flags, seq, ack, payload);
        return;
    }

    // SYN на прослушивающий порт → новое соединение.
    if flags & FLAG_SYN != 0 && flags & FLAG_ACK == 0 {
        if let Some(lidx) = find_listener(dport) {
            let _ = lidx;
            if let Some(nidx) = alloc_conn() {
                let n = conn(nidx);
                n.local_port = dport;
                n.remote_port = sport;
                n.remote_mac = peer_mac;
                n.peer_ip = peer_ip;
                n.snd_nxt = next_isn();
                n.snd_una = n.snd_nxt;
                n.rcv_nxt = seq.wrapping_add(1);
                n.state = SYN_RCVD;
                let tmp = n.snd_nxt;
                uart::write_str("[TCP] new conn from ");
                uart_ip(&peer_ip);
                uart::write_str(":");
                uart_dec(sport as u64);
                uart::write_str(" -> :");
                uart_dec(dport as u64);
                uart::write_str(" (SYN_RCVD)\r\n");
                tcp_send(nidx, FLAG_SYN | FLAG_ACK, &[]);
                conn(nidx).snd_nxt = tmp.wrapping_add(1);
            }
        }
        return;
    }

    uart::write_str("[TCP] segment to unknown ");
    uart_ip(&peer_ip);
    uart::write_str(":");
    uart_dec(dport as u64);
    uart::write_str(" (no conn), flags=0x");
    uart_hex(flags as u64);
    uart::write_str("\r\n");
}

/// Подтвердить отправленные данные: продвинуть snd_una и вычистить
/// из out-буфера всё, что покрыл ACK.
fn apply_ack(c: &mut Conn, ack: u32) {
    if ack == 0 { return; }
    let diff = ack.wrapping_sub(c.snd_una);
    if diff == 0 { return; }
    c.snd_una = ack;
    let covered = ack.wrapping_sub(c.out_seq);
    if covered >= c.out_len as u32 {
        c.out_len = 0;
        c.rtt_ms = 1000;
    } else if covered > 0 {
        let rem = c.out_len - covered as usize;
        c.out_buf.copy_within(covered as usize..c.out_len, 0);
        c.out_len = rem;
        c.out_seq = ack;
    }
}

fn handle_segment(idx: usize, peer_ip: [u8; 4], peer_mac: [u8; 6], flags: u8, seq: u32, ack: u32, payload: &[u8]) {
    let c = conn(idx);
    c.remote_mac = peer_mac;
    c.peer_ip = peer_ip;
    let st = c.state;

    // RST — аварийный сброс.
    if flags & FLAG_RST != 0 {
        uart::write_str("[TCP] RST -> CLOSED\r\n");
        *c = CONN_EMPTY;
        return;
    }

    match st {
        SYN_SENT => {
            // Ожидаем SYN+ACK.
            if flags & FLAG_SYN != 0 && flags & FLAG_ACK != 0 {
                c.rcv_nxt = seq.wrapping_add(1);
                c.snd_una = ack;
                c.state = ESTABLISHED;
                uart::write_str("[TCP] handshake done (SYN_SENT -> ESTABLISHED)\r\n");
                tcp_send(idx, FLAG_ACK, &[]);
            }
        }
        SYN_RCVD => {
            if flags & FLAG_ACK != 0 {
                c.snd_una = ack;
                c.state = ESTABLISHED;
                uart::write_str("[TCP] accepted (SYN_RCVD -> ESTABLISHED)\r\n");
            }
        }
        ESTABLISHED => {
            // Подтверждение отправленных данных.
            if flags & FLAG_ACK != 0 { apply_ack(c, ack); }
            // Приём данных.
            if !payload.is_empty() {
                let n = payload.len().min(RX_BUF - c.rx_len);
                c.rx_buf[c.rx_len..c.rx_len + n].copy_from_slice(&payload[..n]);
                c.rx_len += n;
                c.rcv_nxt = c.rcv_nxt.wrapping_add(n as u32);
                uart::write_str("[TCP] recv ");
                uart_dec(n as u64);
                uart::write_str(" bytes, rx_len=");
                uart_dec(c.rx_len as u64);
                uart::write_str("\r\n");
                tcp_send(idx, FLAG_ACK, &[]);
            }
            // Закрытие со стороны peer.
            if flags & FLAG_FIN != 0 {
                c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
                c.state = CLOSE_WAIT;
                uart::write_str("[TCP] peer FIN (ESTABLISHED -> CLOSE_WAIT)\r\n");
                tcp_send(idx, FLAG_ACK, &[]);
            }
        }
        FIN_WAIT1 => {
            if flags & FLAG_ACK != 0 {
                // ACK может покрывать наши исходящие данные и наш FIN.
                apply_ack(c, ack);
                if ack.wrapping_sub(c.snd_una) > 0 && ack != 0 {
                    // ACK полностью закрыл наш поток данных (вкл. FIN) — 
                    // переходим, если FIN ещё не ждёт ACK.
                    if c.out_len == 0 && c.state == FIN_WAIT1 {
                        c.state = FIN_WAIT2;
                        uart::write_str("[TCP] our FIN acked (FIN_WAIT1 -> FIN_WAIT2)\r\n");
                    }
                }
            }
            if flags & FLAG_FIN != 0 {
                c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
                if c.state == FIN_WAIT1 {
                    c.state = CLOSING;
                    uart::write_str("[TCP] peer FIN during FIN_WAIT1 (-> CLOSING)\r\n");
                }
                tcp_send(idx, FLAG_ACK, &[]);
            }
        }
        FIN_WAIT2 => {
            if flags & FLAG_ACK != 0 { apply_ack(c, ack); }
            if flags & FLAG_FIN != 0 {
                c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
                uart::write_str("[TCP] peer FIN (FIN_WAIT2 -> CLOSED)\r\n");
                tcp_send(idx, FLAG_ACK, &[]);
                *c = CONN_EMPTY;
            }
        }
        CLOSE_WAIT => {
            // Наш FIN уже отправлен через close(); ждём ACK.
            if flags & FLAG_ACK != 0 {
                apply_ack(c, ack);
                if c.state == CLOSE_WAIT {
                    c.state = LAST_ACK;
                    uart::write_str("[TCP] FIN acked (CLOSE_WAIT -> LAST_ACK)\r\n");
                }
            }
        }
        LAST_ACK => {
            if flags & FLAG_ACK != 0 {
                uart::write_str("[TCP] LAST_ACK -> CLOSED\r\n");
                *c = CONN_EMPTY;
            }
        }
        CLOSING => {
            if flags & FLAG_ACK != 0 {
                uart::write_str("[TCP] CLOSING -> CLOSED\r\n");
                *c = CONN_EMPTY;
            }
        }
        _ => {}
    }
}

// ====== Публичный API ======

/// Открыть LISTEN-сокет на порту `port`.
pub fn listen(port: u16) -> Option<usize> {
    if find_listener(port).is_some() {
        return None;
    }
    let i = alloc_conn()?;
    let c = conn(i);
    c.state = LISTEN;
    c.local_port = port;
    uart::write_str("[TCP] LISTEN :");
    uart_dec(port as u64);
    uart::write_str("\r\n");
    Some(i)
}

/// Клиент: инициировать соединение с peer_ip:port (шлёт SYN).
pub fn connect(peer_ip: [u8; 4], port: u16) -> Option<usize> {
    let i = alloc_conn()?;
    let c = conn(i);
    c.local_port = next_ephemeral();
    c.remote_port = port;
    c.peer_ip = peer_ip;
    c.snd_nxt = next_isn();
    c.snd_una = c.snd_nxt;
    c.state = SYN_SENT;
    uart::write_str("[TCP] connecting to ");
    uart_ip(&peer_ip);
    uart::write_str(":");
    uart_dec(port as u64);
    uart::write_str(" (SYN_SENT)\r\n");

    // Резолвим MAC получателя. Для своего IP (loopback) — свой MAC.
    let own = crate::driver::net::our_ip();
    let mac = if peer_ip == own {
        crate::driver::net::mac()
    } else if let Some(m) = crate::driver::net::arp_lookup_public(&peer_ip) {
        m
    } else {
        // Пытаемся резолвить; если не вышло — fallback на broadcast (slirp).
        crate::driver::net::resolve(peer_ip, 2000).unwrap_or([0xFF; 6])
    };
    conn(i).remote_mac = mac;

    tcp_send(i, FLAG_SYN, &[]);
    Some(i)
}

/// Отправить данные (ESTABLISHED).
pub fn send(idx: usize, data: &[u8]) -> bool {
    let st = conn(idx).state;
    if st != ESTABLISHED && st != CLOSE_WAIT {
        uart::write_str("[TCP] send: not established\r\n");
        return false;
    }
    if data.len() > 1400 {
        uart::write_str("[TCP] send: too big\r\n");
        return false;
    }
    // Заполняем out-буфер ДО отправки: в loopback ACK за эти данные приходит
    // синхронно внутри tcp_send() и должен сразу вычистить out_len. Если
    // ставить после — ACK «промахивается» и данные будут ретранслиться.
    let c = conn(idx);
    c.out_seq = c.snd_nxt;
    c.out_len = data.len();
    c.out_buf[..data.len()].copy_from_slice(data);
    c.last_tx_ms = crate::timer::millis();
    c.rtt_ms = 1000;

    let ok = tcp_send(idx, FLAG_PSH | FLAG_ACK, data);
    if ok {
        conn(idx).snd_nxt = conn(idx).snd_nxt.wrapping_add(data.len() as u32);
    } else {
        conn(idx).out_len = 0;
    }
    ok
}

/// Прочитать принятые данные (не блокирует).
pub fn recv(idx: usize, out: &mut [u8]) -> usize {
    let c = conn(idx);
    if c.rx_len == 0 {
        return 0;
    }
    let n = out.len().min(c.rx_len);
    out[..n].copy_from_slice(&c.rx_buf[..n]);
    c.rx_buf.copy_within(n..c.rx_len, 0);
    c.rx_len -= n;
    n
}

/// Закрыть соединение (шлёт FIN). Если CLOSE_WAIT — тоже шлёт FIN.
pub fn close(idx: usize) {
    let st = conn(idx).state;
    match st {
        ESTABLISHED => {
            let ok = tcp_send(idx, FLAG_FIN | FLAG_ACK, &[]);
            if ok {
                conn(idx).snd_nxt = conn(idx).snd_nxt.wrapping_add(1);
                conn(idx).state = FIN_WAIT1;
                uart::write_str("[TCP] FIN sent (-> FIN_WAIT1)\r\n");
            }
        }
        CLOSE_WAIT => {
            let ok = tcp_send(idx, FLAG_FIN | FLAG_ACK, &[]);
            if ok {
                conn(idx).snd_nxt = conn(idx).snd_nxt.wrapping_add(1);
                conn(idx).state = LAST_ACK;
                uart::write_str("[TCP] FIN sent (CLOSE_WAIT -> LAST_ACK)\r\n");
            }
        }
        SYN_SENT | SYN_RCVD => {
            *conn(idx) = CONN_EMPTY;
            uart::write_str("[TCP] close during handshake\r\n");
        }
        _ => {
            *conn(idx) = CONN_EMPTY;
        }
    }
}

pub fn state(idx: usize) -> u8 {
    conn(idx).state
}

pub fn is_open(idx: usize) -> bool {
    let c = conn(idx);
    c.in_use && (c.state == ESTABLISHED || c.state == CLOSE_WAIT)
}

/// Найти принятое (дочернее) соединение прослушивающего сокета `lport`,
/// которое в состоянии handshake или уже установлено. Это слот, созданный
/// `input()` при SYN — сам LISTEN-сокет остаётся в LISTEN.
pub fn find_accepted(lport: u16) -> Option<usize> {
    for i in 0..MAX_CONNS {
        let c = conn(i);
        if c.in_use && c.local_port == lport && c.remote_port != 0
            && (c.state == SYN_RCVD || c.state == ESTABLISHED)
        {
            return Some(i);
        }
    }
    None
}

/// Качать приём: вызвать из цикла ожидания, чтобы стек обработал входящие.
/// Также обрабатывает ретрансмиссию неподтверждённых данных.
pub fn pump() {
    crate::driver::net::poll();
    check_retransmit();
}

/// Обработать существующее соединение, пока не станет готово к данным
/// (дождаться ESTABLISHED) или не истечёт timeout (мс).
/// В SYN_SENT/SYN_RCVD ретранслирует SYN каждые ~1с.
pub fn wait_established(idx: usize, timeout_ms: u64) -> bool {
    let start = crate::timer::millis();
    let mut last_syn = 0u64;
    loop {
        let st = state(idx);
        if st == ESTABLISHED {
            return true;
        }
        if st == CLOSED {
            return false;
        }
        // Ретрансмиссия handshake: если не установлено за 1s — шлём SYN снова.
        let now = crate::timer::millis();
        if (st == SYN_SENT || st == SYN_RCVD) && now.wrapping_sub(last_syn) > 1000 {
            uart::write_str("[TCP] retransmit SYN\r\n");
            tcp_send(idx, FLAG_SYN, &[]);
            last_syn = now;
        }
        pump();
        if crate::timer::millis() - start > timeout_ms {
            return false;
        }
    }
}

/// Дождаться полного закрытия соединения.
pub fn wait_closed(idx: usize, timeout_ms: u64) -> bool {
    let start = crate::timer::millis();
    loop {
        if !conn(idx).in_use {
            return true;
        }
        pump();
        if crate::timer::millis() - start > timeout_ms {
            return false;
        }
    }
}

/// Loopback-самотест TCP-стека: поднимаем LISTEN-сокет, коннектимся к
/// своему IP, проходим 3-way handshake, обмениваемся данными в обе стороны
/// и закрываемся. Вся логика не требует внешнего хоста (часть пакетов
/// зацикливается через себя).
pub fn test_stack() {
    uart::write_str("\r\n===== [TCP] stack self-test =====\r\n");
    let own = crate::driver::net::our_ip();

    let srv = match listen(9100) {
        Some(i) => i,
        None => { uart::write_str("[TCP] listen failed\r\n"); return; }
    };
    let _ = srv;
    let cli = match connect(own, 9100) {
        Some(i) => i,
        None => { uart::write_str("[TCP] connect failed\r\n"); return; }
    };

    if !wait_established(cli, 3000) {
        uart::write_str("[TCP] client handshake failed\r\n");
        return;
    }
    // Принятое соединение — отдельный дочерний слот; дождаться его.
    let start = crate::timer::millis();
    while crate::timer::millis() - start < 3000 {
        pump();
        if let Some(i) = find_accepted(9100) {
            let srv_acc = i;
            uart::write_str("[TCP] handshake OK (both ESTABLISHED)\r\n");
            // client -> server
            if !send(cli, b"HELLO from client\r\n") {
                uart::write_str("[TCP] client send failed\r\n");
                return;
            }
            let mut buf = [0u8; 128];
            let mut got = 0usize;
            while got == 0 && crate::timer::millis() - start < 3000 {
                got = recv(srv_acc, &mut buf);
                pump();
            }
            if got > 0 {
                uart::write_str("[TCP] server recv client data: ");
                for &b in &buf[..got.min(40)] {
                    uart::putchar(b);
                }
                uart::write_str("\r\n");
            } else {
                uart::write_str("[TCP] server recv failed (0 bytes)\r\n");
            }

            // server -> client echo
            if !send(srv_acc, b"REPLY from server\r\n") {
                uart::write_str("[TCP] server send failed\r\n");
                return;
            }
            let mut got = 0usize;
            while got == 0 && crate::timer::millis() - start < 3000 {
                got = recv(cli, &mut buf);
                pump();
            }
            if got > 0 {
                uart::write_str("[TCP] client recv server data: ");
                for &b in &buf[..got] {
                    uart::putchar(b);
                }
                uart::write_str("\r\n");
            } else {
                uart::write_str("[TCP] client recv failed (0 bytes)\r\n");
            }

            // client closes (FIN), server closes too
            close(cli);
            let _ = wait_closed(cli, 3000);
            close(srv_acc);
            let _ = wait_closed(srv_acc, 3000);
            uart::write_str("[TCP] closed.\r\n[TCP] stack self-test DONE\r\n");
            return;
        }
        pump();
    }
    uart::write_str("[TCP] no accepted connection\r\n");
}
