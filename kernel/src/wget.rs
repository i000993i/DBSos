/// Минимальный HTTP/1.0 клиент (wget).
///
/// Поддерживает:
///   - HTTP GET с DNS-резолвом
///   - Сохранение ответа в файл
///   - Вывод в консоль
///   - Таймаут соединения

use crate::driver::uart;
use crate::driver::dns;
use crate::driver::tcp;
use crate::driver::net;
use crate::vfs;
use crate::timer;

const HTTP_PORT: u16 = 80;
const CONNECT_TIMEOUT_MS: u64 = 5000;
const RECV_TIMEOUT_MS: u64 = 10000;

fn uart_print(s: &str) { uart::write_str(s); }

/// Парсит URL вида "http://host:port/path", "host:port/path", "host/path" или "host".
/// Возвращает (host, port, path). Порт по умолчанию 80.
fn parse_url(url: &[u8]) -> (&[u8], u16, &[u8]) {
    // Убираем http://
    let mut u = url;
    if u.len() > 7 && &u[..7] == b"http://" {
        u = &u[7..];
    }
    // Разделяем host[:port] и path по первому '/'
    let (hostport, path) = match u.iter().position(|&c| c == b'/') {
        Some(pos) => (&u[..pos], &u[pos..]),
        None => (u, b"/".as_slice()),
    };
    // Порт после последнего ':' (только если суффикс — цифры)
    let mut port = HTTP_PORT;
    let mut host = hostport;
    if let Some(cp) = hostport.iter().rposition(|&c| c == b':') {
        let ps = &hostport[cp + 1..];
        if !ps.is_empty() && ps.iter().all(|&c| c >= b'0' && c <= b'9') {
            let mut v: u32 = 0;
            for &c in ps { v = v * 10 + (c - b'0') as u32; }
            if v > 0 && v <= 65535 {
                port = v as u16;
                host = &hostport[..cp];
            }
        }
    }
    (host, port, path)
}

/// Разобрать IPv4-литерал "a.b.c.d" без DNS.
fn parse_ipv4(s: &[u8]) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut part = 0usize;
    let mut val: u32 = 0;
    let mut digits = 0u8;
    let mut i = 0;
    while i <= s.len() {
        let c = if i < s.len() { s[i] } else { b'.' }; // терминатор как разделитель
        if c >= b'0' && c <= b'9' {
            val = val * 10 + (c - b'0') as u32;
            if val > 255 { return None; }
            digits += 1;
            if digits > 3 { return None; }
        } else if c == b'.' {
            if digits == 0 || part >= 4 { return None; }
            out[part] = val as u8;
            part += 1;
            val = 0;
            digits = 0;
        } else {
            return None;
        }
        i += 1;
    }
    if part != 4 { return None; }
    Some(out)
}

/// Устанавливает TCP-соединение с host:port.
/// Возвращает индекс соединения или None.
fn tcp_connect(host: &[u8], port: u16) -> Option<usize> {
    // IP-литерал — сразу без DNS (реестр пакетов живёт на 10.0.2.2:8080)
    let ip = if let Some(ip) = parse_ipv4(host) {
        uart_print("[wget] literal IP, no DNS\r\n");
        ip
    } else {
        // Резолвим DNS
        let host_str = core::str::from_utf8(host).unwrap_or("");
        uart_print("[wget] resolving ");
        uart_print(host_str);
        uart_print("...\r\n");

        match dns::resolve(host, 5000) {
            Some(ip) => ip,
            None => {
                uart_print("[wget] DNS failed\r\n");
                return None;
            }
        }
    };

    uart_print("[wget] connecting to ");
    let mut ip_str = [0u8; 16];
    let mut n = 0;
    for i in 0..4 {
        let mut v = ip[i] as u32;
        let mut d = [0u8; 3];
        let mut m = 0;
        while v > 0 { d[m] = b'0' + (v % 10) as u8; v /= 10; m += 1; }
        if m == 0 { ip_str[n] = b'0'; n += 1; } else {
            let mut j = 0;
            while j < m { ip_str[n] = d[m - 1 - j]; n += 1; j += 1; }
        }
        if i < 3 { ip_str[n] = b'.'; n += 1; }
    }
    uart_print(core::str::from_utf8(&ip_str[..n]).unwrap_or("?"));
    uart_print("...\r\n");

    let conn = tcp::connect(ip, port)?;

    // Ждём ESTABLISHED
    let deadline = timer::millis() + CONNECT_TIMEOUT_MS;
    while timer::millis() < deadline {
        net::poll();
        if tcp::is_open(conn) {
            uart_print("[wget] connected\r\n");
            return Some(conn);
        }
        timer::usleep(10000);
    }

    uart_print("[wget] connection timeout\r\n");
    tcp::close(conn);
    None
}

