// Spawn: spawn, spawn_user

use super::{TaskState, Task, STACK_SIZE, MAX_TASKS, TASKS, NEXT_ID, GDT_SEL_USER_CODE, GDT_SEL_USER_DATA, STACK_CANARY};
use super::task;
use dbsos_abi::ipc::Message;

const KSTACK_PAGES: usize = STACK_SIZE / crate::memory::PAGE_SIZE;

pub fn spawn(entry: extern "C" fn()) -> Option<u64> {
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS);
        let slot = (0..MAX_TASKS).find(|&i| (*tasks)[i].state == TaskState::Free)?;
        let stack = crate::memory::palloc_n(KSTACK_PAGES);
        if stack == 0 { return None; }
        // Write stack canary at the bottom (lowest address) of the stack
        *(stack as *mut u64) = STACK_CANARY;
        let ksp = stack + STACK_SIZE as u64;
        let mut sp = (ksp as u64 - 8) as *mut u64;
        sp = sp.sub(1); *sp = 0x10;
        sp = sp.sub(1); *sp = ksp;
        sp = sp.sub(1); *sp = 0x200;
        sp = sp.sub(1); *sp = 0x08;
        sp = sp.sub(1); *sp = entry as u64;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        sp = sp.sub(1); *sp = 0;
        let id = NEXT_ID;
        NEXT_ID += 1;
        let prev = &TASKS[slot];
        if prev.kstack_phys != 0 {
            crate::memory::pfree_n(prev.kstack_phys, KSTACK_PAGES);
        }
        TASKS[slot] = Task {
            state: TaskState::Ready,
            stack_base: stack as *mut u8,
            sp: sp as u64,
            id,
            parent_id: 0, exit_status: 0,
            ipc_partner: 0, ipc_val: 0, ipc_msg: Message::empty(),
            pml4: core::ptr::null_mut(), pml4_phys: 0,
            gdt_phys: 0, tss_phys: 0, ring3: false, sys_ursave: 0,
            code_phys: 0, user_stack_phys: 0, kstack_phys: stack, fpu_buf_phys: task::fpu_alloc_buf(),
            pending_msg: Message::empty(),
            fds: [const { super::FdEntry::empty() }; super::MAX_FDS],
            vmas: [const { super::vma::Vma::empty() }; super::vma::MAX_VMAS],
            vma_count: 0,
        };
        Some(id)
    }
}

pub unsafe fn spawn_user(entry: u64, user_rsp: u64,
    pml4: *mut u64, gdt_phys: u64, tss_phys: u64,
    code_phys: u64, user_stack_phys: u64) -> Option<u64>
{
    let tasks = core::ptr::addr_of!(TASKS);
    let slot = (0..MAX_TASKS).find(|&i| (*tasks)[i].state == TaskState::Free)?;
    let prev = &TASKS[slot];
    if prev.kstack_phys != 0 {
        crate::memory::pfree_n(prev.kstack_phys, KSTACK_PAGES);
    }
    let kstack = crate::memory::palloc_n(KSTACK_PAGES);
    if kstack == 0 { return None; }
    // Write stack canary at the bottom of the kernel stack
    *(kstack as *mut u64) = STACK_CANARY;
    let ksp = (kstack + STACK_SIZE as u64) as *mut u64;
    let mut sp = ksp as *mut u64;
    sp = sp.sub(1); *sp = GDT_SEL_USER_DATA;
    sp = sp.sub(1); *sp = user_rsp;
    sp = sp.sub(1); *sp = 0x202;
    sp = sp.sub(1); *sp = GDT_SEL_USER_CODE;
    sp = sp.sub(1); *sp = entry;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    sp = sp.sub(1); *sp = 0;
    let id = NEXT_ID;
    NEXT_ID += 1;
    let fpu_buf = task::fpu_alloc_buf();
    TASKS[slot] = Task {
        state: TaskState::Ready,
        stack_base: kstack as *mut u8,
        sp: sp as u64,
        id,
        parent_id: 0, exit_status: 0,
        ipc_partner: 0, ipc_val: 0, ipc_msg: Message::empty(),
        pml4, pml4_phys: pml4 as u64, gdt_phys, tss_phys, ring3: true, sys_ursave: user_rsp,
        code_phys, user_stack_phys, kstack_phys: kstack, fpu_buf_phys: fpu_buf,
        pending_msg: Message::empty(),
        fds: [const { super::FdEntry::empty() }; super::MAX_FDS],
        vmas: [const { super::vma::Vma::empty() }; super::vma::MAX_VMAS],
        vma_count: 0,
    };
    Some(id)
}
