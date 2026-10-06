use crate::display;
use crate::driver::uart;
use crate::driver::net;
use crate::driver::tcp;
use crate::driver::nvme;
use crate::fs_server;
use crate::memory;
use crate::timer;
use crate::font::FONT_8X16;

static mut CWD: [u8; 128] = [0; 128];

const CHAR_W: u32 = 8;
const CHAR_H: u32 = 16;

static mut TERM_COL: u32 = 0;
static mut TERM_ROW: u32 = 0;
static mut TERM_COLS: u32 = 0;
static mut TERM_ROWS: u32 = 0;

fn sw() -> u32 { display::width() }
fn sh() -> u32 { display::height() }

fn cwd_init() {
    unsafe { CWD[0] = b'/'; CWD[1] = 0; }
    crate::scheduler::set_current_cwd(b"/");
}

pub fn cwd_get_public() -> &'static [u8] {
    // Prefer per-task CWD if scheduler is active
    let slice = crate::scheduler::current_cwd_slice();
    if slice.len()>0 && !(slice.len()==1 && slice[0]==0) { return slice; }
    unsafe {
        let ptr = core::ptr::addr_of_mut!(CWD) as *const u8;
        let len = core::slice::from_raw_parts(ptr, 128)
            .iter().position(|&b| b == 0).unwrap_or(128);
        if len==0 { return b"/"; }
        &CWD[..len]
    }
}

pub fn cwd_set_public(path: &[u8]) {
    // Sync both global and per-task
    let len = path.len().min(127);
    unsafe {
        // global fallback
        CWD[..len].copy_from_slice(&path[..len]);
        CWD[len] = 0;
        if len < 127 { CWD[len+1]=0; }
    }
    // per-task
    // need to handle null-terminated vs slice: pass without trailing 0
    let actual_len = path.iter().position(|&c| c==0).unwrap_or(path.len()).min(127);
    crate::scheduler::set_current_cwd(&path[..actual_len]);
}

fn build_path<'a>(path: &[u8], buf: &'a mut [u8]) -> &'a mut [u8] {
    if path.len() > 0 && path[0] == b'/' {
        let len = path.len().min(buf.len() - 1);
        buf[..len].copy_from_slice(&path[..len]);
        buf[len] = 0;
        &mut buf[..len + 1]
    } else {
        let cwd = cwd_get_public();
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

// ── TUI rendering ────────────────────────────────────────────────

fn render_char_at(col: u32, row: u32, ch: u8) {
    let px = col * CHAR_W;
    let py = row * CHAR_H;
    let idx = (ch as usize) * 16;
    if idx + 16 > FONT_8X16.len() { return; }
    let glyph = &FONT_8X16[idx..idx + 16];
    let fb = display::framebuffer();
    let s = display::stride();
    let w = sw();
    let h = sh();
    for gy in 0..16u32 {
        let bits = glyph[gy as usize];
        let py2 = py + gy;
        if py2 >= h { break; }
        for gx in 0..8u32 {
            if bits & (0x80 >> gx) == 0 { continue; }
            let px2 = px + gx;
            if px2 >= w { break; }
            let off = (py2 * s + px2) as usize * 4;
            unsafe {
                *fb.add(off) = 0x41;
                *fb.add(off + 1) = 0xFF;
                *fb.add(off + 2) = 0x00;
            }
        }
    }
}

fn fill_rect(x: u32, y: u32, rw: u32, rh: u32) {
    let fb = display::framebuffer();
    let s = display::stride();
    let w = sw();
    let h = sh();
    for dy in 0..rh {
        let py = y + dy;
        if py >= h { break; }
        for dx in 0..rw {
            let px = x + dx;
            if px >= w { break; }
            let off = (py * s + px) as usize * 4;
            unsafe { *fb.add(off) = 0; *fb.add(off + 1) = 0; *fb.add(off + 2) = 0; }
        }
    }
}

fn present() { display::mark_dirty(0,0,sw(),sh()); unsafe { display::present(); } }

fn term_init() {
    unsafe {
        TERM_COL = 0;
        TERM_ROW = 0;
        // min 1x1: деление 800/8=100 ок, но до GOP init sw()/sh()==0 —
        // нули приводили к TERM_ROWS-1 underflow panic в term_putchar.
        TERM_COLS = (sw() / CHAR_W).max(1);
        TERM_ROWS = (sh() / CHAR_H).max(1);
    }
}
pub fn reset_term() {
    display::clear_screen(0,0,0);
    term_init();
    present();
}

fn scroll_up() {
    let fb = display::framebuffer();
    let s = display::stride();
    let h = sh();
    let w = sw();
    if fb.is_null() || s == 0 || w == 0 || h <= CHAR_H { return; }
    let copy_lines = h - CHAR_H;
    unsafe {
        // memmove: сдвиг вверх перекрывается (тот же UB-баг, что в dbs_gr).
        core::ptr::copy(
            fb.add((CHAR_H * s * 4) as usize),
            fb,
            (copy_lines * s * 4) as usize,
        );
    }
    fill_rect(0, copy_lines, w, CHAR_H);
}

fn term_putchar(ch: u8) {
    // Ленивая инициализация: консоль shell может быть активна без reset_term
    // (вход через F2 или вызовы из DBS-GR) — иначе COLS/ROWS=0 и первый же
    // символ роняет ядро на TERM_ROWS-1 underflow.
    unsafe {
        if TERM_COLS == 0 || TERM_ROWS == 0 { term_init(); }
        if TERM_COLS == 0 || TERM_ROWS == 0 { return; }
        if TERM_COL >= TERM_COLS { TERM_COL = 0; }
        if TERM_ROW >= TERM_ROWS { TERM_ROW = TERM_ROWS - 1; }
    }
    unsafe {
        match ch {
            b'\r' => { TERM_COL = 0; }
            b'\n' => {
                TERM_COL = 0;
                TERM_ROW += 1;
                if TERM_ROW >= TERM_ROWS { scroll_up(); TERM_ROW = TERM_ROWS - 1; }
            }
            0x08 => {
                if TERM_COL > 0 {
                    TERM_COL -= 1;
                    let px = TERM_COL * CHAR_W;
                    let py = TERM_ROW * CHAR_H;
                    fill_rect(px, py, CHAR_W, CHAR_H);
                } else if TERM_ROW > 0 {
                    // Wrap to previous line
                    TERM_ROW -= 1;
                    TERM_COL = TERM_COLS - 1;
                    let px = TERM_COL * CHAR_W;
                    let py = TERM_ROW * CHAR_H;
                    fill_rect(px, py, CHAR_W, CHAR_H);
                }
            }
            b' '..=b'~' => {
                render_char_at(TERM_COL, TERM_ROW, ch);
                TERM_COL += 1;
                if TERM_COL >= TERM_COLS {
                    TERM_COL = 0;
                    TERM_ROW += 1;
                    if TERM_ROW >= TERM_ROWS { scroll_up(); TERM_ROW = TERM_ROWS - 1; }
                }
            }
            _ => {}
        }
    }
}

fn w(s: &str) {
    if unsafe { crate::dbs_gr::GR_ACTIVE } { crate::dbs_gr::w(s); return; }
    uart::write_str(s);
    for ch in s.bytes() { term_putchar(ch); }
}

fn present_cursor() {
    unsafe {
        let px = TERM_COL * CHAR_W;
        let py = TERM_ROW * CHAR_H;
        fill_rect(px, py + CHAR_H - 2, CHAR_W, 2);
        present();
    }
}

fn erase_cursor() {
    unsafe {
        let px = TERM_COL * CHAR_W;
        let py = TERM_ROW * CHAR_H;
        fill_rect(px, py, CHAR_W, CHAR_H);
    }
}

fn readline(buf: &mut [u8]) -> usize {
    let mut i = 0;
    let mut cursor_on = false;
    let mut last_blink_ms: u64 = 0;

    loop {
        let now = timer::millis();
        if now - last_blink_ms >= 500 {
            last_blink_ms = now;
            cursor_on = !cursor_on;
            erase_cursor();
            if cursor_on { present_cursor(); }
            else { present(); }
        }

        let c = if let Some(c) = crate::driver::ps2::poll_char() {
            Some(c)
        } else if let Some(c) = uart::poll_char() {
            Some(c)
        } else {
            timer::usleep(1000);
            continue;
        };

        let ch = if let Some(v) = c { v } else { continue; };

        cursor_on = false;
        erase_cursor();

        match ch {
            b'\r' | b'\n' => {
                w("\r\n");
                buf[i] = 0;
                return i;
            }
            0x03 => {
                w("^C\r\n");
                buf[0] = 0;
                return 0;
            }
            0x7F | 0x08 => {
                if i > 0 {
                    i -= 1;
                    buf[i] = 0;
                    unsafe {
                        if TERM_COL > 0 {
                            TERM_COL -= 1;
                        } else if TERM_ROW > 0 {
                            TERM_ROW -= 1;
                            TERM_COL = TERM_COLS - 1;
                        }
                    }
                    let px = unsafe { TERM_COL } * CHAR_W;
                    let py = unsafe { TERM_ROW } * CHAR_H;
                    fill_rect(px, py, CHAR_W, CHAR_H);
                }
            }
            0x04 => { // Ctrl+D — EOF
                if i == 0 { buf[0] = 0; return 0; }
            }
            b' '..=b'~' => {
                if i < buf.len() - 1 {
                    buf[i] = ch;
                    i += 1;
                    term_putchar(ch);
                }
            }
            _ => {}
        }
    }
}

// ── Number formatting ─────────────────────────────────────────────

fn dec(mut v: u64) {
    if unsafe { crate::dbs_gr::GR_ACTIVE } { crate::dbs_gr::dec(v); return; }
    if v == 0 { uart::putchar(b'0'); term_putchar(b'0'); return; }
    let mut b = [0u8; 20]; let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; let c = b[i]; uart::putchar(c); term_putchar(c); }
}