/// Отправить HTTP GET запрос.
fn http_get(conn: usize, host: &[u8], mut port: u16, path: &[u8]) -> bool {
    // Формируем запрос: GET /path HTTP/1.0\r\nHost: host[:port]\r\n\r\n
    let mut req = [0u8; 512];
    let mut ri = 0;

    // GET
    for &c in b"GET " {
        if ri < req.len() - 1 { req[ri] = c; ri += 1; }
    }
    for &c in path {
        if ri < req.len() - 1 { req[ri] = c; ri += 1; }
    }
    for &c in b" HTTP/1.0\r\nHost: " {
        if ri < req.len() - 1 { req[ri] = c; ri += 1; }
    }
    for &c in host {
        if ri < req.len() - 1 { req[ri] = c; ri += 1; }
    }
    if port != HTTP_PORT {
        // :port в Host (нужно виртуальным хостам на нестандартном порту)
        let mut div = 10000u16;
        let mut started = false;
        if ri < req.len() - 1 { req[ri] = b':'; ri += 1; }
        while div > 0 {
            let d = (port / div) as u8;
            if d != 0 || started || div == 1 {
                if ri < req.len() - 1 { req[ri] = b'0' + d; ri += 1; }
                started = true;
            }
            port %= div;
            div /= 10;
        }
    }
    for &c in b"\r\nConnection: close\r\n\r\n" {
        if ri < req.len() - 1 { req[ri] = c; ri += 1; }
    }

    tcp::send(conn, &req[..ri])
}

/// Принять ответ и вернуть (body_len, status, hdr_len). Заголовки копируются в hdr_out.
fn http_recv_full(conn: usize, body_buf: &mut [u8], hdr_out: &mut [u8]) -> (usize, u16, usize) {
    // Чинит баги: заголовок мог разбиться между пакетами (старый код искал
    // CRLFCRLF только внутри одного recv), плюс chunked-transfer decoding.
    let deadline = timer::millis() + RECV_TIMEOUT_MS;
    // Сырой буфер под заголовки+тело (до 64K+2K заголовков)
    let mut raw = [0u8; 66560];
    let mut raw_len = 0usize;

    while timer::millis() < deadline {
        tcp::pump();
        let mut tmp = [0u8; 2048];
        let n = tcp::recv(conn, &mut tmp);
        if n > 0 {
            let take = n.min(raw.len() - raw_len);
            raw[raw_len..raw_len + take].copy_from_slice(&tmp[..take]);
            raw_len += take;
            if raw_len >= raw.len() { break; }
        }
        // FIN + есть данные — выходим; FIN без данных — тоже выходим
        if !tcp::is_open(conn) && (n == 0 || raw_len > 0) {
            // Даём 200мс на остаток пакетов после FIN
            let extra = timer::millis() + 200;
            while timer::millis() < extra {
                tcp::pump();
                let m = tcp::recv(conn, &mut tmp);
                if m > 0 {
                    let take = m.min(raw.len() - raw_len);
                    raw[raw_len..raw_len + take].copy_from_slice(&tmp[..take]);
                    raw_len += take;
                } else { break; }
            }
            break;
        }
        if n == 0 { timer::usleep(5000); }
        // Сервер закрыл + таймаут по неактивности: если 500мс нет данных после заголовков — выходим
    }

    if raw_len == 0 { return (0, 0, 0); }
    // Найти конец заголовков во всём raw (а не в одном пакете)
    let mut hdr_end = 0usize;
    let mut status: u16 = 0;
    for i in 0..raw_len.saturating_sub(3) {
        if raw[i] == b'\r' && raw[i+1] == b'\n' && raw[i+2] == b'\r' && raw[i+3] == b'\n' {
            hdr_end = i + 4;
            break;
        }
    }
    if hdr_end == 0 { return (0, 0, 0); } // заголовки не пришли целиком
    // Парс статуса: "HTTP/1.x NNN"
    if raw_len >= 12 && raw[0] == b'H' {
        let mut v: u16 = 0;
        for &c in &raw[9..12] {
            if c.is_ascii_digit() { v = v * 10 + (c - b'0') as u16; }
        }
        status = v;
    }
    let body_raw = &raw[hdr_end..raw_len];
    // Chunked? (Transfer-Encoding: chunked)
    let is_chunked = {
        let mut found = false;
        // Ищем "chunked" в заголовках case-insensitive
        for i in 0..hdr_end.saturating_sub(7) {
            if (raw[i] | 0x20) == b'c' && (raw[i+1] | 0x20) == b'h' {
                // грубая проверка подстроки "chunked"
                if hdr_end - i >= 7 {
                    let w = &raw[i..(i+7).min(hdr_end)];
                    if w.len() == 7
                        && (w[0]|0x20)==b'c' && (w[1]|0x20)==b'h' && (w[2]|0x20)==b'u'
                        && (w[3]|0x20)==b'n' && (w[4]|0x20)==b'k' && (w[5]|0x20)==b'e'
                        && (w[6]|0x20)==b'd' { found = true; break; }
                }
            }
        }
        found
    };
    let mut total = 0usize;
    if is_chunked {
        // Декодируем чанки: "HEX-len\r\n<data>\r\n ... 0\r\n\r\n"
        let mut p = 0usize;
        while p < body_raw.len() && total < body_buf.len() {
            // hex длина строки
            let mut line_end = p;
            while line_end + 1 < body_raw.len()
                && !(body_raw[line_end] == b'\r' && body_raw[line_end+1] == b'\n') {
                line_end += 1;
            }
            if line_end + 1 >= body_raw.len() { break; }
            let mut len: usize = 0;
            for &c in &body_raw[p..line_end] {
                let d = if c.is_ascii_digit() { (c - b'0') as usize }
                    else if (c|0x20) >= b'a' && (c|0x20) <= b'f' { ((c|0x20) - b'a' + 10) as usize }
                    else if c == b';' { break; } else { continue; };
                len = len * 16 + d;
            }
            p = line_end + 2;
            if len == 0 { break; }
            let take = len.min(body_raw.len() - p).min(body_buf.len() - total);
            body_buf[total..total + take].copy_from_slice(&body_raw[p..p + take]);
            total += take;
            p += len;
            // пропустить \r\n после данных
            if p + 1 < body_raw.len() && body_raw[p] == b'\r' { p += 2; }
            if take < len { break; } // буфер полон
        }
    } else {
        total = body_raw.len().min(body_buf.len());
        body_buf[..total].copy_from_slice(&body_raw[..total]);
    }
    let hl = hdr_end.min(hdr_out.len());
    hdr_out[..hl].copy_from_slice(&raw[..hl]);
    (total, status, hl)
}

