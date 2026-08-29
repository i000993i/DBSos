/// System Console — LEVEL_1_SYSTEM
///
/// Text-mode TTY for system interaction. Switch between graphical desktop
/// and this console via Ctrl+Alt+F1 (graphical) or Ctrl+Alt+F2 (console).
///
/// Provides: text display, scrolling, command prompt, system commands.

use crate::display;
use crate::driver::uart;
use crate::timer;

pub const CONSOLE_COLS: u32 = 80;
pub const CONSOLE_ROWS: u32 = 25;
pub const CELL_W: u32 = 8;
pub const CELL_H: u32 = 16;
pub const CONSOLE_W: u32 = CONSOLE_COLS * CELL_W;   // 640
pub const CONSOLE_H: u32 = CONSOLE_ROWS * CELL_H;   // 400

// Colors
const BG_COLOR: u32 = 0x1A1A2E;      // Dark blue-gray
const TEXT_COLOR: u32 = 0x00FF41;     // Green (phosphor look)
const PROMPT_COLOR: u32 = 0x00BFFF;  // Light blue
const BORDER_COLOR: u32 = 0x333355;
const TITLE_COLOR: u32 = 0xCCCCCC;

static mut SCREEN: [[u8; CONSOLE_COLS as usize]; CONSOLE_ROWS as usize] = [[0; 80]; 25];
static mut CURSOR_X: u32 = 0;
static mut CURSOR_Y: u32 = 0;
static mut SCROLL_Y: u32 = 0;
static mut INPUT_BUF: [u8; 256] = [0; 256];
static mut INPUT_LEN: usize = 0;
static mut ACTIVE: bool = false;

// ── Legacy API (used by shell.rs) ────────────────────────────────

/// Legacy API — called by display.rs with framebuffer info (ignored, we use display module directly)
pub fn init(_fb: u64, _w: u32, _h: u32, _stride: u32) {
    uart::write_str("[CONSOLE] Initializing framebuffer console\r\n");
}

/// Initialize the system console (LEVEL_1_SYSTEM)
pub fn init_console() {
    uart::write_str("[CONSOLE] LEVEL_1_SYSTEM console ready\r\n");
}

pub fn write_str(s: &str) {
    for ch in s.bytes() {
        putchar(ch);
    }
}

pub fn putchar(ch: u8) {
    put_char(ch);
    // Also render to framebuffer if active
    if unsafe { ACTIVE } {
        render_char_fb(ch);
    }
}

pub fn screen_cols() -> u32 {
    CONSOLE_COLS
}

pub fn screen_rows() -> u32 {
    CONSOLE_ROWS
}

pub fn set_cursor(col: u32, row: u32) {
    unsafe {
        CURSOR_X = col.min(CONSOLE_COLS - 1);
        CURSOR_Y = row.min(CONSOLE_ROWS - 1);
    }
}

pub fn get_cursor() -> (u32, u32) {
    unsafe { (CURSOR_X, CURSOR_Y) }
}

pub fn clear_screen_fb() {
    unsafe {
        CURSOR_X = 0;
        CURSOR_Y = 0;
        SCROLL_Y = 0;
        display::rect(0, 0, display::width() as usize, display::height() as usize,
            ((BG_COLOR >> 16) & 0xFF) as u8,
            ((BG_COLOR >> 8) & 0xFF) as u8,
            (BG_COLOR & 0xFF) as u8,
        );
    }
}

pub fn draw_rect(x: u32, y: u32, w: u32, h: u32, color: u32) {
    let fb = display::framebuffer();
    let sw = display::width();
    let sh = display::height();
    let fb_stride = sw * 4;

    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;

    for dy in 0..h {
        let py = y + dy;
        if py >= sh { break; }
        for dx in 0..w {
            let px = x + dx;
            if px >= sw { break; }
            let offset = (py * fb_stride + px * 4) as usize;
            unsafe {
                *fb.add(offset) = b;
                *fb.add(offset + 1) = g;
                *fb.add(offset + 2) = r;
            }
        }
    }
}

fn render_char_fb(ch: u8) {
    let fb = display::framebuffer();
    let sw = display::width();
    let sh = display::height();
    let fb_stride = sw * 4;

    let r = ((TEXT_COLOR >> 16) & 0xFF) as u8;
    let g = ((TEXT_COLOR >> 8) & 0xFF) as u8;
    let b = (TEXT_COLOR & 0xFF) as u8;

    let idx = (ch as usize) * 8;
    if idx + 8 > crate::font::FONT_8X16.len() { return; }
    let glyph = &crate::font::FONT_8X16[idx..idx + 8];

    let (col, row) = unsafe { (CURSOR_X, CURSOR_Y) };

    let px_base = col * CELL_W;
    let py_base = row * CELL_H;

    for glyph_row in 0..8 {
        let bits = glyph[glyph_row];
        for glyph_col in 0..8 {
            if bits & (0x80 >> glyph_col) != 0 {
                let px = px_base + glyph_col as u32;
                let py = py_base + glyph_row as u32;
                if px < sw && py < sh {
                    let offset = (py * fb_stride + px * 4) as usize;
                    unsafe {
                        *fb.add(offset) = b;
                        *fb.add(offset + 1) = g;
                        *fb.add(offset + 2) = r;
                    }
                }
            }
        }
    }
}