fn hex(mut v: u64) {
    if unsafe { crate::dbs_gr::GR_ACTIVE } { crate::dbs_gr::hex(v); return; }
    if v == 0 { uart::putchar(b'0'); term_putchar(b'0'); return; }
    let mut b = [0u8; 16]; let mut i = 0;
    while v > 0 { let n = (v&0xF) as u8; b[i]=if n<10{b'0'+n}else{b'A'+n-10}; v>>=4; i+=1; }
    while i > 0 { i -= 1; let c = b[i]; uart::putchar(c); term_putchar(c); }
}

unsafe fn cpuid_raw(leaf: u32) -> (u32, u32, u32, u32) {
    let eax_out: u32; let ecx_out: u32; let edx_out: u32;
    let mut ebx_buf: u32 = 0;
    core::arch::asm!(
        "push rbx", "cpuid", "mov [{ebx_ptr}], ebx", "pop rbx",
        inlateout("eax") leaf => eax_out, out("ecx") ecx_out, out("edx") edx_out,
        ebx_ptr = in(reg) &mut ebx_buf,
    );
    (eax_out, ebx_buf, ecx_out, edx_out)
}

// ── Commands ──────────────────────────────────────────────────────

fn cmd_help() {
    w("=== DBSos Levels ===\r\n");
    w("  LEVEL_0 kernel  — memory, drivers, scheduler\r\n");
    w("  LEVEL_1 TUI     — this shell (Ubuntu Server-like)\r\n");
    w("  LEVEL_2 Ubuntu  — Yaru/GNOME Wayland desktop\r\n");
    w("Commands: help, info, hw, cpu, mem, top, time, date, clear, reboot, poweroff\r\n");
    w("  ls [PATH], cat PATH, mkdir PATH, rm PATH, rmdir PATH\r\n");
    w("  cd PATH, pwd, echo TEXT, write PATH TEXT\r\n");
    w("  ping, dhcp, dns HOST, tcp IP PORT TEXT, wget URL [FILE]\r\n");
    w("  net [status|use e1000|rtl8139], wifi [scan|pmk SSID PSK]\r\n");
    w("  usb, audio — controller probes\r\n");
    w("  ide — PATA/ATAPI drives\r\n");
    w("  connect [list|add SSID [PSK]|forget SSID|auto|test|SSID] — networks\r\n");
    w("  selftest — RS-Kernel-Test: проверка целостности ядра (OK/NO)\r\n");
    w("  pkg install|remove|list|update, script PATH\r\n");
    w("  nvme info|read|write, exec PATH (DBSos or Linux ELF), lexec PATH\r\n");
    w("  fm [PATH] — file manager (tree, gray folders)\r\n");
    w("  level [0|1|2] — show/switch level\r\n");
    w("  driver list|fetch PKG — adaptive drivers\r\n");
    w("  gpu, display — GPU / screen info\r\n");
    w("  whoami, id, users, useradd NAME [PW], login USER [PW], su USER, passwd\r\n");
    w("  chmod OCTAL PATH, chown USER PATH\r\n");
    w("  plasma — start Ubuntu desktop\r\n");
    w("  wldemo, wlinfo — Wayland compositor demo\r\n");
    w("FS: FAT12/16/32, ext4 (stub), VFS multi-mount, perms rwx, Wayland surfaces\r\n");
}

fn cmd_level(arg: &[u8]) {
    let lvl = crate::system::current();
    if arg.is_empty() {
        w("Current: "); w(lvl.name()); w("\r\n");
        w("  "); w(lvl.desc()); w("\r\n");
        crate::system::boot_banner();
        return;
    }
    // trim spaces
    let mut p = 0;
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    if p < arg.len() && arg[p] == b'0' { crate::system::set(crate::system::Level::L0Kernel); w(" -> L0 (kernel only, reboot to apply)\r\n"); }
    else if p < arg.len() && arg[p] == b'1' { crate::system::set(crate::system::Level::L1Tui); w(" -> L1 TUI\r\n"); }
    else if p < arg.len() && arg[p] == b'2' { w(" -> L2 Plasma...\r\n"); present(); crate::plasma::run(); crate::system::set(crate::system::Level::L1Tui); reset_term(); w("back to L1\r\n"); }
    else { w("Usage: level [0|1|2]\r\n"); }
}

fn cmd_driver(arg: &[u8]) {
    // trim
    let mut p = 0;
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let sub = &arg[p..];
    if sub.starts_with(b"list") || sub.is_empty() {
        crate::driver::adapt::print_report();
        // Also render to framebuffer
        w("Adaptive drivers: see UART log\r\n");
        w("Use: driver fetch <pkg>  e.g. driver fetch driver-virtio\r\n");
    } else if sub.starts_with(b"fetch ") {
        let pkg = &sub[6..];
        let mut s = 0;
        while s < pkg.len() && pkg[s] == b' ' { s += 1; }
        if s >= pkg.len() { w("Usage: driver fetch <pkg>\r\n"); return; }
        let name = &pkg[s..];
        w("Fetching "); w(core::str::from_utf8(name).unwrap_or("?")); w("...\r\n");
        if crate::driver::adapt::fetch(name) { w("fetch done\r\n"); } else { w("fetch failed\r\n"); }
    } else {
        w("Usage: driver list | driver fetch <pkg>\r\n");
    }
}

