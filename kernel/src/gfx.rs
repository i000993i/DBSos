/// GFX — Graphics Protocol for DBSos Layer System
///
/// Architecture:
///   LEVEL_1_SYSTEM — CLI only (current)
///   LEVEL_2_GFX    — Framebuffer graphics protocol
///   LEVEL_3_COMPOSITOR — Layered compositor
///   LEVEL_4_DESKTOP — Full desktop environment
///
/// This module defines the protocol interface for graphics layers.
/// Each level registers render/input callbacks via function pointers.
use crate::display;

// ── Layer definitions ──────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LayerId {
    Desktop = 0,
    Windows = 1,
    Panel   = 2,
    Overlay = 3,
    Cursor  = 4,
}

// ── Framebuffer abstraction ───────────────────────────────────

pub struct Framebuffer {
    pub ptr: *mut u8,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bpp: u32,
}

impl Framebuffer {
    pub fn from_display() -> Self {
        Self {
            ptr: display::framebuffer(),
            width: display::width(),
            height: display::height(),
            stride: display::stride(),
            bpp: 4,
        }
    }

    pub unsafe fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8) {
        if x >= self.width || y >= self.height { return; }
        let off = (y * self.stride + x * self.bpp) as usize;
        let is_bgr = display::is_bgr();
        if is_bgr {
            *self.ptr.add(off) = b;
            *self.ptr.add(off + 1) = g;
            *self.ptr.add(off + 2) = r;
        } else {
            *self.ptr.add(off) = r;
            *self.ptr.add(off + 1) = g;
            *self.ptr.add(off + 2) = b;
        }
    }

    pub unsafe fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, r: u8, g: u8, b: u8) {
        for dy in 0..h {
            let py = y + dy;
            if py >= self.height { break; }
            for dx in 0..w {
                let px = x + dx;
                if px >= self.width { break; }
                self.set_pixel(px, py, r, g, b);
            }
        }
    }

    pub unsafe fn fill_rect_alpha(&mut self, x: u32, y: u32, w: u32, h: u32, r: u8, g: u8, b: u8, alpha: u8) {
        let a = alpha as u32;
        let inv_a = 255 - a;
        let is_bgr = display::is_bgr();
        for dy in 0..h {
            let py = y + dy;
            if py >= self.height { break; }
            for dx in 0..w {
                let px = x + dx;
                if px >= self.width { break; }
                let off = (py * self.stride + px * self.bpp) as usize;
                let db = *self.ptr.add(off) as u32;
                let dg = *self.ptr.add(off + 1) as u32;
                let dr = *self.ptr.add(off + 2) as u32;
                if is_bgr {
                    *self.ptr.add(off) = ((b as u32 * a + db * inv_a) / 255) as u8;
                    *self.ptr.add(off + 1) = ((g as u32 * a + dg * inv_a) / 255) as u8;
                    *self.ptr.add(off + 2) = ((r as u32 * a + dr * inv_a) / 255) as u8;
                } else {
                    *self.ptr.add(off) = ((r as u32 * a + dr * inv_a) / 255) as u8;
                    *self.ptr.add(off + 1) = ((g as u32 * a + dg * inv_a) / 255) as u8;
                    *self.ptr.add(off + 2) = ((b as u32 * a + db * inv_a) / 255) as u8;
                }
            }
        }
    }

    pub unsafe fn clear(&mut self, r: u8, g: u8, b: u8) {
        self.fill_rect(0, 0, self.width, self.height, r, g, b);
    }

    pub fn present(&self) {
        unsafe { display::present(); }
    }
}

// ── Input events ──────────────────────────────────────────────

pub enum InputEvent {
    KeyPress { scancode: u8, ascii: u8 },
    MouseMove { x: i32, y: i32 },
    MouseDown { x: i32, y: i32, button: MouseButton },
    MouseUp { x: i32, y: i32, button: MouseButton },
}

#[derive(Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

// ── Layer callbacks (function pointers) ───────────────────────

type RenderFn = unsafe fn(&mut Framebuffer);
type InputFn = unsafe fn(&InputEvent) -> bool;
type DirtyFn = unsafe fn() -> bool;

pub struct LayerEntry {
    pub id: LayerId,
    pub render: Option<RenderFn>,
    pub handle_input: Option<InputFn>,
    pub is_dirty: Option<DirtyFn>,
}

// ── Compositor ────────────────────────────────────────────────

