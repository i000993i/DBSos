use crate::console;
use crate::driver::uart;
use crate::driver::net;
use crate::driver::tcp;
use crate::driver::nvme;
use crate::fs;
use crate::memory;
use crate::timer;

/// Current working directory (absolute path, starts at root "/")
static mut CWD: [u8; 128] = [0; 128];

fn cwd_init() {
    unsafe {
        CWD[0] = b'/';
        CWD[1] = 0;
    }
}

fn cwd_get() -> &'static [u8] {
    unsafe { &CWD[..] }
}

const CWD_SIZE: usize = 256;

fn cwd_set(path: &[u8]) {
    unsafe {
        let len = path.len().min(CWD_SIZE - 1);
        CWD[..len].copy_from_slice(&path[..len]);
        CWD[len] = 0;
    }
}

/// Build absolute path: if `path` starts with '/', use as-is; else prepend cwd
fn build_path<'a>(path: &[u8], buf: &'a mut [u8]) -> &'a mut [u8] {
    if path.len() > 0 && path[0] == b'/' {
        let len = path.len().min(buf.len() - 1);
        buf[..len].copy_from_slice(&path[..len]);
        buf[len] = 0;
        &mut buf[..len + 1]
    } else {
        let cwd = cwd_get();
        let cwd_len = cwd.len().min(buf.len() - path.len() - 2);
        buf[..cwd_len].copy_from_slice(&cwd[..cwd_len]);
        if cwd_len > 1 {
            buf[cwd_len] = b'/';
            let start = cwd_len + 1;
            let copy_len = path.len().min(buf.len() - start - 1);
            buf[start..start + copy_len].copy_from_slice(&path[..copy_len]);
            buf[start + copy_len] = 0;
            &mut buf[..start + copy_len + 1]
        } else {
            let copy_len = path.len().min(buf.len() - 1);
            buf[..copy_len].copy_from_slice(&path[..copy_len]);
            buf[copy_len] = 0;
            &mut buf[..copy_len + 1]
        }
    }
}

fn w(s: &str) { console::write_str(s); uart::write_str(s); }

fn wb(data: &[u8]) {
    for &b in data { uart::putchar(b); console::putchar(b); }
}

fn readline(buf: &mut [u8]) -> usize {
    let mut i = 0;
    loop {
        let c = if let Some(c) = crate::driver::ps2::poll_char() {
            Some(c)
        } else if let Some(c) = uart::poll_char() {
            Some(c)
        } else {
            None
        };
        if let Some(c) = c {
            match c {
                b'\r' | b'\n' => {
                    w("\r\n"); buf[i] = 0; return i;
                }
                0x03 => { // Ctrl+C
                    w("^C\r\n");
                    buf[0] = 0;
                    return 0;
                }
                0x7F | 0x08 => {
                    if i > 0 { i -= 1; w("\x08 \x08"); }
                }
                b' '..=b'~' => {
                    if i < buf.len() - 1 {
                        buf[i] = c; i += 1;
                        let out = [c];
                        let s = core::str::from_utf8(&out).unwrap();
                        console::write_str(s); uart::putchar(c);
                    }
                }
                _ => {}
            }
        }
    }
}

fn dec(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); console::putchar(b'0'); return; }
    let mut b = [0u8; 20]; let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; let c = b[i]; uart::putchar(c); console::putchar(c); }
}

fn hex(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); console::putchar(b'0'); return; }
    let mut b = [0u8; 16]; let mut i = 0;
    while v > 0 { let n = (v&0xF) as u8; b[i]=if n<10{b'0'+n}else{b'A'+n-10}; v>>=4; i+=1; }
    while i > 0 { i -= 1; let c = b[i]; uart::putchar(c); console::putchar(c); }
}

fn cmd_help() {
    w("Commands:\r\n  help   - this help\r\n  mem    - memory info\r\n");
    w("  time   - timer info\r\n  clear  - clear screen\r\n");
    w("  info   - system info\r\n  ls [PATH] - list directory\r\n");
    w("  cat PATH    - print file\r\n  mkdir PATH  - create dir\r\n");
    w("  write PATH TEXT - write text file\r\n  rm PATH     - delete file\r\n");
    w("  rmdir PATH  - delete dir\r\n  ping   - ARP ping\r\n");
    w("  reboot - restart system\r\n  poweroff - shutdown\r\n");
    w("  exec PATH   - run ELF binary\r\n");
    w("  cd PATH     - change directory\r\n  pwd    - print working dir\r\n");
    w("  echo TEXT   - print text\r\n");
    w("  tcp IP PORT TEXT - TCP connect + send\r\n");
}