// ── Публичный API ──────────────────────────────────────────────────

/// Скачать файл по HTTP GET и сохранить в file_path.
/// Если file_path пусто — вывести в консоль.
/// Следует за 301/302/307/308 редиректами (до 3 hops).
pub fn wget(url: &[u8], file_path: &[u8]) -> bool {
    let mut cur = [0u8; 256];
    let n = url.len().min(cur.len());
    cur[..n].copy_from_slice(&url[..n]);
    let mut cur_len = n;
    for _hop in 0..4 {
        match fetch_once(&cur[..cur_len], file_path) {
            FetchResult::Done(ok) => return ok,
            FetchResult::Redirect(next, next_len) => {
                cur[..next_len].copy_from_slice(&next[..next_len]);
                cur_len = next_len;
                uart_print("[wget] redirect -> ");
                uart_print(core::str::from_utf8(&cur[..cur_len]).unwrap_or("?"));
                uart_print("\r\n");
                continue;
            }
        }
    }
    uart_print("[wget] too many redirects\r\n");
    false
}

enum FetchResult {
    Done(bool),
    Redirect([u8; 256], usize),
}

/// Одна попытка fetch.
fn fetch_once(url: &[u8], file_path: &[u8]) -> FetchResult {
    let (host, port, path) = parse_url(url);
    let conn = match tcp_connect(host, port) {
        Some(c) => c,
        None => return FetchResult::Done(false),
    };
    if !http_get(conn, host, port, path) {
        uart_print("[wget] failed to send request\r\n");
        tcp::close(conn);
        return FetchResult::Done(false);
    }
    let mut body = [0u8; 65536];
    let mut hdr = [0u8; 2048];
    let (body_len, status, hdr_len) = http_recv_full(conn, &mut body, &mut hdr);
    tcp::close(conn);
    if status == 301 || status == 302 || status == 307 || status == 308 {
        if let Some((loc, loc_len)) = find_location(&hdr[..hdr_len]) {
            let mut next = [0u8; 256];
            let nl = resolve_redirect(url, &loc[..loc_len], &mut next);
            return FetchResult::Redirect(next, nl);
        }
        uart_print("[wget] redirect without Location\r\n");
        return FetchResult::Done(false);
    }
    if body_len == 0 || (status != 200 && status != 0) {
        uart_print("[wget] HTTP status=");
        uart_print(core::str::from_utf8(&u16_to_str(status)).unwrap_or("?"));
        uart_print("\r\n");
        if body_len == 0 { return FetchResult::Done(false); }
        if status != 200 { return FetchResult::Done(false); }
    } else {
        uart_print("[wget] 200 OK, ");
        uart_print(core::str::from_utf8(&u32_to_str(body_len as u32)).unwrap_or("?"));
        uart_print(" bytes\r\n");
    }
    let actual_body = &body[..body_len];
    if file_path.is_empty() {
        crate::shell::print_str_pub(actual_body);
    } else {
        let ffd = vfs::open(file_path, 0x200 | 0x100);
        if ffd < 0 {
            uart_print("[wget] cannot create file\r\n");
            return FetchResult::Done(false);
        }
        vfs::write(ffd, actual_body);
        vfs::close(ffd);
        uart_print("[wget] saved to ");
        crate::shell::print_str_pub(file_path);
        uart_print("\r\n");
    }
    FetchResult::Done(true)
}

