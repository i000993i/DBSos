use core::arch::asm;

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64;

/// Scancode set 1 -> ASCII (no shift). Index by scancode.
static KEYMAP: [u8; 128] = [
    0,   // 0x00
    27,  // 0x01 ESC
    b'1',b'2',b'3',b'4',b'5',b'6',b'7',b'8',b'9',b'0', // 0x02-0x0B
    b'-',b'=', // 0x0C-0x0D
    0x08, // 0x0E Backspace
    0x09, // 0x0F Tab
    b'q',b'w',b'e',b'r',b't',b'y',b'u',b'i',b'o',b'p', // 0x10-0x19
    b'[',b']', // 0x1A-0x1B
    0x0a, // 0x1C Enter
    0, // 0x1D LCtrl
    b'a',b's',b'd',b'f',b'g',b'h',b'j',b'k',b'l', // 0x1E-0x26
    b';',b'\'',b'`', // 0x27-0x29
    0, // 0x2A LShift
    b'\\', // 0x2B
    b'z',b'x',b'c',b'v',b'b',b'n',b'm', // 0x2C-0x32
    b',',b'.',b'/', // 0x33-0x35
    0, // 0x36 RShift
    b'*', // 0x37
    0, // 0x38 LAlt
    b' ', // 0x39 Space
    0, // 0x3A CapsLock
    0,0,0,0,0,0,0,0,0,0, // 0x3B-0x44 F1-F10
    0, // 0x45 NumLock
    0, // 0x46 ScrollLock
    b'7',b'8',b'9',b'-', // 0x47-0x4A KP
    b'4',b'5',b'6',b'+', // 0x4B-0x4E
    b'1',b'2',b'3',b'0', // 0x4F-0x52
    b'.', // 0x53
    0,0,0,0,0,0,0,0,0,0,0,0, // 0x54-0x5F
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, // 0x60-0x6F
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, // 0x70-0x7F
];

/// Scancode set 1 -> ASCII (shift pressed). Index by scancode.
static KEYMAP_SHIFT: [u8; 128] = [
    0,   // 0x00
    27,  // 0x01 ESC
    b'!',b'@',b'#',b'$',b'%',b'^',b'&',b'*',b'(',b')', // 0x02-0x0B
    b'_',b'+', // 0x0C-0x0D
    0x08, // 0x0E Backspace
    0x09, // 0x0F Tab
    b'Q',b'W',b'E',b'R',b'T',b'Y',b'U',b'I',b'O',b'P', // 0x10-0x19
    b'{',b'}', // 0x1A-0x1B
    0x0a, // 0x1C Enter
    0, // 0x1D LCtrl
    b'A',b'S',b'D',b'F',b'G',b'H',b'J',b'K',b'L', // 0x1E-0x26
    b':',b'"',b'~', // 0x27-0x29
    0, // 0x2A LShift
    b'|', // 0x2B
    b'Z',b'X',b'C',b'V',b'B',b'N',b'M', // 0x2C-0x32
    b'<',b'>',b'?', // 0x33-0x35
    0, // 0x36 RShift
    b'*', // 0x37
    0, // 0x38 LAlt
    b' ', // 0x39 Space
    0, // 0x3A CapsLock
    0,0,0,0,0,0,0,0,0,0, // 0x3B-0x44 F1-F10
    0, // 0x45 NumLock
    0, // 0x46 ScrollLock
    b'7',b'8',b'9',b'-', // 0x47-0x4A KP
    b'4',b'5',b'6',b'+', // 0x4B-0x4E
    b'1',b'2',b'3',b'0', // 0x4F-0x52
    b'.', // 0x53
    0,0,0,0,0,0,0,0,0,0,0,0, // 0x54-0x5F
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, // 0x60-0x6F
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, // 0x70-0x7F
];

// ── Special keys (0xE0 extended scancodes) ──
// Высокие биты: 0x80 = extended prefix, нижние = virtual key code
pub const KEY_UP: u8 = 0x80 | 0x48;
pub const KEY_DOWN: u8 = 0x80 | 0x50;
pub const KEY_LEFT: u8 = 0x80 | 0x4B;
pub const KEY_RIGHT: u8 = 0x80 | 0x4D;
pub const KEY_HOME: u8 = 0x80 | 0x47;
pub const KEY_END: u8 = 0x80 | 0x4F;
pub const KEY_DELETE: u8 = 0x80 | 0x53;
pub const KEY_INSERT: u8 = 0x80 | 0x52;
pub const KEY_PAGE_UP: u8 = 0x80 | 0x49;
pub const KEY_PAGE_DOWN: u8 = 0x80 | 0x51;

const BUF_SIZE: usize = 256;
static mut KEYBUF: [u8; BUF_SIZE] = [0; BUF_SIZE];
static mut KEYBUF_R: usize = 0;
static mut KEYBUF_W: usize = 0;

static mut SHIFT: bool = false;
static mut ALT: bool = false;
static mut CTRL: bool = false;
static mut EXTENDED: bool = false;
static mut LAYOUT_RU: bool = false; // false=EN, true=RU
static mut CAPS: bool = false; // CapsLock toggle