fn cmd_pwd() {
    let cwd = cwd_get();
    if cwd.len() == 0 || (cwd.len() == 1 && cwd[0] == b'/') {
        w("/");
    } else {
        let s = core::str::from_utf8(cwd).unwrap_or("?");
        w(s);
    }
    w("\r\n");
}

fn cmd_cd(arg: &[u8]) {
    if arg.len() == 0 {
            // cd without args -> go to root
            cwd_set(b"/");
            return;
        }
        let mut path_buf = [0u8; 256];
        let abs_path = build_path(arg, &mut path_buf);

        // Resolve the path: handle ".." and "."
        let mut resolved = [0u8; 128];
        let mut rpos = 0;

        let mut i = 0;
        while i < abs_path.len() && abs_path[i] != 0 {
            // Skip leading slashes
            while i < abs_path.len() && abs_path[i] == b'/' { i += 1; }
            if i >= abs_path.len() || abs_path[i] == 0 { break; }

            // Extract component
            let start = i;
            while i < abs_path.len() && abs_path[i] != b'/' && abs_path[i] != 0 { i += 1; }
            let comp = &abs_path[start..i];

            if comp == b".." {
                // Go up: remove last component
                if rpos > 1 {
                    rpos -= 1; // remove trailing slash
                    while rpos > 0 && resolved[rpos - 1] != b'/' { rpos -= 1; }
                    if rpos > 0 { rpos -= 1; } // remove the slash itself
                    if rpos == 0 { resolved[0] = b'/'; rpos = 1; }
                } else {
                    resolved[0] = b'/';
                    rpos = 1;
                }
        } else if comp != b"." {
            // Add component
            if rpos > 0 && resolved[rpos - 1] != b'/' {
                if rpos < resolved.len() { resolved[rpos] = b'/'; rpos += 1; }
            }
            let copy_len = comp.len().min(resolved.len() - rpos - 1);
            resolved[rpos..rpos + copy_len].copy_from_slice(&comp[..copy_len]);
            rpos += copy_len;
        }
    }

    if rpos == 0 {
        resolved[0] = b'/';
        rpos = 1;
    }
    resolved[rpos] = 0;

    // Verify the directory exists
    if fs::is_dir(&resolved[..rpos]) {
        cwd_set(&resolved[..rpos]);
    } else {
        w("cd: not a directory: ");
        let s = core::str::from_utf8(arg).unwrap_or("?");
        w(s);
        w("\r\n");
    }
}

fn cmd_echo(arg: &[u8]) {
    if arg.len() > 0 {
        let s = core::str::from_utf8(arg).unwrap_or("<binary>");
        w(s);
    }
    w("\r\n");
}

fn cmd_mem() {
    let free = memory::free_count() as u64;
    w("Free pages: "); dec(free); w(" ("); dec(free * 4096 / 1024); w(" KB)\r\n");
    let p = memory::palloc();
    if p != 0 { w("palloc: 0x"); hex(p); w("\r\n"); memory::pfree(p); }
}

fn cmd_time() {
    let t0 = timer::millis(); let c0 = timer::ticks();
    timer::usleep(10_000);
    let c1 = timer::ticks(); let t1 = timer::millis();
    w("ticks: "); dec(c0); w(" -> "); dec(c1);
    w(", delta: "); dec(c1 - c0); w(" (10ms), ms: "); dec(t1 - t0); w("\r\n");
}

fn cmd_info() {
    w("DBSos v0.1\r\nArch: x86_64  Boot: UEFI  Display: GOP\r\n");
    cmd_mem();
}

fn cmd_ping() {
    let gw: [u8; 4] = [10, 0, 2, 1];
    w("[NET] sending ARP...\r\n");
    net::send_arp_request(gw);
    let deadline = timer::millis() + 2000;
    while timer::millis() < deadline { net::poll(); core::hint::spin_loop(); }
    w("[NET] poll done\r\n");
}