fn cmd_whoami() {
    w(crate::user::current_name());
    w(" uid="); dec(crate::user::current_uid() as u64);
    w(" gid="); dec(crate::user::current_gid() as u64); w("\r\n");
}
fn cmd_users() { crate::user::list_users(); w("users listed (UART)\r\n"); }
fn cmd_useradd(arg: &[u8]) {
    let mut p=0; while p<arg.len() && arg[p]==b' '{p+=1;}
    if p>=arg.len(){ w("Usage: useradd <name> [password]\r\n"); return; }
    let s=p; while p<arg.len() && arg[p]!=b' ' && arg[p]!=0 {p+=1;}
    let name=&arg[s..p];
    while p<arg.len() && arg[p]==b' '{p+=1;}
    let pw=if p<arg.len(){&arg[p..]} else {b""};
    if crate::user::add_user(name, pw) { w("user "); w(core::str::from_utf8(name).unwrap_or("?")); w(" added\r\n"); } else { w("useradd failed\r\n"); }
}
fn cmd_login(arg: &[u8]) {
    let mut p=0; while p<arg.len() && arg[p]==b' '{p+=1;}
    let s=p; while p<arg.len() && arg[p]!=b' ' {p+=1;}
    if s>=p { w("Usage: login <user> [password]\r\n"); return; }
    let name=&arg[s..p];
    while p<arg.len() && arg[p]==b' '{p+=1;}
    let pw=&arg[p..];
    if crate::user::login(name, pw) {
        w("logged in as "); w(core::str::from_utf8(name).unwrap_or("?")); w("\r\n");
        // Auto cd to home
        if crate::user::find_by_name(name).is_some() {
            let mut home = [0u8; 64];
            let pre = b"/home/";
            home[..pre.len()].copy_from_slice(pre);
            let nl = name.len().min(64 - pre.len());
            home[pre.len()..pre.len()+nl].copy_from_slice(&name[..nl]);
            if crate::vfs::is_dir(&home[..pre.len()+nl]) {
                crate::shell::cwd_set_public(&home[..pre.len()+nl]);
            }
        }
    } else { w("login failed\r\n"); }
}
fn cmd_id() {
    w("uid="); dec(crate::user::current_uid() as u64);
    w(" gid="); dec(crate::user::current_gid() as u64);
    w(" user="); w(crate::user::current_name()); w("\r\n");
}
fn parse_octal(s: &[u8]) -> Option<u16> {
    let mut v: u16 = 0;
    if s.is_empty() { return None; }
    for &c in s {
        if c < b'0' || c > b'7' { return None; }
        v = (v << 3) | ((c - b'0') as u16);
        if v > 0o777 { return None; }
    }
    Some(v)
}
fn cmd_chmod(arg: &[u8]) {
    let mut p=0; while p<arg.len() && arg[p]==b' '{p+=1;}
    let s=p; while p<arg.len() && arg[p]!=b' ' {p+=1;}
    if s>=p { w("Usage: chmod OCTAL PATH\r\n"); return; }
    let mode_s=&arg[s..p];
    while p<arg.len() && arg[p]==b' '{p+=1;}
    if p>=arg.len() { w("Usage: chmod OCTAL PATH\r\n"); return; }
    let path=&arg[p..];
    let mode = match parse_octal(mode_s) { Some(m)=>m, None=>{w("bad mode\r\n"); return;}};
    let perm = crate::permissions::get(path);
    if crate::user::current_uid() != 0 && perm.owner != crate::user::current_uid() { w("chmod: not owner\r\n"); return; }
    crate::permissions::set(path, crate::permissions::Perm { owner: perm.owner, group: perm.group, mode });
    w("chmod ok\r\n");
}
fn cmd_chown(arg: &[u8]) {
    if crate::user::current_uid() != 0 { w("chown: only root\r\n"); return; }
    let mut p=0; while p<arg.len() && arg[p]==b' '{p+=1;}
    let s=p; while p<arg.len() && arg[p]!=b' ' {p+=1;}
    if s>=p { w("Usage: chown USER PATH\r\n"); return; }
    let user_s=&arg[s..p];
    while p<arg.len() && arg[p]==b' '{p+=1;}
    if p>=arg.len() { w("Usage: chown USER PATH\r\n"); return; }
    let path=&arg[p..];
    let uid = if let Some(u)=crate::user::uid_by_name(user_s) { u } else {
        // try numeric uid
        let mut v: u32 = 0; let mut ok=true;
        for &c in user_s { if c<b'0'||c>b'9'{ok=false; break;} v=v*10+(c-b'0')as u32; }
        if !ok { w("unknown user\r\n"); return; }
        v
    };
    let perm = crate::permissions::get(path);
    crate::permissions::set(path, crate::permissions::Perm { owner: uid, group: uid, mode: perm.mode });
    w("chown ok\r\n");
}
fn cmd_su(arg: &[u8]) {
    let mut p=0; while p<arg.len() && arg[p]==b' '{p+=1;}
    if p>=arg.len() { w("Usage: su USER [PASS]\r\n"); return; }
    let s=p; while p<arg.len() && arg[p]!=b' ' {p+=1;}
    let name=&arg[s..p];
    while p<arg.len() && arg[p]==b' '{p+=1;}
    let pw=&arg[p..];
    if crate::user::login(name, pw) { w("su: logged in as "); w(core::str::from_utf8(name).unwrap_or("?")); w("\r\n"); } else { w("su: failed\r\n"); }
}
fn cmd_passwd(arg: &[u8]) {
    let mut p=0; while p<arg.len() && arg[p]==b' '{p+=1;}
    if p>=arg.len(){
        // change own password: need old + new? simplified: arg is new password
        w("Usage: passwd [USER] NEWPASS\r\n"); return;
    }
    // parse optional USER
    let s=p; while p<arg.len() && arg[p]!=b' ' {p+=1;}
    let first=&arg[s..p];
    let mut pp=p; while pp<arg.len() && arg[pp]==b' '{pp+=1;}
    if pp>=arg.len(){
        // single arg: change own password to first
        let cur = crate::user::current_name();
        let cur_bytes = cur.as_bytes();
        if crate::user::set_password(cur_bytes, first){ w("passwd: changed\r\n"); } else { w("passwd: failed\r\n"); }
    } else {
        // two args: USER NEWPASS, only root
        if crate::user::current_uid()!=0 { w("passwd: only root can change others\r\n"); return; }
        let pw=&arg[pp..];
        if crate::user::set_password(first, pw){ w("passwd: changed\r\n"); } else { w("passwd: failed\r\n"); }
    }
}
fn cmd_gpu() {
    if let Some(g) = crate::driver::gpu::info().or(crate::driver::gpu::detect()) {
        w("GPU: "); dec(g.vendor as u64); w(":"); dec(g.device as u64);
        w(" "); dec(g.width as u64); w("x"); dec(g.height as u64); w("\r\n");
    } else { w("GPU: not found\r\n"); }
    crate::driver::gpu::print_info();
}
fn cmd_display() {
    w("Display: "); dec(crate::display::width() as u64); w("x"); dec(crate::display::height() as u64);
    w(" stride="); dec(crate::display::stride() as u64); w("\r\n");
    w("GOP modes (set at boot only):\r\n");
    let n = crate::display::mode_count();
    let cw = crate::display::width();
    let ch = crate::display::height();
    for i in 0..n {
        if let Some((mw, mh)) = crate::display::mode_info(i) {
            w("  "); dec(i as u64); w(": "); dec(mw as u64); w("x"); dec(mh as u64);
            if mw == cw && mh == ch { w(" *current*"); }
            w("\r\n");
        }
    }
    w("FS: FAT12/16/32, ext4 (stub), VFS multi-mount\r\n");
}

fn cmd_pwd() {
    let cwd = cwd_get_public();
    if cwd.len() <= 1 { w("/"); } else { w(core::str::from_utf8(cwd).unwrap_or("?")); }
    w("\r\n");
}

fn cmd_cd(arg: &[u8]) {
    if arg.len() == 0 { cwd_set_public(b"/"); return; }
    let mut path_buf = [0u8; 256];
    let abs_path = build_path(arg, &mut path_buf);
    let mut resolved = [0u8; 128];
    let mut rpos = 0;
    let mut i = 0;
    while i < abs_path.len() && abs_path[i] != 0 {
        while i < abs_path.len() && abs_path[i] == b'/' { i += 1; }
        if i >= abs_path.len() || abs_path[i] == 0 { break; }
        let start = i;
        while i < abs_path.len() && abs_path[i] != b'/' && abs_path[i] != 0 { i += 1; }
        let comp = &abs_path[start..i];
        if comp == b".." {
            if rpos > 1 { rpos -= 1; while rpos > 0 && resolved[rpos - 1] != b'/' { rpos -= 1; } if rpos > 0 { rpos -= 1; } if rpos == 0 { resolved[0] = b'/'; rpos = 1; } }
        } else if comp != b"." {
            if rpos > 1 { resolved[rpos] = b'/'; rpos += 1; }
            for &b in comp { if rpos < 127 { resolved[rpos] = b; rpos += 1; } }
        }
    }
    if rpos == 0 { resolved[0] = b'/'; rpos = 1; }
    resolved[rpos] = 0;
    cwd_set_public(&resolved[..rpos + 1]);
}

fn cmd_echo(arg: &[u8]) {
    if arg.len() > 0 { w(core::str::from_utf8(arg).unwrap_or("<binary>")); }
    w("\r\n");
}

fn cmd_mem() {
    let free = memory::free_count() as u64;
    w("Free pages: "); dec(free); w(" ("); dec(free * 4096 / 1024); w(" KB)");
    w(" pressure: "); dec(memory::pressure() as u64); w("%");
    if memory::pressure() >= 95 { w(" [OOM-RISK]"); }
    w("\r\n");
}

fn cmd_time() {
    let t0 = timer::millis(); let c0 = timer::ticks();
    timer::usleep(10_000);
    let c1 = timer::ticks(); let t1 = timer::millis();
    w("ticks: "); dec(c0); w(" -> "); dec(c1);
    w(", delta: "); dec(c1 - c0); w(" (10ms), ms: "); dec(t1 - t0); w("\r\n");
}

fn cmd_date() {
    // Московское время (Europe/Moscow, UTC+3)
    let t = crate::driver::rtc::read_msk();
    p2(t.day); w("."); p2(t.month); w("."); dec(t.year as u64);
    w(" "); p2(t.hour); w(":"); p2(t.minute); w(":"); p2(t.second);
    w(" MSK (UTC+3)\r\n");
}

fn p2(v: u8) {
    if v < 10 { w("0"); }
    dec(v as u64);
}

fn cmd_info() {
    w("DBSos v0.1\r\nArch: x86_64  Boot: UEFI  Display: GOP\r\n");
    w("Kernel: Rust no_std  Shell: TUI\r\n");
    cmd_mem();
}

