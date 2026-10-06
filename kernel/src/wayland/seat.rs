//! wl_seat — keyboard + pointer forwarding for Wayland clients
//!
//! Receives PS/2 keyboard and mouse events from drivers and forwards them
//! to the focused Wayland surface via per-surface input event ring buffers.
//! Uses Linux evdev keycodes for Wayland protocol compatibility.

use crate::driver::uart;

// ── Focus tracking ─────────────────────────────────────────────────

static mut FOCUSED_SURFACE: u32 = 0;

pub fn set_focus(id: u32) {
    unsafe {
        if FOCUSED_SURFACE != 0 && FOCUSED_SURFACE != id {
            let ev = crate::gfx::WlInputEvent { kind: 4, key: 0, state: 0, time: 0, dx: 0, dy: 0, button: 0 };
            crate::gfx::push_input_event(FOCUSED_SURFACE, ev);
        }
        FOCUSED_SURFACE = id;
        let ev = crate::gfx::WlInputEvent { kind: 5, key: 0, state: 0, time: 0, dx: 0, dy: 0, button: 0 };
        crate::gfx::push_input_event(id, ev);
    }
}

pub fn get_focus() -> u32 { unsafe { FOCUSED_SURFACE } }

pub fn clear_focus() {
    unsafe {
        if FOCUSED_SURFACE != 0 {
            let ev = crate::gfx::WlInputEvent { kind: 4, key: 0, state: 0, time: 0, dx: 0, dy: 0, button: 0 };
            crate::gfx::push_input_event(FOCUSED_SURFACE, ev);
        }
        FOCUSED_SURFACE = 0;
    }
}

// ── PS/2 scancode → Linux evdev keycode mapping ────────────────────

static SCANCODE_MAP: [u32; 89] = [
    0,    // 0x00 unused
    1,    // 0x01 ESC
    2,3,4,5,6,7,8,9,10,11, // 0x02-0x0B: 1-0
    12,13, // 0x0C-0x0D: MINUS, EQUAL
    14,   // 0x0E BACKSPACE
    15,   // 0x0F TAB
    16,17,18,19,20,21,22,23,24,25, // 0x10-0x19: Q-P
    26,27, // 0x1A-0x1B: [, ]
    28,   // 0x1C ENTER
    29,   // 0x1D LEFTCTRL
    30,31,32,33,34,35,36,37,38, // 0x1E-0x26: A-L
    39,40,41, // 0x27-0x29: SEMICOLON, APOSTROPHE, GRAVE
    42,   // 0x2A LEFTSHIFT
    43,   // 0x2B BACKSLASH
    44,45,46,47,48,49,50, // 0x2C-0x32: Z-M
    51,52,53, // 0x33-0x35: COMMA, DOT, SLASH
    54,   // 0x36 RIGHTSHIFT
    55,   // 0x37 KPASTERISK
    56,   // 0x38 LEFTALT
    57,   // 0x39 SPACE
    58,   // 0x3A CAPSLOCK
    59,60,61,62,63,64,65,66,67,68, // 0x3B-0x44: F1-F10
    69,   // 0x45 NUMLOCK
    70,   // 0x46 SCROLLLOCK
    71,72,73, // 0x47-0x49: KP7, KP8, KP9
    74,   // 0x4A KPMINUS
    75,76,77, // 0x4B-0x4D: KP4, KP5, KP6
    78,   // 0x4E KPPLUS
    79,80,81, // 0x4F-0x51: KP1, KP2, KP3
    82,   // 0x52 KP0
    83,   // 0x53 KPDOT
    0,0,0,0,0, // 0x54-0x58 unused
];

pub fn scancode_to_keycode(raw: u8, extended: bool) -> u32 {
    if extended {
        match raw {
            0x48 => 103, // UP
            0x50 => 108, // DOWN
            0x4B => 105, // LEFT
            0x4D => 106, // RIGHT
            0x47 => 102, // HOME
            0x4F => 107, // END
            0x53 => 111, // DELETE
            0x52 => 110, // INSERT
            0x49 => 104, // PAGEUP
            0x51 => 109, // PAGEDOWN
            0x1D => 97,  // RIGHTCTRL
            0x38 => 100, // RIGHTALT
            0x35 => 98,  // KPSLASH
            0x5B => 125, // LEFTMETA
            0x5C => 126, // RIGHTMETA
            0x5D => 127, // COMPOSE
            _ => 0,
        }
    } else {
        if (raw as usize) < SCANCODE_MAP.len() {
            SCANCODE_MAP[raw as usize]
        } else {
            0
        }
    }
}

// ── Seat ID allocation ─────────────────────────────────────────────

static mut NEXT_SEAT_ID: u32 = 200;

pub fn init() { uart::write_str("[WAYLAND-SEAT] init (keyboard+pointer)\n"); }