/// Parse "a.b.c.d" into an IPv4 address. Returns true on success.
fn parse_ip(s: &[u8], out: &mut [u8; 4]) -> bool {
    let mut oct = 0u32;
    let mut part = 0usize;
    for &c in s {
        if c >= b'0' && c <= b'9' {
            if part >= 4 { return false; }
            oct = oct * 10 + (c - b'0') as u32;
            if oct > 255 { return false; }
        } else if c == b'.' {
            if part >= 4 { return false; }
            out[part] = oct as u8;
            part += 1;
            oct = 0;
        } else {
            return false;
        }
    }
    if part != 3 { return false; }
    out[3] = oct as u8;
    true
}

/// `tcp <ip> <port> [text...]` — резолвит MAC, коннектится к хосту,
/// отправляет `text` и печатает ответ.
fn cmd_tcp(arg: &[u8]) {
    let mut ipb = [0u8; 64];
    let mut portb = [0u8; 8];
    let mut p = 0usize;
    // first token: ip
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let mut n = 0usize;
    while p < arg.len() && arg[p] != b' ' && arg[p] != 0 {
        if n < ipb.len() { ipb[n] = arg[p]; n += 1; }
        p += 1;
    }
    let ip = &ipb[..n];
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let mut n2 = 0usize;
    while p < arg.len() && arg[p] != b' ' && arg[p] != 0 {
        if n2 < portb.len() { portb[n2] = arg[p]; n2 += 1; }
        p += 1;
    }
    // остаток — произвольный текст
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let body = &arg[p..];

    let mut ip4 = [0u8; 4];
    if !parse_ip(ip, &mut ip4) {
        w("Usage: tcp <ip> <port> [text]\r\n");
        return;
    }
    let mut port: u32 = 0;
    for &c in &portb[..n2] {
        if c < b'0' || c > b'9' { port = u32::MAX; break; }
        port = port * 10 + (c - b'0') as u32;
    }
    if port > 65535 {
        w("bad port\r\n");
        return;
    }

    w("tcp -> "); wb(ip); w(":"); dec(port as u64);
    w(" msg='"); wb(body); w("'\r\n");
    if body.is_empty() {
        w("no payload, just connect+close\r\n");
    }

    let idx = match tcp::connect(ip4, port as u16) {
        Some(i) => i,
        None => { w("connect: no conn slot\r\n"); return; }
    };
    if !tcp::wait_established(idx, 5000) {
        w("handshake failed/timeout\r\n");
        return;
    }
    w("[TCP] established\r\n");

    if !body.is_empty() {
        if !tcp::send(idx, body) {
            w("[TCP] send failed\r\n");
        }
    }

    // Качаем приём ~3s, печатая полученное.
    let start = timer::millis();
    let mut got_any = false;
    let mut buf = [0u8; 512];
    while timer::millis() - start < 3000 {
        let n = tcp::recv(idx, &mut buf);
        if n > 0 {
            got_any = true;
            for &b in &buf[..n] {
                uart::putchar(b);
                console::putchar(b);
            }
        }
        tcp::pump();
        core::hint::spin_loop();
    }
    if !got_any {
        w("\r\n(no reply in 3s)\r\n");
    }
    w("\r\n");

    tcp::close(idx);
    let _ = tcp::wait_closed(idx, 2000);
    w("[TCP] closed\r\n");
}

const DARK_RED: u32 = 0x00880000;
const BRIGHT_RED: u32 = 0x00CC0000;
const GOLD: u32 = 0x00FFD700;

fn draw_scale_char(x: u32, y: u32, ch: u8, scale: u32, fg: u32, bg: u32) {
    let font = &crate::font::FONT_8X16;
    let off = (ch as usize) * 16;
    for row in 0..16u32 {
        let bits = font[off + row as usize];
        for col in 0..8u32 {
            let color = if bits & (0x80 >> col) != 0 { fg } else { bg };
            if color != bg {
                console::draw_rect(x + col * scale, y + row * scale, scale, scale, color);
            }
        }
    }
}

