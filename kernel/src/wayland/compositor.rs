/// Wayland compositor — main compositor logic
///
/// Manages clients, processes requests, composites surfaces.
/// Uses GOP framebuffer as the display backend.

use super::protocol::*;
use super::surface::{self, SurfaceState};
use super::input;
use super::render;
use crate::display;
use crate::driver::uart;


/// Maximum connected clients
pub const MAX_CLIENTS: usize = 8;

/// A Wayland client connection
pub struct WlClient {
    pub in_use: bool,
    pub id: u32,
    pub next_object_id: WlId,
    pub surfaces: [WlId; 16],
    pub surface_count: usize,
    pub registry: WlId,
}

impl WlClient {
    pub const fn empty() -> Self {
        Self {
            in_use: false, id: 0, next_object_id: 1,
            surfaces: [0; 16], surface_count: 0, registry: 0,
        }
    }
}

static mut CLIENTS: [WlClient; MAX_CLIENTS] = {
    const INIT: WlClient = WlClient::empty();
    [INIT; MAX_CLIENTS]
};

static mut SERIAL: u32 = 1;
static mut RUNNING: bool = true;

fn next_serial() -> u32 {
    unsafe { SERIAL += 1; SERIAL }
}

#[allow(dead_code)]
fn next_object_id() -> WlId {
    unsafe {
        for c in CLIENTS.iter_mut() {
            if c.in_use {
                let id = c.next_object_id;
                c.next_object_id += 1;
                return id;
            }
        }
        1
    }
}

/// Initialize the compositor
pub fn init() {
    uart::write_str("[WAYLAND] Compositor initializing\r\n");
    render::init();
    input::init();
    uart::write_str("[WAYLAND] Compositor ready\r\n");
}

/// Run the compositor main loop
pub fn run() {
    uart::write_str("[WAYLAND] Starting compositor loop\r\n");

    // Create a default client (for the compositor's own surfaces)
    create_client();

    loop {
        // 1. Poll input
        input::poll();
        input::poll_keyboard();

        // 2. Handle requests (in a real system, these come from IPC/sockets)
        process_pending_requests();

        // 3. Composite
        render::composite();

        // 4. Send frame callbacks
        send_frame_callbacks();

        // 5. Yield to other tasks
        crate::scheduler::context::yield_now();
    }
}

/// Create a new client connection
pub fn create_client() -> Option<u32> {
    unsafe {
        for i in 0..MAX_CLIENTS {
            if !CLIENTS[i].in_use {
                CLIENTS[i].in_use = true;
                CLIENTS[i].id = i as u32 + 1;
                CLIENTS[i].next_object_id = 1;
                CLIENTS[i].surface_count = 0;
                return Some(CLIENTS[i].id);
            }
        }
        None
    }
}

