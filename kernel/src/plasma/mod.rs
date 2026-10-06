//! Ubuntu 24.04 LTS — Yaru / GNOME Shell рабочий стол DBSos
//!
//! Прямая отрисовка в framebuffer (без double-buffer memcpy).
//! Копирует интерфейс Ubuntu: top bar + left dock, Yaru orange #E95420, aubergine gradient.
//! Кэш обоев используется только для восстановления regions behind window.
#![allow(dead_code, unused_variables, unused_imports)]

mod icons;

use crate::display;
use crate::font::FONT_8X16;
use crate::driver::mouse;
use crate::driver::ps2;
use crate::timer;

// ═══════════════ Ubuntu 24.04 LTS — Yaru / GNOME Shell ═══════════════
const C_BG: u32 = 0x2C001E; // aubergine dark
const C_BG_MID: u32 = 0x5E2750; // gradient mid
const C_BG_LIGHT: u32 = 0x772953; // aubergine light
const C_BG_DARK: u32 = 0x1E0A1A;
const C_HOVER: u32 = 0x3d3d3d;
const C_ACCENT: u32 = 0xE95420; // Ubuntu orange
const C_ACCENT_H: u32 = 0xFF6B35;
const C_TEXT: u32 = 0xFFFFFF;
const C_DIM: u32 = 0xAEA79F; // warm grey
const C_PANEL: u32 = 0x0E0E0E; // top bar jet black
const C_PANEL_B: u32 = 0x2E2E2E;
const C_DOCK_BG: u32 = 0x211D1E; // Ubuntu dock #211D1E
const C_DOCK_HOVER: u32 = 0x3A3A3A;
const C_WIN_BG: u32 = 0x303030; // Yaru dark window
const C_WIN_HDR: u32 = 0x3D3D3D;
const C_WIN_HDR_UNF: u32 = 0x2B2B2B;
const C_WIN_BDR: u32 = 0x4A4A4A;
const C_CLOSE: u32 = 0xE95420; // Ubuntu orange close (Yaru uses #E95420, we keep red-ish but orange)
const C_CLOSE_H: u32 = 0xD93025; // darker on hover
const C_MIN: u32 = 0xAEA79F;
const C_MAX: u32 = 0xAEA79F;
const C_FOLDER: u32 = 0xE95420;
const C_FILE: u32 = 0xAEA79F;
const C_TRASH: u32 = 0xE95420;
const C_ICON_BG: u32 = 0x3D3D3D;
const C_TASK_HOVER: u32 = 0x3A3A3A;
const C_TASK_ACTIVE: u32 = 0xE95420;
const C_MENU_BG: u32 = 0x1E1E1E;
const C_MENU_SEP: u32 = 0x3A3A3A;
const C_WIN_BTN: u32 = 0x3D3D3D;
// Ubuntu GNOME Shell geometry
const TOPBAR_H: u32 = 28;
const DOCK_W: u32 = 64;
const PANEL_H: u32 = TOPBAR_H; // keep legacy alias for window clamping
const WIN_HDR: u32 = 32;

// ═══════════════ Состояние окна ═══════════════
static mut WIN_X: i32 = 300;
static mut WIN_Y: i32 = 120;
static mut WIN_W: u32 = 640;
static mut WIN_H: u32 = 400;
static mut WIN_MINIMIZED: bool = false;
static mut WIN_MAXIMIZED: bool = false;
static mut DRAGGING: bool = false;
static mut RESIZING: bool = false;
static mut DRAG_OX: i32 = 0;
static mut DRAG_OY: i32 = 0;
static mut SHOW_START: bool = false;
static mut WIN_FOCUSED: bool = true;
static mut SHOW_DESKTOP_ICONS: bool = true;
static mut RIGHT_CLICK_MENU: bool = false;
static mut RIGHT_CLICK_X: i32 = 0;
static mut RIGHT_CLICK_Y: i32 = 0;

// Терминал
static mut TERM_BUF: [u8; 128] = [0; 128];
static mut TERM_LEN: usize = 0;
static mut TERM_LINES: [[u8; 80]; 16] = [[0; 80]; 16];
static mut TERM_LINES_LEN: [usize; 16] = [0; 16];
static mut TERM_LINE_COUNT: usize = 0;

// Кэш обоев — рисуем 1 раз, восстанавливаем построчно
static mut WP_CACHE: *mut u8 = core::ptr::null_mut();
static mut WP_CACHE_SIZE: usize = 0;

// Backing store для курсора (12x18 пикселей)
static mut CURSOR_BACKING: [u32; 12 * 18] = [0; 12 * 18];
static mut CURSOR_LAST_X: i32 = -100;
static mut CURSOR_LAST_Y: i32 = -100;

// Предыдущая позиция окна — для восстановления обоев
static mut PREV_WIN_X: i32 = 0;
static mut PREV_WIN_Y: i32 = 0;
static mut PREV_WIN_W: u32 = 0;
static mut PREV_WIN_H: u32 = 0;
static mut PREV_WIN_MINIMIZED: bool = true;
static mut PREV_WIN_MAXIMIZED: bool = false;

// Window animation state
static mut WIN_ANIM_SLIDE_Y: i32 = 0;
static mut WIN_ANIM_SLIDE_TARGET: i32 = 0;

fn fb() -> *mut u8 { display::framebuffer() }
fn sw() -> u32 { display::width() }
fn sh() -> u32 { display::height() }
fn strd() -> u32 { display::stride() }

// ═══════════════ Графика ═══════════════

fn fill_rect(x: i32, y: i32, w: u32, h: u32, col: u32) {
    if w == 0 || h == 0 { return; }
    let r = ((col >> 16) & 0xFF) as u8;
    let g = ((col >> 8) & 0xFF) as u8;
    let b = (col & 0xFF) as u8;
    let f = fb(); let s = strd() as i32;
    let swi = sw() as i32; let shi = sh() as i32;
    for dy in 0..h as i32 {
        let py = y + dy;
        if py < 0 || py >= shi { continue; }
        let row_off = (py * s) as usize * 4;
        let x0 = if x < 0 { 0 } else { x };
        let x1 = if x + w as i32 > swi { swi } else { x + w as i32 };
        let mut off = row_off + x0 as usize * 4;
        for _dx in x0..x1 {
            unsafe { *f.add(off) = b; *f.add(off+1) = g; *f.add(off+2) = r; }
            off += 4;
        }
    }
}

fn draw_pixel_px(px: i32, py: i32, col: u32) {
    if px < 0 || py < 0 || px >= sw() as i32 || py >= sh() as i32 { return; }
    let off = (py * strd() as i32 + px) as usize * 4;
    let f = fb();
    unsafe { *f.add(off) = (col & 0xFF) as u8; *f.add(off+1) = ((col>>8)&0xFF) as u8; *f.add(off+2) = ((col>>16)&0xFF) as u8; }
}
fn draw_circle_filled(cx: i32, cy: i32, r: i32, col: u32) {
    for dy in -r..=r {
        for dx in -r..=r {
            if dx*dx + dy*dy <= r*r {
                draw_pixel_px(cx+dx, cy+dy, col);
            }
        }
    }
}
fn draw_round_rect(x: i32, y: i32, w: u32, h: u32, radius: i32, col: u32) {
    // central rects + circles at corners
    fill_rect(x+radius, y, w - (radius*2) as u32, radius as u32, col);
    fill_rect(x, y+radius, w, h - (radius*2) as u32, col);
    fill_rect(x+radius, y+h as i32 - radius, w - (radius*2) as u32, radius as u32, col);
    draw_circle_filled(x+radius, y+radius, radius, col);
    draw_circle_filled(x+w as i32 - radius -1, y+radius, radius, col);
    draw_circle_filled(x+radius, y+h as i32 - radius -1, radius, col);
    draw_circle_filled(x+w as i32 - radius -1, y+h as i32 - radius -1, radius, col);
}