/// RU JCUKEN (CP1251 single-byte) — без Shift
static KEYMAP_RU: [u8; 128] = [
    0, 27,
    b'1',b'2',b'3',b'4',b'5',b'6',b'7',b'8',b'9',b'0',
    b'-',b'=',
    0x08, 0x09,
    0xE9,0xF6,0xF3,0xEA,0xE5,0xED,0xE3,0xF8,0xF9,0xE7, // q->й w->ц e->у r->к t->е y->н u->г i->ш o->щ p->з
    0xF5,0xFA, // [->х ]->ъ
    0x0a,
    0,
    0xF4,0xFB,0xE2,0xE0,0xEF,0xF0,0xEE,0xEB,0xE4, // a->ф s->ы d->в f->а g->п h->р j->о k->л l->д
    0xE6,0xFD,0xB8, // ;->ж '->э `->ё
    0,
    0x5C, // \ -> \ (keep)
    0xFF,0xF7,0xF1,0xEC,0xE8,0xF2,0xFC, // z->я x->ч c->с v->м b->и n->т m->ь
    0xE1,0xFE,0x2E, // ,->б .->ю /->.
    0, b'*', 0, b' ', 0,
    0,0,0,0,0,0,0,0,0,0,
    0, 0,
    b'7',b'8',b'9',b'-',
    b'4',b'5',b'6',b'+',
    b'1',b'2',b'3',b'0',
    b'.',
    0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
];
/// RU JCUKEN Shift (заглавные)
static KEYMAP_RU_SHIFT: [u8; 128] = [
    0, 27,
    b'!',b'"',b'#',b';',b'%',b':',b'?',b'*',b'(',b')',
    b'_',b'+',
    0x08, 0x09,
    0xC9,0xD6,0xD3,0xCA,0xC5,0xCD,0xC3,0xD8,0xD9,0xC7,
    0xD5,0xDA,
    0x0a,
    0,
    0xD4,0xDB,0xC2,0xC0,0xCF,0xD0,0xCE,0xCB,0xC4,
    0xC6,0xDD,0xA8,
    0,
    0x7C,
    0xDF,0xD7,0xD1,0xCC,0xC8,0xD2,0xDC,
    0xC1,0xDE,0x2E,
    0, b'*', 0, b' ', 0,
    0,0,0,0,0,0,0,0,0,0,
    0, 0,
    b'7',b'8',b'9',b'-',
    b'4',b'5',b'6',b'+',
    b'1',b'2',b'3',b'0',
    b'.',
    0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
];

pub fn is_ru() -> bool { unsafe{ LAYOUT_RU } }
pub fn layout_name() -> &'static str { if unsafe{ LAYOUT_RU } {"RU"} else {"EN"} }

fn inb(port: u16) -> u8 {
    let v: u8;
    unsafe { asm!("in al, dx", out("al") v, in("dx") port); }
    v
}

fn outb(port: u16, val: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") val); }
}

fn write_buf(c: u8) {
    unsafe {
        let next = (KEYBUF_W + 1) % BUF_SIZE;
        if next != KEYBUF_R {
            KEYBUF[KEYBUF_W] = c;
            KEYBUF_W = next;
        }
    }
    // Also push to unified event system
    crate::event::push_key(true, c);
}

fn handle_scancode(scancode: u8) {
    // Extended prefix: next scancode is 0xE0 + actual scancode
    if scancode == 0xE0 {
        unsafe { EXTENDED = true; }
        return;
    }
    if scancode == 0xE1 {
        // E1 prefix ( Pause/Break ) — skip next 7 bytes
        unsafe { EXTENDED = false; }
        return;
    }

    // Modifier keys
    if scancode == 0x2A || scancode == 0x36 {
        unsafe { SHIFT = true; if ALT { LAYOUT_RU = !LAYOUT_RU; } }
        return;
    }
    if scancode == 0xAA || scancode == 0xB6 {
        unsafe { SHIFT = false; }
        return;
    }
    if scancode == 0x38 {
        unsafe { ALT = true; if SHIFT { LAYOUT_RU = !LAYOUT_RU; } }
        return;
    }
    if scancode == 0xB8 {
        unsafe { ALT = false; }
        return;
    }
    if scancode == 0x1D {
        unsafe { CTRL = true; }
        return;
    }
    if scancode == 0x9D {
        unsafe { CTRL = false; }
        return;
    }

    // Key release (bit 7 set) — ignore (except modifiers above)
    if scancode & 0x80 != 0 {
        unsafe { EXTENDED = false; }
        return;
    }

    // CapsLock press — toggle (release 0xBA игнорируется выше)
    if scancode == 0x3A {
        unsafe { CAPS = !CAPS; }
        return;
    }

    let extended = unsafe { EXTENDED };
    unsafe { EXTENDED = false; }

    if extended {
        // 0xE0 extended scancode — also handle RAlt/RCtrl
        if scancode == 0x38 {
            unsafe { ALT = true; if SHIFT { LAYOUT_RU = !LAYOUT_RU; } }
            return;
        }
        if scancode == 0xB8 {
            unsafe { ALT = false; }
            return;
        }
        if scancode == 0x1D {
            unsafe { CTRL = true; }
            return;
        }
        if scancode == 0x9D {
            unsafe { CTRL = false; }
            return;
        }
        let key = match scancode {
            0x48 => KEY_UP,
            0x50 => KEY_DOWN,
            0x4B => KEY_LEFT,
            0x4D => KEY_RIGHT,
            0x47 => KEY_HOME,
            0x4F => KEY_END,
            0x53 => KEY_DELETE,
            0x52 => KEY_INSERT,
            0x49 => KEY_PAGE_UP,
            0x51 => KEY_PAGE_DOWN,
            _ => 0,
        };
        if key != 0 {
            write_buf(key);
        }
    } else {
        // Standard scancode — EN/RU. CapsLock инвертирует Shift только для букв
        // (EN A-Z + RU CP1251 А-Я/а-я/Ё/ё), цифры и символы не трогает.
        let ru = unsafe{ LAYOUT_RU };
        let caps = unsafe{ CAPS };
        let c = unsafe {
            let plain = if ru { KEYMAP_RU[scancode as usize] } else { KEYMAP[scancode as usize] };
            let shifted = if ru { KEYMAP_RU_SHIFT[scancode as usize] } else { KEYMAP_SHIFT[scancode as usize] };
            if caps && is_letter_pair(plain, shifted) { if SHIFT { plain } else { shifted } }
            else if SHIFT { shifted } else { plain }
        };
        if c != 0 {
            write_buf(c);
        }
    }
}

