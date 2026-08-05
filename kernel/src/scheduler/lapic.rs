// LAPIC timer functions

use core::arch::asm;
use crate::io;

const LAPIC_BASE: u64 = 0xFFFF_FFFF_FEE0_0000;

fn lapic_write(offset: u32, value: u32) {
    unsafe { io::mmio_write32((LAPIC_BASE + offset as u64) as *mut u32, value) }
}

fn lapic_read(offset: u32) -> u32 {
    unsafe { io::mmio_read32((LAPIC_BASE + offset as u64) as *mut u32) }
}

fn lapic_base() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        asm!("rdmsr", out("eax") lo, out("edx") hi, in("ecx") 0x1Bu32);
        ((hi as u64) << 32) | (lo as u64) & 0xFFFFF000
    }
}

pub fn lapic_timer_init() {
    unsafe {
        let base = lapic_base();
        crate::driver::uart::write_str("[LAPIC] base=");
        super::uart_hex(base);
        crate::driver::uart::write_str("\r\n");

        let sivr = lapic_read(0xF0);
        crate::driver::uart::write_str("[LAPIC] SIVR=");
        crate::driver::uart::putchar(b'0');
        crate::driver::uart::putchar(b'x');
        let hex = b"0123456789ABCDEF";
        for i in (0..8).rev() {
            crate::driver::uart::putchar(hex[((sivr >> (i * 4)) & 0xF) as usize]);
        }
        crate::driver::uart::write_str("\r\n");

        if sivr & 0x100 == 0 {
            lapic_write(0xF0, sivr | 0x100 | 0x32);
        }

        lapic_write(0x3E0, 0x3);      // DCR = divide by 16
        lapic_write(0x380, 62500);     // initial count ~100 Hz
        lapic_write(0x320, 0x20u32 | (1u32 << 17)); // LVT Timer: vector=0x20, periodic

        crate::driver::uart::write_str("[LAPIC] timer configured\r\n");

        super::context::install_timer_isr();
        asm!("sti");
    }
}