fn draw_char_px(x: i32, y: i32, ch: u8, col: u32) {
    if ch < 32 || ch > 126 { return; }
    let idx = ch as usize * 16;
    if idx + 16 > FONT_8X16.len() { return; }
    let g = &FONT_8X16[idx..idx+16];
    let r = ((col >> 16) & 0xFF) as u8;
    let gg = ((col >> 8) & 0xFF) as u8;
    let bb = (col & 0xFF) as u8;
    let f = fb(); let s = strd() as i32;
    let swi = sw() as i32; let shi = sh() as i32;
    for row in 0..16u32 {
        let bits = g[row as usize];
        for col_bit in 0..8u32 {
            if bits & (0x80 >> col_bit) == 0 { continue; }
            let px = x + col_bit as i32;
            let py = y + row as i32;
            if px < 0 || py < 0 || px >= swi || py >= shi { continue; }
            let off = (py * s + px) as usize * 4;
            unsafe { *f.add(off) = bb; *f.add(off+1) = gg; *f.add(off+2) = r; }
        }
    }
}

fn draw_text_px(x: i32, y: i32, s: &[u8], col: u32) {
    let mut cx = x;
    for &c in s {
        draw_char_px(cx, y, c, col);
        cx += 8;
    }
}

fn draw_text_centered(x: i32, y: i32, w: u32, s: &[u8], col: u32) {
    let tw = s.len() as i32 * 8;
    let tx = x + (w as i32 - tw) / 2;
    draw_text_px(tx, y, s, col);
}

fn draw_text_shadow(x: i32, y: i32, s: &[u8], fg: u32, sh_col: u32) {
    draw_text_px(x+1, y+1, s, sh_col);
    draw_text_px(x, y, s, fg);
}

// ═══════════════ Иконки Breeze (16x16) ═══════════════

struct Icon16 { data: &'static [u8; 256], colors: &'static [u32] }

static ICON_TERMINAL: Icon16 = Icon16 {
    colors: &[0, 0x232629, 0x31363b, 0x3daee9, 0xfcfcfc],
    data: &[
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,
        0,1,2,2,2,2,2,2,2,2,2,2,2,2,1,0,
        0,1,4,4,3,4,4,4,4,4,4,4,4,4,1,0,
        0,1,4,4,4,4,4,4,4,4,4,4,4,4,1,0,
        0,1,4,3,4,4,4,4,4,4,4,4,4,4,1,0,
        0,1,4,4,4,3,4,4,4,4,4,4,4,4,1,0,
        0,1,4,4,4,4,4,4,4,4,4,4,4,4,1,0,
        0,1,4,3,4,4,4,4,4,4,4,4,4,4,1,0,
        0,1,4,4,4,3,3,3,3,4,4,4,4,4,1,0,
        0,1,4,4,4,4,4,4,4,4,4,4,4,4,1,0,
        0,1,4,4,4,4,4,4,4,4,4,4,4,4,1,0,
        0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,
        0,0,1,1,1,1,1,1,1,1,1,1,1,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    ]
};

static ICON_SETTINGS: Icon16 = Icon16 {
    colors: &[0, 0x9ca0a4, 0xfcfcfc, 0x3daee9, 0x5a5e62],
    data: &[
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,
        0,0,0,0,0,0,1,2,2,1,0,0,0,0,0,0,
        0,0,0,1,1,0,1,2,2,1,0,1,1,0,0,0,
        0,0,1,2,2,1,1,2,2,1,1,2,2,1,0,0,
        0,0,0,1,1,1,1,2,2,1,1,1,1,0,0,0,
        0,0,0,0,0,1,1,2,2,1,1,0,0,0,0,0,
        0,0,0,1,1,1,1,2,2,1,1,1,1,0,0,0,
        0,0,1,2,2,1,1,2,2,1,1,2,2,1,0,0,
        0,0,0,1,1,0,1,2,2,1,0,1,1,0,0,0,
        0,0,0,0,0,0,1,2,2,1,0,0,0,0,0,0,
        0,0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    ]
};

static ICON_DOLPHIN: Icon16 = Icon16 {
    colors: &[0, 0x3daee9, 0x51bfff, 0x2980b9, 0xfcfcfc],
    data: &[
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,0,0,0,0,1,1,1,1,0,0,0,0,0,0,0,
        0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0,
        0,0,0,1,2,4,2,2,2,2,1,0,0,0,0,0,
        0,0,0,1,2,2,2,2,2,2,1,1,0,0,0,0,
        0,0,0,1,2,2,2,2,2,2,2,2,1,0,0,0,
        0,0,1,2,2,2,2,2,2,2,2,2,2,1,0,0,
        0,1,2,2,2,2,2,2,2,2,2,2,2,2,1,0,
        0,1,2,2,2,2,2,2,2,2,2,2,2,2,1,0,
        0,0,1,2,2,2,2,2,2,2,2,2,2,1,0,0,
        0,0,0,1,2,2,2,2,2,2,2,2,1,0,0,0,
        0,0,0,0,1,2,2,2,2,2,2,1,0,0,0,0,
        0,0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,
        0,0,0,0,0,0,1,1,1,1,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    ]
};

fn draw_icon(x: i32, y: i32, icon: &Icon16) {
    let f = fb(); let s = strd() as i32;
    let swi = sw() as i32; let shi = sh() as i32;
    for row in 0..16 {
        for col in 0..16 {
            let v = icon.data[row * 16 + col];
            if v == 0 { continue; }
            let px = x + col as i32; let py = y + row as i32;
            if px < 0 || px >= swi || py < 0 || py >= shi { continue; }
            let col32 = icon.colors[v as usize];
            let off = (py * s + px) as usize * 4;
            unsafe {
                *f.add(off) = (col32 & 0xFF) as u8;
                *f.add(off+1) = ((col32>>8)&0xFF) as u8;
                *f.add(off+2) = ((col32>>16)&0xFF) as u8;
            }
        }
    }
}

/// DBSpack BGRA 32x32 иконка — рисует из [u8; 4096] массива
fn draw_icon_bgra32(x: i32, y: i32, bgra: &'static [u8; 4096]) {
    let f = fb(); let s = strd() as i32;
    let swi = sw() as i32; let shi = sh() as i32;
    for row in 0..32 {
        for col in 0..32 {
            let idx = (row * 32 + col) as usize * 4;
            let a = bgra[idx + 3];
            if a < 128 { continue; }
            let px = x + col as i32;
            let py = y + row as i32;
            if px < 0 || px >= swi || py < 0 || py >= shi { continue; }
            let off = (py * s + px) as usize * 4;
            unsafe {
                *f.add(off) = bgra[idx];
                *f.add(off+1) = bgra[idx+1];
                *f.add(off+2) = bgra[idx+2];
            }
        }
    }
}

// ═══════════════ Логотип DBS ═══════════════