/// Найти Location: в raw-заголовках. Возвращает ссылку на статический буфер.
fn find_location(hdr: &[u8]) -> Option<([u8; 256], usize)> {
    // ищем "location:" case-insensitive
    if hdr.len() < 9 { return None; }
    for i in 0..hdr.len().saturating_sub(9) {
        if (hdr[i]|0x20)==b'l' && (hdr[i+1]|0x20)==b'o' && (hdr[i+2]|0x20)==b'c'
            && (hdr[i+3]|0x20)==b'a' && (hdr[i+4]|0x20)==b't' && (hdr[i+5]|0x20)==b'i'
            && (hdr[i+6]|0x20)==b'o' && (hdr[i+7]|0x20)==b'n' && hdr[i+8]==b':' {
            let mut s = i + 9;
            while s < hdr.len() && (hdr[s]==b' '||hdr[s]==b'\t') { s+=1; }
            let mut e = s;
            while e < hdr.len() && hdr[e]!=b'\r' && hdr[e]!=b'\n' { e+=1; }
            let mut out=[0u8;256];
            let n=(e-s).min(255);
            // trim trailing spaces
            let mut nn=n;
            out[..n].copy_from_slice(&hdr[s..s+n]);
            // rtrim
            while nn>0 && (out[nn-1]==b' '||out[nn-1]==b'\t') { nn-=1; }
            return Some((out, nn));
        }
    }
    None
}

/// Построить абсолютный URL из Location (абсолютный или path-only).
fn resolve_redirect(base: &[u8], loc: &[u8], out: &mut [u8;256]) -> usize {
    if loc.len()>=7 && (&loc[..7]==b"http://") {
        let n=loc.len().min(256);
        out[..n].copy_from_slice(&loc[..n]);
        return n;
    }
    if !loc.is_empty() && loc[0]==b'/' {
        // scheme+host из base + новый path
        let host_end: usize;
        if base.len()>7 && &base[..7]==b"http://" {
            let mut i=7;
            while i<base.len() && base[i]!=b'/' { i+=1; }
            host_end=i;
        } else {
            let mut i=0;
            while i<base.len() && base[i]!=b'/' { i+=1; }
            host_end=i;
        }
        let n=(host_end+loc.len()).min(256);
        out[..host_end.min(n)].copy_from_slice(&base[..host_end.min(n)]);
        let rest=n-host_end.min(n);
        out[host_end.min(n)..host_end.min(n)+rest].copy_from_slice(&loc[..rest]);
        return n;
    }
    let n=loc.len().min(256);
    out[..n].copy_from_slice(&loc[..n]);
    n
}

fn u16_to_str(v: u16) -> [u8; 6] {
    let mut buf=[0u8;6];
    if v==0 { buf[0]=b'0'; return buf; }
    let mut tmp=[0u8;6]; let mut n=0; let mut val=v;
    while val>0 { tmp[n]=b'0'+(val%10) as u8; val/=10; n+=1; }
    let mut j=0; while j<n { buf[j]=tmp[n-1-j]; j+=1; }
    buf
}

fn u32_to_str(v: u32) -> [u8; 12] {
    let mut buf = [0u8; 12];
    if v == 0 { buf[0] = b'0'; return buf; }
    let mut tmp = [0u8; 12];
    let mut n = 0;
    let mut val = v;
    while val > 0 { tmp[n] = b'0' + (val % 10) as u8; val /= 10; n += 1; }
    let mut j = 0;
    while j < n { buf[j] = tmp[n - 1 - j]; j += 1; }
    buf
}