fn draw_logo() {
    let cols = console::screen_cols();
    let cw = 8;
    let ch = 16;

    let banner_w = cols * cw;
    let banner_h = 10 * ch;
    console::draw_rect(0, 0, banner_w, banner_h, DARK_RED);

    console::draw_rect(0, 0, banner_w, 2, BRIGHT_RED);
    console::draw_rect(0, banner_h - 2, banner_w, 2, BRIGHT_RED);

    for y in 0..banner_h {
        console::draw_rect(0, y, 2, 1, GOLD);
        console::draw_rect(banner_w - 2, y, 2, 1, GOLD);
    }

    // "DBS" at 3x scale, in gold
    let scale = 3;
    let char_w = 8 * scale;
    let char_h = 16 * scale;
    let gap = char_w;
    let total_w = char_w * 3 + gap * 2;
    let x0 = (1280 - total_w) / 2;
    let y0 = (banner_h - char_h) / 2;
    draw_scale_char(x0, y0, b'D', scale, GOLD, DARK_RED);
    draw_scale_char(x0 + char_w + gap, y0, b'B', scale, GOLD, DARK_RED);
    draw_scale_char(x0 + char_w * 2 + gap * 2, y0, b'S', scale, GOLD, DARK_RED);

    console::set_cursor((cols - 14) / 2, 8);
    console::write_str("LEVEL_1_SYSTEM");
    console::set_cursor((cols - 24) / 2, 9);
    console::write_str("DBSos_1.0.0.Build.Test");
}

fn hex32(v: u32) {
    let h = b"0123456789ABCDEF";
    for i in (0..8).rev() { let n = (v >> (i * 4)) & 0xF; uart::putchar(h[n as usize]); console::putchar(h[n as usize]); }
}

/// Parse first word from a null-terminated buffer (skipping the command name)
fn parse_first_arg(buf: &[u8]) -> &[u8] {
    let mut i = 0;
    while buf[i] != 0 && buf[i] != b' ' { i += 1; }
    while buf[i] == b' ' { i += 1; }
    let start = i;
    while buf[i] != 0 && buf[i] != b' ' { i += 1; }
    &buf[start..i]
}

fn cmd_nvme(buf: &[u8]) {
    let sub = parse_first_arg(&buf);
    if sub.len() == 0 { w("nvme: missing subcommand (info|read|write)\r\n"); return; }
    if sub[0] == b'i' { // info
        if unsafe { nvme::INIT } {
            w("NVMe: initialized\r\n");
            w("  NSID=1 LBAs="); dec(unsafe { nvme::NS_LBA_COUNT }); w(" LBA_size="); dec(unsafe { nvme::NS_LBA_SIZE as u64 }); w("\r\n");
        } else {
            w("NVMe: not present\r\n");
        }
        return;
    }
    let (n1, n2) = parse_two_ints(&buf);
    if sub[0] == b'r' { // read
        if n1 as u64 == u64::MAX { w("Usage: nvme read <lba> <count>\r\n"); return; }
        let count = if n2 as u64 != u64::MAX { n2 as u16 } else { 1 };
        let mut tmp = [0u8; 512];
        w("NVMe read LBA="); dec(n1 as u64); w(" count="); dec(count as u64); w("\r\n");
        for s in 0..count as u64 {
            if !nvme::read_sectors(n1 as u64 + s, 1, tmp.as_mut_ptr()) {
                w("  READ ERROR\r\n"); return;
            }
            w("  "); dec(n1 as u64 + s); w(": ");
            for i in 0..16 {
                let v = (tmp[i*2] as u32) | ((tmp[i*2+1] as u32) << 8);
                hex32(v); w(" ");
            }
            w(" |");
            for i in 0..32 {
                let c = if tmp[i] >= 0x20 && tmp[i] < 0x7F { tmp[i] } else { b'.' };
                w(core::str::from_utf8(&[c]).unwrap_or("."));
            }
            w("|\r\n");
        }
    } else if sub[0] == b'w' { // write
        if n1 as u64 == u64::MAX { w("Usage: nvme write <lba> <count>\r\n"); return; }
        let count = if n2 as u64 != u64::MAX { n2 as u16 } else { 1 };
        let mut tmp = [0u8; 512];
        // Fill with pattern
        for i in 0..512 { tmp[i] = (i & 0xFF) as u8; }
        w("NVMe write LBA="); dec(n1 as u64); w(" count="); dec(count as u64); w(" (pattern)\r\n");
        for s in 0..count as u64 {
            if !nvme::write_sectors(n1 as u64 + s, 1, tmp.as_mut_ptr()) {
                w("  WRITE ERROR\r\n"); return;
            }
        }
        w("  OK\r\n");
    } else {
        w("nvme: unknown subcommand (info|read|write)\r\n");
    }
}

