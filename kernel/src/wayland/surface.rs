/// Wayland surfaces — buffer management and damage tracking
///
/// Each surface has a buffer (shared memory or pixel data),
/// damage region, and transform state.

use super::protocol::WlId;

pub const MAX_SURFACES: usize = 32;
pub const MAX_BUFFERS: usize = 64;

/// Surface state
#[derive(Clone, Copy, PartialEq)]
pub enum SurfaceState {
    Uninitialized,
    Attached,
    Committed,
    Destroyed,
}

/// A Wayland buffer (shared memory or pixel data)
pub struct WlBuffer {
    pub id: WlId,
    pub in_use: bool,
    pub data_phys: u64,    // Physical address of pixel data
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,       // WL_SHM_FORMAT_*
    pub size: u32,
}

impl WlBuffer {
    pub const fn empty() -> Self {
        Self {
            id: 0, in_use: false, data_phys: 0,
            width: 0, height: 0, stride: 0,
            format: 0, size: 0,
        }
    }
}

/// A Wayland surface
pub struct WlSurface {
    pub id: WlId,
    pub in_use: bool,
    pub state: SurfaceState,
    pub buffer_id: WlId,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub damage_x: i32,
    pub damage_y: i32,
    pub damage_w: i32,
    pub damage_h: i32,
    pub has_damage: bool,
    pub opaque: bool,
    pub input_region: u64,  // Region ID (0 = none)
    pub callback_id: WlId,  // Frame callback
    pub title: [u8; 64],
    pub title_len: usize,
}

impl WlSurface {
    pub const fn empty() -> Self {
        Self {
            id: 0, in_use: false, state: SurfaceState::Uninitialized,
            buffer_id: 0, x: 0, y: 0, width: 0, height: 0,
            damage_x: 0, damage_y: 0, damage_w: 0, damage_h: 0,
            has_damage: false, opaque: false, input_region: 0,
            callback_id: 0, title: [0u8; 64], title_len: 0,
        }
    }
}

static mut BUFFERS: [WlBuffer; MAX_BUFFERS] = {
    const INIT: WlBuffer = WlBuffer::empty();
    [INIT; MAX_BUFFERS]
};

static mut SURFACES: [WlSurface; MAX_SURFACES] = {
    const INIT: WlSurface = WlSurface::empty();
    [INIT; MAX_SURFACES]
};

// ── Buffer management ─────────────────────────────────────────────

pub fn alloc_buffer() -> Option<&'static mut WlBuffer> {
    unsafe {
        for i in 0..MAX_BUFFERS {
            if !BUFFERS[i].in_use {
                BUFFERS[i].in_use = true;
                return Some(&mut BUFFERS[i]);
            }
        }
        None
    }
}

pub fn find_buffer(id: WlId) -> Option<&'static mut WlBuffer> {
    unsafe {
        for i in 0..MAX_BUFFERS {
            if BUFFERS[i].in_use && BUFFERS[i].id == id {
                return Some(&mut BUFFERS[i]);
            }
        }
        None
    }
}

pub fn free_buffer(id: WlId) {
    unsafe {
        for i in 0..MAX_BUFFERS {
            if BUFFERS[i].in_use && BUFFERS[i].id == id {
                BUFFERS[i].in_use = false;
                return;
            }
        }
    }
}

// ── Surface management ────────────────────────────────────────────

pub fn alloc_surface() -> Option<&'static mut WlSurface> {
    unsafe {
        for i in 0..MAX_SURFACES {
            if !SURFACES[i].in_use {
                SURFACES[i].in_use = true;
                SURFACES[i].state = SurfaceState::Uninitialized;
                return Some(&mut SURFACES[i]);
            }
        }
        None
    }
}

pub fn find_surface(id: WlId) -> Option<&'static mut WlSurface> {
    unsafe {
        for i in 0..MAX_SURFACES {
            if SURFACES[i].in_use && SURFACES[i].id == id {
                return Some(&mut SURFACES[i]);
            }
        }
        None
    }
}

pub fn free_surface(id: WlId) {
    unsafe {
        for i in 0..MAX_SURFACES {
            if SURFACES[i].in_use && SURFACES[i].id == id {
                SURFACES[i].in_use = false;
                SURFACES[i].state = SurfaceState::Destroyed;
                return;
            }
        }
    }
}

pub fn for_each_surface<F: FnMut(usize, &WlSurface)>(mut f: F) {
    unsafe {
        for i in 0..MAX_SURFACES {
            if SURFACES[i].in_use && SURFACES[i].state != SurfaceState::Destroyed {
                f(i, &SURFACES[i]);
            }
        }
    }
}

pub fn for_each_surface_mut<F: FnMut(usize, &mut WlSurface)>(mut f: F) {
    unsafe {
        for i in 0..MAX_SURFACES {
            if SURFACES[i].in_use && SURFACES[i].state != SurfaceState::Destroyed {
                f(i, &mut SURFACES[i]);
            }
        }
    }
}

/// Find surface at given screen coordinates
pub fn surface_at(sx: u32, sy: u32) -> Option<(usize, WlId)> {
    unsafe {
        let mut best: Option<(usize, WlId, i32)> = None;
        for i in 0..MAX_SURFACES {
            if !SURFACES[i].in_use || SURFACES[i].state == SurfaceState::Destroyed { continue; }
            let s = &SURFACES[i];
            let sx = sx as i32;
            let sy = sy as i32;
            if sx >= s.x && sx < s.x + s.width as i32
                && sy >= s.y && sy < s.y + s.height as i32 {
                match best {
                    None => best = Some((i, s.id, s.id as i32)),
                    Some((_, _, bz)) if s.id as i32 > bz => best = Some((i, s.id, s.id as i32)),
                    _ => {}
                }
            }
        }
        best.map(|(i, id, _)| (i, id))
    }
}

/// Get mutable reference to surfaces array (for compositor use)
pub fn surfaces_mut() -> &'static mut [WlSurface; MAX_SURFACES] {
    unsafe { &mut SURFACES }
}

pub fn buffers_mut() -> &'static mut [WlBuffer; MAX_BUFFERS] {
    unsafe { &mut BUFFERS }
}
