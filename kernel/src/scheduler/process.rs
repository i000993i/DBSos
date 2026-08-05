// Process management: exit, init

use core::arch::asm;
use super::{TaskState, Task, STACK_SIZE, MAX_TASKS, CURRENT, TASKS};
use super::task::find_ready;
use dbsos_abi::ipc::Message;

pub fn exit_with_status(status: i32) {
    unsafe {
        asm!("cli");

        // 1) Read resources of the dying process BEFORE we stop using its GDT/PML4.
        let cur = CURRENT;
        let parent_id = TASKS[cur].parent_id;

        let code_p = TASKS[cur].code_phys;
        let user_stack_p = TASKS[cur].user_stack_phys;
        let gdt_p = TASKS[cur].gdt_phys;
        let tss_p = TASKS[cur].tss_phys;
        let pml4_p = TASKS[cur].pml4_phys;
        let fpu_p = TASKS[cur].fpu_buf_phys;
        let kstack_p = TASKS[cur].kstack_phys;

        TASKS[cur].state = TaskState::Exited;
        TASKS[cur].exit_status = status;

        // 2) Wake up parent if it's waiting (BlockedRecv on this child).
        if parent_id != 0 {
            for i in 0..MAX_TASKS {
                if TASKS[i].id == parent_id && TASKS[i].state == TaskState::BlockedRecv {
                    TASKS[i].state = TaskState::Ready;
                    TASKS[i].ipc_val = TASKS[cur].id; // pass child id as notification
                }
            }
        }

        // 3) Choose the next task BEFORE freeing the dying one, so that when we
        //    switch address space + GDT to `next`, the current GDT is still mapped.
        let next = find_ready();
        if next == cur {
            loop { asm!("hlt"); }
        }

        // 4) Switch to the *next* task while the dying task's GDT/TSS/PM4 are
        //    still mapped (nothing freed yet). This is crucial: the CPU GDTR may
        //    still point at the dying task's per-task GDT (if it was ring-3), and
        //    we must only free it after we've moved to a valid GDT.
        TASKS[next].state = TaskState::Running;
        CURRENT = next;
        if TASKS[next].ring3 {
            crate::vm::switch_to(TASKS[next].pml4 as *mut u64);
            let ktop = (TASKS[next].stack_base as u64) + STACK_SIZE as u64;
            super::context::load_gdt_tss(TASKS[next].gdt_phys, TASKS[next].tss_phys, ktop);
            crate::syscall::set_sys_krsp(ktop);
            crate::syscall::set_sys_ursave(TASKS[next].sys_ursave);
        } else {
            // Kernel task: restore the default kernel GDT. The CPU may be running
            // on the just-exited process's per-task GDT; we must not keep using a
            // GDT we're about to free.
            super::context::load_kernel_gdt();
            crate::vm::switch_to(crate::vm::KERNEL_PML4 as *mut u64);
        }

        // 5) NOW free the dying process's resources — no longer in use by the CPU.
        if code_p != 0 { crate::memory::pfree(code_p); }
        if user_stack_p != 0 { crate::memory::pfree(user_stack_p); }
        if gdt_p != 0 { crate::memory::pfree(gdt_p); }
        if tss_p != 0 { crate::memory::pfree(tss_p); }
        if pml4_p != 0 { crate::vm::destroy_address_space(pml4_p as *mut u64); }
        if fpu_p != 0 { crate::memory::pfree(fpu_p); }
        if kstack_p != 0 {
            crate::memory::pfree_n(kstack_p, STACK_SIZE / crate::memory::PAGE_SIZE);
        }

        // 6) Jump to the next task.
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

pub fn exit() {
    exit_with_status(0);
}

pub fn init() {
    super::task::fpu_init();
    unsafe {
        CURRENT = 0;
        TASKS[0] = Task {
            state: TaskState::Running,
            stack_base: 0 as *mut u8,
            sp: 0, id: 0,
            parent_id: 0, exit_status: 0,
            ipc_partner: 0, ipc_val: 0, ipc_msg: Message::empty(),
            pml4: core::ptr::null_mut(), pml4_phys: 0,
            gdt_phys: 0, tss_phys: 0, ring3: false, sys_ursave: 0,
            code_phys: 0, user_stack_phys: 0, kstack_phys: 0, fpu_buf_phys: 0,
            pending_msg: Message::empty(),
            fds: [const { super::FdEntry::empty() }; super::MAX_FDS],
        };
    }
}
