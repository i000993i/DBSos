/// Render — low-level drawing primitives with glass blur support
use crate::display;

/// Integer square root (isqrt)
fn isqrt(n: u32) -> u32 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + n / x) / 2; }
    x
}

// ── Fast filled rect ──────────────────────────────────────────────

pub fn fill_rect(x: u32, y: u32, w: u32, h: u32, color: u32) {
    let sw = display::width();
    let sh = display::height();
    let x0 = x.min(sw); let y0 = y.min(sh);
    let x1 = (x + w).min(sw); let y1 = (y + h).min(sh);
    if x0 >= x1 || y0 >= y1 { return; }
    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;
    for row in y0..y1 {
        display::rect(x0 as usize, row as usize, (x1 - x0) as usize, 1, r, g, b);
    }
}

/// Fill rect with per-pixel alpha blend against current framebuffer content
pub fn fill_rect_alpha(x: u32, y: u32, w: u32, h: u32, color: u32, alpha: u8) {
    if alpha == 0 { return; }
    if alpha == 255 { fill_rect(x, y, w, h, color); return; }
    let sw = display::width();
    let sh = display::height();
    let x0 = x.min(sw); let y0 = y.min(sh);
    let x1 = (x + w).min(sw); let y1 = (y + h).min(sh);
    if x0 >= x1 || y0 >= y1 { return; }
    let sr = ((color >> 16) & 0xFF) as u16;
    let sg = ((color >> 8) & 0xFF) as u16;
    let sb = (color & 0xFF) as u16;
    let a = alpha as u16;
    let ia = 256 - a;
    let fb = display::framebuffer();
    if fb.is_null() { return; }
    let stride = display::width() as usize;
    for row in y0..y1 {
        let off = (row as usize * stride + x0 as usize) * 4;
        for col in 0..(x1 - x0) as usize {
            let px = off + col * 4;
            unsafe {
                let db = *fb.add(px) as u16;
                let dg = *fb.add(px + 1) as u16;
                let dr = *fb.add(px + 2) as u16;
                *fb.add(px)     = ((sb * ia + db * a) >> 8) as u8;
                *fb.add(px + 1) = ((sg * ia + dg * a) >> 8) as u8;
                *fb.add(px + 2) = ((sr * ia + dr * a) >> 8) as u8;
            }
        }
    }
}

// ── Rounded rect ──────────────────────────────────────────────────

pub fn fill_rounded_rect(x: u32, y: u32, w: u32, h: u32, color: u32, radius: u32) {
    if radius == 0 { fill_rect(x, y, w, h, color); return; }
    let sw = display::width();
    let sh = display::height();
    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;
    let r2 = radius * radius;
    for dy in 0..h {
        let py = y + dy;
        if py >= sh { break; }
        let mut x_start = 0u32;
        let mut x_end = w;
        // Check corners
        if dy < radius {
            // Top corners
            let cy = radius - dy;
            let dx = isqrt(r2.saturating_sub(cy * cy));
            x_start = x_start.max(radius.saturating_sub(dx));
            x_end = x_end.min(radius + dx);
        } else if dy + radius >= h {
            // Bottom corners
            let cy = dy + radius - h + 1;
            if cy < radius {
                let dx = isqrt(r2.saturating_sub(cy * cy));
                x_start = x_start.max(radius - dx);
                x_end = x_end.min(radius + dx);
            }
        }
        if x_start >= x_end { continue; }
        let x0 = (x + x_start).min(sw);
        let x1 = (x + x_end).min(sw);
        if x0 < x1 {
            display::rect(x0 as usize, py as usize, (x1 - x0) as usize, 1, r, g, b);
        }
    }
}

// ── Rounded rect with alpha ───────────────────────────────────────

