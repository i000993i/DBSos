//! Wayland compositor — surface list and commit handling

use crate::driver::uart;

pub fn init() {
    uart::write_str("[WAYLAND-COMP] init\r\n");
    // wl_registry::global — анонсируем интерфейсы как настоящий compositor
    for g in crate::wayland::protocol::GLOBALS {
        uart::write_str("[WL-REG] global id=");
        dec(g.id);
        uart::write_str(" ");
        uart::write_str(g.interface);
        uart::write_str("@v");
        dec(g.version);
        uart::write_str("\r\n");
    }
}

fn dec(mut v: u32) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut b = [0u8; 10]; let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putchar(b[i]); }
}

/// Create a surface for the calling task. Returns surface id.
pub fn surface_create(x: i32, y: i32, w: u32, h: u32) -> Option<u32> {
    // per-user limit: each uid max 8 surfaces (reuse task limit)
    let uid = crate::user::current_uid();
    if uid!=0 {
        let count = crate::gfx::wl_surface_count();
        if count >= 8 { return None; }
    }
    crate::gfx::wl_surface_create(x, y, w, h)
}
pub fn surface_attach(id: u32, shm_pool: usize, offset: u32, w: u32, h: u32, stride: u32) -> bool {
    // offset within shm pool
    let phys = match crate::wayland::shm::pool_phys(shm_pool) {
        Some(p) => p + offset as u64,
        None => return false,
    };
    crate::gfx::wl_surface_attach(id, phys, w, h, stride)
}
pub fn surface_commit(id: u32) -> bool {
    crate::gfx::wl_surface_damage(id)
}
pub fn surface_destroy(id: u32) -> bool {
    crate::gfx::wl_surface_destroy(id)
}
pub fn surface_set_pos(id: u32, x: i32, y: i32) -> bool {
    crate::gfx::wl_surface_set_position(id, x, y)
}
