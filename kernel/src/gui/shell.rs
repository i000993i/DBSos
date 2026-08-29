/// Desktop Shell — GNOME Shell (clean rewrite, no flicker)
///
/// Architecture:
/// - Full redraw only when windows change (not every mouse move)
/// - Cursor rendered as overlay (never clears background)
/// - Double-buffered cursor (save/restore under cursor)

use super::{theme, render, input, wm};
use crate::display;

static mut OVERVIEW_OPEN: bool = false;
static mut CURSOR_PREV_X: i32 = -1;
static mut CURSOR_PREV_Y: i32 = -1;
static mut CURSOR_SAVE_BUF: [u8; 18 * 24 * 4] = [0; 1728]; // 18x24 max cursor size
static mut CURSOR_SAVE_W: u32 = 0;
static mut CURSOR_SAVE_H: u32 = 0;

const DASH_APPS: &[(&[u8], u8)] = &[
    (b"Terminal",   1),
    (b"Files",      2),
    (b"Editor",     3),
    (b"Settings",   4),
    (b"About",      5),
];

pub fn is_overview_open() -> bool { unsafe { OVERVIEW_OPEN } }
pub fn close_start() { unsafe { OVERVIEW_OPEN = false; } }

// ── Cursor (save/restore overlay — no background redraw needed) ───

pub fn draw_cursor() {
    let mx = crate::driver::mouse::x();
    let my = crate::driver::mouse::y();
    let sw = display::width();
    let sh = display::height();
    let fb = display::framebuffer();
    let stride = sw * 4;

    if mx < 0 || my < 0 || mx >= sw as i32 || my >= sh as i32 { return; }

    // Restore previous cursor area
    unsafe {
        if CURSOR_PREV_X >= 0 && CURSOR_PREV_Y >= 0 && CURSOR_SAVE_W > 0 {
            let sx = CURSOR_PREV_X as u32;
            let sy = CURSOR_PREV_Y as u32;
            let mut off = 0usize;
            for dy in 0..CURSOR_SAVE_H {
                let py = sy + dy;
                if py >= sh { break; }
                for dx in 0..CURSOR_SAVE_W {
                    let px = sx + dx;
                    if px >= sw { break; }
                    let dst = (py * stride + px * 4) as usize;
                    *fb.add(dst) = CURSOR_SAVE_BUF[off];
                    *fb.add(dst + 1) = CURSOR_SAVE_BUF[off + 1];
                    *fb.add(dst + 2) = CURSOR_SAVE_BUF[off + 2];
                    off += 4;
                }
            }
        }
    }

    // Save area under new cursor
    let cw: u32 = 12;
    let ch: u32 = 18;
    let cx = mx as u32;
    let cy = my as u32;

    unsafe {
        CURSOR_PREV_X = mx;
        CURSOR_PREV_Y = my;
        CURSOR_SAVE_W = cw;
        CURSOR_SAVE_H = ch;
        let mut off = 0usize;
        for dy in 0..ch {
            let py = cy + dy;
            if py >= sh { break; }
            for dx in 0..cw {
                let px = cx + dx;
                if px >= sw { break; }
                let src = (py * stride + px * 4) as usize;
                CURSOR_SAVE_BUF[off] = *fb.add(src);
                CURSOR_SAVE_BUF[off + 1] = *fb.add(src + 1);
                CURSOR_SAVE_BUF[off + 2] = *fb.add(src + 2);
                CURSOR_SAVE_BUF[off + 3] = 0;
                off += 4;
            }
        }
    }

    // Draw cursor
    const CURSOR: &[u8] = &[
        1,0,0,0,0,0,0,0,0,0,0,0,
        1,1,0,0,0,0,0,0,0,0,0,0,
        1,2,1,0,0,0,0,0,0,0,0,0,
        1,2,2,1,0,0,0,0,0,0,0,0,
        1,2,2,2,1,0,0,0,0,0,0,0,
        1,2,2,2,2,1,0,0,0,0,0,0,
        1,2,2,2,2,2,1,0,0,0,0,0,
        1,2,2,2,2,2,2,1,0,0,0,0,
        1,2,2,2,2,2,2,2,1,0,0,0,
        1,2,2,2,2,2,2,2,2,1,0,0,
        1,2,2,2,2,2,2,2,2,2,1,0,
        1,2,2,2,2,2,2,2,2,2,2,1,
        1,2,2,2,2,2,1,1,1,1,1,1,
        1,2,2,2,2,1,0,0,0,0,0,0,
        1,2,2,2,1,0,0,0,0,0,0,0,
        1,2,2,1,0,0,0,0,0,0,0,0,
        1,2,1,0,0,0,0,0,0,0,0,0,
        1,1,0,0,0,0,0,0,0,0,0,0,
    ];

    unsafe {
        for (i, &c) in CURSOR.iter().enumerate() {
            let dx = (i % 12) as u32;
            let dy = (i / 12) as u32;
            let px = cx + dx;
            let py = cy + dy;
            if px >= sw || py >= sh { continue; }
            if c == 0 { continue; }

            let (r, g, b) = if c == 1 { (255, 255, 255) } else { (30, 30, 30) };
            let off = (py * stride + px * 4) as usize;
            *fb.add(off) = b;
            *fb.add(off + 1) = g;
            *fb.add(off + 2) = r;
        }
    }
}

