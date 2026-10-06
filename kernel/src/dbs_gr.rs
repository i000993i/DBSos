//! DBS-GR — Графическая растровая графика DBSos
//! Собственная консоль: чистый чёрный фон, prompt красный, ввод белый, адаптив к размеру окна QEMU
//! TUI без заимствований у Linux — полностью свой рендер.
#![allow(unused_assignments, unused_variables, dead_code, unused_unsafe)]

use crate::display;
use crate::driver::{ps2, uart};
use crate::timer;
use crate::font::FONT_8X16;

const CHAR_W: u32 = 8;
const CHAR_H: u32 = 16;

static mut TERM_COL: u32 = 0;
static mut TERM_ROW: u32 = 0;
static mut TERM_COLS: u32 = 0;
static mut TERM_ROWS: u32 = 0;

// History 16 x 128
static mut HIST: [[u8;128];16] = [[0;128];16];
static mut HIST_LEN: [u8;16] = [0;16];
static mut HIST_CNT: usize = 0;
static mut HIST_NAV: usize = 0;

fn sw() -> u32 { display::width() }
fn sh() -> u32 { display::height() }
fn stride() -> u32 { display::stride() }
fn fb() -> *mut u8 { display::framebuffer() }

fn gradient_color(_y: u32, _h: u32, _t: u32) -> (u8,u8,u8) { (0,0,0) }

pub fn draw_gradient(_t: u32) {    // Чистый чёрный фон — без градиента, адаптивен к любому размеру окна QEMU
    let w = sw(); let h = sh();
    let f = fb(); let s = stride();
    if f.is_null() { return; }
    let is_bgr = display::is_bgr();
    for y in 0..h {
        let row_off = (y * s) as usize * 4;
        for x in 0..w {
            let off = row_off + x as usize * 4;
            unsafe {
                if is_bgr { *f.add(off)=0; *f.add(off+1)=0; *f.add(off+2)=0; }
                else { *f.add(off)=0; *f.add(off+1)=0; *f.add(off+2)=0; }
                *f.add(off+3)=0;
            }
        }
    }
    // Обновляем размеры терминала при изменении окна QEMU
    unsafe{
        let new_cols = w / CHAR_W;
        let new_rows = h / CHAR_H;
        if new_cols != TERM_COLS || new_rows != TERM_ROWS {
            TERM_COLS = new_cols;
            TERM_ROWS = new_rows;
            if TERM_COL >= TERM_COLS { TERM_COL = TERM_COLS-1; }
            if TERM_ROW >= TERM_ROWS { TERM_ROW = TERM_ROWS-1; }
        }
    }
    // Фоновый логотип DBS по центру (восстанавливается при каждой очистке)
    draw_logo();
}

// ── Фоновый логотип ────────────────────────────────────────────────
// Большие буквы DBS по центру терминала, тёмно-белые. Глифы 8x16 из
// системного шрифта масштабируются блоком (без сглаживания, в духе TUI).
const LOGO_SCALE: u32 = 8;
const LOGO_COLOR: u32 = 0x2E2E2E; // тёмно-белый на чёрном

fn draw_glyph_scaled(x0: u32, y0: u32, ch: u8, scale: u32, color: u32) {
    let idx = ch as usize * 16;
    if idx + 16 > FONT_8X16.len() || scale == 0 { return; }
    let glyph = &FONT_8X16[idx..idx + 16];
    let f = fb();
    if f.is_null() { return; }
    let s = stride() as i32;
    let w = sw();
    let h = sh();
    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;
    let is_bgr = display::is_bgr();
    for row in 0..16 {
        let bits = glyph[row];
        for c in 0..8 {
            if bits & (0x80 >> c) == 0 { continue; }
            for dy in 0..scale {
                let y = y0 + row as u32 * scale + dy;
                if y >= h { break; }
                for dx in 0..scale {
                    let x = x0 + c as u32 * scale + dx;
                    if x >= w { break; }
                    let off = (y as i32 * s + x as i32) as usize * 4;
                    unsafe {
                        if is_bgr { *f.add(off) = b; *f.add(off + 1) = g; *f.add(off + 2) = r; }
                        else { *f.add(off) = r; *f.add(off + 1) = g; *f.add(off + 2) = b; }
                        *f.add(off + 3) = 0;
                    }
                }
            }
        }
    }
}

/// Логотип DBS по центру экрана. Публичная — для перерисовки из команд.
pub fn draw_logo() {
    let text = b"DBS";
    let gw = 8 * LOGO_SCALE;
    let gh = 16 * LOGO_SCALE;
    let gap = LOGO_SCALE * 2;
    let total_w = gw * text.len() as u32 + gap * (text.len() as u32 - 1);
    let x0 = sw().saturating_sub(total_w) / 2;
    let y0 = sh().saturating_sub(gh) / 2;
    for (i, &ch) in text.iter().enumerate() {
        draw_glyph_scaled(x0 + i as u32 * (gw + gap), y0, ch, LOGO_SCALE, LOGO_COLOR);
    }
}