// ── Screen buffer operations ──────────────────────────────────────

fn put_char(ch: u8) {
    unsafe {
        if ch == b'\n' {
            CURSOR_X = 0;
            CURSOR_Y += 1;
        } else if ch == b'\r' {
            CURSOR_X = 0;
        } else if ch == 0x08 {  // Backspace
            if CURSOR_X > 0 {
                CURSOR_X -= 1;
                SCREEN[CURSOR_Y as usize][CURSOR_X as usize] = 0;
            }
        } else if ch == b'\t' {
            CURSOR_X = (CURSOR_X + 8) & !7;
        } else {
            if CURSOR_Y < CONSOLE_ROWS && CURSOR_X < CONSOLE_COLS {
                SCREEN[CURSOR_Y as usize][CURSOR_X as usize] = ch;
                CURSOR_X += 1;
            }
        }

        // Wrap
        if CURSOR_X >= CONSOLE_COLS {
            CURSOR_X = 0;
            CURSOR_Y += 1;
        }

        // Scroll
        if CURSOR_Y >= CONSOLE_ROWS {
            scroll_up(1);
            CURSOR_Y = CONSOLE_ROWS - 1;
        }
    }
}

fn print_str(s: &[u8]) {
    for &ch in s {
        put_char(ch);
    }
}

fn scroll_up(n: u32) {
    unsafe {
        for _ in 0..n {
            // Move lines up
            for y in 1..CONSOLE_ROWS as usize {
                SCREEN[y - 1] = SCREEN[y];
            }
            // Clear bottom line
            SCREEN[CONSOLE_ROWS as usize - 1] = [0; 80];
        }
    }
}

pub fn clear_screen() {
    unsafe {
        SCREEN = [[0; 80]; 25];
        CURSOR_X = 0;
        CURSOR_Y = 0;
    }
}

// ── Rendering ─────────────────────────────────────────────────────

pub fn render() {
    if !unsafe { ACTIVE } { return; }

    let fb = display::framebuffer();
    let sw = display::width();
    let sh = display::height();
    let fb_stride = sw * 4;

    // Center the console on screen
    let ox = (sw - CONSOLE_W) / 2;
    let oy = (sh - CONSOLE_H) / 2;

    // Background
    display::rect(0, 0, display::width() as usize, display::height() as usize,
        ((BG_COLOR >> 16) & 0xFF) as u8,
        ((BG_COLOR >> 8) & 0xFF) as u8,
        (BG_COLOR & 0xFF) as u8,
    );

    // Title bar
    for x in ox..ox + CONSOLE_W {
        let offset = (oy * fb_stride + x * 4) as usize;
        unsafe {
            *fb.add(offset) = 0x33;
            *fb.add(offset + 1) = 0x33;
            *fb.add(offset + 2) = 0x55;
        }
    }

    // Title text
    let title = b"DBSos System Console  |  F1=Desktop  F2=Console  |  Type 'help' for commands";
    draw_text_in_fb(ox + 8, oy + 4, title, TITLE_COLOR);

    // Border
    let br = ((BORDER_COLOR >> 16) & 0xFF) as u8;
    let bg = ((BORDER_COLOR >> 8) & 0xFF) as u8;
    let bb = (BORDER_COLOR & 0xFF) as u8;
    // Top/bottom border
    for x in ox..ox + CONSOLE_W {
        let top = (oy * fb_stride + x * 4) as usize;
        let bot = ((oy + CONSOLE_H - 1) * fb_stride + x * 4) as usize;
        unsafe {
            *fb.add(top) = bb; *fb.add(top + 1) = bg; *fb.add(top + 2) = br;
            *fb.add(bot) = bb; *fb.add(bot + 1) = bg; *fb.add(bot + 2) = br;
        }
    }
    // Left/right border
    for y in oy..oy + CONSOLE_H {
        let left = (y * fb_stride + ox * 4) as usize;
        let right = (y * fb_stride + (ox + CONSOLE_W - 1) * 4) as usize;
        unsafe {
            *fb.add(left) = bb; *fb.add(left + 1) = bg; *fb.add(left + 2) = br;
            *fb.add(right) = bb; *fb.add(right + 1) = bg; *fb.add(right + 2) = br;
        }
    }

    // Console content
    let content_y = oy + CELL_H;
    unsafe {
        let scroll = SCROLL_Y;
        let visible_rows = ((CONSOLE_H - CELL_H) / CELL_H) as usize;

        for row in 0..visible_rows {
            let src_row = row + scroll as usize;
            if src_row >= CONSOLE_ROWS as usize { break; }

            for col in 0..CONSOLE_COLS as usize {
                let ch = SCREEN[src_row][col];
                if ch == 0 { continue; }

                let px = ox + col as u32 * CELL_W;
                let py = content_y + row as u32 * CELL_H;

                draw_char_in_fb(px, py, ch, TEXT_COLOR);
            }
        }

        // Cursor
        let cursor_row = CURSOR_Y as usize - scroll as usize;
        if cursor_row < visible_rows {
            let cx = ox + CURSOR_X * CELL_W;
            let cy = content_y + cursor_row as u32 * CELL_H;
            if (timer::millis() / 500) % 2 == 0 {
                // Draw cursor block
                for dy in 0..CELL_H {
                    for dx in 0..CELL_W {
                        let px = cx + dx;
                        let py = cy + dy;
                        if px < sw && py < sh {
                            let offset = (py * fb_stride + px * 4) as usize;
                            *fb.add(offset) = 0x00;
                            *fb.add(offset + 1) = 0xFF;
                            *fb.add(offset + 2) = 0x41;
                        }
                    }
                }
            }
        }

        // Input prompt
        let prompt_y = content_y + visible_rows as u32 * CELL_H + 4;
        let prompt = b"> ";
        draw_text_in_fb(ox + 8, prompt_y, prompt, PROMPT_COLOR);
        draw_text_in_fb(ox + 24, prompt_y, &INPUT_BUF[..INPUT_LEN], TEXT_COLOR);
    }
}