static mut COMPOSITOR_LAYERS: [Option<LayerEntry>; 5] = [None, None, None, None, None];
static mut COMPOSITOR_DIRTY: [bool; 5] = [true, true, true, true, true];

pub fn register_layer(entry: LayerEntry) {
    let id = entry.id as usize;
    if id < 5 {
        unsafe {
            COMPOSITOR_LAYERS[id] = Some(entry);
            COMPOSITOR_DIRTY[id] = true;
        }
    }
}

pub fn mark_dirty(layer: LayerId) {
    unsafe { COMPOSITOR_DIRTY[layer as usize] = true; }
}

pub fn composite() {
    let mut fb = Framebuffer::from_display();

    unsafe {
        for i in 0..5 {
            if COMPOSITOR_DIRTY[i] {
                if let Some(ref entry) = COMPOSITOR_LAYERS[i] {
                    if let Some(render_fn) = entry.render {
                        render_fn(&mut fb);
                    }
                }
                COMPOSITOR_DIRTY[i] = false;
            }
        }
    }
    // Wayland surfaces on top (if any)
    wayland_composite(&mut fb);

    fb.present();
}

// ── Wayland Surfaces (Phase A) ──────────────────────────────────

const MAX_SURFACES: usize = 16;

// ── Per-surface input event ring buffer ──────────────────────────

pub const INPUT_RING_SIZE: usize = 64;

