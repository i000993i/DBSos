// syscall/sysret — вход из ring 3 в ring 0

pub mod dispatch;
pub mod env;
pub mod emit;
pub mod tests;

pub use env::{init, prepare_user_pml4, create_user_task_env, setup_user_gdt_tss};
pub use tests::{test_ring3, test_ring3_e1000, test_ring3_console};

extern "C" {
    fn syscall_stub();
    pub static mut sys_krsp: u64;
    static mut sys_ursave: u64;
    pub static mut sys_kret: u64;
}

core::arch::global_asm!(
    ".section .bss",
    ".balign 8",
    ".globl sys_krsp",
    "sys_krsp: .quad 0",
    ".globl sys_ursave",
    "sys_ursave: .quad 0",
    "sys_retval: .quad 0",
    ".globl sys_kret",
    "sys_kret: .quad 0",
    ".section .text",

    ".globl syscall_stub",
    ".balign 64",
    "syscall_stub:",
    "  mov [rip + sys_ursave], rsp",
    "  mov rsp, [rip + sys_krsp]",
    "  push r11",  "  push rcx",  "  push rax",  "  push rdx",
    "  push rbx",  "  push rbp",  "  push rsi",  "  push rdi",
    "  push r8",   "  push r9",   "  push r10",  "  push r12",
    "  push r13",  "  push r14",  "  push r15",
    "  mov rcx, [rsp + 12*8]",
    "  mov rdx, [rsp + 11*8]",
    "  mov r8,  [rsp + 6*8]",
    "  mov r9,  [rsp + 5*8]",
    "  sub rsp, 32",
    "  call syscall_rust_entry",
    "  add rsp, 32",
    "  cmp rax, -1",
    "  je 3f",
    "  mov [rip + sys_retval], rax",
    "  pop r15",  "  pop r14",  "  pop r13",  "  pop r12",
    "  pop r10",  "  pop r9",   "  pop r8",   "  pop rdi",
    "  pop rsi",  "  pop rbp",  "  pop rbx",  "  pop rdx",
    "  pop rax",
    "  mov rax, [rip + sys_retval]",
    "  mov rcx, [rsp]",
    "  mov r11, [rsp + 8]",
    "  add rsp, 16",
    "  push 0x2B",
    "  push [rip + sys_ursave]",
    "  push r11",
    "  push 0x23",
    "  push rcx",
    "  iretq",
    "3:",
    "  add rsp, 15*8",
    "  mov rsp, [rip + sys_krsp]",
    "  mov rax, [rip + sys_kret]",
    "  call rax",
    "  cli",
    "  hlt",
);

pub unsafe fn set_sys_krsp(stack_top: u64) {
    core::ptr::addr_of_mut!(sys_krsp).write(stack_top);
}

pub unsafe fn set_sys_ursave(ursave: u64) {
    core::ptr::addr_of_mut!(sys_ursave).write(ursave);
}

pub unsafe fn read_sys_ursave() -> u64 {
    core::ptr::addr_of!(sys_ursave).read()
}

pub unsafe fn ring3_done() {
    crate::driver::uart::write_str("[RING3] back to kernel\r\n");
    crate::scheduler::exit();
}
