// Process management: exit, init

use core::arch::asm;
use super::{TaskState, Task, STACK_SIZE, CURRENT, TASKS};
use super::task::find_ready;
use dbsos_abi::ipc::Message;

pub fn exit() {
    unsafe {
        asm!("cli");
        let cur = CURRENT;
        let was_ring3 = TASKS[cur].ring3;

        let a = &TASKS[cur];
        let code_p = a.code_phys;
        let user_stack_p = a.user_stack_phys;
        let gdt_p = a.gdt_phys;
        let tss_p = a.tss_phys;
        let pml4_p = a.pml4_phys;
        let fpu_p = a.fpu_buf_phys;
        if code_p != 0 { crate::memory::pfree(code_p); }
        if user_stack_p != 0 { crate::memory::pfree(user_stack_p); }
        if gdt_p != 0 { crate::memory::pfree(gdt_p); }
        if tss_p != 0 { crate::memory::pfree(tss_p); }
        if pml4_p != 0 { crate::vm::destroy_address_space(pml4_p as *mut u64); }
        if fpu_p != 0 { crate::memory::pfree(fpu_p); }

        TASKS[cur].state = TaskState::Exited;
        let next = find_ready();
        if next == cur {
            loop { asm!("hlt"); }
        }
        TASKS[next].state = TaskState::Running;
        CURRENT = next;
        if TASKS[next].ring3 {
            crate::vm::switch_to(TASKS[next].pml4 as *mut u64);
            let ktop = (TASKS[next].stack_base as u64) + STACK_SIZE as u64;
            super::context::load_gdt_tss(TASKS[next].gdt_phys, TASKS[next].tss_phys, ktop);
            crate::syscall::set_sys_krsp(ktop);
            crate::syscall::set_sys_ursave(TASKS[next].sys_ursave);
        } else if was_ring3 {
            crate::vm::switch_to(crate::vm::KERNEL_PML4 as *mut u64);
        }
        let sp = TASKS[next].sp;
        asm!(
            "mov rsp, {0}",
            "pop r15", "pop r14", "pop r13", "pop r12",
            "pop r11", "pop r10", "pop r9",  "pop r8",
            "pop rdi", "pop rsi", "pop rbp", "pop rbx",
            "pop rdx", "pop rcx", "pop rax",
            "iretq",
            in(reg) sp,
        );
    }
}

pub fn init() {
    super::task::fpu_init();
    unsafe {
        CURRENT = 0;
        TASKS[0] = Task {
            state: TaskState::Running,
            stack_base: 0 as *mut u8,
            sp: 0, id: 0,
            ipc_partner: 0, ipc_val: 0,
            pml4: core::ptr::null_mut(), pml4_phys: 0,
            gdt_phys: 0, tss_phys: 0, ring3: false, sys_ursave: 0,
            code_phys: 0, user_stack_phys: 0, kstack_phys: 0, fpu_buf_phys: 0,
            pending_msg: Message::empty(),
        };
    }
}
