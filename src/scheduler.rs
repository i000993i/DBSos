// Кооперативная + вытесняющая многозадачность (LAPIC-таймер)

use core::arch::asm;
use crate::io;
use dbsos_abi::ipc::Message;

const STACK_SIZE: usize = 4096;
pub const MAX_TASKS: usize = 32;
const LAPIC_BASE: u64 = 0xFEE0_0000;
const GDT_SEL_USER_CODE: u64 = 0x23; // CS for ring 3
const GDT_SEL_USER_DATA: u64 = 0x2B; // SS for ring 3

fn uart_hex(val: u64) {
    let hex = b"0123456789ABCDEF";
    crate::driver::uart::putchar(b'0');
    crate::driver::uart::putchar(b'x');
    for i in (0..16).rev() {
        crate::driver::uart::putchar(hex[((val >> (i * 4)) & 0xF) as usize]);
    }
}

// ---- LAPIC timer -----------------------------------------------------------

fn lapic_write(offset: u32, value: u32) {
    unsafe { io::mmio_write32((LAPIC_BASE + offset as u64) as *mut u32, value) }
}

fn lapic_read(offset: u32) -> u32 {
    unsafe { io::mmio_read32((LAPIC_BASE + offset as u64) as *mut u32) }
}

fn lapic_base() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        asm!("rdmsr", out("eax") lo, out("edx") hi, in("ecx") 0x1Bu32);
        ((hi as u64) << 32) | (lo as u64) & 0xFFFFF000
    }
}

pub fn install_timer_isr() {
    unsafe {
        let entry = &mut crate::interrupts::IDT[32];
        entry.set_handler(timer_stub as *const () as u64, 0x08);
    }
}

// ---- Timer interrupt stub (IRQ0, IDT[32], vector 0x20) --------------------

core::arch::global_asm!(
    ".globl timer_stub",
    "timer_stub:",
    "  push rax",
    "  push rcx",
    "  push rdx",
    "  push rbx",
    "  push rbp",
    "  push rsi",
    "  push rdi",
    "  push r8",
    "  push r9",
    "  push r10",
    "  push r11",
    "  push r12",
    "  push r13",
    "  push r14",
    "  push r15",
    // LAPIC EOI
    "  mov rcx, 0xFEE000B0",
    "  xor eax, eax",
    "  mov [rcx], eax",
    "  mov rcx, rsp",
    "  sub rsp, 32",
    "  call reschedule",
    "  add rsp, 32",
    "  mov rsp, rax",
    "  pop r15",
    "  pop r14",
    "  pop r13",
    "  pop r12",
    "  pop r11",
    "  pop r10",
    "  pop r9",
    "  pop r8",
    "  pop rdi",
    "  pop rsi",
    "  pop rbp",
    "  pop rbx",
    "  pop rdx",
    "  pop rcx",
    "  pop rax",
    "  iretq",
);

extern "C" { fn timer_stub(); }

// ---- FPU / SSE ---------------------------------------------------------------

/// Enable x87 + SSE in CR0/CR4 and initialise FPU to a known state.
/// Must be called once before any FPU save/restore.
pub fn fpu_init() {
    unsafe {
        let mut cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0);
        cr0 &= !(1u64 << 2);  // CR0.EM = 0  (FPU present)
        cr0 |= 1u64 << 1;  // CR0.MP = 1  (monitor co-processor)
        cr0 |= 1u64 << 5;  // CR0.NE = 1  (native error)
        asm!("mov cr0, {}", in(reg) cr0);

        let mut cr4: u64;
        asm!("mov {}, cr4", out(reg) cr4);
        cr4 |= 1u64 << 9;  // CR4.OSFXSR = 1  (FXSAVE/FXRSTOR enabled)
        cr4 |= 1u64 << 10; // CR4.OSXMMEXCPT = 1  (#XM exceptions)
        asm!("mov cr4, {}", in(reg) cr4);

        asm!("fninit");
    }
}

/// Allocate and initialise a FPU save buffer for a new task.
/// Returns physical address, or 0 on failure.
pub unsafe fn fpu_alloc_buf() -> u64 {
    let buf = crate::memory::palloc();
    if buf == 0 { return 0; }
    // Save current clean FPU state into the buffer
    asm!("fxsave64 [{}]", in(reg) buf, options(nostack));
    buf
}

pub static mut TICK_COUNT: u64 = 0;

