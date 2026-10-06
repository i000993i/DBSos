//! Wayland wire protocol — opcodes и интерфейсы для DBSos

// wl_display
pub const WL_DISPLAY_GET_REGISTRY: u32 = 1;
pub const WL_DISPLAY_SYNC: u32 = 0;

// wl_registry
pub const WL_REGISTRY_BIND: u32 = 0;
pub const WL_REGISTRY_GLOBAL: u32 = 0; // event

// wl_compositor
pub const WL_COMPOSITOR_CREATE_SURFACE: u32 = 0;
pub const WL_COMPOSITOR_CREATE_REGION: u32 = 1;

// wl_surface
pub const WL_SURFACE_ATTACH: u32 = 1;
pub const WL_SURFACE_DAMAGE: u32 = 2;
pub const WL_SURFACE_COMMIT: u32 = 6;
pub const WL_SURFACE_SET_BUFFER_SCALE: u32 = 8;

// wl_shm
pub const WL_SHM_CREATE_POOL: u32 = 0;
pub const WL_SHM_POOL_CREATE_BUFFER: u32 = 0;
pub const WL_SHM_POOL_DESTROY: u32 = 1;

// xdg_wm_base
pub const XDG_WM_BASE_GET_XDG_SURFACE: u32 = 2;
pub const XDG_WM_BASE_PONG: u32 = 3;

// xdg_surface
pub const XDG_SURFACE_GET_TOPLEVEL: u32 = 1;
pub const XDG_SURFACE_ACK_CONFIGURE: u32 = 4;

// xdg_toplevel
pub const XDG_TOPLEVEL_SET_TITLE: u32 = 2;
pub const XDG_TOPLEVEL_SET_APP_ID: u32 = 3;

// wl_seat
pub const WL_SEAT_GET_POINTER: u32 = 0;
pub const WL_SEAT_GET_KEYBOARD: u32 = 1;

// wl_output
pub const WL_OUTPUT_GEOMETRY: u32 = 0;
pub const WL_OUTPUT_MODE: u32 = 1;

// Globals
pub const WL_COMPOSITOR_ID: u32 = 1;
pub const WL_SHM_ID: u32 = 2;
pub const WL_SEAT_ID: u32 = 3;
pub const WL_OUTPUT_ID: u32 = 4;
pub const XDG_WM_BASE_ID: u32 = 5;

/// Registry: какой интерфейс за каким global id и какая версия РЕАЛЬНО
/// поддержана сервером (не заявляем больше, чем умеем — иначе клиенты
/// типа plasmashell упадут на первом же запросе).
pub struct WlGlobal {
    pub id: u32,
    pub interface: &'static str,
    pub version: u32,
}

pub const GLOBALS: &[WlGlobal] = &[
    WlGlobal { id: WL_COMPOSITOR_ID, interface: "wl_compositor", version: 4 },
    WlGlobal { id: WL_SHM_ID,        interface: "wl_shm",        version: 1 },
    WlGlobal { id: WL_SEAT_ID,       interface: "wl_seat",       version: 1 },
    WlGlobal { id: WL_OUTPUT_ID,      interface: "wl_output",     version: 2 },
    WlGlobal { id: XDG_WM_BASE_ID,   interface: "xdg_wm_base",   version: 1 },
];

pub fn global_by_id(id: u32) -> Option<&'static WlGlobal> {
    GLOBALS.iter().find(|g| g.id == id)
}
