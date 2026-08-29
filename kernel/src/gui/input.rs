/// Input — event queue, mouse & keyboard state

#[derive(Clone, Copy, Debug)]
pub enum Event {
    MouseMove { x: i32, y: i32, dx: i32, dy: i32 },
    MouseDown { x: i32, y: i32, btn: MouseButton },
    MouseUp { x: i32, y: i32, btn: MouseButton },
    KeyDown { scancode: u8, key: u8 },
    KeyUp { scancode: u8, key: u8 },
    CharInput(u8),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

// ── State ─────────────────────────────────────────────────────────

static mut MOUSE_X: i32 = 640;
static mut MOUSE_Y: i32 = 400;
static mut MOUSE_LB: bool = false;
static mut MOUSE_RB: bool = false;
static mut MOUSE_MB: bool = false;

// Event queue (ring buffer)
const QUEUE_SIZE: usize = 64;
static mut QUEUE: [Event; QUEUE_SIZE] = [Event::MouseMove { x: 0, y: 0, dx: 0, dy: 0 }; QUEUE_SIZE];
static mut QUEUE_HEAD: usize = 0;
static mut QUEUE_TAIL: usize = 0;

fn enqueue(ev: Event) {
    unsafe {
        let next = (QUEUE_HEAD + 1) % QUEUE_SIZE;
        if next != QUEUE_TAIL {
            QUEUE[QUEUE_HEAD] = ev;
            QUEUE_HEAD = next;
        }
    }
}

pub fn next_event() -> Option<Event> {
    unsafe {
        if QUEUE_TAIL == QUEUE_HEAD { return None; }
        let ev = QUEUE[QUEUE_TAIL];
        QUEUE_TAIL = (QUEUE_TAIL + 1) % QUEUE_SIZE;
        Some(ev)
    }
}

pub fn flush_events() {
    unsafe { QUEUE_TAIL = QUEUE_HEAD; }
}

// ── Called from IRQ / driver layer ────────────────────────────────

pub fn update_mouse(x: i32, y: i32) {
    unsafe {
        let dx = x - MOUSE_X;
        let dy = y - MOUSE_Y;
        MOUSE_X = x;
        MOUSE_Y = y;
        if dx != 0 || dy != 0 {
            enqueue(Event::MouseMove { x, y, dx, dy });
        }
    }
}

pub fn update_mouse_button(btn: MouseButton, pressed: bool) {
    unsafe {
        match btn {
            MouseButton::Left   => { if MOUSE_LB == pressed { return; } MOUSE_LB = pressed; }
            MouseButton::Right  => { if MOUSE_RB == pressed { return; } MOUSE_RB = pressed; }
            MouseButton::Middle => { if MOUSE_MB == pressed { return; } MOUSE_MB = pressed; }
        }
        if pressed {
            enqueue(Event::MouseDown { x: MOUSE_X, y: MOUSE_Y, btn });
        } else {
            enqueue(Event::MouseUp { x: MOUSE_X, y: MOUSE_Y, btn });
        }
    }
}

pub fn push_key_down(scancode: u8) {
    let key = scancode_to_ascii(scancode, false);
    enqueue(Event::KeyDown { scancode, key });
    if key != 0 { enqueue(Event::CharInput(key)); }
}

pub fn push_key_up(scancode: u8) {
    let key = scancode_to_ascii(scancode, true);
    enqueue(Event::KeyUp { scancode, key });
}

// ── Getters ───────────────────────────────────────────────────────

pub fn mouse_x() -> i32 { unsafe { MOUSE_X } }
pub fn mouse_y() -> i32 { unsafe { MOUSE_Y } }
pub fn mouse_left() -> bool { unsafe { MOUSE_LB } }
pub fn mouse_right() -> bool { unsafe { MOUSE_RB } }

// ── Scancode → ASCII ──────────────────────────────────────────────

fn scancode_to_ascii(sc: u8, _up: bool) -> u8 {
    match sc {
        0x1E => b'a', 0x30 => b'b', 0x2E => b'c', 0x20 => b'd',
        0x12 => b'e', 0x21 => b'f', 0x22 => b'g', 0x23 => b'h',
        0x17 => b'i', 0x24 => b'j', 0x25 => b'k', 0x26 => b'l',
        0x32 => b'm', 0x31 => b'n', 0x18 => b'o', 0x19 => b'p',
        0x10 => b'q', 0x13 => b'r', 0x1F => b's', 0x14 => b't',
        0x16 => b'u', 0x2F => b'v', 0x11 => b'w', 0x2D => b'x',
        0x15 => b'y', 0x2C => b'z',
        0x02 => b'1', 0x03 => b'2', 0x04 => b'3', 0x05 => b'4',
        0x06 => b'5', 0x07 => b'6', 0x08 => b'7', 0x09 => b'8',
        0x0A => b'9', 0x0B => b'0',
        0x39 => b' ', 0x1C => b'\r', 0x0E => 0x08, // backspace
        0x01 => 0x03, // esc → Ctrl-C
        _ => 0,
    }
}
