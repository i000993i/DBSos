/// Драйвер UART 16550 (Serial Port) — реализует Driver trait
/// Поддерживает IRQ-driven приём (IRQ4 → vector 0x24)

use crate::io;
use super::traits::*;

const COM1: u16 = 0x3F8;

const LSR_THR_EMPTY: u8 = 1 << 5;
const LSR_DATA_READY: u8 = 1 << 0;
const LCR_DLAB: u8 = 1 << 7;
const LCR_8N1: u8 = 0x03;
const MCR_DTR: u8 = 1 << 0;
const MCR_RTS: u8 = 1 << 1;
const IER_RX_DATA: u8 = 1 << 0;

static mut INITIALIZED: bool = false;

// IRQ-driven receive ring buffer
const RX_BUF_SIZE: usize = 1024;
static mut RX_BUF: [u8; RX_BUF_SIZE] = [0; RX_BUF_SIZE];
static mut RX_HEAD: usize = 0;
static mut RX_TAIL: usize = 0;

pub struct UartDriver;

impl Driver for UartDriver {
    fn name(&self) -> &'static str {
        "UART 16550 (COM1)"
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Legacy
    }

    fn init(&self) -> DriverStatus {
        if unsafe { INITIALIZED } {
            return DriverStatus::Ok;
        }
        unsafe {
            io::outb(COM1 + 3, LCR_DLAB);
            io::outb(COM1 + 0, 12);
            io::outb(COM1 + 1, 0);
            io::outb(COM1 + 3, LCR_8N1);
            io::outb(COM1 + 2, 0xC7);
            io::outb(COM1 + 4, MCR_DTR | MCR_RTS);
            INITIALIZED = true;
        }
        write_str("[UART] Serial port ready\r\n");
        DriverStatus::Ok
    }
}

/// Enable UART RX interrupt (IRQ4 → vector 0x24)
pub fn enable_irq() {
    unsafe {
        // Set IDT entry for vector 0x24 (IRQ4 + PIC remap 0x20)
        let entry = &mut crate::interrupts::IDT[0x24];
        entry.set_handler(uart_irq_stub as *const () as u64, 0x08);
        // Enable RX data interrupt in UART IER
        io::outb(COM1 + 1, IER_RX_DATA);
        // Unmask IRQ4 in PIC (clear bit 4)
        let mask = io::inb(0x21);
        io::outb(0x21, mask & !0x10);
    }
}

/// IRQ handler called from uart_irq_stub
#[no_mangle]
extern "C" fn uart_irq_handler() {
    unsafe {
        while io::inb(COM1 + 5) & LSR_DATA_READY != 0 {
            let byte = io::inb(COM1);
            let next = (RX_HEAD + 1) % RX_BUF_SIZE;
            if next != RX_TAIL {
                RX_BUF[RX_HEAD] = byte;
                RX_HEAD = next;
            }
            // Overflow: drop oldest byte
        }
    }
    // EOI to PIC master
    unsafe { io::outb(0x20, 0x20); }
}

core::arch::global_asm!(
    ".globl uart_irq_stub",
    "uart_irq_stub:",
    "  push rax", "push rcx", "push rdx", "push rbx",
    "  push rbp", "push rsi", "push rdi",
    "  push r8", "push r9", "push r10", "push r11",
    "  push r12", "push r13", "push r14", "push r15",
    "  sub rsp, 32",
    "  call uart_irq_handler",
    "  add rsp, 32",
    "  pop r15", "pop r14", "pop r13", "pop r12",
    "  pop r11", "pop r10", "pop r9", "pop r8",
    "  pop rdi", "pop rsi", "pop rbp",
    "  pop rbx", "pop rdx", "pop rcx", "pop rax",
    "  iretq",
);

extern "C" { fn uart_irq_stub(); }

pub fn putchar(byte: u8) {
    unsafe {
        while io::inb(COM1 + 5) & LSR_THR_EMPTY == 0 {}
        io::outb(COM1, byte);
    }
}

pub fn getchar() -> u8 {
    loop {
        unsafe {
            if RX_HEAD != RX_TAIL {
                let c = RX_BUF[RX_TAIL];
                RX_TAIL = (RX_TAIL + 1) % RX_BUF_SIZE;
                return c;
            }
        }
        core::hint::spin_loop();
    }
}

pub fn poll_char() -> Option<u8> {
    unsafe {
        if RX_HEAD != RX_TAIL {
            let c = RX_BUF[RX_TAIL];
            RX_TAIL = (RX_TAIL + 1) % RX_BUF_SIZE;
            Some(c)
        } else {
            None
        }
    }
}

pub fn write_str(s: &str) {
    for &byte in s.as_bytes() {
        if byte == b'\n' {
            putchar(b'\r');
        }
        putchar(byte);
    }
}

pub fn write_bytes(data: &[u8]) {
    for &b in data {
        putchar(b);
    }
}