fn draw_char_in_fb(x: u32, y: u32, ch: u8, color: u32) {
    let fb = display::framebuffer();
    let sw = display::width();
    let sh = display::height();
    let fb_stride = sw * 4;

    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;

    let idx = (ch as usize) * 8;
    if idx + 8 > crate::font::FONT_8X16.len() { return; }
    let glyph = &crate::font::FONT_8X16[idx..idx + 8];

    for row in 0..8 {
        let bits = glyph[row];
        for col in 0..8 {
            if bits & (0x80 >> col) != 0 {
                let px = x + col as u32;
                let py = y + row as u32;
                if px < sw && py < sh {
                    let offset = (py * fb_stride + px * 4) as usize;
                    unsafe {
                        *fb.add(offset) = b;
                        *fb.add(offset + 1) = g;
                        *fb.add(offset + 2) = r;
                    }
                }
            }
        }
    }
}

fn draw_text_in_fb(x: u32, y: u32, text: &[u8], color: u32) {
    let mut cx = x;
    for &ch in text {
        if ch == 0 { break; }
        draw_char_in_fb(cx, y, ch, color);
        cx += CELL_W;
    }
}

// ── Input handling ────────────────────────────────────────────────

pub fn handle_key(ch: u8) {
    if !unsafe { ACTIVE } { return; }

    if ch == b'\r' || ch == b'\n' {
        // Process command
        unsafe {
            let mut cmd = [0u8; 256];
            let len = INPUT_LEN;
            cmd[..len].copy_from_slice(&INPUT_BUF[..len]);
            INPUT_LEN = 0;

            // Echo command
            print_str(b"\r\n");
            print_str(&cmd[..len]);
            print_str(b"\r\n");

            process_command(&cmd[..len]);
        }
    } else if ch == 0x08 {  // Backspace
        unsafe {
            if INPUT_LEN > 0 {
                INPUT_LEN -= 1;
                INPUT_BUF[INPUT_LEN] = 0;
            }
        }
    } else if ch >= b' ' && ch < 0x7F {
        unsafe {
            if INPUT_LEN < 255 {
                INPUT_BUF[INPUT_LEN] = ch;
                INPUT_LEN += 1;
            }
        }
    }
}

