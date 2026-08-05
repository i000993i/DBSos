// Модуль планировщика: кооперативная + вытесняющая многозадачность (LAPIC-таймер)

pub mod context;
pub mod lapic;
pub mod task;
pub mod spawn;
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
        }
    }
}

pub static mut TASKS: [Task; MAX_TASKS] = [const { Task::free() }; MAX_TASKS];
pub static mut CURRENT: usize = 0;
pub static mut NEXT_ID: u64 = 1;

pub unsafe fn find_task(id: u64) -> Option<usize> {
    (0..MAX_TASKS).find(|&i| TASKS[i].state != TaskState::Free && TASKS[i].id == id)
}

pub fn uart_hex(val: u64) {
    let hex = b"0123456789ABCDEF";
    crate::driver::uart::putchar(b'0');
    crate::driver::uart::putchar(b'x');
    for i in (0..16).rev() {
        crate::driver::uart::putchar(hex[((val >> (i * 4)) & 0xF) as usize]);
    }
}
