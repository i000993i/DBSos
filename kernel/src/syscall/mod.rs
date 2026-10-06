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
    // exec-редирект: ставит sys_execve, потребляет syscall_stub на возврате.
    // RIP=точка входа нового образа, RSP=его стек, CR3=его pml4.
    pub static mut exec_redir_rip: u64;
    pub static mut exec_redir_rsp: u64;
    pub static mut exec_redir_cr3: u64;
}

/// Запросить редирект возврата из syscall на новый образ (execve).
/// Вызывать ДО возврата из хендлера; stub сам сбросит флаг.
pub unsafe fn request_exec_redirect(rip: u64, rsp: u64, cr3: u64) {
    exec_redir_rip = rip;
    exec_redir_rsp = rsp;
    exec_redir_cr3 = cr3;
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
    ".globl exec_redir_rip",
    "exec_redir_rip: .quad 0",
    ".globl exec_redir_rsp",
    "exec_redir_rsp: .quad 0",
    ".globl exec_redir_cr3",
    "exec_redir_cr3: .quad 0",
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
    // Push order: r11, rcx, rax, rdx, rbx, rbp, rsi, rdi, r8, r9, r10, r12, r13, r14, r15
    // Stack offsets after push:
    //   [rsp+12*8]=rax(user num) [rsp+11*8]=rdx(user arg3) [rsp+8*8]=rsi(user arg2)
    //   [rsp+7*8]=rdi(user arg1) [rsp+4*8]=r10(user arg4)
    // Win64 ABI — target x86_64-unknown-uefi: extern "C" == win64:
    //   rcx=num, rdx=arg1, r8=arg2, r9=arg3, [rsp+32]=arg4
    "  mov rcx, [rsp + 12*8]",   // rcx = user rax = num
    "  mov rdx, [rsp + 7*8]",    // rdx = user rdi = arg1
    "  mov r8,  [rsp + 8*8]",    // r8  = user rsi = arg2
    "  mov r9,  [rsp + 11*8]",   // r9  = user rdx = arg3
    "  mov rax, [rsp + 4*8]",    // rax = user r10 = arg4 (5-й аргумент)
    "  sub rsp, 40",             // shadow space (32) + слот 5-го аргумента (8)
    "  mov [rsp + 32], rax",     // arg4 в 5-й слот
    "  call syscall_rust_entry",
    "  add rsp, 40",
    // exec-редирект: sys_execve подменил образ — вернуться надо не в точку
    // вызова, а в entry нового образа с его стеком и CR3.
    // Слоты на стеке (после add rsp,40): [rsp+13*8]=saved RCX(RIP),
    // [rsp+12*8]=saved RAX(retval), [rsp+0..11*8]=остальные регистры.
    "  mov r10, [rip + exec_redir_rip]",
    "  test r10, r10",
    "  jz 4f",
    "  mov rax, [rip + exec_redir_cr3]",
    "  mov cr3, rax",
    "  mov rax, [rip + exec_redir_rsp]",
    "  mov [rip + sys_ursave], rax",
    "  xor eax, eax",
    "  mov [rsp + 0*8], rax",
    "  mov [rsp + 1*8], rax",
    "  mov [rsp + 2*8], rax",
    "  mov [rsp + 3*8], rax",
    "  mov [rsp + 4*8], rax",
    "  mov [rsp + 5*8], rax",
    "  mov [rsp + 6*8], rax",
    "  mov [rsp + 7*8], rax",
    "  mov [rsp + 8*8], rax",
    "  mov [rsp + 9*8], rax",
    "  mov [rsp + 10*8], rax",
    "  mov [rsp + 11*8], rax",
    "  mov [rsp + 12*8], rax",
    "  mov [rsp + 13*8], r10",
    "  mov qword ptr [rip + exec_redir_rip], 0",
    "  jmp 5f",
    "4:",
    "  cmp rax, -1",
    "  je 3f",
    "5:",
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