fn draw_dbs_logo(x: i32, y: i32, size: u32) {
    let s = size as i32;
    fill_rect(x, y, (s*5/10) as u32, s as u32, C_ACCENT);
    fill_rect(x + s/10, y + s/6, (s*3/10) as u32, (s*2/3) as u32, C_BG);
    fill_rect(x + s*6/10, y, (s/5) as u32, s as u32, C_ACCENT);
    fill_rect(x + s*6/10, y, (s*3/10) as u32, (s/6) as u32, C_ACCENT);
    fill_rect(x + s*6/10, y + s/2 - s/12, (s*3/10) as u32, (s/6) as u32, C_ACCENT);
    fill_rect(x + s*6/10, y + s - s/6, (s*3/10) as u32, (s/6) as u32, C_ACCENT);
    fill_rect(x + s*8/10, y + s/6, (s/5) as u32, (s/3-s/12) as u32, C_BG);
    fill_rect(x + s*8/10, y + s/2+s/12, (s/5) as u32, (s/3-s/12) as u32, C_BG);
    fill_rect(x + s*11/10, y, (s/5) as u32, (s/6) as u32, C_ACCENT);
    fill_rect(x + s, y + s/6, (s/5) as u32, (s/6) as u32, C_ACCENT);
    fill_rect(x + s*11/10, y + s/2-s/12, (s/5) as u32, (s/6) as u32, C_ACCENT);
    fill_rect(x + s, y + s/2+s/12, (s/5) as u32, (s/6) as u32, C_ACCENT);
    fill_rect(x + s*11/10, y + s-s/6, (s/5) as u32, (s/6) as u32, C_ACCENT);
}

// ═══════════════ Курсор ═══════════════

static CURSOR_ARROW: [[u8; 12]; 18] = [
    [1,1,0,0,0,0,0,0,0,0,0,0],
    [1,2,1,0,0,0,0,0,0,0,0,0],
    [1,2,2,1,0,0,0,0,0,0,0,0],
    [1,2,2,2,1,0,0,0,0,0,0,0],
    [1,2,2,2,2,1,0,0,0,0,0,0],
    [1,2,2,2,2,2,1,0,0,0,0,0],
    [1,2,2,2,2,2,2,1,0,0,0,0],
    [1,2,2,2,2,2,2,2,1,0,0,0],
    [1,2,2,2,2,2,2,2,2,1,0,0],
    [1,2,2,2,2,2,2,2,2,2,1,0],
    [1,2,2,2,2,2,1,1,1,1,1,1],
    [1,2,2,1,2,2,1,0,0,0,0,0],
    [1,2,1,0,1,2,2,1,0,0,0,0],
    [1,1,0,0,1,2,2,1,0,0,0,0],
    [0,0,0,0,1,2,2,1,0,0,0,0],
    [0,0,0,0,1,2,1,0,0,0,0,0],
    [0,0,0,0,1,1,0,0,0,0,0,0],
    [0,0,0,0,1,0,0,0,0,0,0,0],
];

fn restore_cursor_backing() {
    let mx = unsafe { CURSOR_LAST_X };
    let my = unsafe { CURSOR_LAST_Y };
    let swi = sw() as i32; let shi = sh() as i32;
    if mx < -12 || my < -18 || mx >= swi || my >= shi { return; }
    let f = fb(); let s = strd() as i32;
    for row in 0i32..18 {
        for col in 0i32..12 {
            let px = mx + col; let py = my + row;
            if px < 0 || py < 0 || px >= swi || py >= shi { continue; }
            let off = (py * s + px) as usize * 4;
            let pixel = unsafe { CURSOR_BACKING[(row * 12 + col) as usize] };
            unsafe {
                *f.add(off) = pixel as u8;
                *f.add(off+1) = (pixel >> 8) as u8;
                *f.add(off+2) = (pixel >> 16) as u8;
            }
        }
    }
}

fn save_cursor_backing(mx: i32, my: i32) {
    let swi = sw() as i32; let shi = sh() as i32;
    if mx < -12 || my < -18 || mx >= swi || my >= shi { return; }
    let f = fb(); let s = strd() as i32;
    for row in 0i32..18 {
        for col in 0i32..12 {
            let px = mx + col; let py = my + row;
            let idx = (row * 12 + col) as usize;
            if px < 0 || py < 0 || px >= swi || py >= shi {
                unsafe { CURSOR_BACKING[idx] = 0; }
                continue;
            }
            let off = (py * s + px) as usize * 4;
            let pixel = unsafe {
                (*f.add(off) as u32)
                | ((*f.add(off+1) as u32) << 8)
                | ((*f.add(off+2) as u32) << 16)
            };
            unsafe { CURSOR_BACKING[idx] = pixel; }
        }
    }
}

fn draw_cursor(mx: i32, my: i32) {
    let swi = sw() as i32; let shi = sh() as i32;
    if mx < 0 || my < 0 || mx >= swi || my >= shi { return; }
    for row in 0i32..18 {
        for col in 0i32..12 {
            let v = CURSOR_ARROW[row as usize][col as usize];
            if v == 0 { continue; }
            let px = mx + col; let py = my + row;
            if px < 0 || py < 0 || px >= swi || py >= shi { continue; }
            let c = if v == 1 { 0x000000u32 } else { 0xFFFFFFu32 };
            draw_pixel_px(px, py, c);
        }
    }
}

// ═══════════════ Восстановление обоев построчно ═══════════════

/// Восстановить обои из кэша для заданной полосы (row_start..row_end)
fn restore_wallpaper_rows(row_start: i32, row_end: i32) {
    unsafe {
        if WP_CACHE.is_null() || display::framebuffer().is_null() { return; }
    }
    let s = strd() as usize;
    let w = sw() as usize;
    let row_bytes = s * 4;
    let start = row_start.max(0) as usize;
    let end = (row_end as usize).min(sh() as usize);
    if start >= end { return; }
    let copy_bytes = (end - start) * row_bytes;
    let src_offset = start * row_bytes;
    unsafe {
        let src = WP_CACHE.add(src_offset);
        let dst = display::framebuffer().add(src_offset);
        core::ptr::copy_nonoverlapping(src, dst, copy_bytes);
    }
    // КРИТИЧНО: помечаем восстановленные строки как dirty для present()
    display::mark_dirty(0, row_start, sw(), (end - start) as u32);
}

// ═══════════════ Обои — Ubuntu Aubergine Gradient ═══════════════

fn draw_wallpaper() {
    let w = sw() as i32; let h = sh() as i32;
    // Aubergine vertical gradient: 2C001E -> 5E2750 -> 772953
    for y in 0..h {
        let t = (y * 255 / h.max(1)) as u32;
        // interpolate 2C001E -> 772953
        let r1=0x2C; let g1=0x00; let b1=0x1E;
        let r2=0x77; let g2=0x29; let b2=0x53;
        let r = r1 + (r2 - r1)* t / 255;
        let g = g1 + (g2 - g1)* t / 255;
        let b = b1 + (b2 - b1)* t / 255;
        // subtle vignette
        fill_rect(0, y, w as u32, 1, (r<<16)|(g<<8)|b);
    }
    // Ubuntu circle-of-friends logo (simplified: 3 dots around)
    let cx = w/2; let cy = (h - TOPBAR_H as i32)/2 + TOPBAR_H as i32/2;
    // big orange circle (Yaru)
    let r_big = 48i32;
    for dy in -r_big..r_big {
        for dx in -r_big..r_big {
            let d2 = dx*dx+dy*dy;
            if d2 > r_big*r_big { continue; }
            let col = if d2 < (r_big-6)*(r_big-6) { C_ACCENT } else { 0xFFFFFF };
            draw_pixel_px(cx+dx, cy+dy, col);
        }
    }
    // 3 white dots
    for &(ox,oy) in &[(0,-14),( -12, 12),( 12, 12)]{
        for dy in -4..4 { for dx in -4..4 { if dx*dx+dy*dy<=12 { draw_pixel_px(cx+ox+dx, cy+oy+dy, 0xFFFFFF); } } }
    }
    let msg = b"Ubuntu";
    let tw = msg.len() as i32 * 8;
    draw_text_shadow(cx - tw/2, cy + r_big + 14, msg, 0xFFFFFF, 0x1E0A1A);
    let sub = b"DBSos 24.04 LTS - Yaru";
    let sw2 = sub.len() as i32 * 8;
    draw_text_px(cx - sw2/2, cy + r_big + 30, sub, 0xAEA79F);
}

