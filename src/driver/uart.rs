/// Драйвер UART 16550 (Serial Port) — реализует Driver trait

use crate::io;
use super::traits::*;

const COM1: u16 = 0x3F8;

const LSR_THR_EMPTY: u8 = 1 << 5;
const LSR_DATA_READY: u8 = 1 << 0;
const LCR_DLAB: u8 = 1 << 7;
const LCR_8N1: u8 = 0x03;
const MCR_DTR: u8 = 1 << 0;
const MCR_RTS: u8 = 1 << 1;

static mut INITIALIZED: bool = false;

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

pub fn putchar(byte: u8) {
    unsafe {
        while io::inb(COM1 + 5) & LSR_THR_EMPTY == 0 {}
        io::outb(COM1, byte);
    }
}

pub fn getchar() -> u8 {
    unsafe {
        while io::inb(COM1 + 5) & LSR_DATA_READY == 0 {}
        io::inb(COM1)
    }
}

pub fn poll_char() -> Option<u8> {
    unsafe {
        if io::inb(COM1 + 5) & LSR_DATA_READY != 0 {
            Some(io::inb(COM1))
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