unsafe fn load_gdt_tss(gdt_phys: u64, tss_phys: u64, stack_top: u64) {
    let pd = crate::interrupts::GdtPacked { limit: (8*8 - 1) as u16, base: gdt_phys };
    asm!("lgdt [{p}]", p = in(reg) &pd as *const _ as u64);
    // Reset TSS descriptor type to AVAILABLE (0x09) in case ltr was already done once
    core::ptr::write_unaligned((gdt_phys + 53) as *mut u8, 0x89u8);
    // Update TSS.RSP0 to this task's kernel stack top
    core::ptr::write_unaligned((tss_phys + 4) as *mut u64, stack_top);
    asm!("mov ax, 0x30", "ltr ax", out("ax") _, options(nostack));
}

#[no_mangle]
unsafe extern "C" fn reschedule(rsp: u64) -> u64 {
    TICK_COUNT += 1;
    let cur = CURRENT;
    let next = find_ready();
    if next != cur {
        if TASKS[cur].state == TaskState::Running || TASKS[cur].state == TaskState::Ready {
            TASKS[cur].state = TaskState::Ready;
        }
        // Save current task's FPU state
        let cur_fpu = TASKS[cur].fpu_buf_phys;
        if cur_fpu != 0 {
            asm!("fxsave64 [{}]", in(reg) cur_fpu, options(nostack));
        }
        TASKS[cur].sp = rsp;
        let n = &TASKS[next];
        TASKS[next].state = TaskState::Running;
        CURRENT = next;
        if n.ring3 {
            crate::vm::switch_to(n.pml4 as *mut u64);
            let ktop = (n.stack_base as u64) + STACK_SIZE as u64;
            load_gdt_tss(n.gdt_phys, n.tss_phys, ktop);
            crate::syscall::set_sys_krsp(ktop);
            crate::syscall::set_sys_ursave(n.sys_ursave);
        }
        // Restore next task's FPU state
        let next_fpu = TASKS[next].fpu_buf_phys;
        if next_fpu != 0 {
            asm!("fxrstor64 [{}]", in(reg) next_fpu, options(nostack));
        }
        TASKS[next].sp
    } else {
        rsp
    }
}

pub fn lapic_timer_init() {
    unsafe {
        let base = lapic_base();
        crate::driver::uart::write_str("[LAPIC] base=");
        uart_hex(base);
        crate::driver::uart::write_str("\r\n");

        let sivr = lapic_read(0xF0);
        crate::driver::uart::write_str("[LAPIC] SIVR=");
        uart_hex(sivr as u64);
        crate::driver::uart::write_str("\r\n");

        if sivr & 0x100 == 0 {
            lapic_write(0xF0, sivr | 0x100 | 0x32);
        }

        // DCR = divide by 16, initial count ~100 Hz
        lapic_write(0x3E0, 0x3);
        lapic_write(0x380, 62500);
        // LVT Timer: vector=0x20, periodic
        lapic_write(0x320, 0x20u32 | (1u32 << 17));

        crate::driver::uart::write_str("[LAPIC] timer configured\r\n");

        install_timer_isr();
        asm!("sti");
    }
}

// ---- Task management -------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum TaskState { Free, Ready, Running, Exited, BlockedSend, BlockedRecv }

#[derive(Clone, Copy)]
pub struct Task {
    pub state: TaskState,
    /// Kernel stack virtual (= physical, identity mapped)
    pub stack_base: *mut u8,
    pub sp: u64,
    pub id: u64,
    pub ipc_partner: u64,
    pub ipc_val: u64,
    pub pml4: *mut u64,
    /// Physical address of the PML4 page (for freeing on exit)
    pub pml4_phys: u64,
    pub gdt_phys: u64,
    pub tss_phys: u64,
    pub ring3: bool,
    pub sys_ursave: u64,
        pub code_phys: u64,
    pub user_stack_phys: u64,
    pub kstack_phys: u64,
    /// Physical address of 512-byte FPU save buffer (0 = none)
    pub fpu_buf_phys: u64,
    /// Pending IPC message (delivered via fast path, stored here for stability)
    pub pending_msg: Message,
}

impl Task {
    pub const fn free() -> Self {
        Task {
            state: TaskState::Free,
            stack_base: 0 as *mut u8, sp: 0, id: 0,
            ipc_partner: 0, ipc_val: 0,
            pml4: 0 as *mut u64, pml4_phys: 0, gdt_phys: 0, tss_phys: 0, ring3: false, sys_ursave: 0,
            code_phys: 0, user_stack_phys: 0, kstack_phys: 0, fpu_buf_phys: 0,
            pending_msg: Message::empty(),
        }
    }
}