fn draw_char_at(col: u32, row: u32, ch: u8, color: u32) {
    let px = col * CHAR_W;
    let py = row * CHAR_H;
    if px >= sw() || py >= sh() { return; }
    let idx = ch as usize * 16;
    if idx+16 > FONT_8X16.len() { return; }
    let glyph = &FONT_8X16[idx..idx+16];
    let f = fb(); let s = stride() as i32;
    let r = ((color>>16)&0xFF) as u8;
    let g = ((color>>8)&0xFF) as u8;
    let b = (color &0xFF) as u8;
    for row in 0..16 {
        let bits = glyph[row];
        for c in 0..8 {
            if bits & (0x80>>c)==0 { continue; }
            let x = px + c;
            let y = py + row as u32;
            if x>=sw()||y>=sh(){ continue; }
            let off=(y as i32 * s + x as i32) as usize*4;
            let is_bgr = display::is_bgr();
            unsafe {
                if is_bgr { *f.add(off)=b; *f.add(off+1)=g; *f.add(off+2)=r; }
                else { *f.add(off)=r; *f.add(off+1)=g; *f.add(off+2)=b; }
                *f.add(off+3)=0;
            }
        }
    }
}

fn clear_line(row: u32) {
    let y = row * CHAR_H;
    let t = (timer::millis()/40 % 255) as u32;
    let h = sh();
    for py in y..(y+CHAR_H).min(h) {
        let (r,g,b)=gradient_color(py, h, t);
        let f=fb(); let s=stride();
        let row_off=(py*s) as usize*4;
        let is_bgr=display::is_bgr();
        for x in 0..sw() {
            let off=row_off + x as usize*4;
            unsafe{
                if is_bgr{*f.add(off)=b;*f.add(off+1)=g;*f.add(off+2)=r;}
                else{*f.add(off)=r;*f.add(off+1)=g;*f.add(off+2)=b;}
                *f.add(off+3)=0;
            }
        }
    }
}

fn scroll_up() {
    let f=fb(); let s=stride() as usize; let h=sh() as usize;
    if f.is_null() || s == 0 || h <= CHAR_H as usize { return; }
    let line_bytes = (CHAR_H as usize)*s*4;
    let total_rows = h - CHAR_H as usize;
    unsafe{
        // memmove: области перекрываются (сдвиг вверх), copy_nonoverlapping здесь UB
        // (падало в debug-проверках nightly при первом скролле TUI).
        core::ptr::copy(f.add(line_bytes), f, total_rows * s *4);
    }
    let row = unsafe{ TERM_ROWS -1 };
    clear_line(row);
}

pub fn init() {
    crate::driver::uart::write_str("[DBS-GR] init gradient console\r\n");
    let w=sw(); let h=sh();
    unsafe{
        TERM_COLS = w / CHAR_W;
        TERM_ROWS = h / CHAR_H;
        TERM_COL=0; TERM_ROW=0;
    }
    draw_gradient(0);
    unsafe{ display::mark_dirty(0,0,w,h); display::present(); }
}

pub static mut GR_ACTIVE: bool = false;

const COLOR_PROMPT: u32 = 0xFF3B30;
const COLOR_TEXT: u32 = 0xFFFFFF;
const COLOR_DIM: u32 = 0xAEA79F;
const COLOR_OK: u32 = 0x4CD964;
const COLOR_ERR: u32 = 0xFF3B30;
const COLOR_WARN: u32 = 0xFFCC02;
const COLOR_INFO: u32 = 0x5AC8FA;

fn put_char(ch: u8, color: u32) {
    unsafe{
        if ch==b'\n' {
            TERM_COL=0; TERM_ROW+=1;
            if TERM_ROW>=TERM_ROWS { scroll_up(); TERM_ROW=TERM_ROWS-1; }
            return;
        }
        if ch==b'\r' { TERM_COL=0; return; }
        if ch==0x08 {
            if TERM_COL>0 {
                TERM_COL-=1;
                let px=TERM_COL*CHAR_W; let py=TERM_ROW*CHAR_H;
                let f=fb(); let s=stride() as i32;
                let t=(timer::millis()/40 %255) as u32;
                let (r,g,b)=gradient_color(py, sh(), t);
                let is_bgr=display::is_bgr();
                for dy in 0..CHAR_H {
                    for dx in 0..CHAR_W {
                        let x=px+dx; let y=py+dy;
                        if x>=sw()||y>=sh(){continue;}
                        let off=(y as i32*s + x as i32) as usize*4;
                        if is_bgr{*f.add(off)=b;*f.add(off+1)=g;*f.add(off+2)=r;} else{*f.add(off)=r;*f.add(off+1)=g;*f.add(off+2)=b;}
                    }
                }
            }
            return;
        }
        if ch < 32 { return; }
        draw_char_at(TERM_COL, TERM_ROW, ch, color);
        TERM_COL+=1;
        if TERM_COL>=TERM_COLS {
            TERM_COL=0; TERM_ROW+=1;
            if TERM_ROW>=TERM_ROWS { scroll_up(); TERM_ROW=TERM_ROWS-1; }
        }
    }
}
pub fn print(s: &[u8], col: u32) { for &c in s { put_char(c, col); } }
fn print_str(s: &str, col: u32) { print(s.as_bytes(), col); }

/// Shell-compatible output — prints with COLOR_TEXT + UART, like shell::w()
pub fn w(s: &str) {
    crate::driver::uart::write_str(s);
    for ch in s.bytes() { put_char(ch, COLOR_TEXT); }
}
pub fn dec(mut v: u64) {
    if v==0 { put_char(b'0', COLOR_TEXT); return; }
    let mut b=[0u8;20]; let mut i=0;
    while v>0 { b[i]=b'0'+(v%10) as u8; v/=10; i+=1; }
    while i>0 { i-=1; put_char(b[i], COLOR_TEXT); }
}
pub fn hex(mut v: u64) {
    if v==0 { put_char(b'0', COLOR_TEXT); return; }
    let mut b=[0u8;16]; let mut i=0;
    while v>0 { let n=(v&0xF) as u8; b[i]=if n<10{b'0'+n}else{b'A'+n-10}; v>>=4; i+=1; }
    while i>0 { i-=1; put_char(b[i], COLOR_TEXT); }
}
fn log_info(s: &[u8]) { print(b"[INFO] ", COLOR_INFO); print(s, COLOR_TEXT); print(b"\n", COLOR_TEXT); }
fn log_warn(s: &[u8]) { print(b"[WARN] ", COLOR_WARN); print(s, COLOR_TEXT); print(b"\n", COLOR_TEXT); }
fn log_err(s: &[u8]) { print(b"[ERR]  ", COLOR_ERR); print(s, COLOR_TEXT); print(b"\n", COLOR_TEXT); }