fn cmd_hw() {
    w("=== Hardware ===\r\n");
    w("CPU: ");
    unsafe {
        let (_eax, ebx, ecx, edx) = cpuid_raw(0);
        let mut vendor = [0u8; 12];
        vendor[0..4].copy_from_slice(&ebx.to_le_bytes());
        vendor[4..8].copy_from_slice(&edx.to_le_bytes());
        vendor[8..12].copy_from_slice(&ecx.to_le_bytes());
        w(core::str::from_utf8(&vendor).unwrap_or("Unknown"));
        for level in [0x80000002u32, 0x80000003, 0x80000004] {
            let (a, b, c, d) = cpuid_raw(level);
            let mut buf = [0u8; 16];
            buf[0..4].copy_from_slice(&a.to_le_bytes());
            buf[4..8].copy_from_slice(&b.to_le_bytes());
            buf[8..12].copy_from_slice(&c.to_le_bytes());
            buf[12..16].copy_from_slice(&d.to_le_bytes());
            let s = core::str::from_utf8(&buf).unwrap_or("");
            if !s.is_empty() { w(" "); w(s); }
        }
    }
    w("\r\n");
    let ncpu = crate::acpi::ncpu();
    w("CPUs: "); dec(ncpu as u64); w("\r\n");
    w("LAPIC: 0x"); hex(crate::acpi::lapic_phys() as u64);
    w(" IOAPICs: "); dec(crate::acpi::ioapic_count() as u64);
    w(" IRQ-overrides: "); dec(crate::acpi::irq_override_count() as u64); w("\r\n");
    w("VirtIO/USB: probe via driver scan below\r\n");
    w("FS: FAT12/16/32 + tmpfs /tmp\r\n");
    w("DBG: backtrace=rbp-chain (on #PF/panic)\r\n");
    w("PCI:\r\n");
    crate::driver::pci::scan_for_hw();
    if unsafe { nvme::INIT } { w("NVMe: OK\r\n"); } else { w("NVMe: none\r\n"); }
}

fn cmd_cpu() {
    w("=== CPU ===\r\n");
    unsafe {
        let (eax, ebx, ecx, edx) = cpuid_raw(0);
        let mut vendor = [0u8; 12];
        vendor[0..4].copy_from_slice(&ebx.to_le_bytes());
        vendor[4..8].copy_from_slice(&edx.to_le_bytes());
        vendor[8..12].copy_from_slice(&ecx.to_le_bytes());
        w("Vendor: "); w(core::str::from_utf8(&vendor).unwrap_or("?")); w("\r\n");
        w("Max leaf: "); dec(eax as u64); w("\r\n");
        let (eax1, ebx1, ecx1, edx1) = cpuid_raw(1);
        let stepping = eax1 & 0xF;
        let mut model = (eax1 >> 4) & 0xFF;
        let mut family = (eax1 >> 8) & 0xF;
        let ext_model = (eax1 >> 16) & 0xF;
        let ext_family = (eax1 >> 20) & 0xFF;
        if family == 0xF { family += ext_family; }
        if family == 0x6 || family == 0xF { model += ext_model << 4; }
        w("Family: 0x"); hex(family as u64);
        w(" Model: 0x"); hex(model as u64);
        w(" Step: "); dec(stepping as u64); w("\r\n");
        w("APIC: "); dec(((ebx1 >> 24) & 0xFF) as u64); w("\r\n");
        let cr0: u64; let cr4: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0);
        core::arch::asm!("mov {}, cr4", out(reg) cr4);
        w("CR0: 0x"); hex(cr0); w("\r\nCR4: 0x"); hex(cr4); w("\r\n");
        let lo: u32; let hi: u32;
        core::arch::asm!("rdmsr", in("ecx") 0xC0000080u32, out("eax") lo, out("edx") hi);
        w("EFER: 0x"); hex(((hi as u64) << 32) | (lo as u64)); w("\r\n");
        w("Features: ");
        if edx1 & (1 << 23) != 0 { w("MMX "); }
        if edx1 & (1 << 25) != 0 { w("SSE "); }
        if edx1 & (1 << 26) != 0 { w("SSE2 "); }
        if ecx1 & (1 << 0) != 0 { w("SSE3 "); }
        if ecx1 & (1 << 19) != 0 { w("SSE4.1 "); }
        if ecx1 & (1 << 20) != 0 { w("SSE4.2 "); }
        if ecx1 & (1 << 28) != 0 { w("AVX "); }
        w("\r\n");
        // Brand string (для Plasma: точная модель CPU)
        let (maxext, _, _, _) = cpuid_raw(0x80000000);
        if maxext >= 0x80000004 {
            w("Brand:");
            for level in [0x80000002u32, 0x80000003, 0x80000004] {
                let (a, b, c, d) = cpuid_raw(level);
                let mut buf = [0u8; 16];
                buf[0..4].copy_from_slice(&a.to_le_bytes());
                buf[4..8].copy_from_slice(&b.to_le_bytes());
                buf[8..12].copy_from_slice(&c.to_le_bytes());
                buf[12..16].copy_from_slice(&d.to_le_bytes());
                // обрезать trailing пробелы/нули
                let mut end = 16;
                while end > 0 && (buf[end - 1] == b' ' || buf[end - 1] == 0) { end -= 1; }
                let mut start = 0;
                while start < end && buf[start] == b' ' { start += 1; }
                if end > start { w(" "); w(core::str::from_utf8(&buf[start..end]).unwrap_or("")); }
            }
            w("\r\n");
        }
        // HTT / логические CPU
        w("HTT: ");
        if edx1 & (1 << 28) != 0 {
            w("yes threads/pkg="); dec((((ebx1 >> 16) & 0xFF)) as u64);
        } else { w("no"); }
        if ecx1 & (1 << 31) != 0 { w(" HV(guest)"); }
        if ecx1 & (1 << 21) != 0 { w(" x2APIC"); }
        w("\r\n");
        // Топология SMP из MADT (BSP + APIC ID всех CPU; AP пока в BSP-only)
        w("SMP: ncpu="); dec(crate::acpi::ncpu() as u64); w(" apic=[");
        for i in 0..crate::acpi::ncpu().min(8) {
            if i > 0 { w(","); }
            match crate::acpi::apic_id_of(i) {
                Some(id) => dec(id as u64),
                None => w("?"),
            }
        }
        w("] mode=BSP-only\r\n");
        // Расширенные фичи 0x80000001 (важно для 64-бит USER: NX, SYSCALL, LM)
        if maxext >= 0x80000001 {
            let (_, _, ecx_e, edx_e) = cpuid_raw(0x80000001);
            w("Ext: ");
            if edx_e & (1 << 29) != 0 { w("LM "); }
            if edx_e & (1 << 11) != 0 { w("SYSCALL "); }
            if edx_e & (1 << 20) != 0 { w("NX "); }
            if edx_e & (1 << 27) != 0 { w("RDTSCP "); }
            if edx_e & (1 << 26) != 0 { w("1GB-page "); }
            if ecx_e & (1 << 5) != 0 { w("LZCNT "); }
            w("\r\n");
        }
        // Физические/виртуальные биты адреса
        if maxext >= 0x80000008 {
            let (eax8, _, _, _) = cpuid_raw(0x80000008);
            w("Addr: phys="); dec((eax8 & 0xFF) as u64);
            w(" virt="); dec(((eax8 >> 8) & 0xFF) as u64); w("\r\n");
        }
        // Кэши L1 (0x80000005) и L2/L3 (0x80000006)
        if maxext >= 0x80000006 {
            let (eax5, ebx5, ecx5, edx5) = cpuid_raw(0x80000005);
            let (_, _, ecx6, edx6) = cpuid_raw(0x80000006);
            w("L1d: "); dec(((ecx5 >> 24) & 0xFF) as u64); w("KB L1i: ");
            dec(((edx5 >> 24) & 0xFF) as u64); w("KB L2: ");
            dec(((ecx6 >> 16) & 0xFFFF) as u64); w("KB L3: ");
            dec((((edx6 >> 18) & 0x3FFF) * 512) as u64); w("KB\r\n");
            let _ = (eax5, ebx5);
        }
        // Invariant TSC (таймеры ядра)
        if maxext >= 0x80000007 {
            let (_, _, _, edx7) = cpuid_raw(0x80000007);
            w("TSC: "); w(if edx7 & (1 << 8) != 0 { "invariant" } else { "variant" }); w("\r\n");
        }
    }
}