// ═══════════════ Рабочий стол — иконки ═══════════════

struct DesktopIcon {
    name: &'static [u8],
    bgra: &'static [u8; 4096],
}

static DESKTOP_ICONS: &[DesktopIcon] = &[
    DesktopIcon { name: b"Home", bgra: &icons::ICON_FOLDER_BGRA },
    DesktopIcon { name: b"Trash", bgra: &icons::ICON_TRASH_BGRA },
];

fn draw_desktop_icons() {
    if !unsafe { SHOW_DESKTOP_ICONS } { return; }
    let ix: i32 = DOCK_W as i32 + 20;
    let mut iy: i32 = TOPBAR_H as i32 + 20;
    for di in DESKTOP_ICONS {
        draw_icon_bgra32(ix, iy, di.bgra);
        draw_text_centered(ix, iy + 36, 32, di.name, C_TEXT);
        iy += 64;
    }
}

// ═══════════════ Ubuntu Top Bar + Left Dock (Yaru) ═══════════════

fn draw_panel() {
    let w = sw() as i32; let h = sh() as i32;
    let mx = mouse::x(); let my = mouse::y();
    // ── Top bar ──
    fill_rect(0, 0, w as u32, TOPBAR_H, C_PANEL);
    // Activities (left)
    let act_w=88i32; let act_h=TOPBAR_H as i32;
    let hover_act = mx>=0 && mx<act_w && my>=0 && my<act_h;
    let act_bg = if unsafe{SHOW_START}{C_ACCENT} else if hover_act{C_HOVER}else{C_PANEL};
    fill_rect(0, 0, act_w as u32, TOPBAR_H, act_bg);
    draw_text_px(10, 8, b"Activities", if hover_act||unsafe{SHOW_START}{C_TEXT}else{C_DIM});
    // Center date-time (e.g., "Sep 05 12:34")
    let t = crate::driver::rtc::read();
    let mut cbuf=[b'0';5];
    cbuf[0]=b'0'+t.hour/10; cbuf[1]=b'0'+t.hour%10; cbuf[2]=b':'; cbuf[3]=b'0'+t.minute/10; cbuf[4]=b'0'+t.minute%10;
    let mut dbuf=[b'0';6];
    dbuf[0]=b'0'+t.day/10; dbuf[1]=b'0'+t.day%10; dbuf[2]=b'.'; dbuf[3]=b'0'+t.month/10; dbuf[4]=b'0'+t.month%10;
    // centered clock
    let center_x = w/2 - 40;
    draw_text_px(center_x, 6, &cbuf, C_TEXT);
    draw_text_px(center_x+48, 6, &dbuf, C_DIM);
    // Right: system tray + user
    let mut rx = w - 8;
    // power icon (simple)
    rx-=18; fill_rect(rx, 10, 8, 8, C_DIM); fill_rect(rx+2, 8, 4, 4, C_PANEL);
    // network + volume like before but smaller
    rx-=22; for i in 0..5{ let bh=(i+1)*3; fill_rect(rx + i*4, 18 - bh, 2, bh as u32, C_DIM); }
    rx-=22; fill_rect(rx+2,12,2,6,C_DIM); fill_rect(rx+5,10,2,10,C_DIM); fill_rect(rx+8,12,2,6,C_DIM);
    rx-=8; fill_rect(rx,6,1,TOPBAR_H-12, C_PANEL_B);
    // user
    rx-=80;
    let uname=crate::user::current_name();
    let uw = (uname.len() as i32)*6; // 6px per char approx 8 but smaller for top bar
    draw_text_px(rx, 8, uname.as_bytes(), C_TEXT);
    // ubuntu version
    draw_text_px(w-80, 8, b"", C_DIM);

    // ── Left Dock ──
    fill_rect(0, TOPBAR_H as i32, DOCK_W, (h - TOPBAR_H as i32) as u32, C_DOCK_BG);
    // vertical separator
    fill_rect(DOCK_W as i32 -1, TOPBAR_H as i32, 1, (h - TOPBAR_H as i32) as u32, 0x000000);
    let dock_icons: &[(&[u8], &Icon16)] = &[
        (b"Firefox", &ICON_DOLPHIN), // reuse dolphin as firefox orange
        (b"Files", &ICON_DOLPHIN),
        (b"Terminal", &ICON_TERMINAL),
        (b"Settings", &ICON_SETTINGS),
    ];
    let iy = TOPBAR_H as i32 + 12;
    for (idx, (_name, icon)) in dock_icons.iter().enumerate() {
        let y = iy + idx as i32 * 56;
        let x = 8;
        let hover = mx>=x && mx< x+48 && my>=y && my<y+48;
        let is_active = idx==0 && unsafe{WIN_FOCUSED && !WIN_MINIMIZED};
        let bg = if hover {C_DOCK_HOVER} else {C_DOCK_BG};
        fill_rect(x, y, 48, 48, bg);
        if is_active { fill_rect(0, y+8, 3, 32, C_ACCENT); }
        if hover { fill_rect(0, y+8, 3, 32, C_ACCENT); }
        draw_icon(x+16, y+16, icon);
    }
    // Show Apps 9 dots at bottom
    let apps_y = h - 64;
    let ax = 8;
    let hover_apps = mx>=ax && mx<ax+48 && my>=apps_y && my<apps_y+48;
    let apps_bg = if unsafe{SHOW_START}{C_ACCENT} else if hover_apps{C_DOCK_HOVER}else{C_DOCK_BG};
    fill_rect(ax, apps_y, 48, 48, apps_bg);
    // 9 dots grid
    for row in 0..3{ for col in 0..3{ fill_rect(ax+12+col*8, apps_y+12+row*8, 4,4, C_TEXT); } }
    if unsafe{SHOW_START}{ fill_rect(0, apps_y+8, 3,32, C_ACCENT); }

    // ── Start-menu (Ubuntu App Grid) — anchored next to dock bottom
    if unsafe { SHOW_START } {
        // keep original menu at bottom to the right of dock, but offset y to bottom
        let py = h - 0; // bottom
        draw_start_menu(mx, my, py);
    }
}

// ═══════════════ Старт-меню (KDE App Launcher стиль) ═══════════════

