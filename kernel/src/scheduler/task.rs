// Task management helpers

use core::arch::asm;
use super::{TaskState, MAX_TASKS, CURRENT, TASKS};

pub fn find_ready() -> usize {
    let cur = unsafe { CURRENT };
    for i in 1..=MAX_TASKS {
        let idx = (cur + i) % MAX_TASKS;
        if unsafe { TASKS[idx].state == TaskState::Ready } {
            return idx;
        }
    }
    cur
}

pub fn fpu_init() {
    unsafe {
        let mut cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0);
        cr0 &= !(1u64 << 2);
        cr0 |= 1u64 << 1;
        cr0 |= 1u64 << 5;
        asm!("mov cr0, {}", in(reg) cr0);

        let mut cr4: u64;
        asm!("mov {}, cr4", out(reg) cr4);
        cr4 |= 1u64 << 9;
        cr4 |= 1u64 << 10;
        asm!("mov cr4, {}", in(reg) cr4);

        asm!("fninit");
    }
}

pub unsafe fn fpu_alloc_buf() -> u64 {
    let buf = crate::memory::palloc();
    if buf == 0 { return 0; }
    asm!("fxsave64 [{}]", in(reg) buf, options(nostack));
    buf
}