fn cmd_top() {
    let my_uid = crate::user::current_uid();
    let is_root = my_uid==0;
    w("IDX PID    UID  STATE      RING\r\n");
    w("--- ------ ---- ---------- ----\r\n");
    crate::scheduler::for_each_task(|idx, task| {
        if !is_root && task.uid != my_uid { return; }
        dec(idx as u64);
        if idx < 10 { w("   "); } else { w("  "); }
        dec(task.id);
        let pad = if task.id < 10 { "      " } else if task.id < 100 { "     " } else if task.id < 1000 { "    " } else { "   " };
        w(pad);
        dec(task.uid as u64);
        w(if task.uid<10 {"   "} else if task.uid<100 {"  "} else {" "});
        let state = match task.state {
            crate::scheduler::TaskState::Running => "Running",
            crate::scheduler::TaskState::Ready => "Ready",
            crate::scheduler::TaskState::Exited => "Exited",
            crate::scheduler::TaskState::BlockedSend => "BlkSend",
            crate::scheduler::TaskState::BlockedRecv => "BlkRecv",
            _ => "???",
        };
        w(state);
        for _ in state.len()..10 { w(" "); }
        w(if task.ring3 { "user" } else { "kern" });
        w("\r\n");
    });
    if !is_root { w("(filtered by uid)\r\n"); }
    w("--- linux ---\r\n");
    w("IDX PID    ENTRY        STATE\r\n");
    crate::linux::task::for_each_task(|idx, pid, entry, alive| {
        if !alive { return; }
        dec(idx as u64);
        if idx < 10 { w("   "); } else { w("  "); }
        dec(pid);
        w(if pid < 10 { "      " } else if pid < 100 { "     " } else if pid < 1000 { "    " } else { "   " });
        w("0x"); hex(entry);
        w("  alive\r\n");
    });
    cmd_mem();
    w("Tasks for uid "); dec(my_uid as u64);
    w(": "); dec(crate::scheduler::task_count_for_uid(my_uid) as u64);
    w("/"); dec(crate::scheduler::MAX_TASKS_PER_USER as u64); w("\r\n");
}

fn cmd_ping() {
    let gw = net::gateway();
    w("ARP ping...\r\n");
    net::send_arp_request(gw);
    let deadline = timer::millis() + 2000;
    while timer::millis() < deadline { net::poll(); core::hint::spin_loop(); }
    w("done\r\n");
}

fn cmd_dhcp() {
    if crate::driver::dhcp::run(5000) { w("DHCP OK\r\n"); } else { w("DHCP failed\r\n"); }
}

fn cmd_dns(arg: &[u8]) {
    let mut p = 0;
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let start = p;
    while p < arg.len() && arg[p] != b' ' && arg[p] != 0 { p += 1; }
    if p == start { w("Usage: dns HOST\r\n"); return; }
    let host = &arg[start..p];
    w("resolving...\r\n");
    let deadline = timer::millis() + 3000;
    while timer::millis() < deadline {
        net::poll();
        if let Some(ip) = crate::driver::dns::resolve(host, 3000) {
            dec(ip[0] as u64); w("."); dec(ip[1] as u64); w("."); dec(ip[2] as u64); w("."); dec(ip[3] as u64); w("\r\n");
            return;
        }
    }
    w("timeout\r\n");
}

fn cmd_tcp(arg: &[u8]) {
    let mut p = 0;
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let ip_start = p;
    while p < arg.len() && arg[p] != b' ' && arg[p] != 0 { p += 1; }
    if p == ip_start { w("Usage: tcp IP PORT TEXT\r\n"); return; }
    let ip_str = &arg[ip_start..p];
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let port_start = p;
    while p < arg.len() && arg[p] >= b'0' && arg[p] <= b'9' { p += 1; }
    if p == port_start { w("Usage: tcp IP PORT TEXT\r\n"); return; }
    let port: u16 = arg[port_start..p].iter().fold(0u16, |a, &d| a * 10 + (d - b'0') as u16);
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let body = &arg[p..];

    let mut ip = [0u8; 4]; let mut oct = 0u8; let mut idx = 0;
    for &b in ip_str {
        if b == b'.' { idx += 1; oct = 0; }
        else if b >= b'0' && b <= b'9' { oct = oct * 10 + (b - b'0'); ip[idx] = oct; }
    }

    w("connecting...\r\n");
    let ci = tcp::connect(ip, port);
    if ci.is_none() { w("failed\r\n"); return; }
    let ci = ci.unwrap();
    if !body.is_empty() { tcp::send(ci, body); }
    let start = timer::millis();
    let mut buf = [0u8; 512];
    while timer::millis() - start < 3000 {
        let n = tcp::recv(ci, &mut buf);
        if n > 0 { for &b in &buf[..n] {
            if unsafe { crate::dbs_gr::GR_ACTIVE } { crate::dbs_gr::w(core::str::from_utf8(&[b]).unwrap_or(" ")); }
            else { uart::putchar(b); term_putchar(b); }
        } }
        tcp::pump();
        core::hint::spin_loop();
    }
    w("\r\n");
    tcp::close(ci);
    let _ = tcp::wait_closed(ci, 2000);
}

fn cmd_connect_dialog() {
    use crate::netman;
    // Диалог сетей: скан адаптеров + запомненные профили + подключение.
    // Wi-Fi-радио в QEMU нет — честно показываем wired; каркас под WPA готов.
    let (eth, wifi) = netman::scan_adapters();
    let cols = (sw() / CHAR_W) as i32;
    let rows_n = (sh() / CHAR_H) as i32;
    let bw: i32 = 52;
    let n = netman::count();
    let bh: i32 = 10 + n as i32;
    let bx = ((cols - bw) / 2).max(0);
    let by = ((rows_n - bh) / 2).max(0);
    let mut status: &[u8] = b"0-7: connect  q: quit";
    loop {
        // рамка (shell fill_rect — чёрный; текст поверх терминала)
        fill_rect((bx * 8) as u32, (by * 16) as u32, (bw * 8) as u32, (bh * 16) as u32);
        dlg_line(bx, by, bw, 0, b"+-- Connect: networks --+", 0xE95420);
        let mut tmp = [0u8; 48];
        let l = dlg_adapters(&mut tmp, eth, wifi);
        dlg_line(bx, by, bw, 2, &tmp[..l], 0xFFFFFF);
        if wifi == 0 {
            dlg_line(bx, by, bw, 3, b"no wireless adapter (wired e1000)", 0xAEA79F);
        } else {
            dlg_line(bx, by, bw, 3, b"wireless adapter present", 0x4CD964);
        }
        if n == 0 {
            dlg_line(bx, by, bw, 5, b"<no remembered networks>", 0xAEA79F);
            dlg_line(bx, by, bw, 6, b"connect add SSID [PSK]", 0xAEA79F);
        }
        for i in 0..n {
            if let Some(p) = netman::get(i) {
                let mut line = [b' '; 50];
                line[0] = b'0' + i as u8;
                line[1] = b' ';
                let mark = if Some(i) == netman::last() { b'*' } else { b' ' };
                line[2] = b'['; line[3] = mark; line[4] = b']'; line[5] = b' ';
                let mut li = 6;
                let sn = p.ssid_slice();
                let cn = sn.len().min(24);
                line[li..li + cn].copy_from_slice(&sn[..cn]);
                li += cn;
                let tag: &[u8] = if p.secured() { b" [WPA]" } else { b" [open]" };
                let tl = tag.len().min(50 - li - 1);
                line[li..li + tl].copy_from_slice(&tag[..tl]);
                li += tl;
                let fg = if p.secured() { 0x9E9E9E } else { 0xFFFFFF };
                dlg_line(bx, by, bw, 5 + i as i32, &line[..li], fg);
            }
        }
        dlg_line(bx, by, bw, bh - 2, status, 0xE95420);
        present();
        // ввод
        let c = loop {
            if let Some(c) = crate::driver::ps2::poll_char() { break c; }
            if let Some(c) = crate::driver::uart::poll_char() { break c; }
            timer::usleep(10000);
        };
        if c == b'q' || c == b'Q' || c == 27 { break; }
        if c >= b'0' && (c - b'0') as usize >= n {
            status = b"no such network";
            continue;
        }
        if c >= b'0' && c <= b'7' {
            let idx = (c - b'0') as usize;
            status = b"connecting (DHCP)...";
            dlg_line(bx, by, bw, bh - 2, status, 0xE95420);
            present();
            status = if netman::connect(idx) { b"connected" } else { b"connect failed" };
        }
    }
    reset_term();
}

fn dlg_line(bx: i32, by: i32, bw: i32, row: i32, s: &[u8], fg: u32) {
    let mut cx = bx + 2;
    let max = bx + bw - 2;
    for &ch in s {
        if cx >= max { break; }
        render_char_at(cx as u32, (by + row) as u32, ch);
        let _ = fg;
        cx += 1;
    }
}