/// Process a request from a client
pub fn process_request(client_id: u32, req: WlRequest) -> Option<WlEvent> {
    match req {
        // ── wl_display ───────────────────────────────────────────
        WlRequest::Sync { callback: _ } => {
            let serial = next_serial();
            Some(WlEvent::CallbackDone { callback_data: serial })
        }

        WlRequest::GetRegistry { registry } => {
            // Send all globals
            unsafe {
                for c in CLIENTS.iter_mut() {
                    if c.in_use && c.id == client_id {
                        c.registry = registry;
                        break;
                    }
                }
            }
            // Send globals
            for &(_name, interface, _version) in &[
                (1u32, b"wl_compositor\0" as &[u8], 4u32),
                (2, b"wl_shm\0", 1),
                (3, b"wl_shell\0", 1),
                (4, b"wl_seat\0", 4),
                (5, b"wl_output\0", 2),
            ] {
                let mut iface = [0u8; 32];
                let len = interface.len().min(31);
                iface[..len].copy_from_slice(&interface[..len]);
                // In real implementation, this would be sent as event
            }
            None
        }

        // ── wl_compositor ────────────────────────────────────────
        WlRequest::CreateSurface { surface } => {
            if let Some(s) = surface::alloc_surface() {
                s.id = surface;
                s.state = SurfaceState::Uninitialized;
            }
            None
        }

        // ── wl_surface ───────────────────────────────────────────
        WlRequest::Attach { buffer, x, y } => {
            if let Some(surf) = surface::find_surface(surface_id_from_req(&req)) {
                surf.buffer_id = buffer;
                surf.x = x;
                surf.y = y;
                surf.state = SurfaceState::Attached;
            }
            None
        }

        WlRequest::Damage { x, y, w, h } => {
            if let Some(surf) = surface::find_surface(surface_id_from_req(&req)) {
                surf.damage_x = x;
                surf.damage_y = y;
                surf.damage_w = w;
                surf.damage_h = h;
                surf.has_damage = true;
            }
            None
        }

        WlRequest::Frame { callback } => {
            if let Some(surf) = surface::find_surface(surface_id_from_req(&req)) {
                surf.callback_id = callback;
            }
            None
        }

        WlRequest::Commit => {
            if let Some(surf) = surface::find_surface(surface_id_from_req(&req)) {
                surf.state = SurfaceState::Committed;
            }
            None
        }

        // ── wl_shell ─────────────────────────────────────────────
        WlRequest::GetShellSurface { surface: _, shell_surface: _ } => {
            // Shell surface is created, next requests will configure it
            None
        }

        // ── wl_shell_surface ─────────────────────────────────────
        WlRequest::SetToplevel => {
            None
        }

        WlRequest::SetTitle { title, title_len } => {
            if let Some(surf) = surface::find_surface(surface_id_from_req(&req)) {
                let len = title_len.min(63);
                surf.title[..len].copy_from_slice(&title[..len]);
                surf.title_len = len;
            }
            None
        }

        WlRequest::SetFullscreen { .. } => {
            // Make surface fill the screen
            let sw = display::width();
            let sh = display::height();
            if let Some(surf) = surface::find_surface(surface_id_from_req(&req)) {
                surf.x = 0;
                surf.y = 0;
                surf.width = sw;
                surf.height = sh;
            }
            None
        }

        WlRequest::Pong { serial: _ } => {
            None
        }

        // ── wl_seat ──────────────────────────────────────────────
        WlRequest::GetPointer { id: _ } => {
            None
        }

        WlRequest::GetKeyboard { id: _ } => {
            None
        }

        // ── wl_shm ───────────────────────────────────────────────
        WlRequest::CreatePool { fd: _, size: _, pool: _ } => {
            // In a real system, this would map the shared memory
            None
        }

        // ── wl_buffer ────────────────────────────────────────────
        WlRequest::DestroyBuffer => {
            // Buffer will be freed when surface detaches
            None
        }

        _ => None,
    }
}

fn surface_id_from_req(req: &WlRequest) -> WlId {
    match req {
        WlRequest::Attach { .. } => 0,
        WlRequest::Damage { .. } => 0,
        WlRequest::Frame { .. } => 0,
        WlRequest::Commit => 0,
        WlRequest::SetTitle { .. } => 0,
        WlRequest::SetToplevel => 0,
        WlRequest::SetFullscreen { .. } => 0,
        WlRequest::Destroy => 0,
        WlRequest::SetOpaqueRegion { .. } => 0,
        WlRequest::SetInputRegion { .. } => 0,
        _ => 0,
    }
}

/// Process pending requests from all clients
fn process_pending_requests() {
    // In a real system, this would read from IPC channels
    // For now, the compositor processes internal events
}

/// Send frame done callbacks to all surfaces with pending callbacks
fn send_frame_callbacks() {
    let _now = (crate::timer::millis() / 1000) as u32;
    surface::for_each_surface_mut(|_idx, surf| {
        if surf.callback_id != 0 {
            // Send wl_callback.done event
            surf.callback_id = 0;  // Reset for next frame
        }
    });
}

/// Shutdown the compositor
pub fn shutdown() {
    unsafe { RUNNING = false; }
}

pub fn is_running() -> bool {
    unsafe { RUNNING }
}
