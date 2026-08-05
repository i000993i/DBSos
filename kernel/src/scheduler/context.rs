// Context switching: timer_stub, reschedule, load_gdt_tss, yield_now

use core::arch::asm;
use super::{TaskState, STACK_SIZE, CURRENT, TASKS};
use super::task;

static mut TICK_COUNT: u64 = 0;

core::arch::global_asm!(
    ".globl timer_stub",
    "timer_stub:",
    "  push rax",  "  push rcx",  "  push rdx",  "  push rbx",
    "  push rbp",  "  push rsi",  "  push rdi",  "  push r8",
    "  push r9",   "  push r10",  "  push r11",  "  push r12",
    "  push r13",  "  push r14",  "  push r15",
    "  mov rcx, 0xFEE000B0",
    "  xor eax, eax",
    "  mov [rcx], eax",
    "  mov rcx, rsp",
    "  sub rsp, 32",
    "  call reschedule",
    "  add rsp, 32",
    "  mov rsp, rax",
    "  pop r15",  "  pop r14",  "  pop r13",  "  pop r12",
    "  pop r11",  "  pop r10",  "  pop r9",   "  pop r8",
    "  pop rdi",  "  pop rsi",  "  pop rbp",  "  pop rbx",
    "  pop rdx",  "  pop rcx",  "  pop rax",
    "  iretq",
);

extern "C" { fn timer_stub(); }

pub unsafe fn load_gdt_tss(gdt_phys: u64, tss_phys: u64, stack_top: u64) {
    let pd = crate::interrupts::GdtPacked { limit: (8*8 - 1) as u16, base: gdt_phys };
    asm!("lgdt [{p}]", p = in(reg) &pd as *const _ as u64);
    core::ptr::write_unaligned((gdt_phys + 53) as *mut u8, 0x89u8);
    core::ptr::write_unaligned((tss_phys + 4) as *mut u64, stack_top);
    asm!("mov ax, 0x30", "ltr ax", out("ax") _, options(nostack));
}

#[no_mangle]
unsafe extern "C" fn reschedule(rsp: u64) -> u64 {
    TICK_COUNT += 1;
    let cur = CURRENT;
    let next = task::find_ready();
    if next != cur {
        if TASKS[cur].state == TaskState::Running || TASKS[cur].state == TaskState::Ready {
            TASKS[cur].state = TaskState::Ready;
        }
        let cur_fpu = TASKS[cur].fpu_buf_phys;
        if cur_fpu != 0 {
            asm!("fxsave64 [{}]", in(reg) cur_fpu, options(nostack));
        }
        TASKS[cur].sp = rsp;
        TASKS[next].state = TaskState::Running;
        CURRENT = next;
        let n = &TASKS[next];
        if n.ring3 {
            crate::vm::switch_to(n.pml4 as *mut u64);
            let ktop = (n.stack_base as u64) + STACK_SIZE as u64;
            load_gdt_tss(n.gdt_phys, n.tss_phys, ktop);
            crate::syscall::set_sys_krsp(ktop);
            crate::syscall::set_sys_ursave(n.sys_ursave);
        }
        let next_fpu = TASKS[next].fpu_buf_phys;
        if next_fpu != 0 {
            asm!("fxrstor64 [{}]", in(reg) next_fpu, options(nostack));
        }
        TASKS[next].sp
    } else {
        rsp
    }
}

core::arch::global_asm!(
    ".globl yield_now_asm",
    "yield_now_asm:",
    "  cli",
    "  push rax",
    "  push 0x10",
    "  lea rax, [rsp + 8]",
    "  push rax",
    "  pushfq",
    "  push 0x08",
    "  lea rax, [rip + 3f]",
    "  push rax",
    "  mov rax, [rsp + 40]",
    "  push rax",  "  push rcx",  "  push rdx",  "  push rbx",
    "  push rbp",  "  push rsi",  "  push rdi",  "  push r8",
    "  push r9",   "  push r10",  "  push r11",  "  push r12",
    "  push r13",  "  push r14",  "  push r15",
    "  mov rcx, rsp",
    "  sub rsp, 32",
    "  call reschedule",
    "  add rsp, 32",
    "  mov rsp, rax",
    "  pop r15",  "  pop r14",  "  pop r13",  "  pop r12",
    "  pop r11",  "  pop r10",  "  pop r9",   "  pop r8",
    "  pop rdi",  "  pop rsi",  "  pop rbp",  "  pop rbx",
    "  pop rdx",  "  pop rcx",  "  pop rax",
    "  iretq",
    "3:",
    "  add rsp, 8",
    "  ret",
);

extern "C" { fn yield_now_asm(); }

pub fn yield_now() {
    unsafe { yield_now_asm(); }
}

pub fn install_timer_isr() {
    unsafe {
        let entry = &mut crate::interrupts::IDT[32];
        entry.set_handler(timer_stub as *const () as u64, 0x08);
    }
}