#[derive(Copy, Clone)]
pub struct WlInputEvent {
    pub kind: u8,     // 0=none, 1=key, 2=motion, 3=button
    pub key: u32,     // evdev keycode (for key/button)
    pub state: u32,   // 0=released, 1=pressed
    pub time: u32,    // timestamp ms
    pub dx: i32,      // motion delta X
    pub dy: i32,      // motion delta Y
    pub button: u32,  // evdev button code (for button events)
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
struct WlSurface {
    in_use: bool,
    id: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    buf_ptr: *mut u8, // phys mapped as virt (identity)
    buf_w: u32,
    buf_h: u32,
    stride: u32, // bytes per row
    damage: bool,
    #[allow(dead_code)]
    z: u32,
    input_ring: [WlInputEvent; INPUT_RING_SIZE],
    input_read: u16,
    input_write: u16,
}
unsafe impl Send for WlSurface {}
unsafe impl Sync for WlSurface {}

static mut WL_SURFACES: [WlSurface; MAX_SURFACES] = [WlSurface{
    in_use:false, id:0, x:0, y:0, w:0, h:0,
    buf_ptr: core::ptr::null_mut(), buf_w:0, buf_h:0, stride:0, damage:false, z:0,
    input_ring: [WlInputEvent{kind:0, key:0, state:0, time:0, dx:0, dy:0, button:0}; INPUT_RING_SIZE],
    input_read:0, input_write:0,
}; MAX_SURFACES];
static mut WL_NEXT_ID: u32 = 1;
static mut WL_SURFACE_COUNT: usize = 0;

pub fn wl_surface_create(x: i32, y: i32, w: u32, h: u32) -> Option<u32> {
    unsafe {
        if WL_SURFACE_COUNT >= MAX_SURFACES { return None; }
        let slot = (0..MAX_SURFACES).find(|&i| !WL_SURFACES[i].in_use)?;
        let id = WL_NEXT_ID; WL_NEXT_ID+=1;
        WL_SURFACES[slot] = WlSurface{ in_use:true, id, x, y, w, h, buf_ptr: core::ptr::null_mut(), buf_w:0, buf_h:0, stride:0, damage:true, z: slot as u32,
            input_ring: [WlInputEvent{kind:0, key:0, state:0, time:0, dx:0, dy:0, button:0}; INPUT_RING_SIZE],
            input_read:0, input_write:0,
        };
        WL_SURFACE_COUNT+=1;
        Some(id)
    }
}
pub fn wl_surface_destroy(id: u32) -> bool {
    unsafe {
        if let Some(idx) = (0..MAX_SURFACES).find(|&i| WL_SURFACES[i].in_use && WL_SURFACES[i].id==id) {
            WL_SURFACES[idx].in_use=false;
            WL_SURFACE_COUNT-=1;
            return true;
        }
        false
    }
}
pub fn wl_surface_attach(id: u32, phys: u64, w: u32, h: u32, stride: u32) -> bool {
    unsafe {
        if let Some(idx) = (0..MAX_SURFACES).find(|&i| WL_SURFACES[i].in_use && WL_SURFACES[i].id==id) {
            // Ensure phys is mapped in kernel (identity_map_2mb already covers heap region)
            WL_SURFACES[idx].buf_ptr = phys as *mut u8;
            WL_SURFACES[idx].buf_w=w; WL_SURFACES[idx].buf_h=h; WL_SURFACES[idx].stride=stride;
            WL_SURFACES[idx].damage=true;
            return true;
        }
        false
    }
}
pub fn wl_surface_set_position(id: u32, x: i32, y: i32) -> bool {
    unsafe {
        if let Some(idx) = (0..MAX_SURFACES).find(|&i| WL_SURFACES[i].in_use && WL_SURFACES[i].id==id) {
            WL_SURFACES[idx].x=x; WL_SURFACES[idx].y=y; WL_SURFACES[idx].damage=true; return true;
        }
        false
    }
}
pub fn wl_surface_damage(id: u32) -> bool {
    unsafe {
        if let Some(idx) = (0..MAX_SURFACES).find(|&i| WL_SURFACES[i].in_use && WL_SURFACES[i].id==id) {
            WL_SURFACES[idx].damage=true; return true;
        }
        false
    }
}
pub fn wayland_composite(fb: &mut Framebuffer) {
    unsafe {
        let is_bgr = crate::display::is_bgr();
        for i in 0..MAX_SURFACES {
            let s=&WL_SURFACES[i];
            if !s.in_use || s.buf_ptr.is_null() { continue; }
            let src_stride = if s.stride==0 { s.buf_w*4 } else { s.stride };
            for row in 0..s.h.min(s.buf_h) {
                let dst_y = s.y + row as i32;
                if dst_y <0 || dst_y >= fb.height as i32 { continue; }
                let dst_x = s.x;
                let copy_w = s.w.min(s.buf_w).min((fb.width as i32 - dst_x).max(0) as u32);
                if copy_w==0 { continue; }
                for col in 0..copy_w {
                    let src_off = (row * src_stride + col*4) as usize;
                    let dst_px = dst_x + col as i32;
                    if dst_px <0 || dst_px >= fb.width as i32 { continue; }
                    let dst_off = (dst_y as u32 * fb.stride + dst_px as u32 * fb.bpp) as usize;
                    let sb = *s.buf_ptr.add(src_off) as u32;
                    let sg = *s.buf_ptr.add(src_off+1) as u32;
                    let sr = *s.buf_ptr.add(src_off+2) as u32;
                    let sa = *s.buf_ptr.add(src_off+3) as u32;
                    if sa==0 { continue; }
                    if sa==255 {
                        if is_bgr {
                            *fb.ptr.add(dst_off)=sb as u8;
                            *fb.ptr.add(dst_off+1)=sg as u8;
                            *fb.ptr.add(dst_off+2)=sr as u8;
                        } else {
                            *fb.ptr.add(dst_off)=sr as u8;
                            *fb.ptr.add(dst_off+1)=sg as u8;
                            *fb.ptr.add(dst_off+2)=sb as u8;
                        }
                    } else {
                        let inv = 255 - sa;
                        let db = *fb.ptr.add(dst_off) as u32;
                        let dg = *fb.ptr.add(dst_off+1) as u32;
                        let dr = *fb.ptr.add(dst_off+2) as u32;
                        if is_bgr {
                            *fb.ptr.add(dst_off)=((sb*sa + db*inv)/255) as u8;
                            *fb.ptr.add(dst_off+1)=((sg*sa + dg*inv)/255) as u8;
                            *fb.ptr.add(dst_off+2)=((sr*sa + dr*inv)/255) as u8;
                        } else {
                            *fb.ptr.add(dst_off)=((sr*sa + dr*inv)/255) as u8;
                            *fb.ptr.add(dst_off+1)=((sg*sa + dg*inv)/255) as u8;
                            *fb.ptr.add(dst_off+2)=((sb*sa + db*inv)/255) as u8;
                        }
                    }
                }
            }
        }
    }
}
pub fn wl_surface_count() -> usize { unsafe{WL_SURFACE_COUNT} }

// ── Per-surface input event ring ──────────────────────────────────

pub fn push_input_event(id: u32, event: WlInputEvent) -> bool {
    unsafe {
        if let Some(idx) = (0..MAX_SURFACES).find(|&i| WL_SURFACES[i].in_use && WL_SURFACES[i].id==id) {
            let next = (WL_SURFACES[idx].input_write as usize + 1) % INPUT_RING_SIZE;
            if next != WL_SURFACES[idx].input_read as usize {
                WL_SURFACES[idx].input_ring[WL_SURFACES[idx].input_write as usize] = event;
                WL_SURFACES[idx].input_write = next as u16;
                return true;
            }
        }
        false
    }
}

pub fn pop_input_event(id: u32) -> Option<WlInputEvent> {
    unsafe {
        if let Some(idx) = (0..MAX_SURFACES).find(|&i| WL_SURFACES[i].in_use && WL_SURFACES[i].id==id) {
            if WL_SURFACES[idx].input_read != WL_SURFACES[idx].input_write {
                let ev = WL_SURFACES[idx].input_ring[WL_SURFACES[idx].input_read as usize];
                WL_SURFACES[idx].input_read = ((WL_SURFACES[idx].input_read as usize + 1) % INPUT_RING_SIZE) as u16;
                return Some(ev);
            }
        }
        None
    }
}

pub fn input_event_count(id: u32) -> usize {
    unsafe {
        if let Some(idx) = (0..MAX_SURFACES).find(|&i| WL_SURFACES[i].in_use && WL_SURFACES[i].id==id) {
            let w = WL_SURFACES[idx].input_write as usize;
            let r = WL_SURFACES[idx].input_read as usize;
            if w >= r { w - r } else { INPUT_RING_SIZE - r + w }
        } else { 0 }
    }
}

pub fn find_surface_at(px: i32, py: i32) -> Option<u32> {
    unsafe {
        for i in 0..MAX_SURFACES {
            let s = &WL_SURFACES[i];
            if !s.in_use || s.buf_ptr.is_null() { continue; }
            if px >= s.x && px < s.x + s.w as i32 && py >= s.y && py < s.y + s.h as i32 {
                return Some(s.id);
            }
        }
        None
    }
}

pub fn surface_position(id: u32) -> Option<(i32, i32)> {
    unsafe {
        if let Some(idx) = (0..MAX_SURFACES).find(|&i| WL_SURFACES[i].in_use && WL_SURFACES[i].id==id) {
            Some((WL_SURFACES[idx].x, WL_SURFACES[idx].y))
        } else { None }
    }
}
pub fn composite_surfaces_onto_current_fb() {
    let mut fb = Framebuffer::from_display();
    wayland_composite(&mut fb);
}

// ── Graphics protocol commands (for IPC/syscall) ─────────────

pub enum GfxCommand {
    DrawRect { x: u32, y: u32, w: u32, h: u32, r: u8, g: u8, b: u8 },
    DrawPixel { x: u32, y: u32, r: u8, g: u8, b: u8 },
    DrawText { x: u32, y: u32, r: u8, g: u8, b: u8 },
    Clear { r: u8, g: u8, b: u8 },
    Present,
    SetDirty { layer: LayerId },
}

pub fn process_command(cmd: GfxCommand) {
    match cmd {
        GfxCommand::DrawRect { x, y, w, h, r, g, b } => {
            unsafe {
                let mut fb = Framebuffer::from_display();
                fb.fill_rect(x, y, w, h, r, g, b);
            }
        }
        GfxCommand::DrawPixel { x, y, r, g, b } => {
            unsafe {
                let mut fb = Framebuffer::from_display();
                fb.set_pixel(x, y, r, g, b);
            }
        }
        GfxCommand::Clear { r, g, b } => {
            unsafe {
                let mut fb = Framebuffer::from_display();
                fb.clear(r, g, b);
            }
        }
        GfxCommand::Present => {
            unsafe { display::present(); }
        }
        GfxCommand::SetDirty { layer } => {
            mark_dirty(layer);
        }
        _ => {}
    }
}

// ── LEVEL_1_SYSTEM initialization ─────────────────────────────

pub fn init() {
    crate::driver::uart::write_str("[GFX] Graphics protocol initialized (LEVEL_1_SYSTEM)\r\n");
    let w = display::width();
    let h = display::height();
    crate::driver::uart::write_str("[GFX] Display: ");
    uart_u32(w);
    crate::driver::uart::write_str("x");
    uart_u32(h);
    crate::driver::uart::write_str("\r\n");
}

fn uart_u32(mut v: u32) {
    if v == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut buf = [0u8; 10];
    let mut i = 0;
    while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; crate::driver::uart::putchar(buf[i]); }
}