pub static mut TASKS: [Task; MAX_TASKS] = [const { Task::free() }; MAX_TASKS];
pub static mut CURRENT: usize = 0;
pub static mut NEXT_ID: u64 = 1;

fn find_ready() -> usize {
    let cur = unsafe { CURRENT };
    for i in 1..=MAX_TASKS {
        let idx = (cur + i) % MAX_TASKS;
        if unsafe { TASKS[idx].state == TaskState::Ready } {
            return idx;
        }
    }
    cur
}

const IPC_ERR: u64 = !1u64; // -2 (distinct from EXIT_MAGIC = -1)

const IPC_ANY: u64 = !0u64;

/// Find task slot by ID (any state except Free).
unsafe fn find_task(id: u64) -> Option<usize> {
    (0..MAX_TASKS).find(|&i| TASKS[i].state != TaskState::Free && TASKS[i].id == id)
}

/// Try one-shot match of a send against a waiting receiver.
unsafe fn try_match_send(cur: usize, dst_id: u64, msg_ptr: u64) -> bool {
    if let Some(dst_slot) = find_task(dst_id) {
        if TASKS[dst_slot].state == TaskState::BlockedRecv
            && (TASKS[dst_slot].ipc_partner == IPC_ANY
                || TASKS[dst_slot].ipc_partner == TASKS[cur].id)
        {
            TASKS[dst_slot].ipc_val = msg_ptr;
            TASKS[dst_slot].state = TaskState::Ready;
            return true;
        }
    }
    false
}

/// Try one-shot match of a recv against a waiting sender.
/// Returns msg_ptr if found.
unsafe fn try_match_recv(cur: usize, src_id: u64) -> Option<u64> {
    let maybe_slot = if src_id == 0 {
        (0..MAX_TASKS).find(|&i| {
            TASKS[i].state == TaskState::BlockedSend
                && (TASKS[i].ipc_partner == IPC_ANY || TASKS[i].ipc_partner == TASKS[cur].id)
        })
    } else {
        find_task(src_id).filter(|&s| {
            TASKS[s].state == TaskState::BlockedSend
                && (TASKS[s].ipc_partner == IPC_ANY || TASKS[s].ipc_partner == TASKS[cur].id)
        })
    };
    if let Some(src_slot) = maybe_slot {
        let msg_ptr = TASKS[src_slot].ipc_val;
        TASKS[src_slot].state = TaskState::Ready;
        Some(msg_ptr)
    } else {
        None
    }
}

/// IPC: send message pointer to `dst_id`. Blocks until receiver is ready.
/// Returns 0 on success, IPC_ERR if dst_id not found.
pub unsafe fn ipc_send(dst_id: u64, msg_ptr: u64) -> u64 {
    let cur = CURRENT;
    if find_task(dst_id).is_none() { return IPC_ERR; }
    if try_match_send(cur, dst_id, msg_ptr) { return 0; }
    TASKS[cur].state = TaskState::BlockedSend;
    TASKS[cur].ipc_partner = dst_id;
    TASKS[cur].ipc_val = msg_ptr;
    if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
    yield_now();
    0
}

/// IPC: send u64 value (legacy compat для ring-3 тестов)
pub unsafe fn ipc_send_u64(dst_id: u64, val: u64) -> u64 {
    let cur = CURRENT;
    if find_task(dst_id).is_none() { return IPC_ERR; }
    if try_match_send_u64(cur, dst_id, val) { return 0; }
    TASKS[cur].state = TaskState::BlockedSend;
    TASKS[cur].ipc_partner = dst_id;
    TASKS[cur].ipc_val = val;
    if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
    yield_now();
    0
}

unsafe fn try_match_send_u64(cur: usize, dst_id: u64, val: u64) -> bool {
    if let Some(dst_slot) = find_task(dst_id) {
        if TASKS[dst_slot].state == TaskState::BlockedRecv
            && (TASKS[dst_slot].ipc_partner == IPC_ANY
                || TASKS[dst_slot].ipc_partner == TASKS[cur].id)
        {
            TASKS[dst_slot].ipc_val = val;
            TASKS[dst_slot].state = TaskState::Ready;
            return true;
        }
    }
    false
}

