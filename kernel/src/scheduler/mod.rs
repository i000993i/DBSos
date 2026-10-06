// Модуль планировщика: кооперативная + вытесняющая многозадачность (LAPIC-таймер)

pub mod context;
pub mod lapic;
pub mod smp;
pub mod task;
pub mod spawn;
pub mod vma;
pub mod ipc_sched;
pub mod process;
pub mod tests;

pub use context::yield_now;
pub use lapic::lapic_timer_init;
pub use spawn::{spawn, spawn_user};
pub use ipc_sched::{ipc_send, ipc_recv, ipc_send_u64, ipc_recv_u64};
pub use process::{exit, init};
pub use tests::{test, preempt_test};

use dbsos_abi::ipc::Message;
use dbsos_abi::syscall::MAX_FDS;

pub const STACK_SIZE: usize = 65536;
pub const MAX_TASKS: usize = 32;
pub const IDLE_SLOT: usize = MAX_TASKS - 1;

/// Stack canary: written at bottom of kernel stack, checked on context switch.
/// If this value is overwritten, a stack overflow occurred.
pub const STACK_CANARY: u64 = 0xDEAD_BEEF_CAFE_BABE;

pub const GDT_SEL_KERNEL_CODE: u64 = 0x08;
pub const GDT_SEL_KERNEL_DATA: u64 = 0x10;
pub const GDT_SEL_USER_CODE: u64 = 0x23;
pub const GDT_SEL_USER_DATA: u64 = 0x2B;

pub const IPC_ERR: u64 = !1u64;
pub const IPC_ANY: u64 = !0u64;

#[derive(Clone, Copy, PartialEq)]
pub enum TaskState { Free, Ready, Running, Exited, BlockedSend, BlockedRecv }

/// File descriptor entry
#[derive(Clone, Copy)]
pub struct FdEntry {
    pub in_use: bool,
    pub path: [u8; 128],   // file path
    pub offset: u32,       // current read/write offset
    pub size: u32,         // file size (cached at open)
    pub flags: u64,        // O_RDONLY, O_WRONLY, etc.
}