fn draw_start_menu(mx: i32, my: i32, panel_y: i32) {
    let mw = 340i32; let mh = 440i32;
    let menu_x = 0i32;
    let menu_y = panel_y - mh;

    // Тень
    fill_rect(menu_x + 6, menu_y + 6, mw as u32, mh as u32, 0x0a0a0a);
    // Фон
    fill_rect(menu_x, menu_y, mw as u32, mh as u32, C_MENU_BG);
    // Border
    fill_rect(menu_x, menu_y, mw as u32, 1, C_WIN_BDR);
    fill_rect(menu_x + mw - 1, menu_y, 1, mh as u32, C_WIN_BDR);
    fill_rect(menu_x, menu_y + mh - 1, mw as u32, 1, C_WIN_BDR);

    // Поиск
    fill_rect(menu_x + 12, menu_y + 10, (mw - 24) as u32, 30, C_BG_LIGHT);
    draw_text_px(menu_x + 20, menu_y + 17, b"Search applications...", C_DIM);

    // ═══ Категории ═══
    let cats: &[(&[u8], &[&[u8]])] = &[
        (b"System", &[b"Konsole", b"Dolphin", b"Settings"]),
        (b"Accessories", &[b"Kate", b"Calculator"]),
        (b"Power / Session", &[b"Shutdown", b"Reboot", b"Logout"]),
    ];

    let mut iy = menu_y + 50;
    for (cat_name, items) in cats {
        draw_text_px(menu_x + 12, iy, cat_name, C_ACCENT);
        fill_rect(menu_x + 12, iy + 14, (mw - 24) as u32, 1, C_MENU_SEP);
        iy += 18;

        for item in *items {
            let item_h = 28i32;
            let hover = mx >= menu_x && mx < menu_x + mw && my >= iy && my < iy + item_h;
            if hover {
                fill_rect(menu_x + 8, iy, (mw - 16) as u32, item_h as u32, C_HOVER);
            }
            draw_text_px(menu_x + 20, iy + 6, item, if hover { C_ACCENT } else { C_TEXT });
            iy += item_h;
        }
        iy += 8;
    }

    // Кнопка пользователя внизу — dynamic
    let uy = menu_y + mh - 40;
    fill_rect(menu_x + 8, uy, (mw - 16) as u32, 30, C_BG_LIGHT);
    let uname = crate::user::current_name();
    let mut ubuf=[0u8;40];
    let mut ul=0usize;
    for &c in uname.as_bytes(){ if ul<30{ubuf[ul]=c; ul+=1;}}
    if ul+6 < 40 {
        ubuf[ul]=b'@'; ul+=1;
        let suf=b"DBSos"; ubuf[ul..ul+5].copy_from_slice(suf); ul+=5;
    }
    draw_text_px(menu_x + 16, uy + 8, &ubuf[..ul], C_TEXT);
    draw_text_px(menu_x + mw - 80, uy + 8, b"v0.1", C_DIM);
    // small uid hint
    let mut idbuf=[b'0';8];
    let uid=crate::user::current_uid();
    let n: usize;
    if uid==0 { idbuf[0]=b'0'; n=1; } else { let mut tmp=[0u8;10]; let mut ti=0; let mut v2=uid; while v2>0{tmp[ti]=b'0'+(v2%10)as u8; v2/=10; ti+=1;} for i in 0..ti{ idbuf[i]=tmp[ti-1-i]; } n=ti; }
    draw_text_px(menu_x + mw - 120, uy + 8, b"uid:", C_DIM);
    draw_text_px(menu_x + mw - 88, uy + 8, &idbuf[..n], C_DIM);
}

// ═══════════════ Контекстное меню ═══════════════

fn draw_context_menu(mx: i32, my: i32) {
    if !unsafe { RIGHT_CLICK_MENU } { return; }
    let cx = unsafe { RIGHT_CLICK_X };
    let cy = unsafe { RIGHT_CLICK_Y };
    let mw = 180u32; let mh = 130u32;

    fill_rect(cx+4, cy+4, mw, mh, 0x0a0a0a);
    fill_rect(cx, cy, mw, mh, C_MENU_BG);
    fill_rect(cx, cy, mw, 1, C_WIN_BDR);
    fill_rect(cx+mw as i32-1, cy, 1, mh, C_WIN_BDR);
    fill_rect(cx, cy+mh as i32-1, mw, 1, C_WIN_BDR);

    let items: &[&[u8]] = &[b"New Folder", b"New File", b"Open Terminal", b"---", b"Paste", b"Properties", b"Change Background"];
    let mut iy = cy + 6;
    for item in items {
        if *item == b"---" {
            fill_rect(cx + 8, iy + 2, mw - 16, 1, C_MENU_SEP);
            iy += 8;
            continue;
        }
        let hover = mx >= cx && mx < cx + mw as i32 && my >= iy && my < iy + 22;
        if hover { fill_rect(cx + 4, iy, mw - 8, 22, C_HOVER); }
        draw_text_px(cx + 14, iy + 4, item, if hover { C_ACCENT } else { C_TEXT });
        iy += 22;
    }
}

// ═══════════════ Окно ═══════════════

/// Вычислить bounding rect текущего окна в экранных координатах
/// (включая заголовок). Возвращает (x, y, w, h).
fn window_screen_rect() -> (i32, i32, u32, u32) {
    unsafe {
        if WIN_MINIMIZED { return (0, 0, 0, 0); }
        if WIN_MAXIMIZED { return (DOCK_W as i32, TOPBAR_H as i32, sw() - DOCK_W, sh() - TOPBAR_H); }
        let slide = WIN_ANIM_SLIDE_Y;
        let y = WIN_Y - WIN_HDR as i32 + slide;
        let h = WIN_H + WIN_HDR;
        (WIN_X, y, WIN_W, h)
    }
}

