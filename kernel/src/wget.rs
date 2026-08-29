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

/// Парсит URL вида "host/path" или просто "host".
/// Возвращает (host, path).
fn parse_url(url: &[u8]) -> (&[u8], &[u8]) {
    // Убираем http://
    let mut u = url;
    if u.len() > 7 && &u[..7] == b"http://" {
        u = &u[7..];
    }
    // Разделяем host и path по第一个 '/'
    if let Some(pos) = u.iter().position(|&c| c == b'/') {
        (&u[..pos], &u[pos..])
    } else {
        (u, b"/")
    }
}

/// Устанавливает TCP-соединение с host:port.
/// Возвращает индекс соединения или None.
fn tcp_connect(host: &[u8], port: u16) -> Option<usize> {
    // Резолвим DNS
    let host_str = core::str::from_utf8(host).unwrap_or("");
    uart_print("[wget] resolving ");
    uart_print(host_str);
    uart_print("...\r\n");

    let ip = match dns::resolve(host, 5000) {
        Some(ip) => ip,
        None => {
            uart_print("[wget] DNS failed\r\n");
            return None;
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
fn http_get(conn: usize, host: &[u8], path: &[u8]) -> bool {
    // Формируем запрос: GET /path HTTP/1.0\r\nHost: host\r\n\r\n
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
    for &c in b"\r\nConnection: close\r\n\r\n" {
        if ri < req.len() - 1 { req[ri] = c; ri += 1; }
    }

    tcp::send(conn, &req[..ri])
}

/// Принять ответ и вернуть тело.
fn http_recv(conn: usize, body_buf: &mut [u8]) -> usize {
    let deadline = timer::millis() + RECV_TIMEOUT_MS;
    let mut total = 0;
    let mut header_done = false;

    while timer::millis() < deadline {
        net::poll();
        let mut tmp = [0u8; 2048];
        let n = tcp::recv(conn, &mut tmp);
        if n > 0 {
            if !header_done {
                // Ищем конец заголовков (\r\n\r\n)
                for i in 0..n {
                    if i + 3 < n && tmp[i] == b'\r' && tmp[i+1] == b'\n' && tmp[i+2] == b'\r' && tmp[i+3] == b'\n' {
                        header_done = true;
                        let body_start = i + 4;
                        let body_avail = n - body_start;
                        let copy = body_avail.min(body_buf.len() - total);
                        for j in 0..copy { body_buf[total + j] = tmp[body_start + j]; }
                        total += copy;
                        break;
                    }
                }
            } else {
                let copy = n.min(body_buf.len() - total);
                for j in 0..copy { body_buf[total + j] = tmp[j]; }
                total += copy;
            }
        }
        // Проверяем FIN
        if !tcp::is_open(conn) { break; }
        if n == 0 && header_done { break; }
        timer::usleep(5000);
    }

    total
}

// ── Публичный API ──────────────────────────────────────────────────

/// Скачать файл по HTTP GET и сохранить в file_path.
/// Если file_path пусто — вывести в консоль.
pub fn wget(url: &[u8], file_path: &[u8]) -> bool {
    let (host, path) = parse_url(url);

    let conn = match tcp_connect(host, HTTP_PORT) {
        Some(c) => c,
        None => return false,
    };

    if !http_get(conn, host, path) {
        uart_print("[wget] failed to send request\r\n");
        tcp::close(conn);
        return false;
    }

    let mut body = [0u8; 65536]; // 64KB max
    let body_len = http_recv(conn, &mut body);

    tcp::close(conn);

    if body_len == 0 {
        uart_print("[wget] empty response\r\n");
        return false;
    }

    // Проверяем HTTP статус
    if body_len >= 12 && &body[..12] == b"HTTP/1.0 200" || &body[..12] == b"HTTP/1.1 200" {
        uart_print("[wget] 200 OK, ");
        let len_str = u32_to_str(body_len as u32);
        uart_print(core::str::from_utf8(&len_str).unwrap_or("?"));
        uart_print(" bytes\r\n");
    }

    // Ищем начало тела (после \r\n\r\n)
    let mut body_start = 0;
    for i in 0..body_len.saturating_sub(3) {
        if body[i] == b'\r' && body[i+1] == b'\n' && body[i+2] == b'\r' && body[i+3] == b'\n' {
            body_start = i + 4;
            break;
        }
    }
    let actual_body = &body[body_start..body_len];

    if file_path.is_empty() {
        // Выводим в консоль
        crate::shell::print_str_pub(actual_body);
    } else {
        // Сохраняем в файл
        let ffd = vfs::open(file_path, 0x200 | 0x100); // O_CREAT | O_WRONLY
        if ffd < 0 {
            uart_print("[wget] cannot create file\r\n");
            return false;
        }
        vfs::write(ffd, actual_body);
        vfs::close(ffd);
        uart_print("[wget] saved to ");
        crate::shell::print_str_pub(file_path);
        uart_print("\r\n");
    }

    true
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
