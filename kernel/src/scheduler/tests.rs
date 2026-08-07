// Scheduler tests: cooperative + preemption

use super::TASKS;
use super::context::yield_now;
use super::spawn::spawn;

extern "C" fn task_a() {
    crate::driver::uart::write_str("[A] start\r\n");
    yield_now();
    crate::driver::uart::write_str("[A] resumed\r\n");
    super::process::exit();
}

pub fn test() {
    crate::driver::uart::write_str("\r\n=== Cooperative test ===\r\n");
    let id = spawn(task_a).unwrap();
    crate::driver::uart::write_str("[MAIN] spawned id=");
    super::uart_hex(id);
    unsafe {
        let t = &TASKS[id as usize];
        crate::driver::uart::write_str(" sp=");
        super::uart_hex(t.sp);
        let frame = t.sp as *const u64;
        crate::driver::uart::write_str(" [RIP]=");
        super::uart_hex(*frame.add(15));
        crate::driver::uart::write_str(" [CS]=");
        super::uart_hex(*frame.add(16));
        crate::driver::uart::write_str(" [RFL]=");
        super::uart_hex(*frame.add(17));
        crate::driver::uart::write_str("\r\n");
    }
    crate::driver::uart::write_str("[MAIN] before yield\r\n");
    yield_now();
    crate::driver::uart::write_str("[MAIN] first back\r\n");
    yield_now();
    crate::driver::uart::write_str("[MAIN] second back\r\n");
    crate::driver::uart::write_str("=== done ===\r\n");
}

const NUM_WORKERS: usize = 4;
static mut COUNTERS: [u64; NUM_WORKERS] = [0; NUM_WORKERS];
static mut WORKERS_STOP: bool = false;
extern "C" fn worker0() { loop { if unsafe { WORKERS_STOP } { super::process::exit(); } unsafe { COUNTERS[0] += 1; } } }
extern "C" fn worker1() { loop { if unsafe { WORKERS_STOP } { super::process::exit(); } unsafe { COUNTERS[1] += 1; } } }
extern "C" fn worker2() { loop { if unsafe { WORKERS_STOP } { super::process::exit(); } unsafe { COUNTERS[2] += 1; } } }
extern "C" fn worker3() { loop { if unsafe { WORKERS_STOP } { super::process::exit(); } unsafe { COUNTERS[3] += 1; } } }

pub fn preempt_test() {
    crate::driver::uart::write_str("\r\n=== Preemption test ===\r\n");
    let workers: [extern "C" fn(); NUM_WORKERS] = [worker0, worker1, worker2, worker3];
    for &w in &workers { spawn(w); }
    crate::driver::uart::write_str("[PREEMPT] 4 workers spawned, waiting 2s...\r\n");
    let deadline = crate::timer::millis() + 2000;
    while crate::timer::millis() < deadline {
        core::hint::spin_loop();
    }
    unsafe {
        for i in 0..NUM_WORKERS {
            crate::driver::uart::write_str("  Worker ");
            crate::driver::uart::putchar(b'0' + i as u8);
            crate::driver::uart::write_str(": ");
            let mut v = COUNTERS[i];
            let mut buf = [0u8; 20]; let mut bi = 0;
            if v == 0 { crate::driver::uart::putchar(b'0'); } else {
                while v > 0 { buf[bi] = b'0' + (v % 10) as u8; v /= 10; bi += 1; }
                while bi > 0 { bi -= 1; crate::driver::uart::putchar(buf[bi]); }
            }
            crate::driver::uart::write_str(" iterations\r\n");
        }
    }
    unsafe { WORKERS_STOP = true; }
    crate::driver::uart::write_str("=== Preemption done ===\r\n");
}