pub fn erase_cursor() {
    unsafe {
        if CURSOR_PREV_X >= 0 && CURSOR_PREV_Y >= 0 && CURSOR_SAVE_W > 0 {
            let sw = display::width();
            let sh = display::height();
            let fb = display::framebuffer();
            let stride = sw * 4;
            let sx = CURSOR_PREV_X as u32;
            let sy = CURSOR_PREV_Y as u32;
            let mut off = 0usize;
            for dy in 0..CURSOR_SAVE_H {
                let py = sy + dy;
                if py >= sh { break; }
                for dx in 0..CURSOR_SAVE_W {
                    let px = sx + dx;
                    if px >= sw { break; }
                    let dst = (py * stride + px * 4) as usize;
                    *fb.add(dst) = CURSOR_SAVE_BUF[off];
                    *fb.add(dst + 1) = CURSOR_SAVE_BUF[off + 1];
                    *fb.add(dst + 2) = CURSOR_SAVE_BUF[off + 2];
                    off += 4;
                }
            }
            CURSOR_PREV_X = -1;
            CURSOR_PREV_Y = -1;
        }
    }
}

// ── Top Panel ─────────────────────────────────────────────────────

pub fn draw_panel() {
    let sw = display::width();
    let fb = display::framebuffer();
    let stride = sw * 4;
    let h = theme::PANEL_H;

    // Panel background
    for y in 0..h {
        for x in 0..sw {
            let off = (y * stride + x * 4) as usize;
            unsafe {
                *fb.add(off) = 0x1B;
                *fb.add(off + 1) = 0x1B;
                *fb.add(off + 2) = 0x1B;
            }
        }
    }

    // "Activities" button
    let mx = input::mouse_x();
    let my = input::mouse_y();
    let acts_hover = mx < 100 && my < h as i32;
    if acts_hover || unsafe { OVERVIEW_OPEN } {
        for y in 0..h {
            for x in 0u32..100 {
                let off = (y * stride + x * 4) as usize;
                unsafe {
                    *fb.add(off) = 0x30;
                    *fb.add(off + 1) = 0x30;
                    *fb.add(off + 2) = 0x30;
                }
            }
        }
    }
    render::draw_text(14, 10, b"Activities", 0xFFFFFF);

    // Center clock
    let ms = crate::timer::millis();
    let secs = (ms / 1000) as u64;
    let mins = (secs / 60) % 60;
    let hours = (secs / 3600) % 24;
    let mut clock = [0u8; 5];
    clock[0] = b'0' + (hours / 10) as u8;
    clock[1] = b'0' + (hours % 10) as u8;
    clock[2] = b':';
    clock[3] = b'0' + (mins / 10) as u8;
    clock[4] = b'0' + (mins % 10) as u8;
    render::draw_text((sw / 2) - 24, 10, &clock, 0xBBBBBB);

    // Right side indicators
    render::draw_text(sw - 80, 10, b"V  N", 0x888888);

    // Bottom line
    render::hline(0, h - 1, sw, 0x333333);
}

// ── Dash (left dock) ─────────────────────────────────────────────

pub fn draw_dash() {
    if unsafe { OVERVIEW_OPEN } { return; }

    let sw = display::width();
    let sh = display::height();
    let fb = display::framebuffer();
    let stride = sw * 4;

    let icon_size: u32 = 44;
    let gap: u32 = 4;
    let pad: u32 = 8;
    let count = DASH_APPS.len() as u32;
    let total_h = count * (icon_size + gap) + pad * 2;
    let dx = 0u32;
    let dy = (sh - total_h) / 2;
    let dw = 56;

    // Glass background
    for y in dy..dy + total_h {
        for x in dx..dx + dw {
            if x >= sw || y >= sh { continue; }
            let off = (y * stride + x * 4) as usize;
            unsafe {
                *fb.add(off) = 0x20;
                *fb.add(off + 1) = 0x20;
                *fb.add(off + 2) = 0x20;
            }
        }
    }

    // App icons
    let mx = input::mouse_x();
    let my = input::mouse_y();
    let mut iy = dy + pad;

    for &(label, id) in DASH_APPS {
        let hover = mx >= dx as i32 && mx < (dx + dw) as i32
            && my >= iy as i32 && my < (iy + icon_size) as i32;
        let ix = dx + (dw - icon_size) / 2;

        let bg = if hover { 0x404040 } else { 0x303030 };
        render::fill_rounded_rect(ix, iy, icon_size, icon_size, bg, 8);

        // First 2 chars
        render::draw_text(ix + 10, iy + 18, &label[..2.min(label.len())], 0xFFFFFF);

        // Running indicator
        let mut running = false;
        wm::for_each_window(|_, w| {
            if w.kind == match id {
                1 => wm::WinKind::Terminal,
                2 => wm::WinKind::FileManager,
                3 => wm::WinKind::TextEditor,
                4 => wm::WinKind::Settings,
                _ => wm::WinKind::Custom,
            } { running = true; }
        });
        if running {
            render::fill_rect(dx, iy + 18, 3, 8, theme::ACCENT);
        }

        iy += icon_size + gap;
    }
}