fn draw_window() {
    if unsafe { WIN_MINIMIZED } { return; }
    let x = unsafe { WIN_X };
    let y = unsafe { WIN_Y };
    let w = unsafe { WIN_W };
    let h = unsafe { WIN_H };
    let focused = unsafe { WIN_FOCUSED };
    let slide_y = unsafe { WIN_ANIM_SLIDE_Y };

    let (wx, wy, ww, wh) = if unsafe { WIN_MAXIMIZED } {
        (DOCK_W as i32, TOPBAR_H as i32, sw() - DOCK_W, sh() - TOPBAR_H)
    } else {
        (x, y + slide_y, w, h)
    };

    // Тень (KDE Breeze — размытая, 4px смещение)
    if !unsafe { WIN_MAXIMIZED } {
        fill_rect(wx + 3, wy + wh as i32 + 3, ww, 4, 0x0a0a0a);
        fill_rect(wx + ww as i32 + 3, wy + 3, 4, wh, 0x0a0a0a);
        fill_rect(wx + 4, wy + wh as i32 + 4, ww, 3, 0x050505);
        fill_rect(wx + ww as i32 + 4, wy + 4, 3, wh, 0x050505);
    }

    // Заголовок
    let hdr_bg = if focused { C_WIN_HDR } else { C_WIN_HDR_UNF };
    fill_rect(wx, wy - WIN_HDR as i32, ww, WIN_HDR, hdr_bg);

    // Иконка + название
    draw_icon(wx + 6, wy - WIN_HDR as i32 + 7, &ICON_TERMINAL);
    draw_text_px(wx + 28, wy - 7, b"Konsole - DBSos", if focused { C_TEXT } else { C_DIM });

    // Yaru window controls — circles (Ubuntu style) справа
    let btn_cy = wy - WIN_HDR as i32 + (WIN_HDR as i32)/2;
    let br = 9i32;
    let close_cx = wx + ww as i32 - 18;
    let max_cx = close_cx - 24;
    let min_cx = max_cx - 24;
    let mx = mouse::x(); let my = mouse::y();
    let hover_close = (mx-close_cx)*(mx-close_cx)+(my-btn_cy)*(my-btn_cy) <= br*br;
    let hover_max = (mx-max_cx)*(mx-max_cx)+(my-btn_cy)*(my-btn_cy) <= br*br;
    let hover_min = (mx-min_cx)*(mx-min_cx)+(my-btn_cy)*(my-btn_cy) <= br*br;
    // close — Ubuntu orange/red
    draw_circle_filled(close_cx, btn_cy, br, if hover_close { C_CLOSE_H } else { C_CLOSE });
    draw_text_px(close_cx-3, btn_cy-5, b"x", 0xFFFFFF);
    // maximize — grey with square
    draw_circle_filled(max_cx, btn_cy, br, if hover_max { C_HOVER } else { C_WIN_BTN });
    // square icon inside
    fill_rect(max_cx-4, btn_cy-4, 8, 8, C_DIM);
    fill_rect(max_cx-3, btn_cy-3, 6, 6, if hover_max { C_HOVER } else { C_WIN_BTN });
    // minimize — grey with dash
    draw_circle_filled(min_cx, btn_cy, br, if hover_min { C_HOVER } else { C_WIN_BTN });
    draw_text_px(min_cx-3, btn_cy-5, b"-", C_DIM);

    // Тело окна
    fill_rect(wx, wy, ww, wh, C_WIN_BG);

    // ═══ Содержимое терминала ═══
    let mut ly = wy + 8;
    unsafe {
        for i in 0..TERM_LINE_COUNT {
            draw_text_px(wx + 10, ly, &TERM_LINES[i][..TERM_LINES_LEN[i]], C_DIM);
            ly += 16;
            if ly + 16 > wy + wh as i32 - 24 { break; }
        }
        let prompt = b"root@DBSos$ ";
        draw_text_px(wx + 10, ly, prompt, C_ACCENT);
        let mut cx = wx + 10 + prompt.len() as i32 * 8;
        for i in 0..TERM_LEN {
            draw_char_px(cx, ly, TERM_BUF[i], C_TEXT);
            cx += 8;
        }
        if (timer::millis() / 500) % 2 == 0 {
            fill_rect(cx, ly + 14, 8, 2, C_ACCENT);
        }
        if TERM_LINE_COUNT == 0 && TERM_LEN == 0 {
            draw_text_px(wx + 10, wy + 10, b"DBSos Konsole v0.1", C_DIM);
            draw_text_px(wx + 10, wy + 30, b"Type 'help' for commands", 0x4a5058);
        }
    }
}

// ═══════════════ Команды терминала ═══════════════

fn push_line(s: &[u8]) {
    unsafe {
        if TERM_LINE_COUNT < 16 {
            let n = s.len().min(80);
            TERM_LINES[TERM_LINE_COUNT][..n].copy_from_slice(&s[..n]);
            TERM_LINES_LEN[TERM_LINE_COUNT] = n;
            TERM_LINE_COUNT += 1;
        } else {
            for i in 1..16 { TERM_LINES[i-1] = TERM_LINES[i]; TERM_LINES_LEN[i-1] = TERM_LINES_LEN[i]; }
            let n = s.len().min(80);
            TERM_LINES[15][..n].copy_from_slice(&s[..n]);
            TERM_LINES_LEN[15] = n;
        }
    }
}

fn exec_term_cmd(cmd: &[u8]) {
    if cmd.is_empty() { return; }
    push_line(cmd);
    if cmd == b"help" {
        push_line(b"  help        - this text");
        push_line(b"  clear       - clear screen");
        push_line(b"  echo X      - print text");
        push_line(b"  ls          - list files");
        push_line(b"  cat X       - show file");
        push_line(b"  uname       - system info");
        push_line(b"  whoami      - current user");
        push_line(b"  date        - date and time");
        push_line(b"  neofetch    - system info");
    } else if cmd == b"clear" {
        unsafe { TERM_LINE_COUNT = 0; }
    } else if cmd.starts_with(b"echo ") {
        push_line(&cmd[5..]);
    } else if cmd == b"ls" {
        push_line(b"  Desktop/  Documents/  Downloads/");
        push_line(b"  Music/    Pictures/   Videos/");
        push_line(b"  .bashrc   .profile");
    } else if cmd == b"uname" || cmd == b"uname -a" {
        push_line(b"  DBSos 0.1.0 x86_64 UEFI Plasma/Breeze");
    } else if cmd == b"whoami" {
        push_line(b"  root");
    } else if cmd == b"date" {
        let t = crate::driver::rtc::read();
        let mut buf = [0u8; 24];
        buf[0] = b'0'+t.day/10; buf[1] = b'0'+t.day%10;
        buf[2] = b'.'; buf[3] = b'0'+t.month/10; buf[4] = b'0'+t.month%10;
        buf[5] = b'.'; buf[6] = b'2'; buf[7] = b'0'; buf[8] = b'2'; buf[9] = b'6';
        buf[10] = b' '; buf[11] = b'0'+t.hour/10; buf[12] = b'0'+t.hour%10;
        buf[13] = b':'; buf[14] = b'0'+t.minute/10; buf[15] = b'0'+t.minute%10;
        buf[16] = b':'; buf[17] = b'0'+t.second/10; buf[18] = b'0'+t.second%10;
        push_line(&buf[..19]);
    } else if cmd == b"neofetch" {
        push_line(b"  DBSos 0.1.0 - Plasma/Breeze Dark");
        push_line(b"  Kernel:  x86_64 UEFI bare-metal");
        push_line(b"  Display: GOP double-buffered");
        push_line(b"  Shell:   Konsole (Plasma)");
        push_line(b"  Memory:  16 MiB heap");
        push_line(b"  CPU:     x86_64 (QEMU)");
    } else if cmd.starts_with(b"cat ") {
        push_line(b"  cat: file not found");
    } else {
        let mut tmp = [0u8; 80];
        let pre = b"  unknown: ";
        tmp[..pre.len()].copy_from_slice(pre);
        let mut n2 = pre.len();
        let cn = cmd.len().min(80 - n2 - 4);
        tmp[n2..n2+cn].copy_from_slice(&cmd[..cn]);
        n2 += cn;
        push_line(&tmp[..n2]);
    }
}

// ═══════════════ Анимации ═══════════════

fn update_animations() {
    unsafe {
        if WIN_ANIM_SLIDE_Y == WIN_ANIM_SLIDE_TARGET { return; }
        if WIN_ANIM_SLIDE_Y < WIN_ANIM_SLIDE_TARGET {
            WIN_ANIM_SLIDE_Y = (WIN_ANIM_SLIDE_Y + 40).min(WIN_ANIM_SLIDE_TARGET);
        } else {
            WIN_ANIM_SLIDE_Y = (WIN_ANIM_SLIDE_Y - 40).max(WIN_ANIM_SLIDE_TARGET);
        }
    }
}

fn animate_window_open() {
    unsafe { WIN_ANIM_SLIDE_Y = 30; WIN_ANIM_SLIDE_TARGET = 0; }
}

fn animate_window_minimize() {
    unsafe { WIN_ANIM_SLIDE_TARGET = 500; }
}

fn animate_window_restore() {
    unsafe { WIN_ANIM_SLIDE_Y = 500; WIN_ANIM_SLIDE_TARGET = 0; }
}

