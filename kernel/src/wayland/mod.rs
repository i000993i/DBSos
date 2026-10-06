//! Wayland compositor — minimal server for DBSos (Phase A/B)
//!
//! Provides surface-based compositing via `gfx::wl_surface_*`.
//! Wire protocol is syscall-based (not Unix socket) for no_std.
//! Clients (Ring3) use `dbsos-abi::wayland` helpers which do syscalls.

pub mod protocol;
pub mod display;
pub mod shm;
pub mod compositor;
pub mod seat;
pub mod output;
pub mod xdg;

use crate::driver::uart;

pub fn init() {
    uart::write_str("[WAYLAND] compositor init (Phase A)\r\n");
    crate::gfx::wl_surface_count(); // ensure gfx linked
    uart::write_str("[WAYLAND] gfx surfaces ready, max 16\r\n");
    compositor::init();
    shm::init();
    uart::write_str("[WAYLAND] ready — clients can create surfaces via syscalls\r\n");
}

/// For debug: count surfaces
pub fn debug_surfaces() {
    uart::write_str("[WAYLAND] surfaces: ");
    let n = crate::gfx::wl_surface_count();
    if n==0 { uart::write_str("0\r\n"); } else {
        let mut buf=[0u8;4];
        let mut v=n; let mut i=0;
        while v>0 { buf[i]=b'0'+(v%10)as u8; v/=10; i+=1; }
        while i>0 { i-=1; uart::putchar(buf[i]); }
        uart::write_str("\r\n");
    }
}