fn process_command(cmd: &[u8]) {
    if cmd == b"help" || cmd == b"h" {
        print_str(b"System Console Commands:\r\n");
        print_str(b"  help     - Show this help\r\n");
        print_str(b"  clear    - Clear screen\r\n");
        print_str(b"  mem      - Memory info\r\n");
        print_str(b"  time     - Current time\r\n");
        print_str(b"  ps       - Process list\r\n");
        print_str(b"  ls       - List files\r\n");
        print_str(b"  cat      - Read file\r\n");
        print_str(b"  uptime   - System uptime\r\n");
        print_str(b"  reboot   - Reboot system\r\n");
        print_str(b"  desktop  - Switch to desktop (F1)\r\n");
        print_str(b"\r\n");
    } else if cmd == b"clear" || cmd == b"cls" {
        clear_screen();
    } else if cmd == b"mem" {
        let free = crate::memory::free_count();
        let total = crate::memory::total_pages();
        print_str(b"Memory: ");
        let mut buf = [0u8; 20];
        let n = fmt_usize(total, &mut buf);
        print_str(&buf[..n]);
        print_str(b" total, ");
        let n = fmt_usize(free, &mut buf);
        print_str(&buf[..n]);
        print_str(b" free pages (");
        let n = fmt_usize(free * 4096 / 1024, &mut buf);
        print_str(&buf[..n]);
        print_str(b" KB free)\r\n");
    } else if cmd == b"time" {
        let ms = timer::millis();
        let secs = (ms / 1000) as u64;
        let mins = (secs / 60) % 60;
        let hours = (secs / 3600) % 24;
        let mut buf = [0u8; 8];
        buf[0] = b'0' + (hours / 10) as u8;
        buf[1] = b'0' + (hours % 10) as u8;
        buf[2] = b':';
        buf[3] = b'0' + (mins / 10) as u8;
        buf[4] = b'0' + (mins % 10) as u8;
        print_str(&buf[..5]);
        print_str(b"\r\n");
    } else if cmd == b"uptime" {
        let ms = timer::millis();
        let secs = (ms / 1000) as u64;
        print_str(b"Uptime: ");
        let mut buf = [0u8; 20];
        let n = fmt_usize(secs as usize, &mut buf);
        print_str(&buf[..n]);
        print_str(b" seconds\r\n");
    } else if cmd == b"ls" {
        let mut entries = [crate::vfs::DirEntry {
            name: [0u8; crate::vfs::MAX_NAME], is_dir: false, size: 0,
        }; 32];
        let n = crate::vfs::readdir(b"/", &mut entries);
        for i in 0..n as usize {
            if entries[i].is_dir { print_str(b"[DIR] "); }
            let nlen = entries[i].name.iter().position(|&c| c == 0).unwrap_or(32);
            print_str(&entries[i].name[..nlen]);
            print_str(b"\r\n");
        }
    } else if cmd.len() > 4 && &cmd[..4] == b"cat " {
        let path = &cmd[4..];
        let fd = crate::vfs::open(path, 0);
        if fd < 0 {
            print_str(b"cat: file not found\r\n");
        } else {
            let mut buf = [0u8; 4096];
            loop {
                let n = crate::vfs::read(fd, &mut buf);
                if n <= 0 { break; }
                print_str(&buf[..n as usize]);
            }
            crate::vfs::close(fd);
            print_str(b"\r\n");
        }
    } else if cmd == b"ps" {
        print_str(b"PID  NAME          STATE\r\n");
        print_str(b"---  ----          -----\r\n");
        print_str(b"  1  kernel        running\r\n");
        print_str(b"  2  gui           running\r\n");
        print_str(b"  3  shell         running\r\n");
    } else if cmd == b"desktop" || cmd == b"exit" {
        deactivate();
        unsafe { crate::gui::NEED_REDRAW = true; }
    } else if cmd == b"reboot" {
        print_str(b"Rebooting...\r\n");
        crate::timer::usleep(1000000);
        // Reset via keyboard controller
        unsafe {
            crate::io::outb(0x64, 0xFE);
        }
    } else if cmd.len() > 0 {
        print_str(b"Unknown command: ");
        print_str(cmd);
        print_str(b"\r\nType 'help' for commands.\r\n");
    }
    print_str(b"> ");
}

fn fmt_usize(mut v: usize, buf: &mut [u8]) -> usize {
    if v == 0 { buf[0] = b'0'; return 1; }
    let mut tmp = [0u8; 20];
    let mut n = 0;
    while v > 0 { tmp[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    let mut j = 0;
    while j < n { buf[j] = tmp[n - 1 - j]; j += 1; }
    n
}

// ── Activation / Deactivation ─────────────────────────────────────

pub fn activate() {
    unsafe {
        ACTIVE = true;
        clear_screen();
        print_str(b"DBSos System Console v1.0\r\n");
        print_str(b"Type 'help' for available commands.\r\n\r\n");
        print_str(b"> ");
    }
    uart::write_str("[CONSOLE] LEVEL_1_SYSTEM activated\r\n");
}

pub fn deactivate() {
    unsafe { ACTIVE = false; }
    uart::write_str("[CONSOLE] LEVEL_1_SYSTEM deactivated\r\n");
}

pub fn is_active() -> bool {
    unsafe { ACTIVE }
}