// ═══════════════ Entry points ═══════════════

pub fn init() {
    crate::driver::uart::write_str("[PLASMA] Breeze Dark desktop initialized\r\n");
}

pub fn run() {
    crate::driver::uart::write_str("[PLASMA] desktop enter\r\n");
    crate::system::set(crate::system::Level::L2Plasma);
    unsafe {
        TERM_LEN = 0; TERM_LINE_COUNT = 0; WIN_FOCUSED = true;
        WIN_MINIMIZED = false; WIN_MAXIMIZED = false;
        SHOW_START = false; RIGHT_CLICK_MENU = false;
        WIN_X = 300; WIN_Y = 120; WIN_W = 640; WIN_H = 400;
        PREV_WIN_X = WIN_X; PREV_WIN_Y = WIN_Y - WIN_HDR as i32;
        PREV_WIN_W = WIN_W; PREV_WIN_H = WIN_H + WIN_HDR;
        PREV_WIN_MINIMIZED = true; PREV_WIN_MAXIMIZED = false;
    }
    animate_window_open();

    // Ubuntu: no WP_CACHE — full redraw each frame avoids triple/ghosting
    // Первый кадр — Yaru wallpaper + icons + Wayland windows + panel (panel on top)
    draw_wallpaper();
    draw_desktop_icons();
    draw_window();
    crate::gfx::composite_surfaces_onto_current_fb();
    draw_panel();
    let mx = mouse::x(); let my = mouse::y();
    save_cursor_backing(mx, my);
    draw_cursor(mx, my);
    unsafe {
        CURSOR_LAST_X = mx;
        CURSOR_LAST_Y = my;
        display::mark_dirty(0, 0, sw(), sh());
        display::present();
    }

    let mut last_present = 0u64;
    let mut last_click = 0u64;
    let start_ms = timer::millis();

    loop {
        let mx = mouse::x(); let my = mouse::y();
        let lb = mouse::left();
        let rb = mouse::right();
        let now = timer::millis();

        // ═══ Клик-детекция (debounce + ignore first 800ms to avoid phantom click at boot) ═══
        let clicked = lb && now.saturating_sub(last_click) > 50 && now.saturating_sub(start_ms) > 800;
        let right_clicked = rb && now.saturating_sub(last_click) > 200 && now.saturating_sub(start_ms) > 800;

        // ═══ Курсор: ресайз за угол окна ═══
        if !unsafe { WIN_MINIMIZED } && !unsafe { WIN_MAXIMIZED } {
            let wx = unsafe { WIN_X };
            let wy = unsafe { WIN_Y };
            let ww = unsafe { WIN_W } as i32;
            let wh = unsafe { WIN_H } as i32;
            let corner_x = wx + ww - 8;
            let corner_y = wy + wh - 8;
            if lb && !unsafe { DRAGGING } && !unsafe { RESIZING } {
                if mx >= corner_x && mx <= corner_x + 16 && my >= corner_y && my <= corner_y + 16 {
                    unsafe { RESIZING = true; }
                }
            }
        }

        // ═══ Перетаскивание окна ═══
        unsafe {
            if !WIN_MINIMIZED && !WIN_MAXIMIZED {
                let hx = WIN_X; let hy = WIN_Y - WIN_HDR as i32;
                let hw = WIN_W as i32; let hh = WIN_HDR as i32;
                let over_header = mx >= hx && mx < hx+hw && my >= hy && my < hy+hh;
                if clicked && over_header && !DRAGGING && !RESIZING {
                    DRAGGING = true;
                    DRAG_OX = mx - WIN_X;
                    DRAG_OY = my - WIN_Y;
                    WIN_FOCUSED = true;
                } else if !lb && DRAGGING {
                    DRAGGING = false;
                }
                if DRAGGING {
                    WIN_X = mx - DRAG_OX;
                    WIN_Y = my - DRAG_OY;
                    if WIN_X < DOCK_W as i32 { WIN_X = DOCK_W as i32; }
                    if WIN_Y < (TOPBAR_H + WIN_HDR) as i32 { WIN_Y = (TOPBAR_H + WIN_HDR) as i32; }
                    if WIN_X + WIN_W as i32 > sw() as i32 { WIN_X = sw() as i32 - WIN_W as i32; }
                    if WIN_Y + WIN_H as i32 > sh() as i32 {
                        WIN_Y = sh() as i32 - WIN_H as i32;
                    }
                }
                if lb && RESIZING {
                    let max_w = (sw() as i32 - WIN_X).max(320) as u32;
                    let max_h = (sh() as i32 - WIN_Y).max(200) as u32;
                    WIN_W = (mx as u32 - WIN_X as u32).max(320).min(max_w);
                    WIN_H = (my as u32 - WIN_Y as u32).max(200).min(max_h);
                }
                if !lb { RESIZING = false; }
            }
        }

        // ═══ Yaru кнопки — кружки ═══
        if clicked {
            unsafe {
                if !WIN_MINIMIZED {
                    let wx = WIN_X; let wy = WIN_Y;
                    let ww = WIN_W as i32;
                    let btn_cy = wy - WIN_HDR as i32 + (WIN_HDR as i32)/2;
                    let br=9i32;
                    let close_cx = wx + ww - 18;
                    let max_cx = close_cx - 24;
                    let min_cx = max_cx - 24;
                    let dc2 = (mx-close_cx)*(mx-close_cx)+(my-btn_cy)*(my-btn_cy);
                    if dc2 <= br*br { break; }
                    let dm2 = (mx-min_cx)*(mx-min_cx)+(my-btn_cy)*(my-btn_cy);
                    if dm2 <= br*br { WIN_MINIMIZED=true; animate_window_minimize(); }
                    let dx2 = (mx-max_cx)*(mx-max_cx)+(my-btn_cy)*(my-btn_cy);
                    if dx2 <= br*br { WIN_MAXIMIZED=!WIN_MAXIMIZED; }
                }
            }
        }

        // ═══ Кнопка старт-меню + handling Power items ═══
        if clicked {
            let py = sh() as i32 - PANEL_H as i32;
            let kx = 0i32; let ky = py + 2;
            let menu_x=0i32; let menu_y=py-440; let mw=340i32; // must match draw
            let mut handled=false;
            if unsafe { SHOW_START } && mx>=menu_x && mx<menu_x+mw && my>=menu_y && my< py {
                // compute which item was clicked — replicate draw layout
                let mut iy = menu_y + 50;
                let cats: &[(&[u8], &[&[u8]])] = &[
                    (b"System", &[b"Konsole", b"Dolphin", b"Settings"]),
                    (b"Accessories", &[b"Kate", b"Calculator"]),
                    (b"Power / Session", &[b"Shutdown", b"Reboot", b"Logout"]),
                ];
                'outer: for (_, items) in cats {
                    iy+=18;
                    for item in *items {
                        let item_h=28i32;
                        if my>=iy && my< iy+item_h && mx>=menu_x+8 && mx<menu_x+mw-8 {
                            handled=true;
                            if *item==b"Shutdown" { unsafe{ SHOW_START=false;} crate::acpi::shutdown(); }
                            else if *item==b"Reboot" { unsafe{ SHOW_START=false;} crate::acpi::reboot(); }
                            else if *item==b"Logout" { unsafe{ SHOW_START=false;} break 'outer; } // will break outer and then break plasma loop via flag
                            else { unsafe{ SHOW_START=false;} }
                            break 'outer;
                        }
                        iy+=item_h;
                    }
                    iy+=8;
                }
                if handled {
                    // check if Logout was clicked — we detect by item == Logout already, need to exit plasma
                    // We know Logout is last item; if we handled Logout, break plasma loop
                    // Instead of complex, if my is in Logout row, break
                    // Recompute Logout row y
                    // System 3*28 + Accessories 2*28 = 5 items before Power header
                    // Simpler: if item was Logout, break
                    // We break by checking if we handled and my in last rows and item == Logout; we already broke outer but need to decide to exit plasma
                    // Detect Logout: y range  ~ menu_y+50 + 3*header(18) + 8*2 + items
                    // Approximate: if handled and last cat Power and last item, do logout
                    // For now, if handled and my near bottom Power items, check last
                    let logout_y_start = menu_y + 50 + 18*3 + 8*2 + 28*5; // rough
                    if my >= logout_y_start && my < logout_y_start+28 {
                        break;
                    }
                }
            }
            if !handled {
                // Activities top bar
                let act_w=88i32; let act_h=TOPBAR_H as i32;
                if mx>=0 && mx<act_w && my>=0 && my<act_h {
                    unsafe { SHOW_START = !SHOW_START; }
                } else {
                    // Dock Show Apps 9 dots at bottom
                    let dock_apps_y = sh() as i32 - 64;
                    if mx>=8 && mx<56 && my>=dock_apps_y && my<dock_apps_y+48 {
                        unsafe { SHOW_START = !SHOW_START; }
                    } else if unsafe { SHOW_START } {
                        unsafe { SHOW_START = false; }
                    }
                }
            }
            // Dock — restore minimized window via Terminal icon (index 2)
            if unsafe { WIN_MINIMIZED } {
                let term_y = TOPBAR_H as i32 + 12 + 2*56;
                if mx>=8 && mx<56 && my>=term_y && my<term_y+48 {
                    unsafe { WIN_MINIMIZED = false; WIN_FOCUSED = true; }
                    animate_window_restore();
                }
            }
        }

        // ═══ Правый клик — контекстное меню (не на top bar/dock) ═══
        if right_clicked {
            if my >= TOPBAR_H as i32 && mx >= DOCK_W as i32 {
                unsafe { RIGHT_CLICK_MENU = true; RIGHT_CLICK_X = mx; RIGHT_CLICK_Y = my; }
            }
        }
        if clicked && unsafe { RIGHT_CLICK_MENU } {
            unsafe { RIGHT_CLICK_MENU = false; }
        }

        // ═══ Фокус ═══
        if clicked {
            let in_top = my < TOPBAR_H as i32;
            let in_dock = mx < DOCK_W as i32 && my >= TOPBAR_H as i32;
            if in_top || in_dock {
                unsafe { WIN_FOCUSED = false; }
            } else if !unsafe { SHOW_START } {
                let in_window = unsafe {
                    !WIN_MINIMIZED && mx >= WIN_X && mx < WIN_X + WIN_W as i32
                        && my >= WIN_Y - WIN_HDR as i32 && my < WIN_Y + WIN_H as i32
                };
                if in_window {
                    unsafe { WIN_FOCUSED = true; WIN_MINIMIZED = false; }
                } else {
                    unsafe { WIN_FOCUSED = false; }
                }
            }
        }

        // ═══ Ввод ═══
        if let Some(c) = ps2::poll_char() {
            if c == 27 || c == ps2::KEY_UP || c == ps2::KEY_DOWN
               || c == ps2::KEY_LEFT || c == ps2::KEY_RIGHT {
                unsafe { SHOW_START = false; RIGHT_CLICK_MENU = false; }
            }
            if unsafe { WIN_FOCUSED && !WIN_MINIMIZED } {
                match c {
                    0x08 => { unsafe { if TERM_LEN > 0 { TERM_LEN -= 1; } } }
                    0x0a => {
                        let len = unsafe { TERM_LEN };
                        let mut tmp = [0u8; 128];
                        unsafe { tmp[..len].copy_from_slice(&TERM_BUF[..len]); }
                        unsafe { TERM_LEN = 0; }
                        exec_term_cmd(&tmp[..len]);
                    }
                    ps2::KEY_DELETE => { unsafe { TERM_LEN = 0; } }
                    ps2::KEY_LEFT => { unsafe { if TERM_LEN > 0 { TERM_LEN -= 1; } } }
                    ps2::KEY_RIGHT => { }
                    ps2::KEY_HOME => { unsafe { TERM_LEN = 0; } }
                    ps2::KEY_END => { }
                    b' '..=b'~' => {
                        unsafe { if TERM_LEN < 127 { TERM_BUF[TERM_LEN] = c; TERM_LEN += 1; } }
                    }
                    _ => {}
                }
            }
        }

        // ═══ Throttle 60fps ═══
        if now.saturating_sub(last_present) < 16 {
            continue;
        }
        last_present = now;
        if clicked { last_click = now; }

        // ═══ Анимации ═══
        update_animations();

        // ═══ Полный перерисовка Ubuntu (убирает троение) ═══
        let (cur_wx, cur_wy, cur_ww, cur_wh) = window_screen_rect();
        // Рисуем фон и иконки каждый кадр — просто и без артефактов
        draw_wallpaper();
        draw_desktop_icons();
        let panel_y = sh() as i32; // для start_menu (привязка к низу)

        // ═══ Отрисовка окна и панели Yaru (панель поверх Wayland) ═══
        draw_window();
        crate::gfx::composite_surfaces_onto_current_fb();
        draw_panel();

        if unsafe { RIGHT_CLICK_MENU } {
            draw_context_menu(mx, my);
        }
        if unsafe { SHOW_START } {
            draw_start_menu(mx, my, panel_y);
        }

        // ═══ Курсор — помечаем обе области dirty для present() ═══
        // Старая позиция курсора (чтобы present() восстановил pixels)
        let old_cx = unsafe { CURSOR_LAST_X };
        let old_cy = unsafe { CURSOR_LAST_Y };
        restore_cursor_backing();
        display::mark_dirty(old_cx, old_cy, 12, 18);
        // Новая позиция курсора
        save_cursor_backing(mx, my);
        draw_cursor(mx, my);
        display::mark_dirty(mx, my, 12, 18);
        unsafe {
            CURSOR_LAST_X = mx;
            CURSOR_LAST_Y = my;
        }

        // ═══ Сохраняем текущую позицию как "предыдущую" ═══
        unsafe {
            PREV_WIN_X = cur_wx;
            PREV_WIN_Y = cur_wy;
            PREV_WIN_W = cur_ww;
            PREV_WIN_H = cur_wh;
            PREV_WIN_MINIMIZED = WIN_MINIMIZED;
            PREV_WIN_MAXIMIZED = WIN_MAXIMIZED;
        }

        // Полный кадр — помечаем всё грязным (убирает троение) и презентуем
        display::mark_dirty(0, 0, sw(), sh());
        unsafe { display::present(); }
    }

    crate::driver::uart::write_str("[PLASMA] exit -> TUI\r\n");
    crate::system::set(crate::system::Level::L1Tui);
    display::clear_screen(0,0,0);
    unsafe { display::present(); }
}

fn prev_y() -> i32 { unsafe { PREV_WIN_Y } }
fn prev_h() -> i32 { unsafe { PREV_WIN_H as i32 } }
