// HPET (High Precision Event Timer)
// QEMU q35: MMIO 0xFED00000, базовая частота 10 MHz (100 ns = 100_000 fs)

use core::ptr::{read_volatile, write_volatile};

const HPET_BASE: usize = 0xFED0_0000;

const GCAP_ID: usize = 0x000;
const GEN_CONF: usize = 0x010;
const MAIN_CNT: usize = 0x0F0;

const HPET_ENABLE: u64 = 1;

fn hpet_read(offset: usize) -> u64 {
    unsafe { read_volatile((HPET_BASE + offset) as *const u64) }
}

fn hpet_write(offset: usize, val: u64) {
    unsafe { write_volatile((HPET_BASE + offset) as *mut u64, val) }
}

pub fn init() {
    let cap = hpet_read(GCAP_ID);
    if cap == 0 || cap == u64::MAX {
        crate::driver::uart::write_str("[TIMER] HPET not found\r\n");
        return;
    }

    hpet_write(GEN_CONF, 0);
    hpet_write(MAIN_CNT, 0);
    hpet_write(GEN_CONF, HPET_ENABLE);

    uart_print("[TIMER] HPET ready (10 MHz)\r\n");
}

pub fn usleep(us: u64) {
    let target_ticks = us * 10; // 10 ticks per μs at 10 MHz
    let start = hpet_read(MAIN_CNT);
    loop {
        let now = hpet_read(MAIN_CNT);
        if now.wrapping_sub(start) >= target_ticks {
            break;
        }
        core::hint::spin_loop();
    }
}

pub fn millis() -> u64 {
    hpet_read(MAIN_CNT) / 10_000 // 10,000 ticks per ms
}

pub fn ticks() -> u64 {
    hpet_read(MAIN_CNT)
}

fn uart_print(s: &str) {
    crate::driver::uart::write_str(s);
}