pub fn handle_seat(op: u32) -> u64 {
    match op {
        0 => { // get_pointer
            uart::write_str("[WAYLAND] seat get_pointer\n");
            let id = unsafe { let v = NEXT_SEAT_ID; NEXT_SEAT_ID += 1; v };
            id as u64
        },
        1 => { // get_keyboard
            uart::write_str("[WAYLAND] seat get_keyboard\n");
            let id = unsafe { let v = NEXT_SEAT_ID; NEXT_SEAT_ID += 1; v };
            if unsafe { FOCUSED_SURFACE } == 0 {
                if let Some(sid) = first_active_surface() {
                    set_focus(sid);
                }
            }
            id as u64
        },
        _ => 0,
    }
}

fn first_active_surface() -> Option<u32> {
    for id in 1..200u32 {
        if crate::gfx::surface_position(id).is_some() {
            return Some(id);
        }
    }
    None
}

// ── Key event forwarding ───────────────────────────────────────────

/// Process a raw PS/2 scancode and forward it to the focused Wayland surface.
/// Called from the PS/2 keyboard IRQ handler BEFORE any state is modified.
/// This function maintains its own extended-prefix state independently.
pub fn forward_scancode(scancode: u8) {
    static mut SEAT_EXT: bool = false;

    if scancode == 0xE0 {
        unsafe { SEAT_EXT = true; }
        return;
    }
    if scancode == 0xE1 {
        unsafe { SEAT_EXT = false; }
        return;
    }

    let extended = unsafe { SEAT_EXT };
    let pressed = scancode & 0x80 == 0;

    if !pressed {
        unsafe { SEAT_EXT = false; }
    }

    let raw = scancode & 0x7F;
    let keycode = scancode_to_keycode(raw, extended);

    if keycode != 0 {
        send_key(keycode, if pressed { 1 } else { 0 });
    }
}

/// Forward a key event to the focused Wayland surface.
pub fn send_key(key: u32, state: u32) {
    let focused = unsafe { FOCUSED_SURFACE };
    if focused == 0 { return; }

    let time = crate::timer::millis() as u32;
    let ev = crate::gfx::WlInputEvent {
        kind: 1, key, state, time, dx: 0, dy: 0, button: 0,
    };
    crate::gfx::push_input_event(focused, ev);
}

/// Send keyboard modifier state to the focused surface.
pub fn send_modifiers() {
    let focused = unsafe { FOCUSED_SURFACE };
    if focused == 0 { return; }
    let time = crate::timer::millis() as u32;
    let mods = crate::event::modifiers();
    let ev = crate::gfx::WlInputEvent {
        kind: 6, key: mods as u32, state: 0, time, dx: 0, dy: 0, button: 0,
    };
    crate::gfx::push_input_event(focused, ev);
}

// ── Pointer event forwarding ───────────────────────────────────────

pub fn send_pointer_motion(dx: i32, dy: i32) {
    let x = crate::driver::mouse::x();
    let y = crate::driver::mouse::y();

    if let Some(id) = crate::gfx::find_surface_at(x, y) {
        let time = crate::timer::millis() as u32;
        let ev = crate::gfx::WlInputEvent {
            kind: 2, key: 0, state: 0, time, dx, dy, button: 0,
        };
        crate::gfx::push_input_event(id, ev);

        if let Some((sx, sy)) = crate::gfx::surface_position(id) {
            let ev2 = crate::gfx::WlInputEvent {
                kind: 7, key: 0, state: 0, time, dx: x - sx, dy: y - sy, button: 0,
            };
            crate::gfx::push_input_event(id, ev2);
        }
    }
}

pub fn send_pointer_button(button: u32, pressed: bool) {
    let x = crate::driver::mouse::x();
    let y = crate::driver::mouse::y();

    if let Some(id) = crate::gfx::find_surface_at(x, y) {
        let time = crate::timer::millis() as u32;
        let ev = crate::gfx::WlInputEvent {
            kind: 3, key: button, state: if pressed { 1 } else { 0 }, time, dx: 0, dy: 0, button,
        };
        crate::gfx::push_input_event(id, ev);
    }
}

pub fn send_pointer(x: i32, y: i32, btn: u32) {
    if btn == 0 {
        if let Some(id) = crate::gfx::find_surface_at(x, y) {
            let time = crate::timer::millis() as u32;
            let ev = crate::gfx::WlInputEvent {
                kind: 2, key: 0, state: 0, time, dx: 0, dy: 0, button: 0,
            };
            crate::gfx::push_input_event(id, ev);
            if let Some((sx, sy)) = crate::gfx::surface_position(id) {
                let ev2 = crate::gfx::WlInputEvent {
                    kind: 7, key: 0, state: 0, time, dx: x - sx, dy: y - sy, button: 0,
                };
                crate::gfx::push_input_event(id, ev2);
            }
        }
    } else {
        if let Some(id) = crate::gfx::find_surface_at(x, y) {
            let time = crate::timer::millis() as u32;
            let pressed = btn & 0x8000_0000 == 0;
            let code = btn & 0x7FFF_FFFF;
            let ev = crate::gfx::WlInputEvent {
                kind: 3, key: code, state: if pressed { 1 } else { 0 }, time, dx: 0, dy: 0, button: code,
            };
            crate::gfx::push_input_event(id, ev);
        }
    }
}