fn dlg_adapters(buf: &mut [u8; 48], eth: u32, wifi: u32) -> usize {
    let t = b"adapters: eth=";
    buf[..t.len()].copy_from_slice(t);
    let mut l = t.len();
    l += dlg_num(&mut buf[l..], eth);
    let t2 = b" wifi=";
    buf[l..l + t2.len()].copy_from_slice(t2);
    l += t2.len();
    l += dlg_num(&mut buf[l..], wifi);
    l
}

fn dlg_num(buf: &mut [u8], mut v: u32) -> usize {
    if v == 0 { buf[0] = b'0'; return 1; }
    let mut tmp = [0u8; 10];
    let mut n = 0;
    while v > 0 { tmp[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    for i in 0..n { buf[i] = tmp[n - 1 - i]; }
    n
}

fn cmd_connect(arg: &[u8]) {
    // connect | list | test | auto | add SSID [PSK] | forget SSID | SSID
    let mut p = 0;
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let rest = &arg[p..];
    let mut e = 0;
    while e < rest.len() && rest[e] != b' ' && rest[e] != 0 { e += 1; }
    let sub = &rest[..e];
    if sub.is_empty() {
        cmd_connect_dialog();
        w("back to shell\r\n");
        return;
    }
    if sub == b"list" {
        let n = crate::netman::count();
        if n == 0 { w("no remembered networks\r\n"); return; }
        for i in 0..n {
            if let Some(pr) = crate::netman::get(i) {
                w("  "); dec(i as u64); w(" ");
                w(core::str::from_utf8(pr.ssid_slice()).unwrap_or("?"));
                w(if pr.secured() { " [WPA]\r\n" } else { " [open]\r\n" });
            }
        }
        return;
    }
    if sub == b"test" {
        // KAT криптографии: SHA-256 / AES / HMAC / PBKDF2
        w("crypto self-test...\r\n");
        let err = crate::crypto::self_test();
        if err == 0 {
            w("crypto: ALL PASS (sha/aes/hmac/pbkdf2)\r\n");
        } else {
            w("crypto: FAIL bits=0x"); hex(err as u64); w(" (1=sha 2=aesE 4=aesD 8=cbc 16=hmac 32=pbkdf2)\r\n");
        }
        return;
    }
    if sub == b"auto" {
        let idx = match crate::netman::last() {
            Some(i) => i,
            None => {
                if crate::netman::count() == 0 { w("no networks\r\n"); return; }
                0
            }
        };
        w(if crate::netman::connect(idx) { "connected\r\n" } else { "connect failed\r\n" });
        return;
    }
    if sub == b"add" {
        let mut q = e;
        while q < rest.len() && rest[q] == b' ' { q += 1; }
        let mut r = q;
        while r < rest.len() && rest[r] != b' ' && rest[r] != 0 { r += 1; }
        if r == q { w("Usage: connect add SSID [PSK]\r\n"); return; }
        let ssid = &rest[q..r];
        let mut s2 = r;
        while s2 < rest.len() && rest[s2] == b' ' { s2 += 1; }
        let mut t = s2;
        while t < rest.len() && rest[t] != b' ' && rest[t] != 0 { t += 1; }
        let psk = &rest[s2..t];
        if crate::netman::add(ssid, psk) {
            w(if psk.is_empty() { "remembered (open)\r\n" } else { "remembered (PSK encrypted)\r\n" });
        } else {
            w("add failed (bad name or full)\r\n");
        }
        return;
    }
    if sub == b"forget" {
        let mut q = e;
        while q < rest.len() && rest[q] == b' ' { q += 1; }
        let mut r = q;
        while r < rest.len() && rest[r] != b' ' && rest[r] != 0 { r += 1; }
        if r == q { w("Usage: connect forget SSID\r\n"); return; }
        w(if crate::netman::forget(&rest[q..r]) { "forgotten\r\n" } else { "not found\r\n" });
        return;
    }
    // connect SSID — прямое подключение по имени
    let mut idx = None;
    for i in 0..crate::netman::count() {
        if let Some(pr) = crate::netman::get(i) {
            if pr.ssid_slice() == sub { idx = Some(i); break; }
        }
    }
    match idx {
        Some(i) => w(if crate::netman::connect(i) { "connected\r\n" } else { "connect failed\r\n" }),
        None => w("unknown network (see connect list)\r\n"),
    }
}

fn cmd_usb() {
    crate::driver::usb::probe();
    w("(see UART for details)\r\n");
}

fn cmd_disk(arg: &[u8]) {
    // disk | disk list | disk use nvme|ahci|ide
    let mut q=0;
    while q<arg.len() && arg[q]==b' ' { q+=1; }
    let rest=&arg[q..];
    let mut e=0;
    while e<rest.len() && rest[e]!=b' ' && rest[e]!=0 { e+=1; }
    let sub=&rest[..e];
    if sub.is_empty() || sub==b"list" {
        crate::block::list();
        cmd_ide();
        return;
    }
    if sub==b"use" {
        let mut r=e;
        while r<rest.len() && rest[r]==b' ' { r+=1; }
        let mut s=r;
        while s<rest.len() && rest[s]!=b' ' && rest[s]!=0 { s+=1; }
        let name=&rest[r..s];
        let b=if name==b"nvme" { crate::block::Backend::Nvme }
            else if name==b"ahci"||name==b"sata" { crate::block::Backend::Ahci }
            else if name==b"ide"||name==b"pata" { crate::block::Backend::Ide }
            else { w("Usage: disk use nvme|ahci|ide\r\n"); return; };
        if crate::block::set_backend(b) { w("disk backend switched\r\n"); crate::block::list(); }
        else { w("backend not ready\r\n"); }
        return;
    }
    w("Usage: disk [list|use nvme|ahci|ide]\r\n");
}

fn cmd_ide() {
    w("IDE PATA/ATAPI:\r\n");
    for (i, d) in crate::driver::ide::devices().iter().enumerate() {
        if !d.present {
            continue;
        }
        w("  ide");
        dec((d.channel * 2 + d.drive) as u64);
        w(": ");
        w(if d.atapi { "ATAPI CD " } else { "ATA disk " });
        let mut name = [0u8; 40];
        let mut n = 0;
        while n < 40 && d.model[n] != 0 {
            name[n] = d.model[n];
            n += 1;
        }
        // trim trailing spaces
        while n > 0 && name[n - 1] == b' ' {
            n -= 1;
        }
        w(core::str::from_utf8(&name[..n]).unwrap_or("?"));
        if !d.atapi {
            w(" LBA28=");
            dec(d.lba28 as u64);
        }
        w("\r\n");
        let _ = i;
    }
}

fn cmd_audio() {
    crate::driver::audio::probe();
    w("(see UART for details)\r\n");
}

fn cmd_net_status() {    w("active: "); w(crate::driver::net::active_nic_name());
    w(" mac=");
    let m = crate::driver::net::mac();
    for i in 0..6 {
        let h = b"0123456789ABCDEF";
        let mut tmp = [0u8; 2];
        tmp[0] = h[(m[i] >> 4) as usize];
        tmp[1] = h[(m[i] & 0xF) as usize];
        w(core::str::from_utf8(&tmp).unwrap_or("??"));
        if i < 5 { w(":"); }
    }
    w(" ip=");
    let ip = crate::driver::net::our_ip();
    dec(ip[0] as u64); w("."); dec(ip[1] as u64); w(".");
    dec(ip[2] as u64); w("."); dec(ip[3] as u64); w("\r\n");
    w("e1000: ");
    w(if crate::driver::net::mac() != [0; 6] && crate::driver::net::active_nic() == crate::driver::net::NIC_E1000 { "up" } else if crate::driver::net::mac() != [0; 6] { "present" } else { "absent" });
    w(" rtl8139: ");
    w(if crate::driver::rtl8139::is_ready() { "ready" } else { "absent" });
    w("\r\n");
}

fn cmd_net(arg: &[u8]) {
    // net | net status | net use e1000|rtl8139
    let mut p = 0;
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let rest = &arg[p..];
    let mut e = 0;
    while e < rest.len() && rest[e] != b' ' && rest[e] != 0 { e += 1; }
    let sub = &rest[..e];
    if sub.is_empty() || sub == b"status" {
        cmd_net_status();
        return;
    }
    if sub == b"use" {
        let mut q = e;
        while q < rest.len() && rest[q] == b' ' { q += 1; }
        let mut r = q;
        while r < rest.len() && rest[r] != b' ' && rest[r] != 0 { r += 1; }
        let name = &rest[q..r];
        let nic = if name == b"e1000" {
            crate::driver::net::NIC_E1000
        } else if name == b"rtl8139" || name == b"rtl" {
            crate::driver::net::NIC_RTL8139
        } else {
            w("Usage: net use e1000|rtl8139\r\n");
            return;
        };
        if !crate::driver::net::set_active_nic(nic) {
            w("rtl8139 not present\r\n");
            return;
        }
        if nic == crate::driver::net::NIC_RTL8139 {
            crate::driver::net::set_mac(crate::driver::rtl8139::mac());
            w("switched to rtl8139 (MAC updated, run dhcp)\r\n");
        } else {
            w("switched to e1000 (reboot restores its MAC; run dhcp)\r\n");
        }
        return;
    }
    w("Usage: net [status|use e1000|rtl8139]\r\n");
}

fn cmd_wifi(arg: &[u8]) {
    // wifi | wifi scan | wifi status | wifi init | wifi connect SSID | wifi pmk SSID PSK
    let mut p = 0;
    while p < arg.len() && arg[p] == b' ' { p += 1; }
    let rest = &arg[p..];
    let mut e = 0;
    while e < rest.len() && rest[e] != b' ' && rest[e] != 0 { e += 1; }
    let sub = &rest[..e];
    if sub.is_empty() || sub == b"status" {
        let n = crate::driver::wifi::scan();
        w("radios: "); dec(n as u64);
        w(" state: "); w(crate::driver::wifi::state_name()); w("\r\n");
        if crate::driver::wifi::ath9k::is_initialized() {
            w("ath9k: initialized ch="); dec(crate::driver::wifi::ath9k::current_channel() as u64); w("\r\n");
        } else {
            w("ath9k: not initialized (use `wifi init`)\r\n");
        }
        return;
    }
    if sub == b"init" {
        if crate::driver::wifi::init() {
            w("WiFi hardware initialized\r\n");
        } else {
            w("WiFi init failed (no hardware?)\r\n");
        }
        return;
    }
    if sub == b"scan" {
        w("scanning...\r\n");
        let n = crate::driver::wifi::scan_wifi();
        w("found "); dec(n as u64); w(" networks:\r\n");
        for i in 0..crate::driver::wifi::scan_result_count() {
            if let Some(r) = crate::driver::wifi::get_scan_result(i) {
                w("  ");
                let ssid = r.ssid_str();
                w(ssid);
                // pad to 20 chars
                for _ in ssid.len()..20 { w(" "); }
                w(" ch="); dec(r.channel as u64);
                w(" rssi="); dec(r.rssi as i8 as u64);
                if r.wpa2_psk { w(" [WPA2]"); }
                else if r.has_rsn { w(" [RSN]"); }
                else { w(" [OPEN]"); }
                w(" bssid=");
                for j in 0..6 {
                    let h = b"0123456789ABCDEF";
                    let b = r.bssid[j];
                    let mut tmp = [0u8; 2];
                    tmp[0] = h[(b >> 4) as usize];
                    tmp[1] = h[(b & 0xF) as usize];
                    w(core::str::from_utf8(&tmp).unwrap_or("??"));
                    if j < 5 { w(":"); }
                }
                w("\r\n");
            }
        }
        return;
    }
    if sub == b"connect" || sub == b"join" {
        let mut q = e;
        while q < rest.len() && rest[q] == b' ' { q += 1; }
        let mut r = q;
        while r < rest.len() && rest[r] != b' ' && rest[r] != 0 { r += 1; }
        if r == q { w("Usage: wifi connect SSID [PASSWORD]\r\n"); return; }
        let ssid = &rest[q..r];
        // Optional password
        while r < rest.len() && rest[r] == b' ' { r += 1; }
        let pw_start = r;
        while r < rest.len() && rest[r] != b' ' && rest[r] != 0 { r += 1; }
        let pw = &rest[pw_start..r];

        // If password provided, add profile first
        if !pw.is_empty() {
            crate::netman::add(ssid, pw);
            w("profile saved\r\n");
        }

        w("connecting to '");
        for &c in ssid { crate::shell::w(core::str::from_utf8(&[c]).unwrap_or("?")); }
        w("'\r\n");
        let result = crate::driver::wifi::associate(ssid);
        w("result: "); w(result); w("\r\n");
        return;
    }
    if sub == b"disconnect" {
        crate::driver::wifi::disconnect();
        return;
    }
    if sub == b"pmk" {
        // wifi pmk SSID PSK — вывод PMK для WPA2-PSK (проверка криптотракта)
        let mut q = e;
        while q < rest.len() && rest[q] == b' ' { q += 1; }
        let mut r = q;
        while r < rest.len() && rest[r] != b' ' && rest[r] != 0 { r += 1; }
        let mut s2 = r;
        while s2 < rest.len() && rest[s2] == b' ' { s2 += 1; }
        let mut t = s2;
        while t < rest.len() && rest[t] != b' ' && rest[t] != 0 { t += 1; }
        if r == q || t == s2 {
            w("Usage: wifi pmk SSID PSK(passphrase 8..63)\r\n");
            return;
        }
        let mut pmk = [0u8; 32];
        if !crate::driver::wifi::pmk(&rest[q..r], &rest[s2..t], &mut pmk) {
            w("bad SSID/passphrase length\r\n");
            return;
        }
        w("PMK: ");
        for b in pmk {
            let h = b"0123456789ABCDEF";
            let mut tmp = [0u8; 2];
            tmp[0] = h[(b >> 4) as usize];
            tmp[1] = h[(b & 0xF) as usize];
            w(core::str::from_utf8(&tmp).unwrap_or("??"));
        }
        w("\r\n");
        for b in pmk.iter_mut() { *b = 0; }
        return;
    }
    w("Usage: wifi [status|init|scan|connect SSID [PASS]|disconnect|pmk SSID PSK]\r\n");
}

fn cmd_nvme(buf: &[u8]) {    let mut p = 0;
    while p < buf.len() && buf[p] != b' ' && buf[p] != 0 { p += 1; }
    while p < buf.len() && buf[p] == b' ' { p += 1; }
    let sub = if p < buf.len() { buf[p] } else { 0 };
    if sub == b'i' {
        if unsafe { nvme::INIT } {
            w("NVMe: LBAs="); dec(unsafe { nvme::NS_LBA_COUNT }); w("\r\n");
        } else { w("NVMe: not present\r\n"); }
    } else { w("nvme: info|read|write\r\n"); }
}

// ── Main ──────────────────────────────────────────────────────────

pub fn run() {
    cwd_init();
    display::clear_screen(0, 0, 0);
    term_init();
    present();

    loop {
        tcp::pump();
        w(crate::user::current_name());
        w("@DBSos$> ");

        let mut buf = [0u8; 128];
        let len = readline(&mut buf);
        if len == 0 || buf[0] == 0 { continue; }

        let cmd_end = buf[..len].iter().position(|&c| c == b' ').unwrap_or(len);
        let cmd = &buf[..cmd_end];
        let arg_start = if cmd_end < len { cmd_end + 1 } else { cmd_end };
        let arg = &buf[arg_start..len];

        if cmd == b"help" { cmd_help(); }
        else if cmd == b"mem" { cmd_mem(); }
        else if cmd == b"time" { cmd_time(); }
        else if cmd == b"date" { cmd_date(); }
        else if cmd == b"clear" { display::clear_screen(0, 0, 0); term_init(); }
        else if cmd == b"info" { cmd_info(); }
        else if cmd == b"pwd" { cmd_pwd(); }
        else if cmd == b"cd" { cmd_cd(arg); }
        else if cmd == b"echo" { cmd_echo(arg); }
        else if cmd == b"ls" { if arg.len() > 0 { fs_server::ls(arg); } else { fs_server::ls(b"/"); } }
        else if cmd == b"cat" { if arg.len() > 0 { fs_server::cat(arg); } else { w("Usage: cat PATH\r\n"); } }
        else if cmd == b"mkdir" { if arg.len() > 0 { fs_server::mkdir(arg); } else { w("Usage: mkdir PATH\r\n"); } }
        else if cmd == b"rm" { if arg.len() > 0 { fs_server::rm(arg); } else { w("Usage: rm PATH\r\n"); } }
        else if cmd == b"rmdir" { if arg.len() > 0 { fs_server::rmdir(arg); } else { w("Usage: rmdir PATH\r\n"); } }
        else if cmd == b"write" {
            if arg.len() > 0 {
                if let Some(s) = arg.iter().position(|&c| c == b' ') {
                    let path = &arg[..s];
                    let content = &arg[s + 1..];
                    let cl = content.iter().position(|&c| c == 0).unwrap_or(content.len());
                    fs_server::write(path, &content[..cl]);
                } else { w("Usage: write PATH TEXT\r\n"); }
            } else { w("Usage: write PATH TEXT\r\n"); }
        }
        else if cmd == b"exec" {
            if arg.len() > 0 {
                // Сначала native DBSos ELF, при неудаче — Linux ELF fallback
                let id = crate::elf::load_and_spawn(arg);
                if id == 0 {
                    let pid = crate::linux::spawn_file(arg);
                    if pid == 0 { w("exec: not a DBSos nor Linux ELF\r\n"); }
                    else { w("linux pid: "); dec(pid); w("\r\n"); }
                }
            } else { w("Usage: exec PATH\r\n"); }
        }
        else if cmd == b"lexec" {
            if arg.len() > 0 {
                let pid = crate::linux::spawn_file(arg);
                if pid == 0 { w("lexec failed\r\n"); }
                else { w("linux pid: "); dec(pid); w("\r\n"); }
            } else { w("Usage: lexec PATH\r\n"); }
        }
        else if cmd == b"fm" {
            // Файловый менеджер: fm [PATH], без пути — корень
            let mut p = 0;
            while p < arg.len() && arg[p] == b' ' { p += 1; }
            let mut e = p;
            while e < arg.len() && arg[e] != b' ' && arg[e] != 0 { e += 1; }
            if e == p {
                crate::fm::run(b"/");
            } else {
                crate::fm::run(&arg[p..e]);
            }
            reset_term();
            w("back to shell\r\n");
        }
        else if cmd == b"ping" { cmd_ping(); }
        else if cmd == b"dhcp" { cmd_dhcp(); }
        else if cmd == b"dns" { cmd_dns(arg); }
        else if cmd == b"tcp" { cmd_tcp(arg); }
        else if cmd == b"net" { cmd_net(arg); }
        else if cmd == b"wifi" { cmd_wifi(arg); }
        else if cmd == b"usb" { cmd_usb(); }
        else if cmd == b"audio" { cmd_audio(); }
        else if cmd == b"ide" { cmd_ide(); }
        else if cmd == b"disk" { cmd_disk(arg); }
        else if cmd == b"connect" { cmd_connect(arg); }
        else if cmd == b"selftest" {
            let (ok, total) = crate::rs_kernel_test::run();
            w("selftest: "); dec(ok as u64); w("/"); dec(total as u64);
            w(if ok == total { " ALL OK\r\n" } else { " FAILURES\r\n" });
        }
        else if cmd == b"reboot" { w("reboot\r\n"); crate::acpi::reboot(); }
        else if cmd == b"poweroff" { w("shutdown\r\n"); crate::acpi::shutdown(); }
        else if cmd == b"pkg" { crate::pkg::pkg_main(arg); }
        else if cmd == b"script" { if arg.len() > 0 { let c = crate::script::run_script(arg); w("exit: "); dec(c as u64); w("\r\n"); } else { w("Usage: script PATH\r\n"); } }
        else if cmd == b"nvme" { cmd_nvme(&buf); }
        else if cmd == b"hw" { cmd_hw(); }
        else if cmd == b"cpu" { cmd_cpu(); }
        else if cmd == b"top" { cmd_top(); }
        else if cmd == b"level" { cmd_level(arg); }
        else if cmd == b"driver" { cmd_driver(arg); }
        else if cmd == b"whoami" { cmd_whoami(); }
        else if cmd == b"users" { cmd_users(); }
        else if cmd == b"useradd" { cmd_useradd(arg); }
        else if cmd == b"login" { cmd_login(arg); }
        else if cmd == b"id" { cmd_id(); }
        else if cmd == b"chmod" { cmd_chmod(arg); }
        else if cmd == b"chown" { cmd_chown(arg); }
        else if cmd == b"su" { cmd_su(arg); }
        else if cmd == b"passwd" { cmd_passwd(arg); }
        else if cmd == b"gpu" { cmd_gpu(); }
        else if cmd == b"display" || cmd == b"screen" { cmd_display(); }
        else if cmd == b"plasma" || cmd == b"startplasma" || cmd == b"kwin" {
            w("Starting Plasma...\r\n");
            present();
            crate::plasma::run();
            reset_term();
            w("Plasma exited — back to TUI\r\n");
        }
        else if cmd == b"wldemo" {
            w("Wayland demo: creating Ubuntu orange surface...\r\n");
            let pool = crate::wayland::shm::pool_create(320*200*4);
            if let Some(pidx) = pool {
                if let Some(phys) = crate::wayland::shm::pool_phys(pidx) {
                    // fill with Ubuntu orange + white border + text pattern
                    unsafe{
                        let ptr = phys as *mut u8;
                        for y in 0..200 { for x in 0..320 {
                            let off=(y*320+x)*4;
                            let is_border = x<2||x>=318||y<2||y>=198;
                            let r=if is_border{0xFF}else{0xE9};
                            let g=if is_border{0xFF}else{0x54};
                            let b=if is_border{0xFF}else{0x20};
                            *ptr.add(off)=b; *ptr.add(off+1)=g; *ptr.add(off+2)=r; *ptr.add(off+3)=0;
                        }}
                        // simple "WL" text in center
                        for dx in 0..16 { for dy in 0..8 { *ptr.add(((100+dy)*320+150+dx)*4)=0xFF; } }
                    }
                    if let Some(sid)=crate::wayland::compositor::surface_create(64+200, 28+100, 320, 200) {
                        // need DOCK_W/TOPBAR_H - use constants via plasma? hardcode 64/28
                        let _ = crate::wayland::compositor::surface_attach(sid, pidx, 0, 320,200,320*4);
                        let _ = crate::wayland::compositor::surface_commit(sid);
                        w("surface "); dec(sid as u64); w(" created, pool "); dec(pidx as u64); w("\r\n");
                        w("Check Plasma — orange Wayland window should appear on top\r\n");
                    } else { w("surface create failed\r\n"); }
                } else { w("pool phys failed\r\n"); }
            } else { w("shm pool create failed\r\n"); }
        }
        else if cmd == b"wlinfo" {
            crate::wayland::debug_surfaces();
            w("Wayland globals (registry):\r\n");
            for g in crate::wayland::protocol::GLOBALS {
                w("  id="); dec(g.id as u64);
                w(" "); w(g.interface);
                w("@v"); dec(g.version as u64); w("\r\n");
            }
        }
        else if cmd == b"wget" {
            if arg.len() > 0 {
                if let Some(s) = arg.iter().position(|&c| c == b' ') {
                    let url = &arg[..s]; let file = &arg[s+1..];
                    let fl = file.iter().position(|&c| c == 0).unwrap_or(file.len());
                    crate::wget::wget(url, &file[..fl]);
                } else { crate::wget::wget(arg, b""); }
            } else { w("Usage: wget URL [FILE]\r\n"); }
        }
        else { w("unknown: "); w(core::str::from_utf8(cmd).unwrap_or("?")); w("\r\n"); }

        present();
    }
}

// ── Public API for script module ──────────────────────────────────

pub fn print_str_pub(s: &[u8]) { w(core::str::from_utf8(s).unwrap_or("<bin>")); }
pub fn cmd_help_pub() { cmd_help(); }
pub fn cmd_mem_pub() { cmd_mem(); }
pub fn cmd_info_pub() { cmd_info(); }
pub fn cmd_time_pub() { cmd_time(); }
pub fn cmd_ping_pub() { cmd_ping(); }
pub fn cmd_dhcp_pub() { cmd_dhcp(); }
pub fn cmd_connect_pub(arg: &[u8]) { cmd_connect(arg); }
pub fn cmd_date_pub() { cmd_date(); }
pub fn cmd_cpu_pub() { cmd_cpu(); }
pub fn cmd_net_pub(arg: &[u8]) { cmd_net(arg); }
pub fn cmd_wifi_pub(arg: &[u8]) { cmd_wifi(arg); }
pub fn cmd_usb_pub() { cmd_usb(); }
pub fn cmd_audio_pub() { cmd_audio(); }
pub fn cmd_ide_pub() { cmd_ide(); }

/// Единая таблица команд shell — selftest считает покрытие по ней.
pub const COMMANDS: &[&str] = &[
    "help", "mem", "time", "date", "clear", "info", "pwd", "cd", "echo", "ls",
    "cat", "mkdir", "rm", "rmdir", "write", "exec", "lexec", "fm", "ping", "dhcp",
    "dns", "tcp", "connect", "reboot", "poweroff", "pkg", "script", "nvme", "hw", "cpu",
    "top", "level", "driver", "whoami", "users", "useradd", "login", "id", "chmod", "chown",
    "su", "passwd", "gpu", "display", "screen", "plasma", "startplasma", "kwin", "wldemo", "wlinfo",
    "wget", "selftest", "net", "wifi", "usb", "audio", "ide", "disk",
];

pub fn command_count() -> usize { COMMANDS.len() }
pub fn cd_pub(arg: &[u8]) { cmd_cd(arg); }
pub fn ls_pub(path: &[u8]) { crate::fs_server::ls(path); }
