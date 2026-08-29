/// Wayland Compositor — Rust-based compositor for DBSos
///
/// Simplified Wayland protocol implementation that renders to GOP framebuffer.
/// Provides wl_compositor, wl_surface, wl_shell, wl_seat, wl_output protocols.

pub mod protocol;
pub mod compositor;
pub mod surface;
pub mod input;
pub mod render;

/// Initialize the Wayland compositor
pub fn init() {
    compositor::init();
}

/// Run the compositor main loop (called from kernel)
pub fn run() {
    compositor::run();
}
