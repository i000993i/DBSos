/// GUI — GNOME Shell Desktop (fixed flicker, clean render loop)
///
/// Architecture:
/// - Full redraw only when something actually changes
/// - Cursor is an overlay (save/restore, no background redraw)
/// - Mouse movement does NOT trigger full redraw (only cursor overlay)

pub mod theme;
pub mod render;
pub mod input;
pub mod egui;
pub mod shell;
pub mod wm;
pub mod apps;

use crate::driver::ps2;
use crate::driver::mouse as ps2_mouse;
use crate::timer;

pub static mut NEED_REDRAW: bool = true;
static mut NEED_CURSOR: bool = true;

// ── App launcher ──────────────────────────────────────────────────

pub fn launch_app(id: u8) {
    let sw = crate::display::width();
    let sh = crate::display::height();
    let cx = (sw / 2) as i32 - 250;
    let cy = (sh / 2) as i32 - 175 + 16;

    match id {
        1 => {
            if let Some(idx) = wm::create_window(wm::WinKind::Terminal, b"Terminal", cx, cy, 500, 350) {
                wm::write_to_window(idx, b"DBSos Terminal v5.0\r\n> ");
            }
        }
        2 => {
            if let Some(idx) = wm::create_window(wm::WinKind::FileManager, b"Files", cx, cy, 480, 360) {
                apps::filemanager::populate(idx);
            }
        }
        3 => {
            if let Some(idx) = wm::create_window(wm::WinKind::TextEditor, b"Editor", cx, cy, 500, 400) {
                wm::write_to_window(idx, b"DBSos Text Editor\r\n");
            }
        }
        4 => {
            if let Some(idx) = wm::create_window(wm::WinKind::Settings, b"Settings", cx, cy, 400, 300) {
                wm::write_to_window(idx, b"=== Settings ===\r\n");
                wm::write_to_window(idx, b"Display: 1280x800\r\n");
                wm::write_to_window(idx, b"Theme: Adwaita Dark\r\n");
            }
        }
        5 => {
            if let Some(idx) = wm::create_window(wm::WinKind::Custom, b"About", cx, cy, 350, 200) {
                wm::write_to_window(idx, b"=== DBSos v0.1.0 ===\r\n\r\n");
                wm::write_to_window(idx, b"Microkernel OS\r\n");
                wm::write_to_window(idx, b"Rust + UEFI GOP\r\n");
                wm::write_to_window(idx, b"GNOME Shell Desktop\r\n");
            }
        }
        _ => {}
    }
}

// ── Terminal command processing ────────────────────────────────────

pub fn process_terminal_cmd(win_idx: usize, cmd: &[u8]) {
    if cmd == b"help" {
        wm::write_to_window(win_idx, b"Commands: help, ls, cat, mem, clear, exit\r\n> ");
    } else if cmd == b"clear" {
        wm::clear_window_content(win_idx);
        wm::write_to_window(win_idx, b"> ");
    } else if cmd == b"mem" {
        let free = crate::memory::free_count();
        wm::write_to_window(win_idx, b"Free pages: ");
        let mut buf = [0u8; 20];
        let n = fmt_u64(free as u64, &mut buf);
        wm::write_to_window(win_idx, &buf[..n]);
        wm::write_to_window(win_idx, b"\r\n> ");
    } else if cmd == b"ls" {
        let mut entries = [crate::vfs::DirEntry {
            name: [0u8; crate::vfs::MAX_NAME], is_dir: false, size: 0,
        }; 32];
        let n = crate::vfs::readdir(b"/", &mut entries);
        for i in 0..n as usize {
            if entries[i].is_dir { wm::write_to_window(win_idx, b"[DIR] "); }
            let nlen = entries[i].name.iter().position(|&c| c == 0).unwrap_or(32);
            wm::write_to_window(win_idx, &entries[i].name[..nlen]);
            wm::write_to_window(win_idx, b"\r\n");
        }
        wm::write_to_window(win_idx, b"> ");
    } else if cmd.len() > 4 && &cmd[..4] == b"cat " {
        let path = &cmd[4..];
        let fd = crate::vfs::open(path, 0);
        if fd < 0 {
            wm::write_to_window(win_idx, b"cat: not found\r\n> ");
        } else {
            let mut buf = [0u8; 4096];
            loop {
                let n = crate::vfs::read(fd, &mut buf);
                if n <= 0 { break; }
                wm::write_to_window(win_idx, &buf[..n as usize]);
            }
            crate::vfs::close(fd);
            wm::write_to_window(win_idx, b"\r\n> ");
        }
    } else if cmd == b"exit" {
        if let Some(idx) = wm::active_window() { wm::close_window(idx); }
    } else if cmd == b"gui" {
        launch_app(1);
        wm::write_to_window(win_idx, b"> ");
    } else {
        wm::write_to_window(win_idx, b"Unknown. Type 'help'.\r\n> ");
    }
}