/// IPC: recv from `src_id` (0 = any). Blocks until sender ready.
/// Returns msg_ptr on success, IPC_ERR if no partner found.
pub unsafe fn ipc_recv(src_id: u64) -> u64 {
    let cur = CURRENT;
    if let Some(msg_ptr) = try_match_recv(cur, src_id) { return msg_ptr; }
    TASKS[cur].state = TaskState::BlockedRecv;
    TASKS[cur].ipc_partner = if src_id == 0 { IPC_ANY } else { src_id };
    if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
    yield_now();
    TASKS[cur].ipc_val
}

/// IPC: recv u64 value (legacy compat)
pub unsafe fn ipc_recv_u64() -> u64 {
    let cur = CURRENT;
    if let Some(msg_ptr) = try_match_recv_u64(cur) { return msg_ptr; }
    TASKS[cur].state = TaskState::BlockedRecv;
    TASKS[cur].ipc_partner = IPC_ANY;
    if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
    yield_now();
    TASKS[cur].ipc_val
}

unsafe fn try_match_recv_u64(cur: usize) -> Option<u64> {
    (0..MAX_TASKS).find(|&i| {
        TASKS[i].state == TaskState::BlockedSend
            && (TASKS[i].ipc_partner == IPC_ANY || TASKS[i].ipc_partner == TASKS[cur].id)
    }).map(|slot| {
        let val = TASKS[slot].ipc_val;
        TASKS[slot].state = TaskState::Ready;
        val
    })
}