pub fn fill_rounded_rect_alpha(x: u32, y: u32, w: u32, h: u32, color: u32, alpha: u8, radius: u32) {
    if radius == 0 { fill_rect_alpha(x, y, w, h, color, alpha); return; }
    if alpha == 0 { return; }
    let sw = display::width();
    let sh = display::height();
    let sr = ((color >> 16) & 0xFF) as u16;
    let sg = ((color >> 8) & 0xFF) as u16;
    let sb = (color & 0xFF) as u16;
    let a = alpha as u16;
    let ia = 256 - a;
    let fb = display::framebuffer();
    if fb.is_null() { return; }
    let stride = display::width() as usize;
    let r2 = radius * radius;
    for dy in 0..h {
        let py = y + dy;
        if py >= sh { break; }
        let mut x_start = 0u32;
        let mut x_end = w;
        if dy < radius {
            let cy = radius - dy;
            let dx = isqrt(r2.saturating_sub(cy * cy));
            x_start = x_start.max(radius.saturating_sub(dx));
            x_end = x_end.min(radius + dx);
        } else if dy + radius >= h {
            let cy = dy + radius - h + 1;
            if cy < radius {
                let dx = isqrt(r2.saturating_sub(cy * cy));
                x_start = x_start.max(radius.saturating_sub(dx));
                x_end = x_end.min(radius + dx);
            }
        }
        if x_start >= x_end { continue; }
        let x0 = (x + x_start).min(sw) as usize;
        let x1 = (x + x_end).min(sw) as usize;
        for px in x0..x1 {
            let off = py as usize * stride + px;
            let bp = off * 4;
            unsafe {
                let db = *fb.add(bp) as u16;
                let dg = *fb.add(bp + 1) as u16;
                let dr = *fb.add(bp + 2) as u16;
                *fb.add(bp)     = ((sb * ia + db * a) >> 8) as u8;
                *fb.add(bp + 1) = ((sg * ia + dg * a) >> 8) as u8;
                *fb.add(bp + 2) = ((sr * ia + dr * a) >> 8) as u8;
            }
        }
    }
}

// ── Box blur ──────────────────────────────────────────────────────

pub fn blur_region(x: u32, y: u32, w: u32, h: u32, radius: u32) {
    if radius == 0 { return; }
    let sw = display::width();
    let sh = display::height();
    let fb = display::framebuffer();
    if fb.is_null() { return; }
    let stride = sw as usize;
    let x0 = x.min(sw) as usize;
    let y0 = y.min(sh) as usize;
    let x1 = (x + w).min(sw) as usize;
    let y1 = (y + h).min(sh) as usize;
    let r = radius as i32;
    let _area = ((2 * r + 1) * (2 * r + 1)) as u16;

    // Simple box blur pass (horizontal + vertical)
    for py in y0..y1 {
        for px in x0..x1 {
            let mut sr: u32 = 0; let mut sg: u32 = 0; let mut sb: u32 = 0; let mut cnt: u16 = 0;
            for dy in -r..=r {
                for dx in -r..=r {
                    let nx = px as i32 + dx;
                    let ny = py as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= sw as i32 || ny >= sh as i32 { continue; }
                    let off = (ny as usize * stride + nx as usize) * 4;
                    unsafe {
                        sb += *fb.add(off) as u32;
                        sg += *fb.add(off + 1) as u32;
                        sr += *fb.add(off + 2) as u32;
                    }
                    cnt += 1;
                }
            }
            if cnt == 0 { continue; }
            let off = (py * stride + px) * 4;
            unsafe {
                *fb.add(off)     = (sb / cnt as u32) as u8;
                *fb.add(off + 1) = (sg / cnt as u32) as u8;
                *fb.add(off + 2) = (sr / cnt as u32) as u8;
            }
        }
    }
}

// ── Line ──────────────────────────────────────────────────────────

pub fn hline(x: u32, y: u32, w: u32, color: u32) {
    fill_rect(x, y, w, 1, color);
}

pub fn vline(x: u32, y: u32, h: u32, color: u32) {
    fill_rect(x, y, 1, h, color);
}