impl FdEntry {
    pub const fn empty() -> Self {
        FdEntry {
            in_use: false,
            path: [0; 128],
            offset: 0,
            size: 0,
            flags: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Task {
    pub state: TaskState,
    pub stack_base: *mut u8,
    pub sp: u64,
    pub id: u64,
    pub parent_id: u64,
    pub exit_status: i32,
    pub ipc_partner: u64,
    pub ipc_val: u64,
    pub ipc_msg: Message,
    pub pml4: *mut u64,
    pub pml4_phys: u64,
    pub gdt_phys: u64,
    pub tss_phys: u64,
    pub ring3: bool,
    pub sys_ursave: u64,
    pub code_phys: u64,
    pub user_stack_phys: u64,
    pub kstack_phys: u64,
    pub fpu_buf_phys: u64,
    pub pending_msg: Message,
    pub fds: [FdEntry; MAX_FDS],
    pub vmas: [vma::Vma; vma::MAX_VMAS],
    pub vma_count: u8,
    pub uid: u32,
    pub gid: u32,
    pub cwd: [u8; 128],
    pub cwd_len: u8,
}

impl Task {
    pub const fn free() -> Self {
        Task {
            state: TaskState::Free,
            stack_base: 0 as *mut u8, sp: 0, id: 0,
            parent_id: 0, exit_status: 0,
            ipc_partner: 0, ipc_val: 0,
            ipc_msg: Message::empty(),
            pml4: 0 as *mut u64, pml4_phys: 0, gdt_phys: 0, tss_phys: 0, ring3: false, sys_ursave: 0,
            code_phys: 0, user_stack_phys: 0, kstack_phys: 0, fpu_buf_phys: 0,
            pending_msg: Message::empty(),
            fds: [const { FdEntry::empty() }; MAX_FDS],
            vmas: [const { vma::Vma::empty() }; vma::MAX_VMAS],
            vma_count: 0,
            uid: 0, gid: 0,
            cwd: [0; 128],
            cwd_len: 1,
        }
    }
    pub fn cwd_slice(&self) -> &[u8] {
        &self.cwd[..self.cwd_len as usize]
    }
}

pub static mut TASKS: [Task; MAX_TASKS] = [const { Task::free() }; MAX_TASKS];
pub static mut CURRENT: usize = 0;
pub static mut NEXT_ID: u64 = 1;

pub fn current_task_id() -> u64 {
    unsafe { TASKS[CURRENT].id }
}

pub fn next_task_id() -> u64 {
    unsafe {
        let id = NEXT_ID;
        NEXT_ID += 1;
        id
    }
}

pub unsafe fn find_task(id: u64) -> Option<usize> {
    (0..MAX_TASKS).find(|&i| TASKS[i].state != TaskState::Free && TASKS[i].id == id)
}

pub fn current_cwd() -> [u8; 128] {
    unsafe {
        let cur = CURRENT;
        if TASKS[cur].cwd_len == 0 { let mut a=[0u8;128]; a[0]=b'/'; return a; }
        TASKS[cur].cwd
    }
}
pub fn current_cwd_len() -> usize { unsafe { TASKS[CURRENT].cwd_len as usize } }
pub fn current_cwd_slice() -> &'static [u8] {
    unsafe {
        let cur = CURRENT;
        let len = TASKS[cur].cwd_len as usize;
        if len==0 || TASKS[cur].cwd[0]==0 { return b"/"; }
        &TASKS[cur].cwd[..len]
    }
}
pub fn set_current_cwd(path: &[u8]) {
    unsafe {
        let cur = CURRENT;
        let len = path.len().min(127);
        TASKS[cur].cwd[..len].copy_from_slice(&path[..len]);
        TASKS[cur].cwd[len]=0;
        TASKS[cur].cwd_len=len as u8;
        if len==0 { TASKS[cur].cwd[0]=b'/'; TASKS[cur].cwd[1]=0; TASKS[cur].cwd_len=1; }
    }
}
pub fn task_count_for_uid(uid: u32) -> usize {
    unsafe { TASKS.iter().filter(|t| t.state!=TaskState::Free && t.uid==uid).count() }
}
pub const MAX_TASKS_PER_USER: usize = 8;

pub fn uart_hex(val: u64) {
    let hex = b"0123456789ABCDEF";
    crate::driver::uart::putchar(b'0');
    crate::driver::uart::putchar(b'x');
    for i in (0..16).rev() {
        crate::driver::uart::putchar(hex[((val >> (i * 4)) & 0xF) as usize]);
    }
}

/// Iterate over all non-free tasks
pub fn for_each_task<F: FnMut(usize, &Task)>(mut f: F) {
    unsafe {
        for i in 0..MAX_TASKS {
            if TASKS[i].state != TaskState::Free {
                f(i, &TASKS[i]);
            }
        }
    }
}

/// Fork a scheduler task: creates a new task with cloned kernel stack and context.
/// Returns the new task ID, or 0 on failure.
pub fn fork(parent_id: u64) -> u64 {
    unsafe {
        // Find parent task
        let parent_idx = match find_task(parent_id) {
            Some(i) => i,
            None => return 0,
        };
        let uid = TASKS[parent_idx].uid;
        if uid != 0 && task_count_for_uid(uid) >= MAX_TASKS_PER_USER {
            return 0;
        }

        // Find free slot
        let slot = match (0..MAX_TASKS).find(|&i| TASKS[i].state == TaskState::Free) {
            Some(s) => s,
            None => return 0,
        };

        // Allocate new kernel stack
        let new_kstack = crate::memory::palloc_n(STACK_SIZE / crate::memory::PAGE_SIZE);
        if new_kstack == 0 { return 0; }
        crate::vm::identity_map_2mb(crate::vm::KERNEL_PML4 as *mut u64,
            new_kstack, new_kstack + STACK_SIZE as u64, crate::vm::PTE_WRITABLE);

        // Copy parent's kernel stack
        let parent_sp = TASKS[parent_idx].sp;
        let parent_stack_base = TASKS[parent_idx].stack_base as u64;
        let sp_offset = parent_sp - parent_stack_base;
        core::ptr::copy_nonoverlapping(
            parent_stack_base as *const u8,
            new_kstack as *mut u8,
            STACK_SIZE,
        );
        let new_sp = new_kstack + sp_offset;

        // Write canary
        *(new_kstack as *mut u64) = STACK_CANARY;

        // Allocate new PML4
        let new_pml4_phys = crate::memory::palloc();
        if new_pml4_phys == 0 {
            crate::memory::pfree_n(new_kstack, STACK_SIZE / crate::memory::PAGE_SIZE);
            return 0;
        }
        crate::vm::identity_map_2mb(crate::vm::KERNEL_PML4 as *mut u64,
            new_pml4_phys, new_pml4_phys + 4096, crate::vm::PTE_WRITABLE);
        let new_pml4 = new_pml4_phys as *mut u64;
        core::ptr::write_bytes(new_pml4 as *mut u8, 0, 4096);

        // Clone user-space mappings (share page table pages)
        let parent_pml4 = TASKS[parent_idx].pml4;
        if !parent_pml4.is_null() {
            for i in 0..256 {
                let e = *((parent_pml4 as *const u64).add(i));
                if e & crate::vm::PTE_PRESENT != 0 {
                    *((new_pml4).add(i)) = e;
                }
            }
        }
        // Copy kernel mappings
        for i in 256..512 {
            let e = *((crate::vm::KERNEL_PML4 as *const u64).add(i));
            if e & crate::vm::PTE_PRESENT != 0 {
                *((new_pml4).add(i)) = e;
            }
        }

        // Copy file descriptors
        let mut new_fds = [FdEntry::empty(); MAX_FDS];
        for i in 0..MAX_FDS {
            new_fds[i] = TASKS[parent_idx].fds[i];
        }

        let new_id = NEXT_ID;
        NEXT_ID += 1;

        // Allocate FPU buffer
        let new_fpu = crate::scheduler::task::fpu_alloc_buf();
        if TASKS[parent_idx].fpu_buf_phys != 0 && new_fpu != 0 {
            core::ptr::copy_nonoverlapping(
                TASKS[parent_idx].fpu_buf_phys as *const u8,
                new_fpu as *mut u8,
                512,
            );
        }

        // Create child task — inherit uid/gid/cwd from parent
        TASKS[slot] = Task {
            state: TaskState::Ready,
            stack_base: new_kstack as *mut u8,
            sp: new_sp,
            id: new_id,
            parent_id: parent_id,
            exit_status: 0,
            ipc_partner: 0, ipc_val: 0,
            ipc_msg: Message::empty(),
            pml4: new_pml4,
            pml4_phys: new_pml4_phys,
            gdt_phys: 0, tss_phys: 0,
            ring3: TASKS[parent_idx].ring3,
            sys_ursave: TASKS[parent_idx].sys_ursave,
            code_phys: TASKS[parent_idx].code_phys,
            user_stack_phys: TASKS[parent_idx].user_stack_phys,
            kstack_phys: 0,
            fpu_buf_phys: new_fpu,
            pending_msg: Message::empty(),
            fds: new_fds,
            vmas: [const { vma::Vma::empty() }; vma::MAX_VMAS],
            vma_count: 0,
            uid: TASKS[parent_idx].uid,
            gid: TASKS[parent_idx].gid,
            cwd: TASKS[parent_idx].cwd,
            cwd_len: TASKS[parent_idx].cwd_len,
        };

        new_id
    }
}