fn parse_two_ints(buf: &[u8]) -> (u32, u32) {
    let mut i = 0;
    while buf[i] != 0 && buf[i] != b' ' { i += 1; }
    while buf[i] == b' ' { i += 1; }
    let mut v1: u32 = 0;
    while buf[i] >= b'0' && buf[i] <= b'9' { v1 = v1.wrapping_mul(10).wrapping_add((buf[i] - b'0') as u32); i += 1; }
    if i == 0 || (buf[i] != 0 && buf[i] != b' ') { return (u32::MAX, u32::MAX); }
    while buf[i] == b' ' { i += 1; }
    let mut v2: u32 = 0;
    while buf[i] >= b'0' && buf[i] <= b'9' { v2 = v2.wrapping_mul(10).wrapping_add((buf[i] - b'0') as u32); i += 1; }
    (v1, v2)
}

pub fn run() {
    cwd_init();
    console::clear_screen();
    draw_logo();

    // Shell area starts at row 12
    console::set_cursor(0, 12);

    loop {
        tcp::pump();
        // Show prompt with cwd
        w("\r\n");
        let cwd = cwd_get();
        if cwd.len() <= 1 {
            w("DBSos:/ > ");
        } else {
            w("DBSos:");
            let s = core::str::from_utf8(cwd).unwrap_or("/");
            w(s);
            w("> ");
        }
        let mut buf = [0u8; 128];
        let len = readline(&mut buf);

        if len == 0 || buf[0] == 0 { continue; }

        // Parse: split into command and args
        let cmd_end = buf[..len].iter().position(|&c| c == b' ').unwrap_or(len);
        let cmd = &buf[..cmd_end];
        let arg_start = if cmd_end < len { cmd_end + 1 } else { cmd_end };
        let arg = &buf[arg_start..len];

        if cmd == b"help" { cmd_help(); }
        else if cmd == b"mem" { cmd_mem(); }
        else if cmd == b"time" { cmd_time(); }
        else if cmd == b"clear" { console::clear_screen(); draw_logo(); console::set_cursor(0, 12); }
        else if cmd == b"info" { cmd_info(); }
        else if cmd == b"pwd" { cmd_pwd(); }
        else if cmd == b"cd" { cmd_cd(arg); }
        else if cmd == b"echo" { cmd_echo(arg); }
        else if cmd == b"ls" {
            if arg.len() > 0 { fs::ls_path(arg); } else { fs::ls(); }
        }
        else if cmd == b"cat" {
            if arg.len() > 0 { fs::cat_path(arg); } else { w("Usage: cat PATH\r\n"); }
        }
        else if cmd == b"mkdir" {
            if arg.len() > 0 { fs::mkdir(arg); } else { w("Usage: mkdir PATH\r\n"); }
        }
        else if cmd == b"rm" {
            if arg.len() > 0 { fs::rm(arg); } else { w("Usage: rm PATH\r\n"); }
        }
        else if cmd == b"rmdir" {
            if arg.len() > 0 { fs::rmdir(arg); } else { w("Usage: rmdir PATH\r\n"); }
        }
        else if cmd == b"write" {
            if arg.len() > 0 {
                // Extract content after the path
                let space = arg.iter().position(|&c| c == b' ');
                match space {
                    Some(s) => {
                        let path = &arg[..s];
                        let content = &arg[s + 1..];
                        let content_len = content.iter().position(|&c| c == 0).unwrap_or(content.len());
                        fs::write_file(path, &content[..content_len]);
                    }
                    None => { w("Usage: write PATH TEXT\r\n"); }
                }
            } else { w("Usage: write PATH TEXT\r\n"); }
        }
        else if cmd == b"exec" {
            if arg.len() > 0 { crate::elf::load_and_spawn(arg); }
            else { w("Usage: exec PATH\r\n"); }
        }
        else if cmd == b"ping" { cmd_ping(); }
        else if cmd == b"tcp" { cmd_tcp(arg); }
        else if cmd == b"reboot" { w("Rebooting...\r\n"); crate::acpi::reboot(); }
        else if cmd == b"poweroff" { w("Shutting down...\r\n"); crate::acpi::shutdown(); }
        else if cmd == b"nvme" { cmd_nvme(&buf); }
        else {
            w("Unknown command: ");
            let s = core::str::from_utf8(cmd).unwrap_or("?");
            w(s);
            w("\r\nType 'help' for available commands.\r\n");
        }
    }
}