// ── Activities Overview ───────────────────────────────────────────

pub fn draw_overview() {
    if !unsafe { OVERVIEW_OPEN } { return; }

    let sw = display::width();
    let sh = display::height();

    // Dim
    render::fill_rect_alpha(0, 0, sw, sh, 0x000000, 160);

    // Search
    let sx = (sw - 480) / 2;
    let sy = theme::PANEL_H as u32 + 40;
    render::fill_rounded_rect(sx, sy, 480, 44, 0x303030, 12);
    render::draw_text(sx + 16, sy + 16, b"Type to search...", 0x888888);

    // Window thumbnails
    let thumb_w = 260;
    let thumb_h = 160;
    let pad = 16;
    let cols = ((sw - 200) / (thumb_w + pad)).max(1);
    let total_w = cols * (thumb_w + pad) - pad;
    let start_x = (sw - total_w) / 2;
    let mut ox = start_x;
    let mut oy = sy + 44 + 60;
    let mut col = 0u32;

    wm::for_each_window(|_idx, w| {
        if w.state == wm::WinState::Minimized { return; }

        render::fill_rounded_rect(ox, oy, thumb_w, thumb_h, 0x3D3D3D, 6);
        render::fill_rect(ox + 8, oy + 8, thumb_w - 16, 20, 0x303030);
        render::draw_text(ox + 16, oy + 14, &w.title[..w.title_len.min(16)], 0xBBBBBB);
        render::draw_text(ox, oy + thumb_h + 4, &w.title[..w.title_len], 0xFFFFFF);

        let mx = input::mouse_x();
        let my = input::mouse_y();
        if mx >= ox as i32 && mx < (ox + thumb_w) as i32
            && my >= oy as i32 && my < (oy + thumb_h) as i32 {
            render::draw_text(ox, oy + thumb_h + 4, &w.title[..w.title_len], theme::ACCENT);
        }

        col += 1;
        if col >= cols { col = 0; ox = start_x; oy += thumb_h + pad + 20; }
        else { ox += thumb_w + pad; }
    });

    // Bottom dash
    let total_dash_w = DASH_APPS.len() as u32 * 52;
    let dash_x = (sw - total_dash_w) / 2;
    let dash_y = sh - 52 - 40;
    let mut ix = dash_x;
    for &(label, _id) in DASH_APPS {
        let mx = input::mouse_x();
        let my = input::mouse_y();
        let hover = mx >= ix as i32 && mx < (ix + 48) as i32
            && my >= dash_y as i32 && my < (dash_y + 48) as i32;
        let bg = if hover { 0x404040 } else { 0x303030 };
        render::fill_rounded_rect(ix, dash_y, 48, 48, bg, 10);
        render::draw_text(ix + 4, dash_y + 18, &label[..2.min(label.len())], 0xFFFFFF);
        ix += 52;
    }
}

// ── Click handling ────────────────────────────────────────────────

pub fn handle_taskbar_click(mx: u32, my: u32) -> bool {
    if my >= theme::PANEL_H { return false; }
    if mx < 100 {
        unsafe { OVERVIEW_OPEN = !OVERVIEW_OPEN; }
        return true;
    }
    false
}

pub fn handle_start_click(mx: u32, my: u32) -> bool {
    if !unsafe { OVERVIEW_OPEN } { return false; }

    let sw = display::width();
    let sh = display::height();

    // Bottom dash
    let total_dash_w = DASH_APPS.len() as u32 * 52;
    let dash_x = (sw - total_dash_w) / 2;
    let dash_y = sh - 52 - 40;

    let mut ix = dash_x;
    for &(_label, id) in DASH_APPS {
        if mx >= ix && mx < ix + 48 && my >= dash_y && my < dash_y + 48 {
            unsafe { OVERVIEW_OPEN = false; }
            super::launch_app(id);
            return true;
        }
        ix += 52;
    }

    // Window thumbnails
    let thumb_w = 260u32;
    let thumb_h = 160u32;
    let pad = 16u32;
    let cols = ((sw - 200) / (thumb_w + pad)).max(1);
    let total_w = cols * (thumb_w + pad) - pad;
    let start_x = (sw - total_w) / 2;
    let sy = theme::PANEL_H as u32 + 40 + 44 + 60;
    let mut ox = start_x;
    let mut oy = sy;
    let mut col = 0u32;

    wm::for_each_window(|idx, w| {
        if w.state == wm::WinState::Minimized { return; }
        if mx >= ox && mx < ox + thumb_w && my >= oy && my < oy + thumb_h {
            unsafe {
                OVERVIEW_OPEN = false;
                super::wm::toggle_minimize(idx);
            }
        }
        col += 1;
        if col >= cols { col = 0; ox = start_x; oy += thumb_h + pad + 20; }
        else { ox += thumb_w + pad; }
    });

    true
}