pub fn draw_rect_border(x: u32, y: u32, w: u32, h: u32, _radius: u32, color: u32) {
    hline(x, y, w, color);
    hline(x, y + h - 1, w, color);
    vline(x, y, h, color);
    vline(x + w - 1, y, h, color);
}

// ── Text (existing 8x8 bitmap font) ──────────────────────────────

pub fn text_width(s: &[u8]) -> u32 { s.len() as u32 * 8 }

pub fn draw_text(x: u32, y: u32, s: &[u8], color: u32) {
    let sw = display::width();
    let r = ((color >> 16) & 0xFF) as u8;
    let g = ((color >> 8) & 0xFF) as u8;
    let b = (color & 0xFF) as u8;
    let mut cx = x;
    for &ch in s {
        if ch == 0 { break; }
        if cx + 8 < sw {
            display::draw_char(cx as usize, y as usize, ch, r, g, b);
        }
        cx += 8;
        if cx >= sw { break; }
    }
}

// ── Drop shadow (approximate: dark rect beneath) ──────────────────

pub fn draw_shadow(x: u32, y: u32, w: u32, h: u32, radius: u32) {
    // Render dark semi-transparent rounded rect slightly larger
    let r = radius;
    let offsets: [(i32, i32, u8); 4] = [
        (-(r as i32), 0, 40),
        (r as i32, 0, 40),
        (0, -(r as i32), 40),
        (0, r as i32, 40),
    ];
    for &(dx, dy, a) in &offsets {
        let sx = (x as i32 + dx).max(0) as u32;
        let sy = (y as i32 + dy).max(0) as u32;
        fill_rounded_rect_alpha(sx, sy, w, h, 0x000000, a, r);
    }
}

// ── Cursor ────────────────────────────────────────────────────────

pub const CURSOR_W: u32 = 12;
pub const CURSOR_H: u32 = 16;

pub fn draw_cursor(mx: u32, my: u32) {
    let sw = display::width();
    let sh = display::height();
    if mx >= sw || my >= sh { return; }
    // Simple arrow cursor bitmap
    const CURSOR: [u16; 16] = [
        0b1000_0000_0000_0000,
        0b1100_0000_0000_0000,
        0b1110_0000_0000_0000,
        0b1111_0000_0000_0000,
        0b1111_1000_0000_0000,
        0b1111_1100_0000_0000,
        0b1111_1110_0000_0000,
        0b1111_1111_0000_0000,
        0b1111_1111_1000_0000,
        0b1111_1111_1100_0000,
        0b1111_1100_0000_0000,
        0b1111_1110_0000_0000,
        0b1101_1110_0000_0000,
        0b1000_1111_0000_0000,
        0b0000_0111_0000_0000,
        0b0000_0011_0000_0000,
    ];
    // Outline (black) then fill (white)
    for row in 0..CURSOR_H {
        let bits = CURSOR[row as usize];
        let py = my + row;
        if py >= sh { break; }
        for col in 0..CURSOR_W {
            let px = mx + col;
            if px >= sw { break; }
            if bits & (0x8000 >> col) != 0 {
                // Draw black outline
                if col > 0 && bits & (0x8000 >> (col - 1)) == 0 {
                    display::draw_pixel((px - 1) as usize, py as usize, 0, 0, 0);
                }
                if row > 0 {
                    let prev = CURSOR[(row - 1) as usize];
                    if prev & (0x8000 >> col) == 0 {
                        display::draw_pixel(px as usize, (py - 1) as usize, 0, 0, 0);
                    }
                }
                display::draw_pixel(px as usize, py as usize, 0xFF, 0xFF, 0xFF);
            }
        }
    }
}

// ── Glass panel helper ────────────────────────────────────────────

pub fn glass_rect(x: u32, y: u32, w: u32, h: u32, color: u32, alpha: u8, radius: u32) {
    blur_region(x, y, w, h, 8);
    fill_rounded_rect_alpha(x, y, w, h, color, alpha, radius);
}
