/// Unified event system for DBSos input
/// Replaces ad-hoc KEYBUF/MOUSE_BUF with typed event ring buffers

use core::sync::atomic::{AtomicUsize, Ordering};

// ── Event types ────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EventType {
    KeyPress,
    KeyRelease,
    MouseMove,
    MouseClick,
    MouseRelease,
    FocusIn,
    FocusOut,
}

#[derive(Copy, Clone)]
pub struct Event {
    pub kind: EventType,
    pub key: u8,        // ASCII or special key (KEY_*)
    pub x: i32,         // mouse X (for mouse events)
    pub y: i32,         // mouse Y (for mouse events)
    pub dx: i32,        // mouse delta X
    pub dy: i32,        // mouse delta Y
    pub button: u8,     // 0=none, 1=left, 2=right, 3=middle
    pub modifiers: u8,  // bitmask: SHIFT=1, CTRL=2, ALT=4
}

// ── Ring buffer ────────────────────────────────────────────────

const EVENT_BUF_SIZE: usize = 512;

pub struct EventRing {
    buf: [Event; EVENT_BUF_SIZE],
    read: AtomicUsize,
    write: AtomicUsize,
}

impl EventRing {
    const fn new() -> Self {
        Self {
            buf: [Event {
                kind: EventType::KeyPress,
                key: 0,
                x: 0, y: 0, dx: 0, dy: 0,
                button: 0, modifiers: 0,
            }; EVENT_BUF_SIZE],
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
        }
    }

    /// Push event from IRQ context (single producer)
    pub fn push(&mut self, event: Event) {
        let w = self.write.load(Ordering::Relaxed);
        let r = self.read.load(Ordering::Acquire);
        let next = (w + 1) % EVENT_BUF_SIZE;
        if next == r {
            return; // buffer full — drop event
        }
        self.buf[w] = event;
        self.write.store(next, Ordering::Release);
    }

    /// Pop event from consumer context (single consumer)
    pub fn pop(&mut self) -> Option<Event> {
        let r = self.read.load(Ordering::Relaxed);
        let w = self.write.load(Ordering::Acquire);
        if r == w {
            return None;
        }
        let event = self.buf[r];
        self.read.store((r + 1) % EVENT_BUF_SIZE, Ordering::Release);
        Some(event)
    }

    /// Check if events are available
    pub fn has_events(&self) -> bool {
        self.read.load(Ordering::Relaxed) != self.write.load(Ordering::Acquire)
    }
}

// ── Global event queues ────────────────────────────────────────

pub static mut KEY_EVENTS: EventRing = EventRing::new();
pub static mut MOUSE_EVENTS: EventRing = EventRing::new();
pub static mut FOCUS_EVENTS: EventRing = EventRing::new();

// ── Modifier state ─────────────────────────────────────────────

static mut MOD_SHIFT: bool = false;
static mut MOD_CTRL: bool = false;
static mut MOD_ALT: bool = false;

pub fn modifiers() -> u8 {
    unsafe {
        let mut m = 0u8;
        if MOD_SHIFT { m |= 1; }
        if MOD_CTRL { m |= 2; }
        if MOD_ALT { m |= 4; }
        m
    }
}

pub fn set_shift(v: bool) { unsafe { MOD_SHIFT = v; } }
pub fn set_ctrl(v: bool) { unsafe { MOD_CTRL = v; } }
pub fn set_alt(v: bool) { unsafe { MOD_ALT = v; } }

// ── High-level API ─────────────────────────────────────────────

/// Push a key event
pub fn push_key(pressed: bool, key: u8) {
    let kind = if pressed { EventType::KeyPress } else { EventType::KeyRelease };
    let event = Event {
        kind,
        key,
        x: 0, y: 0, dx: 0, dy: 0,
        button: 0,
        modifiers: modifiers(),
    };
    unsafe { KEY_EVENTS.push(event); }
}

/// Push a mouse event (from IRQ handler)
pub fn push_mouse(dx: i32, dy: i32, left: bool, right: bool, middle: bool) {
    let event = Event {
        kind: EventType::MouseMove,
        key: 0,
        x: 0, y: 0, dx, dy,
        button: if left { 1 } else if right { 2 } else if middle { 3 } else { 0 },
        modifiers: modifiers(),
    };
    unsafe { MOUSE_EVENTS.push(event); }
}

/// Push a focus event
pub fn push_focus(focused: bool) {
    let kind = if focused { EventType::FocusIn } else { EventType::FocusOut };
    let event = Event {
        kind,
        key: 0,
        x: 0, y: 0, dx: 0, dy: 0,
        button: 0, modifiers: 0,
    };
    unsafe { FOCUS_EVENTS.push(event); }
}

/// Poll next key event
pub fn poll_key() -> Option<Event> {
    unsafe { KEY_EVENTS.pop() }
}

/// Poll next mouse event
pub fn poll_mouse() -> Option<Event> {
    unsafe { MOUSE_EVENTS.pop() }
}

/// Poll next focus event
pub fn poll_focus() -> Option<Event> {
    unsafe { FOCUS_EVENTS.pop() }
}

/// Check if key events are pending
pub fn has_key_events() -> bool {
    unsafe { KEY_EVENTS.has_events() }
}

/// Check if mouse events are pending
pub fn has_mouse_events() -> bool {
    unsafe { MOUSE_EVENTS.has_events() }
}