fn is_letter_pair(plain: u8, shifted: u8) -> bool {
    if plain == 0 || shifted == 0 { return false; }
    // EN: a-z <-> A-Z
    if plain >= b'a' && plain <= b'z' && shifted >= b'A' && shifted <= b'Z' { return true; }
    // RU CP1251: а-я (0xE0-0xFF) <-> А-Я (0xC0-0xDF), ё (0xB8) <-> Ё (0xA8)
    if plain >= 0xE0 && shifted >= 0xC0 && shifted <= 0xDF
        && shifted == plain - 0x20 { return true; }
    if (plain == 0xB8 && shifted == 0xA8) || (plain == 0xA8 && shifted == 0xB8) { return true; }
    false
}

#[no_mangle]
extern "C" fn keyboard_handler() {
    let status = inb(STATUS);
    if status & 1 != 0 {
        let scancode = inb(DATA);
        // Forward to Wayland seat BEFORE handle_scancode modifies state
        crate::wayland::seat::forward_scancode(scancode);
        handle_scancode(scancode);
    }
    outb(0x20, 0x20); // EOI to PIC master
}

core::arch::global_asm!(
    ".globl keyboard_stub",
    "keyboard_stub:",
    "  push rax", "push rcx", "push rdx", "push rbx",
    "  push rbp", "push rsi", "push rdi",
    "  push r8", "push r9", "push r10", "push r11",
    "  push r12", "push r13", "push r14", "push r15",
    "  sub rsp, 32",
    "  call keyboard_handler",
    "  add rsp, 32",
    "  pop r15", "pop r14", "pop r13", "pop r12",
    "  pop r11", "pop r10", "pop r9", "pop r8",
    "  pop rdi", "pop rsi", "pop rbp",
    "  pop rbx", "pop rdx", "pop rcx", "pop rax",
    "  iretq",
);

extern "C" { fn keyboard_stub(); }

pub fn init() {
    unsafe {
        let entry = &mut crate::interrupts::IDT[33];
        entry.set_handler(keyboard_stub as *const () as u64, 0x08);
    }
    // Unmask IRQ1 in PIC
    let mask = inb(0x21);
    outb(0x21, mask & !2);
}

/// Blocking read of next keypress (returns ASCII)
pub fn getchar() -> u8 {
    unsafe {
        while KEYBUF_R == KEYBUF_W {}
        let c = KEYBUF[KEYBUF_R];
        KEYBUF_R = (KEYBUF_R + 1) % BUF_SIZE;
        c
    }
}

/// Non-blocking: reads key if available, returns Some(c) or None
pub fn poll_char() -> Option<u8> {
    unsafe {
        if KEYBUF_R != KEYBUF_W {
            let c = KEYBUF[KEYBUF_R];
            KEYBUF_R = (KEYBUF_R + 1) % BUF_SIZE;
            Some(c)
        } else {
            None
        }
    }
}

/// Returns true if a key is available without reading it
pub fn has_char() -> bool {
    unsafe { KEYBUF_R != KEYBUF_W }
}

pub struct Ps2Driver;

impl super::traits::Driver for Ps2Driver {
    fn name(&self) -> &'static str { "PS/2 Keyboard" }
    fn device_type(&self) -> super::traits::DeviceType {
        super::traits::DeviceType::Legacy
    }
    fn init(&self) -> super::traits::DriverStatus {
        init();
        super::traits::DriverStatus::Ok
    }
}