fn fmt_u64(mut v: u64, buf: &mut [u8]) -> usize {
    if v == 0 { buf[0] = b'0'; return 1; }
    let mut tmp = [0u8; 20];
    let mut n = 0;
    while v > 0 { tmp[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    let mut j = 0;
    while j < n { buf[j] = tmp[n - 1 - j]; j += 1; }
    n
}

// ── Main loop ─────────────────────────────────────────────────────

pub fn run() {
    crate::driver::uart::write_str("[GUI] GNOME Shell Desktop v5.0\r\n");

    // Full initial draw
    render_full();
    unsafe { NEED_REDRAW = false; NEED_CURSOR = true; }

    let mut last_diag_ms: u64 = 0;

    loop {
        // ── Console mode ──
        if crate::console::is_active() {
            crate::console::render();
            if let Some(c) = ps2::poll_char() {
                if c == 0x3B {
                    crate::console::deactivate();
                    unsafe { NEED_REDRAW = true; }
                } else {
                    crate::console::handle_key(c);
                }
            }
            timer::usleep(5000);
            continue;
        }

        // ── Process input ──
        process_input();

        // ── Full redraw only when needed ──
        if unsafe { NEED_REDRAW } {
            shell::erase_cursor();
            render_full();
            unsafe { NEED_REDRAW = false; }
        }

        // ── Cursor always renders ──
        shell::draw_cursor();

        // ── Diagnostic: print mouse status every 2 seconds ──
        let now_ms = timer::millis();
        let irq = crate::driver::mouse::irq_count();
        let raw = crate::driver::mouse::raw_byte_count();
        if now_ms - last_diag_ms > 2000 {
            last_diag_ms = now_ms;
            crate::driver::uart::write_str("[GUI] irq=");
            uart_u64(irq);
            crate::driver::uart::write_str(" raw=");
            uart_u64(raw);
            crate::driver::uart::write_str(" mouse=(");
            uart_i32(crate::driver::mouse::x());
            crate::driver::uart::write_str(",");
            uart_i32(crate::driver::mouse::y());
            crate::driver::uart::write_str(")\r\n");
        }

        timer::usleep(8000);
    }
}

fn uart_u64(mut v: u64) {
    if v == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut buf = [0u8; 20];
    let mut n = 0;
    while v > 0 { buf[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    let mut i = 0;
    while i < n { crate::driver::uart::putchar(buf[n - 1 - i]); i += 1; }
}

fn uart_i32(v: i32) {
    if v < 0 { crate::driver::uart::putchar(b'-'); uart_u64((-v) as u64); }
    else { uart_u64(v as u64); }
}

fn process_input() {
    // ── Keyboard ──
    if let Some(c) = ps2::poll_char() {
        if c == 0x3B {  // F1
            crate::console::deactivate();
            unsafe { NEED_REDRAW = true; }
            return;
        }
        if c == 0x3C {  // F2
            crate::console::activate();
            return;
        }

        if let Some(fi) = wm::active_window() {
            if c == b'\r' || c == b'\n' {
                unsafe {
                    let len = wm::window_input_len(fi);
                    if len > 0 {
                        let mut cmd_buf = [0u8; 512];
                        wm::with_window_mut(fi, |w| {
                            cmd_buf[..w.input_len].copy_from_slice(&w.input_buf[..w.input_len]);
                            w.input_len = 0;
                        });
                        wm::write_to_window(fi, &cmd_buf[..len]);
                        wm::write_to_window(fi, b"\r\n");
                        process_terminal_cmd(fi, &cmd_buf[..len]);
                    }
                }
                unsafe { NEED_REDRAW = true; }
            } else if c == 0x08 {
                wm::with_window_mut(fi, |w| {
                    if w.input_len > 0 { w.input_len -= 1; }
                });
                unsafe { NEED_REDRAW = true; }
            } else if c >= b' ' {
                wm::with_window_mut(fi, |w| {
                    if w.kind == wm::WinKind::Terminal && w.input_len < 511 {
                        w.input_buf[w.input_len] = c;
                        w.input_len += 1;
                    }
                });
                unsafe { NEED_REDRAW = true; }
            }
        }
    }

    // ── Mouse (does NOT set NEED_REDRAW — cursor is overlay) ──
    let mx = ps2_mouse::x();
    let my = ps2_mouse::y();
    let lb = ps2_mouse::left();

    input::update_mouse(mx, my);
    input::update_mouse_button(input::MouseButton::Left, lb);

    while let Some(ev) = input::next_event() {
        match ev {
            input::Event::MouseDown { x, y, btn } => {
                if btn == input::MouseButton::Left {
                    if shell::handle_start_click(x as u32, y as u32) {
                        unsafe { NEED_REDRAW = true; }
                        continue;
                    }
                    if shell::handle_taskbar_click(x as u32, y as u32) {
                        unsafe { NEED_REDRAW = true; }
                        continue;
                    }
                    if shell::is_overview_open() { shell::close_start(); unsafe { NEED_REDRAW = true; } }
                    wm::process_event(ev);
                    unsafe { NEED_REDRAW = true; }
                }
            }
            input::Event::MouseMove { .. } => {
                // Mouse move: ONLY redraw cursor (no full redraw!)
                // Cursor is save/restore overlay, no NEED_REDRAW needed
            }
            input::Event::MouseUp { .. } => {
                wm::process_event(ev);
            }
            _ => {}
        }
    }
}

fn render_full() {
    // 1. Background
    crate::display::clear_screen(0x24, 0x24, 0x24);

    // 2. Windows (sorted by z)
    let mut indices = [0usize; wm::MAX_WINDOWS];
    let mut count = 0;
    wm::for_each_window(|idx, _| {
        if count < wm::MAX_WINDOWS { indices[count] = idx; count += 1; }
    });
    for i in 0..count {
        for j in (i + 1)..count {
            let zi = wm::with_window(indices[i], |w| w.z).unwrap_or(0);
            let zj = wm::with_window(indices[j], |w| w.z).unwrap_or(0);
            if zi > zj { indices.swap(i, j); }
        }
    }
    for i in 0..count {
        wm::draw_window_frame(indices[i]);
    }

    // 3. Dash
    shell::draw_dash();

    // 4. Top panel
    shell::draw_panel();

    // 5. Overview
    shell::draw_overview();
}
