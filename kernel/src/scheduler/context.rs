// Context switching: timer_stub, reschedule, load_gdt_tss, yield_now

use core::arch::asm;
use super::{TaskState, STACK_SIZE, STACK_CANARY, CURRENT, TASKS};
use super::task;

static mut TICK_COUNT: u64 = 0;

core::arch::global_asm!(
    ".globl timer_stub",
    "timer_stub:",
    "  push rax",  "  push rcx",  "  push rdx",  "  push rbx",
    "  push rbp",  "  push rsi",  "  push rdi",  "  push r8",
    "  push r9",   "  push r10",  "  push r11",  "  push r12",
    "  push r13",  "  push r14",  "  push r15",
    "  mov rcx, 0xFFFFFFFFFEE000B0",
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

/// Reload the default kernel GDT (built at boot by interrupts::init).
/// Must be called when returning to a ring-0 (non-ring3) task so the CPU stops
/// using a (possibly freed) per-task GDT.
pub unsafe fn load_kernel_gdt() {
    let pd = crate::interrupts::GdtPacked {
        limit: crate::interrupts::KERNEL_GDT_LIMIT,
        base: crate::interrupts::KERNEL_GDT_BASE,
    };
    asm!("lgdt [{p}]", p = in(reg) &pd as *const _ as u64);
}

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

    // Stack canary check for current task
    if TASKS[cur].stack_base as u64 != 0 {
        let canary = *(TASKS[cur].stack_base as *const u64);
        if canary != STACK_CANARY {
            crate::driver::uart::write_str("\r\n!!! STACK OVERFLOW in task ");
            let hex = b"0123456789ABCDEF";
            crate::driver::uart::putchar(hex[((TASKS[cur].id >> 4) & 0xF) as usize]);
            crate::driver::uart::putchar(hex[(TASKS[cur].id & 0xF) as usize]);
            crate::driver::uart::write_str(" — HALTED\r\n");
            loop { core::hint::spin_loop(); }
        }
    }

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
        } else {
            // Ring-0 (kernel) task: always switch to the kernel address space
            // and kernel GDT. Without this, a kernel task would keep running on
            // the previous user task's PML4 (stale CR3) and a freed per-task GDT,
            // which breaks user-pointer dereferences in syscall handlers (#PF)
            // and can produce corrupt segment selectors.
            crate::vm::switch_to(crate::vm::KERNEL_PML4 as *mut u64);
            load_kernel_gdt();
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