pub fn spawn(entry: extern "C" fn()) -> Option<u64> {
    unsafe {
        let tasks = core::ptr::addr_of!(TASKS);
        let slot = (0..MAX_TASKS).find(|&i| (*tasks)[i].state == TaskState::Free)?;
        let stack = crate::memory::palloc();
        if stack == 0 { return None; }
        let ksp = stack + STACK_SIZE as u64;
        let mut sp = (ksp as u64 - 8) as *mut u64;
        // QEMU's 64-bit iretq always pops 5 items (SS, RSP, RFLAGS, CS, RIP)
        // even for same-privilege (CPL=0) return.
        sp = sp.sub(1); *sp = 0x10;               // SS (kernel data)
        sp = sp.sub(1); *sp = ksp;                // RSP (initial stack top)
        sp = sp.sub(1); *sp = 0x200;              // RFLAGS (IF=0, bit1=1)
        sp = sp.sub(1); *sp = 0x08;               // CS
        sp = sp.sub(1); *sp = entry as u64;       // RIP
        // 15 GPRs — порядок как у timer_stub (RAX первый → R15 последний)
        sp = sp.sub(1); *sp = 0; // RAX
        sp = sp.sub(1); *sp = 0; // RCX
        sp = sp.sub(1); *sp = 0; // RDX
        sp = sp.sub(1); *sp = 0; // RBX
        sp = sp.sub(1); *sp = 0; // RBP
        sp = sp.sub(1); *sp = 0; // RSI
        sp = sp.sub(1); *sp = 0; // RDI
        sp = sp.sub(1); *sp = 0; // R8
        sp = sp.sub(1); *sp = 0; // R9
        sp = sp.sub(1); *sp = 0; // R10
        sp = sp.sub(1); *sp = 0; // R11
        sp = sp.sub(1); *sp = 0; // R12
        sp = sp.sub(1); *sp = 0; // R13
        sp = sp.sub(1); *sp = 0; // R14
        sp = sp.sub(1); *sp = 0; // R15
        let id = NEXT_ID;
        NEXT_ID += 1;
        // Drain deferred kstack free before reuse
        let prev = &TASKS[slot];
        if prev.kstack_phys != 0 {
            crate::memory::pfree(prev.kstack_phys);
        }
        TASKS[slot] = Task {
            state: TaskState::Ready,
            stack_base: stack as *mut u8,
            sp: sp as u64,
            id,
            ipc_partner: 0, ipc_val: 0,
            pml4: core::ptr::null_mut(), pml4_phys: 0,
            gdt_phys: 0, tss_phys: 0, ring3: false, sys_ursave: 0,
            code_phys: 0, user_stack_phys: 0, kstack_phys: stack, fpu_buf_phys: fpu_alloc_buf(),
            pending_msg: Message::empty(),
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
    // Drain deferred kstack free before reuse
    let prev = &TASKS[slot];
    if prev.kstack_phys != 0 {
        crate::memory::pfree(prev.kstack_phys);
    }
    let kstack = crate::memory::palloc();
    if kstack == 0 { return None; }
    let ksp = (kstack + STACK_SIZE as u64) as *mut u64;
    let mut sp = ksp as *mut u64;
    // IRETQ frame (5 items — ring change 0→3), from HIGH to LOW:
    //   SS, user RSP, RFLAGS, CS, RIP
    sp = sp.sub(1); *sp = GDT_SEL_USER_DATA; // SS
    sp = sp.sub(1); *sp = user_rsp;           // user RSP
    sp = sp.sub(1); *sp = 0x202;              // RFLAGS (IF=1, bit1=1)
    sp = sp.sub(1); *sp = GDT_SEL_USER_CODE;  // CS
    sp = sp.sub(1); *sp = entry;              // RIP
    // 15 GPRs — порядок КАК В timer_stub: RAX→RCX→...→R15 (R15 в LOW)
    sp = sp.sub(1); *sp = 0; // RAX
    sp = sp.sub(1); *sp = 0; // RCX
    sp = sp.sub(1); *sp = 0; // RDX
    sp = sp.sub(1); *sp = 0; // RBX
    sp = sp.sub(1); *sp = 0; // RBP
    sp = sp.sub(1); *sp = 0; // RSI
    sp = sp.sub(1); *sp = 0; // RDI
    sp = sp.sub(1); *sp = 0; // R8
    sp = sp.sub(1); *sp = 0; // R9
    sp = sp.sub(1); *sp = 0; // R10
    sp = sp.sub(1); *sp = 0; // R11
    sp = sp.sub(1); *sp = 0; // R12
    sp = sp.sub(1); *sp = 0; // R13
    sp = sp.sub(1); *sp = 0; // R14
    sp = sp.sub(1); *sp = 0; // R15  (→ final sp)
    let id = NEXT_ID;
    NEXT_ID += 1;
    TASKS[slot] = Task {
        state: TaskState::Ready,
        stack_base: kstack as *mut u8,
        sp: sp as u64,
        id,
        ipc_partner: 0, ipc_val: 0,
        pml4, pml4_phys: pml4 as u64, gdt_phys, tss_phys, ring3: true, sys_ursave: user_rsp,
            code_phys, user_stack_phys, kstack_phys: kstack, fpu_buf_phys: fpu_alloc_buf(),
            pending_msg: Message::empty(),
    };
    Some(id)
}

// ---- yield_now (кооперативный, с IRETQ-совместимым фреймом) -----------------

core::arch::global_asm!(
    ".globl yield_now_asm",
    "yield_now_asm:",
    "  cli",
    // Frame layout (from HIGH to LOW) – 5 iretq items for QEMU:
    //   return_addr (pushed by call)
    //   rax_orig   (scratch slot, above iretq frame)
    //   0x10       (iretq SS,   item 5 — highest of the 5)
    //   old_rsp    (iretq RSP,  item 4 — points to scratch after iretq)
    //   rflags     (iretq RFL,  item 3)
    //   0x08       (iretq CS,   item 2)
    //   resume_rip (iretq RIP,  item 1 — lowest of the 5)
    //   rax..r15   (15 GPRs: rax at top, r15 at bottom)
    //
    // Restore: pop 15 GPRs (r15..rax), then iretq (5 pops: RIP,CS,RFL,RSP,SS).
    // After iretq, RSP = old_rsp (= scratch address).  "3:" does add rsp,8; ret.
    //
    // 1. Save rax in scratch slot
    "  push rax",
    // 2. Push 5-item iretq frame: SS, RSP, RFLAGS, CS, RIP
    "  push 0x10",            // SS
    "  lea rax, [rsp + 8]",   // OLD_RSP = scratch address (RSP before SS push)
    "  push rax",              // RSP  (restored by iretq)
    "  pushfq",                // RFLAGS
    "  push 0x08",             // CS
    "  lea rax, [rip + 3f]",   // RIP = resume point
    "  push rax",
    // 3. Restore rax from scratch slot ([rsp+40])
    "  mov rax, [rsp + 40]",
    // 4. Push 15 GPRs
    "  push rax",  "  push rcx",  "  push rdx",  "  push rbx",
    "  push rbp",  "  push rsi",  "  push rdi",  "  push r8",
    "  push r9",   "  push r10",  "  push r11",  "  push r12",
    "  push r13",  "  push r14",  "  push r15",
    // Call reschedule
    "  mov rcx, rsp",
    "  sub rsp, 32",
    "  call reschedule",
    "  add rsp, 32",
    "  mov rsp, rax",
    // Switch to next task (pop 15 GPRs + iretq)
    "  pop r15",  "  pop r14",  "  pop r13",  "  pop r12",
    "  pop r11",  "  pop r10",  "  pop r9",   "  pop r8",
    "  pop rdi",  "  pop rsi",  "  pop rbp",  "  pop rbx",
    "  pop rdx",  "  pop rcx",  "  pop rax",
    "  iretq",
    // Resume point: iretq lands here. RSP = old_rsp (= scratch address).
    "3:",
    "  add rsp, 8",  // skip scratch slot (rax_orig)
    "  ret",
);

extern "C" { fn yield_now_asm(); }

pub fn yield_now() {
    unsafe { yield_now_asm(); }
}

pub fn exit() {
    unsafe {
        asm!("cli");
        let cur = CURRENT;
        let was_ring3 = TASKS[cur].ring3;

        // Free task pages (safe: we're not using them right now).
        // kstack_phys stays in slot — deferred free on slot reuse.
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
            load_gdt_tss(TASKS[next].gdt_phys, TASKS[next].tss_phys, ktop);
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
    fpu_init();
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

// ---- Cooperative test ------------------------------------------------------

extern "C" fn task_a() {
    crate::driver::uart::write_str("[A] start\r\n");
    yield_now();
    crate::driver::uart::write_str("[A] resumed\r\n");
    exit();
}

pub fn test() {
    crate::driver::uart::write_str("\r\n=== Cooperative test ===\r\n");
    let id = spawn(task_a).unwrap();
    crate::driver::uart::write_str("[MAIN] spawned id=");
    uart_hex(id);
    unsafe {
        let t = &TASKS[id as usize];
        crate::driver::uart::write_str(" sp=");
        uart_hex(t.sp);
        let frame = t.sp as *const u64;
        crate::driver::uart::write_str(" [RIP]=");
        uart_hex(*frame.add(120/8));
        crate::driver::uart::write_str(" [CS]=");
        uart_hex(*frame.add(128/8));
        crate::driver::uart::write_str(" [RFL]=");
        uart_hex(*frame.add(136/8));
        crate::driver::uart::write_str("\r\n");
    }
    crate::driver::uart::write_str("[MAIN] before yield\r\n");
    yield_now();
    crate::driver::uart::write_str("[MAIN] first back\r\n");
    yield_now();
    crate::driver::uart::write_str("[MAIN] second back\r\n");
    crate::driver::uart::write_str("=== done ===\r\n");
}

// ---- Preemption test -------------------------------------------------------

const NUM_WORKERS: usize = 4;
static mut COUNTERS: [u64; NUM_WORKERS] = [0; NUM_WORKERS];
extern "C" fn worker0() { loop { unsafe { COUNTERS[0] += 1; } } }
extern "C" fn worker1() { loop { unsafe { COUNTERS[1] += 1; } } }
extern "C" fn worker2() { loop { unsafe { COUNTERS[2] += 1; } } }
extern "C" fn worker3() { loop { unsafe { COUNTERS[3] += 1; } } }

pub fn preempt_test() {
    crate::driver::uart::write_str("\r\n=== Preemption test ===\r\n");
    let workers: [extern "C" fn(); NUM_WORKERS] = [worker0, worker1, worker2, worker3];
    for &w in &workers { spawn(w); }
    crate::driver::uart::write_str("[PREEMPT] 4 workers spawned, waiting 2s...\r\n");
    let deadline = crate::timer::millis() + 2000;
    while crate::timer::millis() < deadline {
        core::hint::spin_loop();
    }
    unsafe {
        for i in 0..NUM_WORKERS {
            crate::driver::uart::write_str("  Worker ");
            crate::driver::uart::putchar(b'0' + i as u8);
            crate::driver::uart::write_str(": ");
            let mut v = COUNTERS[i];
            let mut buf = [0u8; 20]; let mut bi = 0;
            if v == 0 { crate::driver::uart::putchar(b'0'); } else {
                while v > 0 { buf[bi] = b'0' + (v % 10) as u8; v /= 10; bi += 1; }
                while bi > 0 { bi -= 1; crate::driver::uart::putchar(buf[bi]); }
            }
            crate::driver::uart::write_str(" iterations\r\n");
        }
    }
    crate::driver::uart::write_str("=== Preemption done ===\r\n");
}
