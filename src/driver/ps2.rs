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

const BUF_SIZE: usize = 256;
static mut KEYBUF: [u8; BUF_SIZE] = [0; BUF_SIZE];
static mut KEYBUF_R: usize = 0;
static mut KEYBUF_W: usize = 0;

static mut SHIFT: bool = false;

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
}

fn handle_scancode(scancode: u8) {
    if scancode == 0x2A || scancode == 0x36 {
        unsafe { SHIFT = true; }
        return;
    }
    if scancode == 0xAA || scancode == 0xB6 {
        unsafe { SHIFT = false; }
        return;
    }

    if scancode & 0x80 != 0 {
        return;
    }

    let c = unsafe {
        if SHIFT {
            KEYMAP_SHIFT[scancode as usize]
        } else {
            KEYMAP[scancode as usize]
        }
    };
    if c != 0 {
        write_buf(c);
    }
}

#[no_mangle]
extern "C" fn keyboard_handler() {
    let status = inb(STATUS);
    if status & 1 != 0 {
        let scancode = inb(DATA);
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
    loop {
        unsafe {
            while KEYBUF_R == KEYBUF_W {}
            let c = KEYBUF[KEYBUF_R];
            KEYBUF_R = (KEYBUF_R + 1) % BUF_SIZE;
            return c;
        }
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
