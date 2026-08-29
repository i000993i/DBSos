/// Wayland protocol types — simplified wl_*.h equivalents
///
/// These are the core Wayland objects and requests/events
/// that a compositor needs to implement.

/// Wayland object IDs are u32
pub type WlId = u32;

/// Wayland protocol request (client → server)
#[derive(Clone, Copy)]
pub enum WlRequest {
    // wl_display
    Sync { callback: WlId },
    GetRegistry { registry: WlId },

    // wl_compositor
    CreateSurface { surface: WlId },
    CreateRegion { region: WlId },

    // wl_surface
    Destroy,
    Attach { buffer: WlId, x: i32, y: i32 },
    Damage { x: i32, y: i32, w: i32, h: i32 },
    Frame { callback: WlId },
    SetOpaqueRegion { region: WlId },
    SetInputRegion { region: WlId },
    Commit,

    // wl_shell
    GetShellSurface { surface: WlId, shell_surface: WlId },

    // wl_shell_surface
    Pong { serial: u32 },
    SetTitle { title: [u8; 64], title_len: usize },
    SetToplevel,
    SetTransient { parent: WlId, x: i32, y: i32, flags: u32 },
    SetFullscreen { method: u32, framerate: u32, output: WlId },

    // wl_seat
    GetPointer { id: WlId },
    GetKeyboard { id: WlId },
    GetTouch { id: WlId },

    // wl_shm
    CreatePool { fd: i32, size: i32, pool: WlId },

    // wl_buffer
    DestroyBuffer,

    // wl_callback
    Cancel,

    Unknown,
}

/// Wayland protocol event (server → client)
#[derive(Clone, Copy)]
pub enum WlEvent {
    // wl_display
    DisplayError { object_id: WlId, code: u32, message: [u8; 64] },
    DisplayDeleteId { id: WlId },

    // wl_registry
    RegistryGlobal { name: u32, interface: [u8; 32], version: u32 },
    RegistryGlobalRemove { name: u32 },

    // wl_surface
    SurfaceEnter { output: WlId },
    SurfaceLeave { output: WlId },

    // wl_shell_surface
    ShellSurfacePing { serial: u32 },
    ShellSurfaceConfigure { edges: u32, width: i32, height: i32, serial: u32 },
    ShellSurfaceConfigureDone,

    // wl_seat
    SeatCapabilities { caps: u32 },
    SeatName { name: [u8; 32] },

    // wl_pointer
    PointerEnter { serial: u32, surface: WlId, surface_x: i32, surface_y: i32 },
    PointerLeave { serial: u32, surface: WlId },
    PointerMotion { time: u32, surface_x: i32, surface_y: i32 },
    PointerButton { serial: u32, time: u32, button: u32, state: u32 },
    PointerAxis { time: u32, axis: u32, value: i32 },

    // wl_keyboard
    KeyboardKeymap { format: u32, fd: i32, size: u32 },
    KeyboardEnter { serial: u32, surface: WlId, keys: [u8; 32] },
    KeyboardLeave { serial: u32, surface: WlId },
    KeyboardKey { serial: u32, time: u32, key: u32, state: u32 },
    KeyboardModifiers { serial: u32, mods_depressed: u32, mods_latched: u32, mods_locked: u32, group: u32 },

    // wl_output
    OutputGeometry { x: i32, y: i32, physical_width: i32, physical_height: i32,
                     subpixel: i32, make: [u8; 32], model: [u8; 32], transform: i32 },
    OutputMode { flags: u32, width: i32, height: i32, refresh: i32 },
    OutputDone,
    OutputScale { factor: i32 },

    // wl_callback
    CallbackDone { callback_data: u32 },

    Unknown,
}

/// Global Wayland interfaces supported by the compositor
pub const WL_GLOBALS: &[(&[u8], u32)] = &[
    (b"wl_compositor", 4),
    (b"wl_shm", 1),
    (b"wl_shell", 1),
    (b"wl_seat", 4),
    (b"wl_output", 2),
    (b"wl_data_device_manager", 1),
];

/// Seat capabilities
pub const WL_SEAT_CAPABILITY_POINTER: u32 = 1;
pub const WL_SEAT_CAPABILITY_KEYBOARD: u32 = 2;
pub const WL_SEAT_CAPABILITY_TOUCH: u32 = 4;

/// Shell surface states
pub const WL_SHELL_SURFACE_STATE_TOPLEVEL: u32 = 0;
pub const WL_SHELL_SURFACE_STATE_TRANSIENT: u32 = 1;
pub const WL_SHELL_SURFACE_STATE_FULLSCREEN: u32 = 2;
pub const WL_SHELL_SURFACE_STATE_POPUP: u32 = 3;

/// Pointer button states
pub const WL_POINTER_BUTTON_STATE_PRESSED: u32 = 1;
pub const WL_POINTER_BUTTON_STATE_RELEASED: u32 = 0;

/// Keyboard key states
pub const WL_KEYBOARD_KEY_STATE_PRESSED: u32 = 1;
pub const WL_KEYBOARD_KEY_STATE_RELEASED: u32 = 0;