const CMDS: &[&[u8]] = &[
    b"help", b"info", b"ls", b"cat", b"mkdir", b"rm", b"cd", b"pwd", b"echo", b"write",
    b"fm", b"mc", b"edit", b"browser", b"web", b"www", b"scan", b"devices", b"periph", b"hwinfo",
    b"mem", b"cpu", b"hw", b"top", b"time", b"clear", b"reboot", b"poweroff",
    b"ping", b"dhcp", b"dns", b"tcp", b"wget", b"pkg", b"exec", b"script", b"nvme",
    b"whoami", b"users", b"useradd", b"login", b"su", b"passwd", b"chmod", b"chown", b"id",
    b"wldemo",
];

fn hist_push(line: &[u8]) {
    if line.is_empty() { return; }
    unsafe{
        let idx = HIST_CNT % 16;
        let l = line.len().min(127);
        HIST[idx][..l].copy_from_slice(&line[..l]);
        HIST[idx][l]=0;
        HIST_LEN[idx]=l as u8;
        HIST_CNT+=1;
        HIST_NAV=HIST_CNT;
    }
}
fn tab_complete(buf: &mut [u8], len: usize) -> usize {
    if len==0 { return len; }
    let mut ws=len;
    while ws>0 && buf[ws-1]!=b' ' && buf[ws-1]!=b'/' { ws-=1; }
    let prefix_len=len-ws;
    if prefix_len==0 { return len; }
    let mut prefix_buf=[0u8;32];
    let pl=prefix_len.min(32);
    prefix_buf[..pl].copy_from_slice(&buf[ws..ws+pl]);
    let prefix=&prefix_buf[..pl];
    let is_first_word = {
        let mut has=false;
        for &c in &buf[..ws] { if c==b' '{ has=true; break; } }
        !has
    };
    let mut cand_bufs: [[u8;32];32] = [[0;32];32];
    let mut cand_lens=[0usize;32];
    let mut cnt=0;
    if is_first_word {
        for &c in CMDS {
            if c.len()>=prefix.len() && &c[..prefix.len()]==prefix {
                if cnt<32 {
                    let l=c.len().min(32);
                    cand_bufs[cnt][..l].copy_from_slice(&c[..l]);
                    cand_lens[cnt]=l;
                    cnt+=1;
                }
            }
        }
    } else {
        let slash = {
            let mut p=None;
            for i in (0..len).rev(){ if buf[i]==b'/'{ p=Some(i); break; } }
            p
        };
        let mut dir_buf=[0u8;128];
        let mut dir_len=0;
        let mut file_pref_len=0;
        let mut file_pref_buf=[0u8;32];
        if let Some(p)=slash {
            dir_len=(p+1).min(128);
            dir_buf[..dir_len].copy_from_slice(&buf[..dir_len]);
            file_pref_len=(len-p-1).min(32);
            if file_pref_len>0 { file_pref_buf[..file_pref_len].copy_from_slice(&buf[p+1..p+1+file_pref_len]); }
        } else {
            let cwd=crate::scheduler::current_cwd_slice();
            dir_len=cwd.len().min(128);
            if dir_len>0 { dir_buf[..dir_len].copy_from_slice(&cwd[..dir_len]); }
            file_pref_len=pl;
            file_pref_buf[..pl].copy_from_slice(prefix);
        }
        let dir_path = if dir_len==0 { &b"/"[..] } else { &dir_buf[..dir_len] };
        let file_pref = &file_pref_buf[..file_pref_len];
        let mut entries=[crate::vfs::DirEntry{name:[0;32], is_dir:false, size:0}; 32];
        let n=crate::vfs::readdir(dir_path, &mut entries);
        if n>0 {
            for i in 0..(n as usize).min(32) {
                let e=&entries[i];
                let nl=e.name.iter().position(|&c|c==0).unwrap_or(32);
                let name=&e.name[..nl];
                if name.len()>=file_pref.len() && &name[..file_pref.len()]==file_pref {
                    if cnt<32 {
                        let l=name.len().min(32);
                        cand_bufs[cnt][..l].copy_from_slice(&name[..l]);
                        cand_lens[cnt]=l;
                        cnt+=1;
                    }
                }
            }
        }
    }
    if cnt==0 { return len; }
    if cnt==1 {
        let cand_len=cand_lens[0];
        let rest_len=cand_len - prefix.len();
        let add=rest_len.min(127-len);
        buf[len..len+add].copy_from_slice(&cand_bufs[0][prefix.len()..prefix.len()+add]);
        return len+add;
    } else {
        print(b"\n", COLOR_TEXT);
        for i in 0..cnt {
            print(&cand_bufs[i][..cand_lens[i]], COLOR_INFO);
            print(b"  ", COLOR_DIM);
            if (i+1)%6==0 { print(b"\n", COLOR_TEXT); }
        }
        print(b"\n", COLOR_TEXT);
        let mut common=prefix.len();
        'outer: for k in prefix.len()..32 {
            if k>=cand_lens[0] { break; }
            let ch=cand_bufs[0][k];
            for j in 1..cnt{ if k>=cand_lens[j] || cand_bufs[j][k]!=ch{ break 'outer; } }
            common+=1;
        }
        if common>prefix.len() {
            let add=(common-prefix.len()).min(127-len);
            buf[len..len+add].copy_from_slice(&cand_bufs[0][prefix.len()..prefix.len()+add]);
            return len+add;
        }
        return len;
    }
}

fn prompt() {
    // show layout EN/RU like Ubuntu
    let layout = crate::driver::ps2::layout_name();
    // draw small layout indicator at top-right (reuse gradient area)
    // For prompt, include layout: root@DBS[EN]: >
    print(b"root@DBS", COLOR_PROMPT);
    print(b"[", COLOR_DIM); print(layout.as_bytes(), COLOR_OK); print(b"]: > ", COLOR_PROMPT);
}

fn read_line(buf: &mut [u8]) -> usize {
    let mut len=0usize;
    let mut cursor_on=true;
    let mut last_blink=timer::millis();
    let start_col=unsafe{TERM_COL};
    let start_row=unsafe{TERM_ROW};
    unsafe{ HIST_NAV=HIST_CNT; }
    loop{
        let now=timer::millis();
        if now - last_blink > 400 {
            last_blink=now;
            cursor_on=!cursor_on;
            unsafe{
                let px=TERM_COL*CHAR_W; let py=TERM_ROW*CHAR_H + CHAR_H -2;
                let f=fb(); let s=stride() as i32;
                if cursor_on {
                    for dx in 0..CHAR_W {
                        let x=px+dx; let y=py;
                        if x>=sw()||y>=sh(){continue;}
                        let off=(y as i32*s + x as i32) as usize*4;
                        *f.add(off)=0xFF; *f.add(off+1)=0xFF; *f.add(off+2)=0xFF;
                    }
                } else {
                    let t=(now/40%255) as u32;
                    let (r,g,b)=gradient_color(py, sh(), t);
                    let is_bgr=display::is_bgr();
                    for dx in 0..CHAR_W {
                        let x=px+dx; let y=py;
                        if x>=sw()||y>=sh(){continue;}
                        let off=(y as i32*s + x as i32) as usize*4;
                        if is_bgr{*f.add(off)=b;*f.add(off+1)=g;*f.add(off+2)=r;} else{*f.add(off)=r;*f.add(off+1)=g;*f.add(off+2)=b;}
                    }
                }
                display::mark_dirty(px as i32, py as i32, CHAR_W, 2);
                display::present();
            }
        }
        let c = if let Some(ch)=ps2::poll_char(){ Some(ch)} else if let Some(ch)=uart::poll_char(){Some(ch)} else {None};
        if let Some(ch)=c {
            unsafe{
                let px=TERM_COL*CHAR_W; let py=TERM_ROW*CHAR_H + CHAR_H -2;
                let t=(timer::millis()/40%255) as u32;
                let (r,g,b)=gradient_color(py, sh(), t);
                let is_bgr=display::is_bgr();
                for dx in 0..CHAR_W {
                    let x=px+dx; let y=py;
                    let off=(y as i32*stride() as i32 + x as i32) as usize*4;
                    if is_bgr{*fb().add(off)=b;*fb().add(off+1)=g;*fb().add(off+2)=r;} else{*fb().add(off)=r;*fb().add(off+1)=g;*fb().add(off+2)=b;}
                }
                display::mark_dirty(px as i32, py as i32, CHAR_W, 2);
            }
            cursor_on=false;
            match ch {
                b'\r'|b'\n' => {
                    print(b"\n", COLOR_TEXT);
                    display::mark_dirty(0,0,sw(),sh()); unsafe{ display::present(); }
                    if len < buf.len(){ buf[len]=0; }
                    if len>0 { hist_push(&buf[..len]); }
                    return len;
                }
                0x08|0x7F => {
                    if len>0 {
                        len-=1;
                        put_char(0x08, COLOR_TEXT);
                        display::mark_dirty(0,0,sw(),sh()); unsafe{ display::present(); }
                    }
                }
                0x09 => { // Tab
                    let new_len=tab_complete(buf, len);
                    if new_len!=len {
                        clear_line(start_row);
                        unsafe{ TERM_COL=0; TERM_ROW=start_row; }
                        prompt();
                        for i in 0..new_len { put_char(buf[i], COLOR_TEXT); }
                        display::mark_dirty(0,0,sw(),sh()); unsafe{ display::present(); }
                        len=new_len;
                    }
                }
                ps2::KEY_UP => {
                    unsafe{
                        if HIST_NAV>0 && HIST_CNT>0 {
                            HIST_NAV-=1;
                            let idx=HIST_NAV%16;
                            let hlen=HIST_LEN[idx] as usize;
                            len=hlen.min(127);
                            buf[..len].copy_from_slice(&HIST[idx][..len]);
                            clear_line(start_row);
                            TERM_COL=0; TERM_ROW=start_row;
                            prompt();
                            for i in 0..len { put_char(buf[i], COLOR_TEXT); }
                            display::mark_dirty(0,0,sw(),sh()); display::present();
                        }
                    }
                }
                ps2::KEY_DOWN => {
                    unsafe{
                        if HIST_NAV+1<HIST_CNT {
                            HIST_NAV+=1;
                            let idx=HIST_NAV%16;
                            let hlen=HIST_LEN[idx] as usize;
                            len=hlen.min(127);
                            buf[..len].copy_from_slice(&HIST[idx][..len]);
                        } else if HIST_NAV+1==HIST_CNT {
                            HIST_NAV+=1; len=0;
                        } else { len=0; }
                        clear_line(start_row);
                        TERM_COL=0; TERM_ROW=start_row;
                        prompt();
                        for i in 0..len { put_char(buf[i], COLOR_TEXT); }
                        display::mark_dirty(0,0,sw(),sh()); display::present();
                    }
                }
                b' '..=b'~' => {
                    if len < buf.len()-1 {
                        buf[len]=ch; len+=1;
                        put_char(ch, COLOR_TEXT);
                        display::mark_dirty(0,0,sw(),sh()); unsafe{ display::present(); }
                    }
                }
                0x03 => {
                    print(b"^C\n", COLOR_ERR);
                    unsafe{ display::present(); }
                    buf[0]=0; return 0;
                }
                _=>{}
            }
        } else {
            timer::usleep(1000);
        }
    }
}

fn cmd_help() {
    print_str("=== DBS-GR v0.2 ===\n", COLOR_OK);
    print_str(" DBS Graphical Raster — своя графика, без Linux-имитации\n", COLOR_DIM);
    print_str(" Gradient: black->dark gray->gray  Prompt: red  Input: white  RU/EN Alt+Shift\n", COLOR_DIM);
    print(b"Commands: help, info, ls, cat, mkdir, rm, cd, pwd, echo, write\n", COLOR_TEXT);
    print(b"  fm/mc (file manager), edit PATH, scan, devices/periph\n", COLOR_TEXT);
    print_str("  browser/web <url> - basic HTTP browser (e1000+DHCP/DNS)\n", COLOR_TEXT);
    print(b"  mem, cpu, hw, top, time, date, clear, reboot, poweroff\n", COLOR_TEXT);
    print(b"  ping, dhcp, dns, tcp, wget, pkg, exec, lexec, script, nvme\n", COLOR_TEXT);
    print(b"  connect, fm, selftest, wlinfo, scan, devices\n", COLOR_TEXT);
    print(b"  net, wifi, usb, audio, ide, date, lexec, cpu\n", COLOR_TEXT);
    print(b"  whoami, users, useradd, login, su, passwd, chmod, chown, id\n", COLOR_TEXT);
    print(b"  Layout: Alt+Shift toggle RU/EN  Auto-detect: PS/2, PCI, NVMe, e1000\n", COLOR_DIM);
}
fn cmd_info() { print_str("DBS-GR 0.2  Arch: x86_64  Boot: UEFI  GFX: DBS-GR gradient\n", COLOR_TEXT); print_str("Kernel: Rust no_std  Linux API: drivers only\n", COLOR_DIM); }
fn cmd_mem() {
    let free=crate::memory::free_count() as u64;
    print(b"Free pages: ", COLOR_DIM); dec(free); print(b" (", COLOR_DIM); dec(free*4096/1024); print(b" KB)\n", COLOR_DIM);
}
fn cmd_top() {
    let uid=crate::user::current_uid(); let is_root=uid==0;
    print(b"IDX PID  UID  STATE     CORE\n", COLOR_DIM);
    print(b"--- ---- ---- --------- ----\n", COLOR_DIM);
    crate::scheduler::for_each_task(|idx, t|{
        if !is_root && t.uid!=uid { return; }
        dec(idx as u64); print(b"  ", COLOR_TEXT); dec(t.id); print(b"   ", COLOR_TEXT); dec(t.uid as u64);
        print(b"  ", COLOR_TEXT);
        let st=match t.state{ crate::scheduler::TaskState::Running=>"Running", crate::scheduler::TaskState::Ready=>"Ready", crate::scheduler::TaskState::BlockedRecv=>"BlkRecv", crate::scheduler::TaskState::BlockedSend=>"BlkSend", _=>"Exited"};
        print(st.as_bytes(), COLOR_TEXT); print(b"  ", COLOR_TEXT);
        let cur = unsafe{ crate::scheduler::CURRENT };
        let core = if idx==cur { 0 } else { 0 };
        dec(core); print(b"\n", COLOR_TEXT);
    });
    if !is_root { print(b"(filtered)\n", COLOR_DIM); }
}

pub fn run() -> ! {
    unsafe{ GR_ACTIVE = true; }
    draw_gradient(0);
    unsafe{ TERM_COL=0; TERM_ROW=0; TERM_COLS=sw()/CHAR_W; TERM_ROWS=sh()/CHAR_H; }
    unsafe{ display::mark_dirty(0,0,sw(),sh()); display::present(); }
    print_str("DBS-GR 0.2  Graphical Raster Console\n", COLOR_OK);
    print_str("Background: black -> dark gray -> gray gradient\n", COLOR_DIM);
    print_str("Type 'help' for commands, 'clear' to redraw gradient\n", COLOR_DIM);
    loop{
        prompt();
        unsafe{ display::mark_dirty(0,0,sw(),sh()); display::present(); }
        let mut buf=[0u8;128];
        let n=read_line(&mut buf);
        if n==0 { continue; }
        let line=&buf[..n];
        let mut s=0; while s<line.len() && line[s]==b' ' {s+=1;}
        let mut e=line.len(); while e>s && line[e-1]==b' ' {e-=1;}
        if s>=e { continue; }
        let cmd_end=line[s..e].iter().position(|&c| c==b' ').map(|p| s+p).unwrap_or(e);
        let cmd=&line[s..cmd_end];
        let arg = if cmd_end<e { let mut a=cmd_end; while a<e && line[a]==b' ' {a+=1;} &line[a..e] } else { &[][..] };
        if cmd==b"help" { cmd_help(); }
        else if cmd==b"info" { cmd_info(); }
        else if cmd==b"mem" { cmd_mem(); }
        else if cmd==b"top" { cmd_top(); }
        else if cmd==b"clear" {
            draw_gradient((timer::millis()/40%255) as u32);
            unsafe{ TERM_COL=0; TERM_ROW=0; }
        }
        else if cmd==b"ls" { 
            if arg.is_empty(){ crate::fs_server::ls(b"/"); } else { crate::fs_server::ls(arg); }
            let mut entries=[crate::vfs::DirEntry{name:[0;32], is_dir:false, size:0}; 32];
            let n2=crate::vfs::readdir(if arg.is_empty(){b"/"}else{arg}, &mut entries);
            if n2>=0 {
                for i in 0..n2 as usize {
                    let e=&entries[i];
                    let nl=e.name.iter().position(|&c| c==0).unwrap_or(32);
                    print(&e.name[..nl], if e.is_dir { COLOR_OK } else { COLOR_TEXT });
                    if e.is_dir { print(b"/", COLOR_OK); } print(b"\n", COLOR_TEXT);
                }
            }
        }
        else if cmd==b"pwd" {
            let cwd=crate::scheduler::current_cwd_slice();
            print(cwd, COLOR_TEXT); print(b"\n", COLOR_TEXT);
        }
        else if cmd==b"cd" {
            if arg.is_empty(){ crate::scheduler::set_current_cwd(b"/"); }
            else {
                if crate::vfs::is_dir(arg) { crate::scheduler::set_current_cwd(arg); }
                else { print(b"cd: not a dir\n", COLOR_ERR); }
            }
        }
        else if cmd==b"echo" { print(arg, COLOR_TEXT); print(b"\n", COLOR_TEXT); }
        else if cmd==b"cat" {
            if arg.is_empty(){ print(b"cat: need path\n", COLOR_ERR); }
            else {
                let mut buf=[0u8; 512];
                let fd=crate::vfs::open(arg, 0);
                if fd>=0 {
                    let n=crate::vfs::read(fd, &mut buf);
                    crate::vfs::close(fd);
                    if n>0 { print(&buf[..n as usize], COLOR_TEXT); print(b"\n", COLOR_TEXT); }
                    else { print(b"(empty)\n", COLOR_DIM); }
                } else { print(b"not found\n", COLOR_ERR); }
            }
        }
        else if cmd==b"mkdir" { if arg.is_empty(){ print(b"mkdir: need path\n", COLOR_ERR);} else { if crate::vfs::mkdir(arg){ print(b"ok\n", COLOR_OK);} else { print(b"failed\n", COLOR_ERR); } } }
        else if cmd==b"rm" { if crate::vfs::unlink(arg){ print(b"ok\n", COLOR_OK);} else { print(b"failed\n", COLOR_ERR);} }
        else if cmd==b"reboot" { print(b"reboot...\n", COLOR_OK); unsafe{ display::present(); } crate::acpi::reboot(); }
        else if cmd==b"poweroff" { print(b"poweroff...\n", COLOR_OK); unsafe{ display::present(); } crate::acpi::shutdown(); }
        else if cmd==b"whoami" { print(crate::user::current_name().as_bytes(), COLOR_OK); print(b"\n", COLOR_TEXT); }
        else if cmd==b"users" { crate::user::list_users(); print(b"(see UART)\n", COLOR_DIM); }
        else if cmd==b"useradd" {
            let mut p=0; while p<arg.len()&&arg[p]==b' '{p+=1;}
            let s=p; while p<arg.len()&&arg[p]!=b' '&&arg[p]!=0{p+=1;}
            if s>=p{ print(b"useradd NAME [PASS]\n", COLOR_ERR);} else {
                let name=&arg[s..p];
                while p<arg.len()&&arg[p]==b' '{p+=1;}
                let pw=&arg[p..];
                if crate::user::add_user(name, pw){ print(b"added\n", COLOR_OK);} else { print(b"failed\n", COLOR_ERR);}
            }
        }
        else if cmd==b"login" {
            let mut p=0; while p<arg.len()&&arg[p]==b' '{p+=1;}
            let s=p; while p<arg.len()&&arg[p]!=b' '{p+=1;}
            if s>=p{ print(b"login USER [PASS]\n", COLOR_ERR);} else {
                let name=&arg[s..p];
                while p<arg.len()&&arg[p]==b' '{p+=1;}
                let pw=&arg[p..];
                if crate::user::login(name,pw){ print(b"logged in\n", COLOR_OK);} else { print(b"failed\n", COLOR_ERR);}
            }
        }
        else if cmd==b"fm" || cmd==b"mc" {
            let p = if arg.is_empty(){ crate::scheduler::current_cwd_slice() } else { arg };
            crate::fm::run(p);
            draw_gradient(0);
            unsafe{ TERM_COL=0; TERM_ROW=0; }
            print_str("DBS-FM closed — back to DBS-GR\n", COLOR_DIM);
        }
        else if cmd==b"edit" {
            if arg.is_empty(){ print(b"edit: need PATH\n", COLOR_ERR); }
            else { crate::editor::edit(arg); draw_gradient(0); unsafe{ TERM_COL=0; TERM_ROW=0; } }
        }
        else if cmd==b"browser" || cmd==b"web" || cmd==b"www" {
            let url = if arg.is_empty(){ b"example.com" as &[u8] } else { arg };
            crate::browser::run(url);
            draw_gradient(0);
            unsafe{ TERM_COL=0; TERM_ROW=0; }
        }
        else if cmd==b"write" {
            if arg.is_empty(){ print(b"write: need PATH TEXT\n", COLOR_ERR); }
            else if let Some(sp)=arg.iter().position(|&c|c==b' ') {
                let path=&arg[..sp]; let data=&arg[sp+1..];
                let fd=crate::vfs::open(path, 0x100|1);
                if fd>=0 { let _=crate::vfs::write(fd, data); crate::vfs::close(fd); print(b"ok\n", COLOR_OK); } else { print(b"failed\n", COLOR_ERR); }
            } else { print(b"write: need PATH TEXT\n", COLOR_ERR); }
        }
        else if cmd==b"pkg" {
            crate::pkg::pkg_main(arg);
        }
        else if cmd==b"exec" {
            if arg.is_empty(){ print(b"exec: need PATH\n", COLOR_ERR); } else { let r=crate::elf::load_and_spawn(arg); if r==0{ print(b"exec failed\n", COLOR_ERR);} else { print(b"spawned id=", COLOR_OK); dec(r); print(b"\n", COLOR_TEXT); } }
        }
        else if cmd==b"script" {
            if arg.is_empty(){ print(b"script: need PATH\n", COLOR_ERR); } else { let c=crate::script::run_script(arg); print(b"exit code=", COLOR_DIM); dec(c as u64); print(b"\n", COLOR_TEXT); }
        }
        else if cmd==b"nvme" {
            let mut p=0; while p<arg.len()&&arg[p]!=b' ' {p+=1;}
            let sub=if p<arg.len(){ arg[p]} else {0};
            if sub==b'i' {
                if unsafe{crate::driver::nvme::INIT}{ print(b"NVMe: OK\n", COLOR_OK);} else { print(b"NVMe: none\n", COLOR_ERR);}
            } else { print(b"nvme info\n", COLOR_DIM); }
        }
        else if cmd==b"ping" { crate::shell::cmd_ping_pub(); }
        else if cmd==b"dhcp" { crate::shell::cmd_dhcp_pub(); }
        else if cmd==b"dns" {
            if arg.is_empty(){ print(b"dns: need HOST\n", COLOR_ERR); } else {
                let ip=crate::driver::dns::resolve(arg, 3000);
                if let Some(ip)=ip { dec(ip[0] as u64); print(b".", COLOR_TEXT); dec(ip[1] as u64); print(b".", COLOR_TEXT); dec(ip[2] as u64); print(b".", COLOR_TEXT); dec(ip[3] as u64); print(b"\n", COLOR_TEXT); } else { print(b"timeout\n", COLOR_ERR); }
            }
        }
        else if cmd==b"tcp" { print(b"use: tcp IP PORT TEXT (via shell)\n", COLOR_DIM); }
        else if cmd==b"wget" {
            if arg.is_empty(){ print(b"wget: need URL\n", COLOR_ERR); }
            else {
                if let Some(sp)=arg.iter().position(|&c|c==b' ') {
                    let url=&arg[..sp]; let file=&arg[sp+1..];
                    crate::wget::wget(url, file);
                } else { crate::wget::wget(arg, b""); }
            }
        }
        else if cmd==b"chmod" {
            if arg.is_empty(){ print(b"chmod: need OCTAL PATH\n", COLOR_ERR); }
            else {
                let mut p=0; while p<arg.len()&&arg[p]==b' '{p+=1;}
                let s=p; while p<arg.len()&&arg[p]!=b' '{p+=1;}
                if s>=p{ print(b"chmod: need OCTAL PATH\n", COLOR_ERR); } else {
                    let mode_s=&arg[s..p];
                    let mut mode:u16=0; let mut ok=true;
                    for &c in mode_s { if c<b'0'||c>b'7'{ok=false;break;} mode=(mode<<3)|(c-b'0')as u16; }
                    if !ok{ print(b"bad mode\n", COLOR_ERR); } else {
                        while p<arg.len()&&arg[p]==b' '{p+=1;}
                        let path=&arg[p..];
                        let perm=crate::permissions::get(path);
                        if crate::user::current_uid()!=0 && perm.owner!=crate::user::current_uid(){ print(b"not owner\n", COLOR_ERR); } else { crate::permissions::set(path, crate::permissions::Perm{owner:perm.owner, group:perm.group, mode}); print(b"ok\n", COLOR_OK); }
                    }
                }
            }
        }
        else if cmd==b"chown" {
            if crate::user::current_uid()!=0{ print(b"chown: only root\n", COLOR_ERR); }
            else {
                let mut p=0; while p<arg.len()&&arg[p]==b' '{p+=1;}
                let s=p; while p<arg.len()&&arg[p]!=b' '{p+=1;}
                if s>=p{ print(b"chown: need USER PATH\n", COLOR_ERR); } else {
                    let user_s=&arg[s..p];
                    while p<arg.len()&&arg[p]==b' '{p+=1;}
                    let path=&arg[p..];
                    if let Some(uid)=crate::user::uid_by_name(user_s) {
                        let perm=crate::permissions::get(path);
                        crate::permissions::set(path, crate::permissions::Perm{owner:uid, group:uid, mode:perm.mode});
                        print(b"ok\n", COLOR_OK);
                    } else { print(b"unknown user\n", COLOR_ERR); }
                }
            }
        }
        else if cmd==b"id" { print(crate::user::current_name().as_bytes(), COLOR_OK); print(b" uid=", COLOR_DIM); dec(crate::user::current_uid() as u64); print(b" gid=", COLOR_DIM); dec(crate::user::current_gid() as u64); print(b" layout=", COLOR_DIM); print(crate::driver::ps2::layout_name().as_bytes(), COLOR_TEXT); print(b"\n", COLOR_TEXT); }
        else if cmd==b"su" {
            let mut p=0; while p<arg.len()&&arg[p]==b' '{p+=1;}
            let s=p; while p<arg.len()&&arg[p]!=b' '{p+=1;}
            if s>=p{ print(b"su: need USER\n", COLOR_ERR); } else {
                let name=&arg[s..p];
                while p<arg.len()&&arg[p]==b' '{p+=1;}
                let pw=&arg[p..];
                if crate::user::login(name,pw){ print(b"ok\n", COLOR_OK); } else { print(b"failed\n", COLOR_ERR); }
            }
        }
        else if cmd==b"passwd" {
            if arg.is_empty(){ print(b"passwd: need NEWPASS or USER NEWPASS\n", COLOR_ERR); }
            else {
                let mut p=0; while p<arg.len()&&arg[p]==b' '{p+=1;}
                let s=p; while p<arg.len()&&arg[p]!=b' '{p+=1;}
                let first=&arg[s..p];
                let mut pp=p; while pp<arg.len()&&arg[pp]==b' '{pp+=1;}
                if pp>=arg.len(){
                    let cur=crate::user::current_name();
                    if crate::user::set_password(cur.as_bytes(), first){ print(b"changed\n", COLOR_OK); } else { print(b"failed\n", COLOR_ERR); }
                } else {
                    if crate::user::current_uid()!=0{ print(b"only root can change others\n", COLOR_ERR); } else {
                        let pw=&arg[pp..];
                        if crate::user::set_password(first, pw){ print(b"changed\n", COLOR_OK); } else { print(b"failed\n", COLOR_ERR); }
                    }
                }
            }
        }
        else if cmd==b"time" { crate::shell::cmd_time_pub(); }
        else if cmd==b"date" { crate::shell::cmd_date_pub(); }
        else if cmd==b"connect" {
            crate::shell::cmd_connect_pub(arg);
            draw_gradient(0);
            unsafe{ TERM_COL=0; TERM_ROW=0; }
        }
        else if cmd==b"lexec" {
            if arg.is_empty(){ print(b"lexec: need PATH\n", COLOR_ERR); }
            else { let pid=crate::linux::spawn_file(arg); if pid==0{ print(b"lexec failed\n", COLOR_ERR);} else { print(b"linux pid=", COLOR_OK); dec(pid); print(b"\n", COLOR_TEXT); } }
        }
        else if cmd==b"selftest" {
            let (ok,total)=crate::rs_kernel_test::run();
            print(b"selftest: ", COLOR_TEXT); dec(ok as u64); print(b"/", COLOR_TEXT); dec(total as u64);
            if ok==total{ print(b" ALL OK\n", COLOR_OK);} else { print(b" FAILURES\n", COLOR_ERR);}
        }
        else if cmd==b"cpu" { crate::shell::cmd_cpu_pub(); }
        else if cmd==b"net" { crate::shell::cmd_net_pub(arg); }
        else if cmd==b"wifi" { crate::shell::cmd_wifi_pub(arg); }
        else if cmd==b"usb" { crate::shell::cmd_usb_pub(); }
        else if cmd==b"audio" { crate::shell::cmd_audio_pub(); }
        else if cmd==b"ide" { crate::shell::cmd_ide_pub(); }
        else if cmd==b"wlinfo" {
            print(b"Wayland globals:\n", COLOR_OK);
            for g in crate::wayland::protocol::GLOBALS {
                print(b"  id=", COLOR_TEXT); dec(g.id as u64);
                print(b" ", COLOR_TEXT); print(g.interface.as_bytes(), COLOR_TEXT);
                print(b"@v", COLOR_TEXT); dec(g.version as u64); print(b"\n", COLOR_TEXT);
            }
        }
        else if cmd==b"hw" { crate::shell::cmd_ping_pub(); crate::driver::pci::scan_for_hw(); }
        else if cmd==b"scan" {
            print_str("Scanning libraries and drivers...\n", COLOR_DIM);
            crate::driver::pci::scan_for_hw();
            crate::driver::adapt::print_report();
            print(b"Done\n", COLOR_OK);
        }
        else if cmd==b"devices" || cmd==b"periph" || cmd==b"hwinfo" {
            print_str("=== Devices (auto-detected) ===\n", COLOR_OK);
            crate::driver::pci::scan_for_hw();
            print_str("Keyboard: ", COLOR_DIM); print_str(crate::driver::ps2::layout_name(), COLOR_TEXT); print_str(" layout, PS/2 IRQ1\n", COLOR_DIM);
            print_str("Mouse: PS/2 IRQ12\n", COLOR_DIM);
            print_str("Audio/Headset: ", COLOR_DIM);
            print_str("detecting...\n", COLOR_TEXT);
            crate::driver::adapt::print_report();
        }
        else if cmd==b"wldemo" { print_str("wldemo kept for compat - use DBS-GR gradient\n", COLOR_DIM); }
        else { print(b"unknown: ", COLOR_ERR); print(cmd, COLOR_TEXT); print(b"\n", COLOR_TEXT); }
        unsafe{ display::mark_dirty(0,0,sw(),sh()); display::present(); }
    }
}
