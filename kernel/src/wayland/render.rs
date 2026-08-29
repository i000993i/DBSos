/// Wayland compositor renderer — composites surfaces into GOP framebuffer
///
/// Handles buffer format conversion, alpha blending, damage-based updates.

use crate::display;
use super::surface::{self, WlBuffer, WlSurface, SurfaceState};

/// Initialize the renderer
pub fn init() {}

/// Composite all surfaces into the framebuffer
pub fn composite() {
    let sw = display::width();
    let sh = display::height();

    // Clear to wallpaper color (GNOME dark)
    display::clear_screen(0x24, 0x24, 0x24);

    // Draw each surface in order
    surface::for_each_surface(|_idx, surf| {
        if surf.state == SurfaceState::Destroyed { return; }
        if surf.buffer_id == 0 { return; }

        if let Some(buf) = surface::find_buffer(surf.buffer_id) {
            if buf.data_phys == 0 { return; }
            composite_surface(surf, buf, sw, sh);
        }
    });

    // Draw cursor
    draw_cursor();
}

/// Composite a single surface
fn composite_surface(surf: &WlSurface, buf: &WlBuffer, sw: u32, sh: u32) {
    let fb = display::framebuffer();
    let fb_stride = display::width() * 4;

    let surf_x = surf.x.max(0) as u32;
    let surf_y = surf.y.max(0) as u32;
    let surf_w = buf.width.min(sw.saturating_sub(surf_x));
    let surf_h = buf.height.min(sh.saturating_sub(surf_y));

    if surf_w == 0 || surf_h == 0 { return; }

    // Copy pixels from buffer to framebuffer
    for row in 0..surf_h {
        let src_row = row;
        let dst_y = surf_y + row;
        if dst_y >= sh { break; }

        for col in 0..surf_w {
            let src_x = col;
            let dst_x = surf_x + col;
            if dst_x >= sw { break; }

            let src_offset = (src_row * buf.stride + src_x * 4) as u64;
            let src = buf.data_phys + src_offset;

            // Read pixel from buffer (identity-mapped)
            let pixel = unsafe { *(src as *const u32) };

            // BGRA format
            let b = (pixel & 0xFF) as u8;
            let g = ((pixel >> 8) & 0xFF) as u8;
            let r = ((pixel >> 16) & 0xFF) as u8;
            let a = ((pixel >> 24) & 0xFF) as u8;

            if a == 0 { continue; }  // Skip fully transparent

            let dst_offset = (dst_y * fb_stride + dst_x * 4) as usize;

            if a == 255 {
                // Opaque: direct write
                unsafe {
                    *fb.add(dst_offset) = b;
                    *fb.add(dst_offset + 1) = g;
                    *fb.add(dst_offset + 2) = r;
                }
            } else {
                // Alpha blend
                let alpha = a as u32;
                let inv_alpha = 255 - alpha;
                unsafe {
                    let db = *fb.add(dst_offset) as u32;
                    let dg = *fb.add(dst_offset + 1) as u32;
                    let dr = *fb.add(dst_offset + 2) as u32;
                    *fb.add(dst_offset) = ((b as u32 * alpha + db * inv_alpha) / 255) as u8;
                    *fb.add(dst_offset + 1) = ((g as u32 * alpha + dg * inv_alpha) / 255) as u8;
                    *fb.add(dst_offset + 2) = ((r as u32 * alpha + dr * inv_alpha) / 255) as u8;
                }
            }
        }
    }
}

/// Draw a simple cursor
fn draw_cursor() {
    let mx = crate::driver::mouse::x().max(0) as u32;
    let my = crate::driver::mouse::y().max(0) as u32;
    let sw = display::width();
    let sh = display::height();

    if mx >= sw || my >= sh { return; }

    // Simple arrow cursor (12x18 pixels)
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

    let fb = display::framebuffer();
    let fb_stride = display::width() * 4;

    for (i, &c) in CURSOR.iter().enumerate() {
        let cx = (i % 12) as u32;
        let cy = (i / 12) as u32;
        let px = mx + cx;
        let py = my + cy;
        if px >= sw || py >= sh { continue; }

        let (r, g, b) = match c {
            1 => (255, 255, 255),  // White outline
            2 => (0, 0, 0),        // Black fill
            _ => continue,
        };

        let offset = (py * fb_stride + px * 4) as usize;
        unsafe {
            *fb.add(offset) = b;
            *fb.add(offset + 1) = g;
            *fb.add(offset + 2) = r;
        }
    }
}
