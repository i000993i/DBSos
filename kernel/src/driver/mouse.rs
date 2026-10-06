/// PS/2 Mouse driver (IRQ12 → vector 0x2C)
/// Protocol: 3-byte packets
///   Byte 0: [always 1] [sign X] [sign Y] [always 0] [middle] [right] [left]
///   Byte 1: X delta (signed)
///   Byte 2: Y delta (signed, inverted — up is positive)

use core::arch::asm;
use crate::io;

const STATUS: u16 = 0x64;
const DATA: u16 = 0x60;

pub static mut LEFT_BTN: bool = false;
pub static mut RIGHT_BTN: bool = false;
pub static mut MIDDLE_BTN: bool = false;

pub static mut MOUSE_X: i32 = 640;
pub static mut MOUSE_Y: i32 = 400;

static mut PACKET: [u8; 3] = [0; 3];
static mut PACKET_BYTE: usize = 0;

pub static mut IRQ_COUNT: u64 = 0;
pub static mut RAW_BYTE_COUNT: u64 = 0;

// ── Mouse event ring buffer ────────────────────────────────────
const MOUSE_BUF_SIZE: usize = 256;

#[derive(Copy, Clone)]
pub struct MouseEvent {
    pub dx: i32,
    pub dy: i32,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

static mut MOUSE_BUF: [MouseEvent; MOUSE_BUF_SIZE] = [MouseEvent { dx: 0, dy: 0, left: false, right: false, middle: false }; MOUSE_BUF_SIZE];
static mut MOUSE_BUF_R: usize = 0;
static mut MOUSE_BUF_W: usize = 0;

fn cli() { unsafe { asm!("cli"); } }
fn sti() { unsafe { asm!("sti"); } }

fn inb(port: u16) -> u8 {
    unsafe { io::inb(port) }
}

fn outb(port: u16, val: u8) {
    unsafe { io::outb(port, val); }
}

/// Send command to mouse via i8042 (cmd + data byte)
fn mouse_cmd(cmd: u8, data: u8) {
    wait_write();
    outb(STATUS, 0xD4);
    wait_write();
    outb(DATA, cmd);
    wait_write();
    outb(DATA, data);
}

/// Send command-only (no data) to mouse via i8042
fn mouse_cmd_no_data(cmd: u8) {
    wait_write();
    outb(STATUS, 0xD4);
    wait_write();
    outb(DATA, cmd);
}

/// Poll one byte from DATA port (with timeout), returns None on timeout
fn read_byte() -> Option<u8> {
    let mut timeout = 100_000u32;
    while timeout > 0 {
        if inb(STATUS) & 1 != 0 {
            return Some(inb(DATA));
        }
        timeout -= 1;
    }
    None
}

/// Flush all pending bytes from the data port
fn flush_buffer() {
    while inb(STATUS) & 1 != 0 {
        inb(DATA);
    }
}

fn wait_write() {
    let mut timeout = 100_000u32;
    while timeout > 0 {
        if inb(STATUS) & 2 == 0 { return; }
        timeout -= 1;
    }
}

fn wait_read() {
    let mut timeout = 100_000u32;
    while timeout > 0 {
        if inb(STATUS) & 1 != 0 { return; }
        timeout -= 1;
    }
}

fn hex_byte(v: u8) {
    let hex = b"0123456789ABCDEF";
    crate::driver::uart::putchar(hex[((v >> 4) & 0xF) as usize]);
    crate::driver::uart::putchar(hex[(v & 0xF) as usize]);
}

pub fn init() {
    cli();

    unsafe {
        let entry = &mut crate::interrupts::IDT[0x2C];
        entry.set_handler(mouse_irq_stub as *const () as u64, 0x08);
    }

    // Step 1: Disable both keyboard and mouse
    wait_write();
    outb(STATUS, 0xAD); // Disable keyboard
    wait_write();
    outb(STATUS, 0xA7); // Disable mouse

    // Step 2: Flush buffer (IRQs are off, no handler can steal bytes)
    flush_buffer();

    // Step 3: Read controller config byte
    wait_write();
    outb(STATUS, 0x20);
    wait_read();
    let config = inb(DATA);

    // Step 4: Write config with IRQ12 enabled (bit 1) and IRQ1 enabled (bit 0)
    wait_write();
    outb(STATUS, 0x60);
    wait_write();
    outb(DATA, config | 0x02 | 0x01); // Enable both IRQ12 and IRQ1

    // Step 5: Re-enable keyboard
    wait_write();
    outb(STATUS, 0xAE);

    // ── Mouse reset (all with IRQs off, no handler race) ──
    // Send 0xFF reset command
    mouse_cmd_no_data(0xFF);
    wait_read(); let ack1 = inb(DATA);  // 0xFA ACK
    wait_read(); let ack2 = inb(DATA);  // 0xAA self-test OK
    let _ = read_byte();                // drain any extra byte

    // Reset packet decoder state
    unsafe {
        PACKET_BYTE = 0;
        MOUSE_X = 640;
        MOUSE_Y = 400;
    }

    // ── Configure mouse ──
    // Set sample rate 200 Hz (standard for PS/2)
    mouse_cmd(0xF3, 200);
    wait_read(); let _ = inb(DATA); // ACK

    // Set resolution 3 = 8 counts/mm
    mouse_cmd(0xE8, 0x03);
    wait_read(); let _ = inb(DATA); // ACK

    // Enable data reporting (REQUIRED for packets to be sent)
    mouse_cmd(0xF4, 0);
    wait_read(); let _ = inb(DATA); // ACK

    // Step 6: Final flush before enabling IRQs
    flush_buffer();

    // ── Verify PIC masks before unmasking ──
    let pic1_before = inb(0x21);
    let pic2_before = inb(0xA1);

    // Step 7: Unmask IRQ2 (cascade) in PIC1 and IRQ12 in PIC2
    outb(0x21, pic1_before & !0x04); // Unmask IRQ2 (cascade)
    let pic2_new = pic2_before & !0x10;
    outb(0xA1, pic2_new);            // Unmask IRQ12

    // Verify after
    let pic1_after = inb(0x21);
    let pic2_after = inb(0xA1);

    // ── Now enable interrupts ──
    sti();

    // ── Print diagnostics ──
    crate::driver::uart::write_str("[MOUSE] init OK\r\n");
    crate::driver::uart::write_str("  config=");
    hex_byte(config);
    crate::driver::uart::write_str(" ACK1=");
    hex_byte(ack1);
    crate::driver::uart::write_str(" ACK2=");
    hex_byte(ack2);
    crate::driver::uart::write_str("\r\n");
    crate::driver::uart::write_str("  PIC1 before=");
    hex_byte(pic1_before);
    crate::driver::uart::write_str(" after=");
    hex_byte(pic1_after);
    crate::driver::uart::write_str("  PIC2 before=");
    hex_byte(pic2_before);
    crate::driver::uart::write_str(" after=");
    hex_byte(pic2_after);
    crate::driver::uart::write_str("\r\n");

    // Check that IRQ12 is unmasked
    if pic2_after & 0x10 != 0 {
        crate::driver::uart::write_str("  [WARN] IRQ12 still masked!\r\n");
    }
    if pic1_after & 0x04 != 0 {
        crate::driver::uart::write_str("  [WARN] IRQ2 (cascade) still masked!\r\n");
    }
}

/// IRQ handler — called from mouse_irq_stub
#[no_mangle]
extern "C" fn mouse_irq_handler() {
    unsafe {
        IRQ_COUNT += 1;

        let status = inb(STATUS);
        // Фильтр AUX: бит 5 = 1 — данные от мыши; бит 0 = выходной буфер полон.
        // Без проверки сюда попадают байты клавиатуры (IRQ1) при гонке i8042.
        if status & 1 != 0 && status & 0x20 != 0 {
            let byte = inb(DATA);
            RAW_BYTE_COUNT += 1;
            PACKET[PACKET_BYTE] = byte;

            match PACKET_BYTE {
                0 => {
                    if byte & 0x08 != 0 {
                        PACKET_BYTE = 1;
                    }
                }
                1 => {
                    PACKET_BYTE = 2;
                }
                2 => {
                    let b0 = PACKET[0];
                    let dx = PACKET[1] as i8 as i32;
                    let dy = -(PACKET[2] as i8 as i32);

                    let old_left = LEFT_BTN;
                    let old_right = RIGHT_BTN;
                    let old_middle = MIDDLE_BTN;

                    LEFT_BTN = b0 & 0x01 != 0;
                    RIGHT_BTN = b0 & 0x02 != 0;
                    MIDDLE_BTN = b0 & 0x04 != 0;

                    MOUSE_X += dx;
                    MOUSE_Y += dy;

                    let w = crate::display::width() as i32;
                    let h = crate::display::height() as i32;
                    if MOUSE_X < 0 { MOUSE_X = 0; }
                    if MOUSE_Y < 0 { MOUSE_Y = 0; }
                    if MOUSE_X >= w { MOUSE_X = w - 1; }
                    if MOUSE_Y >= h { MOUSE_Y = h - 1; }

                    // Forward to Wayland seat (motion + button transitions)
                    crate::wayland::seat::send_pointer_motion(dx, dy);
                    if LEFT_BTN != old_left {
                        crate::wayland::seat::send_pointer_button(0x110, LEFT_BTN);
                    }
                    if RIGHT_BTN != old_right {
                        crate::wayland::seat::send_pointer_button(0x111, RIGHT_BTN);
                    }
                    if MIDDLE_BTN != old_middle {
                        crate::wayland::seat::send_pointer_button(0x112, MIDDLE_BTN);
                    }

                    // Write to ring buffer
                    let next = (MOUSE_BUF_W + 1) % MOUSE_BUF_SIZE;
                    if next != MOUSE_BUF_R {
                        MOUSE_BUF[MOUSE_BUF_W] = MouseEvent {
                            dx, dy,
                            left: LEFT_BTN,
                            right: RIGHT_BTN,
                            middle: MIDDLE_BTN,
                        };
                        MOUSE_BUF_W = next;
                    }

                    // Also push to unified event system
                    crate::event::push_mouse(dx, dy, LEFT_BTN, RIGHT_BTN, MIDDLE_BTN);

                    PACKET_BYTE = 0;
                }
                _ => { PACKET_BYTE = 0; }
            }
        }
    }
    // EOI — PIC2 first (IRQ12 is on slave), then PIC1
    outb(0xA0, 0x20); // EOI to PIC slave
    outb(0x20, 0x20); // EOI to PIC master
}

core::arch::global_asm!(
    ".globl mouse_irq_stub",
    "mouse_irq_stub:",
    "  push rax", "push rcx", "push rdx", "push rbx",
    "  push rbp", "push rsi", "push rdi",
    "  push r8", "push r9", "push r10", "push r11",
    "  push r12", "push r13", "push r14", "push r15",
    "  sub rsp, 32",
    "  call mouse_irq_handler",
    "  add rsp, 32",
    "  pop r15", "pop r14", "pop r13", "pop r12",
    "  pop r11", "pop r10", "pop r9", "pop r8",
    "  pop rdi", "pop rsi", "pop rbp",
    "  pop rbx", "pop rdx", "pop rcx", "pop rax",
    "  iretq",
);

extern "C" { fn mouse_irq_stub(); }

// ── Public API ────────────────────────────────────────────────────

pub fn x() -> i32 { unsafe { MOUSE_X } }
pub fn y() -> i32 { unsafe { MOUSE_Y } }
pub fn left() -> bool { unsafe { LEFT_BTN } }
pub fn right() -> bool { unsafe { RIGHT_BTN } }
pub fn middle() -> bool { unsafe { MIDDLE_BTN } }
pub fn irq_count() -> u64 { unsafe { IRQ_COUNT } }
pub fn raw_byte_count() -> u64 { unsafe { RAW_BYTE_COUNT } }

/// Non-blocking: read next mouse event from ring buffer
pub fn poll_event() -> Option<MouseEvent> {
    unsafe {
        if MOUSE_BUF_R != MOUSE_BUF_W {
            let ev = MOUSE_BUF[MOUSE_BUF_R];
            MOUSE_BUF_R = (MOUSE_BUF_R + 1) % MOUSE_BUF_SIZE;
            Some(ev)
        } else {
            None
        }
    }
}
