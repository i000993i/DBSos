use crate::console;
use crate::driver::uart;
use crate::driver::net;
use crate::driver::nvme;
use crate::fs;
use crate::memory;
use crate::timer;

fn w(s: &str) { console::write_str(s); uart::write_str(s); }

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

fn strcmp(a: &[u8], b: &str) -> bool {
    let b = b.as_bytes();
    if a.len() < b.len() { return false; }
    for i in 0..b.len() { if a[i] != b[i] { return false; } }
    a[b.len()] == 0 || a[b.len()] == b' '
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

fn parse_arg(buf: &[u8]) -> &[u8] {
    let mut i = 0;
    while buf[i] != 0 && buf[i] != b' ' { i += 1; }
    while buf[i] == b' ' { i += 1; }
    let start = i;
    while buf[i] != 0 && buf[i] != b' ' { i += 1; }
    &buf[start..i]
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

fn cmd_nvme(buf: &[u8]) {
    let sub = parse_arg(&buf);
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
    console::clear_screen();
    draw_logo();

    // Shell area starts at row 12
    console::set_cursor(0, 12);

    loop {
        net::poll();
        w("\r\nDBSos> ");
        let mut buf = [0u8; 128];
        let _ = readline(&mut buf);

        if strcmp(&buf, "help") { cmd_help(); }
        else if strcmp(&buf, "mem") { cmd_mem(); }
        else if strcmp(&buf, "time") { cmd_time(); }
        else if strcmp(&buf, "clear") { console::clear_screen(); draw_logo(); console::set_cursor(0, 12); }
        else if strcmp(&buf, "info") { cmd_info(); }
        else if strcmp(&buf, "ls") { fs::ls(); }
        else if buf.len() >= 3 && buf[0] == b'l' && buf[1] == b's' && buf[2] == b' ' {
            let arg = parse_arg(&buf);
            if arg.len() > 0 { fs::ls_path(arg); } else { w("Usage: ls PATH\r\n"); }
        }
        else if strcmp(&buf, "ping") { cmd_ping(); }
        else if strcmp(&buf, "reboot") { w("Rebooting...\r\n"); crate::acpi::reboot(); }
        else if strcmp(&buf, "poweroff") { w("Shutting down...\r\n"); crate::acpi::shutdown(); }
        else if buf.len() >= 8 && buf[0] == b'n' && buf[1] == b'v' && buf[2] == b'm' && buf[3] == b'e' && buf[4] == b' ' {
            cmd_nvme(&buf);
        }
        else if buf[0] == 0 { continue; }
        else {
            let mut is_cat = false;
            let mut is_exec = false;
            let mut is_write = false;
            if buf.len() >= 4 && buf[0] == b'c' && buf[1] == b'a' && buf[2] == b't' && buf[3] == b' ' {
                is_cat = true;
            }
            if buf.len() >= 5 && buf[0] == b'e' && buf[1] == b'x' && buf[2] == b'e' && buf[3] == b'c' && buf[4] == b' ' {
                is_exec = true;
            }
            let mut is_rm = false;
            let mut is_rmdir = false;
            let mut is_mkdir = false;
            if buf.len() >= 3 && buf[0] == b'r' && buf[1] == b'm' && buf[2] == b' ' {
                is_rm = true;
            }
            if buf.len() >= 6 && buf[0] == b'r' && buf[1] == b'm' && buf[2] == b'd' && buf[3] == b'i' && buf[4] == b'r' && buf[5] == b' ' {
                is_rmdir = true;
            }
            if buf.len() >= 6 && buf[0] == b'm' && buf[1] == b'k' && buf[2] == b'd' && buf[3] == b'i' && buf[4] == b'r' && buf[5] == b' ' {
                is_mkdir = true;
            }
            if buf.len() >= 6 && buf[0] == b'w' && buf[1] == b'r' && buf[2] == b'i' && buf[3] == b't' && buf[4] == b'e' && buf[5] == b' ' {
                is_write = true;
            }
            if is_rmdir {
                let arg = parse_arg(&buf);
                if arg.len() > 0 { fs::rmdir(arg); }
                else { w("Usage: rmdir PATH\r\n"); }
            } else if is_rm {
                let arg = parse_arg(&buf);
                if arg.len() > 0 { fs::rm(arg); }
                else { w("Usage: rm PATH\r\n"); }
            } else if is_mkdir {
                let arg = parse_arg(&buf);
                if arg.len() > 0 { fs::mkdir(arg); }
                else { w("Usage: mkdir PATH\r\n"); }
            } else if is_write {
                let arg = parse_arg(&buf);
                if arg.len() > 0 {
                    // Extract content after the path
                    let mut i = 0;
                    while buf[i] != 0 && buf[i] != b' ' { i += 1; }
                    while buf[i] == b' ' { i += 1; }
                    while buf[i] != 0 && buf[i] != b' ' { i += 1; }
                    while buf[i] == b' ' { i += 1; }
                    let content = if buf[i] != 0 { &buf[i..] } else { &[] };
                    // Find the null terminator
                    let content_len = content.iter().position(|&c| c == 0).unwrap_or(content.len());
                    fs::write_file(arg, &content[..content_len]);
                } else { w("Usage: write PATH TEXT\r\n"); }
            } else if is_cat {
                let arg = parse_arg(&buf);
                if arg.len() > 0 { fs::cat_path(arg); }
                else { w("Usage: cat PATH\r\n"); }
            } else if is_exec {
                let arg = parse_arg(&buf);
                if arg.len() > 0 { crate::elf::load_and_spawn(arg); }
                else { w("Usage: exec PATH\r\n"); }
            } else {
                w("Unknown command\r\n");
            }
        }
    }
}