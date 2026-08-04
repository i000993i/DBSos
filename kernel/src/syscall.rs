// syscall/sysret — вход из ring 3 в ring 0

use core::arch::asm;
use dbsos_abi::syscall::*;
use dbsos_abi::cap::*;
use dbsos_abi::ipc::Message;

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
    "  push r11",
    "  push rcx",
    "  push rax",
    "  push rdx",
    "  push rbx",
    "  push rbp",
    "  push rsi",
    "  push rdi",
    "  push r8",
    "  push r9",
    "  push r10",
    "  push r12",
    "  push r13",
    "  push r14",
    "  push r15",
    // Args: RAX=num at RSP+12*8, RDX=arg1 at RSP+11*8,
    //       R8=arg2 at RSP+6*8, R9=arg3 at RSP+5*8
    "  mov rcx, [rsp + 12*8]",
    "  mov rdx, [rsp + 11*8]",
    "  mov r8,  [rsp + 6*8]",
    "  mov r9,  [rsp + 5*8]",
    "  sub rsp, 32",
    "  call syscall_rust_entry",
    "  add rsp, 32",
    "  cmp rax, -1",         // EXIT_MAGIC?
    "  je 3f",
    "  mov [rip + sys_retval], rax",
    "  pop r15",
    "  pop r14",
    "  pop r13",
    "  pop r12",
    "  pop r10",
    "  pop r9",
    "  pop r8",
    "  pop rdi",
    "  pop rsi",
    "  pop rbp",
    "  pop rbx",
    "  pop rdx",
    "  pop rax",
    "  mov rax, [rip + sys_retval]",
    "  mov rcx, [rsp]",      // saved user RIP
    "  mov r11, [rsp + 8]",  // saved user RFLAGS
    "  add rsp, 16",         // remove RCX/R11 from stack
    "  push 0x2B",           // SS = user data (0x28 | 3)
    "  push [rip + sys_ursave]", // push user RSP
    "  push r11",             // push saved RFLAGS
    "  push 0x23",           // CS = user code (0x20 | 3)
    "  push rcx",            // push saved RIP
    "  iretq",
    "3:",                    // exit path: call ring3_done, then halt
    "  add rsp, 15*8",       // discard saved GPRs (15 pushes)
    "  mov rsp, [rip + sys_krsp]", // restore kernel RSP
    "  mov rax, [rip + sys_kret]",
    "  call rax",            // call ring3_done (prints + returns)
    "  cli",
    "  hlt",                 // stop after ring 3 test
);

/// Shared user PML4 — set by test_ring3, reused by test_ring3_e1000.
pub static mut USER_PML4: *mut u64 = core::ptr::null_mut();

const EXIT_MAGIC: u64 = !0u64; // -1

#[no_mangle]
unsafe extern "C" fn syscall_rust_entry(num: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    match num {
        SYS_EXIT => {
            crate::driver::uart::write_str("[SYSCALL] exit\r\n");
            EXIT_MAGIC
        }

        // Legacy u64-based IPC (для ring-3 теста)
        SYS_IPC_SEND_LEGACY => {
            let dst_id = arg1;
            let val = arg2;
            crate::scheduler::ipc_send_u64(dst_id, val)
        }
        SYS_IPC_RECV_LEGACY => {
            let _src_id = arg1;
            crate::scheduler::ipc_recv_u64()
        }

        // Cap-based IPC: arg1 = cap_idx, arg2 = msg_ptr
        SYS_IPC_SEND => {
            let cap_idx = arg1 as u16;
            let msg_ptr = arg2 as *const Message;
            if crate::cap::validate(cap_idx, CAP_SEND) {
                crate::ipc::send_with_cap(cap_idx, &*msg_ptr) as u64
            } else {
                IPC_ERR_DENIED as u64
            }
        }
        SYS_IPC_RECV => {
            let cap_idx = arg1 as u16;
            let buf_ptr = arg2 as *mut Message;
            if crate::cap::validate(cap_idx, CAP_RECV) {
                crate::ipc::recv_with_cap(cap_idx, &mut *buf_ptr) as u64
            } else {
                IPC_ERR_DENIED as u64
            }
        }

        SYS_LOG_WRITE => {
            let ptr = arg1 as *const u8;
            let len = arg2 as usize;
            for i in 0..len {
                crate::driver::uart::putchar(core::ptr::read_volatile(ptr.add(i)));
            }
            0
        }

        // Capability management
        SYS_CAP_GRANT => {
            let dst_task_id = arg1;
            let cap_idx = arg2 as u16;
            crate::cap::duplicate(dst_task_id, cap_idx) as u64
        }

        SYS_SHMEM_MAP => {
            let cap_idx = arg1 as u16;
            let virt = arg2;
            if let Some(cap) = crate::cap::get(cap_idx) {
                if cap.cap_type == CapType::SharedMem as u64 && (cap.rights & CAP_WRITE) != 0 {
                    let phys = cap.data;
                    let cur = crate::scheduler::CURRENT;
                    let pml4 = crate::scheduler::TASKS[cur].pml4;
                    if pml4.is_null() { return IPC_ERR_DENIED as u64; }
                    if crate::vm::map_page(pml4, phys, virt,
                        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 {
                        return IPC_ERR_NO_MEM as u64;
                    }
                    0
                } else {
                    IPC_ERR_DENIED as u64
                }
            } else {
                IPC_ERR_BAD_CAP as u64
            }
        }

        SYS_SHMEM_CREATE => {
            let _pages = arg1;
            let phys = crate::memory::palloc();
            if phys == 0 { return IPC_ERR_NO_MEM as u64; }
            match crate::cap::alloc(
                CapType::SharedMem as u64, 0,
                CAP_READ | CAP_WRITE, phys)
            {
                Some(idx) => idx as u64,
                None => { crate::memory::pfree(phys); IPC_ERR_NO_MEM as u64 }
            }
        }

        SYS_MMIO_MAP => {
            let phys_addr = arg1;
            let size = arg2;
            let virt = arg3;
            // Security: only allow mapping known PCI MMIO regions
            if !crate::driver::pci::validate_mmio(phys_addr, size) {
                return IPC_ERR_DENIED as u64;
            }
            let cur = crate::scheduler::CURRENT;
            let pml4 = crate::scheduler::TASKS[cur].pml4;
            if pml4.is_null() { return IPC_ERR_DENIED as u64; }
            let page_count = ((size + 0xFFF) / 0x1000) as usize;
            for i in 0..page_count {
                let pa = phys_addr + (i as u64) * 0x1000;
                let va = virt + (i as u64) * 0x1000;
                if crate::vm::map_page(pml4, pa, va,
                    crate::vm::PTE_WRITABLE | crate::vm::PTE_USER
                    | crate::vm::PTE_CACHE_DISABLE) != 0
                {
                    return IPC_ERR_NO_MEM as u64;
                }
            }
            0
        }

        SYS_PCI_READ => {
            let bdf = arg1;
            let offset = arg2 as u8;
            let bus = ((bdf >> 8) & 0xFF) as u8;
            let dev = ((bdf >> 3) & 0x1F) as u8;
            let func = (bdf & 0x7) as u8;
            crate::driver::pci::read32(bus, dev, func, offset) as u64
        }

        SYS_PCI_WRITE => {
            let bdf = arg1;
            let offset = arg2 as u8;
            let val = arg3 as u32;
            let bus = ((bdf >> 8) & 0xFF) as u8;
            let dev = ((bdf >> 3) & 0x1F) as u8;
            let func = (bdf & 0x7) as u8;
            crate::driver::pci::write32(bus, dev, func, offset, val);
            0
        }

        SYS_CAP_GET_DATA => {
            let cap_idx = arg1 as u16;
            match crate::cap::get(cap_idx) {
                Some(c) => c.data,
                None => IPC_ERR_BAD_CAP as u64,
            }
        }

        _ => {
            crate::driver::uart::write_str("[SYSCALL] num=");
            let hex = b"0123456789ABCDEF";
            crate::driver::uart::putchar(hex[((num >> 4) & 0xF) as usize]);
            crate::driver::uart::putchar(hex[(num & 0xF) as usize]);
            crate::driver::uart::write_str("\r\n");
            0
        }
    }
}

pub unsafe fn init() {
    // Enable SYSCALL in IA32_EFER (bit 0 = SCE)
    let efer_lo: u32;
    let efer_hi: u32;
    asm!("rdmsr", out("eax") efer_lo, out("edx") efer_hi, in("ecx") 0xC0000080u32);
    asm!("wrmsr", in("ecx") 0xC0000080u32, in("eax") efer_lo | 1, in("edx") efer_hi);

    // Allocate a dedicated kernel stack for syscall handler
    let stack_phys = crate::memory::palloc();
    if stack_phys != 0 {
        // Write kernel stack pointer into the asm variable
        asm!("lea rcx, [rip + sys_krsp]",
             "mov [rcx], {0}",
             in(reg) (stack_phys + 4096));
    }

    // STAR MSR = 0x0008_0020_0000_0000
    // [63:48]=SYSCALL CS (0x08), [47:32]=SYSRET base (0x20, CS=0x20|3=0x23 user code,
    //                                                  SS=0x20+8|3=0x2B user data)
    let star_hi: u32 = 0x0008_0020;  // STAR[63:32]
    asm!("wrmsr",
         in("ecx") 0xC0000081u32,
         in("eax") 0u32,
         in("edx") star_hi);

    // LSTAR MSR = syscall entry point
    let lstar = syscall_stub as *const () as u64;
    asm!("wrmsr",
         in("ecx") 0xC0000082u32,
         in("eax") lstar as u32,
         in("edx") (lstar >> 32) as u32);

    // KERNEL_GS_BASE (for per-CPU data, currently unused but swapgs-safe)
    asm!("wrmsr",
         in("ecx") 0xC0000102u32,
         in("eax") 0u32,
         in("edx") 0u32);

    // FMASK: clear IF, DF, TF on syscall
    asm!("wrmsr",
         in("ecx") 0xC0000084u32,
         in("eax") (1u32 << 9) | (1u32 << 10) | (1u32 << 8),
         in("edx") 0u32);

    crate::driver::uart::write_str("[SYSCALL] MSRs configured\r\n");
}

/// Create a fresh user PML4 with identity map (0-4GB) + kernel high half.
pub unsafe fn prepare_user_pml4() -> Option<*mut u64> {
    let pml4 = crate::vm::create_address_space();
    if pml4.is_null() { return None; }
    let cur = crate::vm::current_pml4() as *mut u64;
    crate::vm::clone_high_half(cur, pml4);
    if !crate::vm::identity_map_2mb(pml4, 0, 0x100000000,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_GLOBAL) { return None; }
    // Add PTE_USER to PML4[0] and PDPT[0] so user-mode walks can reach
    // the identity-mapped range (Intel SDM: every entry in chain needs U/S=1).
    let pml4e0 = *(pml4.add(0));
    if pml4e0 & 1 != 0 {
        *(pml4.add(0)) = pml4e0 | crate::vm::PTE_USER;
        let pdpt = (pml4e0 & 0xFFFF_FFFF_F000) as *mut u64;
        let pdpte0 = *pdpt.add(0);
        if pdpte0 & 1 != 0 { *pdpt.add(0) = pdpte0 | crate::vm::PTE_USER; }
    }
    Some(pml4)
}

/// Prepare all resources for a ring-3 task.
/// Returns (phys of user code, phys of user stack, pml4, gdt_phys, tss_phys).
/// Caller must switch_to(pml4) before writing GDT/TSS.
pub unsafe fn create_user_task_env() -> Option<(u64, u64, *mut u64, u64, u64)> {
    let gdt_phys = crate::memory::palloc(); if gdt_phys == 0 { return None; }
    let tss_page = crate::memory::palloc(); if tss_page == 0 { return None; }
    let user_code_phys = crate::memory::palloc(); if user_code_phys == 0 { return None; }
    let user_stack_phys = crate::memory::palloc(); if user_stack_phys == 0 { return None; }

    let pml4 = match prepare_user_pml4() { Some(v) => v, None => return None };

    // Map user pages at canonical addresses above the 4GB identity map
    let user_entry = 0x100000000u64;     // 4 GB
    let user_stack_virt = 0x200000000u64; // 8 GB
    crate::vm::map_page(pml4, user_code_phys, user_entry,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER);
    crate::vm::map_page(pml4, user_stack_phys, user_stack_virt,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER);

    Some((user_code_phys, user_stack_phys, pml4, gdt_phys, tss_page))
}

/// Write GDT + TSS into palloc'd pages (must be called after switch_to(pml4)).
pub unsafe fn setup_user_gdt_tss(gdt_phys: u64, tss_page: u64, tss_rsp0: u64, tss_busy: bool) {
    core::ptr::write_bytes(tss_page as *mut u8, 0, 512);
    core::ptr::write_unaligned((tss_page + 4) as *mut u64, tss_rsp0);
    let tb = tss_page;
    let typ = if tss_busy { 0x0Bu64 } else { 0x09u64 };
    let tss_lo = 0x67u64
        | ((tb & 0xFFFF) << 16)
        | (((tb >> 16) & 0xFF) << 32)
        | (typ << 40) | (1u64 << 47) | (((tb >> 24) & 0xFF) << 56);
    let tss_hi = (tb >> 32) & 0xFFFF_FFFF;
    asm!("mov qword ptr [{base}], 0", base = in(reg) gdt_phys);
    asm!("mov qword ptr [{base}], {val}", base = in(reg) (gdt_phys+8),   val = in(reg) 0x00209A0000000000u64);
    asm!("mov qword ptr [{base}], {val}", base = in(reg) (gdt_phys+16),  val = in(reg) 0x0000920000000000u64);
    asm!("mov qword ptr [{base}], {val}", base = in(reg) (gdt_phys+24),  val = in(reg) 0x0000F20000000000u64);
    asm!("mov qword ptr [{base}], {val}", base = in(reg) (gdt_phys+32),  val = in(reg) 0x0020FA0000000000u64);
    asm!("mov qword ptr [{base}], {val}", base = in(reg) (gdt_phys+40),  val = in(reg) 0x0000F20000000000u64);
    asm!("mov qword ptr [{base}], {val}", base = in(reg) (gdt_phys+48),  val = in(reg) tss_lo);
    asm!("mov qword ptr [{base}], {val}", base = in(reg) (gdt_phys+56),  val = in(reg) tss_hi);
}

// ── x86-64 code emission helpers ──────────────────────────────────────
// All helpers accept dst (code buffer), off (current offset),
// return bytes written.  For ModRM-based ops we support reg ∈ 0..15.

/// Emit `mov reg64, imm64` — chooses 7-byte sign-extended or 10-byte form.
unsafe fn emit_mov_imm64(dst: *mut u8, off: usize, reg: u8, val: u64) -> usize {
    if val as i64 as u64 == val && (val & 0xFFFFFFFF80000000 == 0 || val & 0xFFFFFFFF80000000 == 0xFFFFFFFF80000000) {
        // mov r/m64, imm32 (sign-extended): REX.W + C7 /0 + modrm + imm32
        let rex = 0x48 | ((reg >> 3) & 1); // REX.W + REX.B
        dst.add(off).write(rex); dst.add(off+1).write(0xC7);
        dst.add(off+2).write(0xC0 | (reg & 7));
        for j in 0..4 { dst.add(off+3+j).write((val >> (j*8)) as u8); }
        7
    } else {
        // mov reg64, imm64: REX.W + B8+reg
        let rex = 0x48 | ((reg >> 3) & 1); // REX.W + REX.B
        dst.add(off).write(rex); dst.add(off+1).write(0xB8 | (reg & 7));
        for j in 0..8 { dst.add(off+2+j).write((val >> (j*8)) as u8); }
        10
    }
}
/// Emit `mov reg32, imm32` (zero-extended to 64-bit).
unsafe fn emit_mov_imm32(dst: *mut u8, off: usize, reg: u8, val: u32) -> usize {
    let rex = 0x48 | ((reg >> 3) & 1); // REX.W + REX.B
    dst.add(off).write(rex); dst.add(off+1).write(0xC7);
    dst.add(off+2).write(0xC0 | (reg & 7));
    for j in 0..4 { dst.add(off+3+j).write((val >> (j*8)) as u8); }
    7
}
/// Emit `syscall` (0F 05).
unsafe fn emit_syscall(dst: *mut u8, off: usize) -> usize {
    dst.add(off).write(0x0F); dst.add(off+1).write(0x05); 2
}
/// Emit `jmp short rel8`.
#[allow(dead_code)]
unsafe fn emit_jmp_rel8(dst: *mut u8, off: usize, disp: i8) -> usize {
    dst.add(off).write(0xEB); dst.add(off+1).write(disp as u8); 2
}
/// Emit `jne rel8` (jump if not zero/equal).
#[allow(dead_code)]
unsafe fn emit_jne_rel8(dst: *mut u8, off: usize, disp: i8) -> usize {
    dst.add(off).write(0x75); dst.add(off+1).write(disp as u8); 2
}
/// Emit `mov r32, [base + disp32]` (32-bit MMIO read, no REX.W).
/// base/r are 0..15.
unsafe fn emit_mmio_read32(dst: *mut u8, off: usize, dst_reg: u8, base: u8, disp: u32) -> usize {
    let mut p = off;
    let rex = (if dst_reg > 7 { 1 << 2 } else { 0 }) | (if base > 7 { 1 << 0 } else { 0 });
    if rex != 0 { dst.add(p).write(0x40 | rex); p += 1; }
    dst.add(p).write(0x8B); p += 1;
    dst.add(p).write(0x80 | ((dst_reg & 7) << 3) | (base & 7)); p += 1;
    for j in 0..4 { dst.add(p+j).write((disp >> (j*8)) as u8); } p += 4;
    p - off
}
/// Emit `mov [base + disp32], r32` (32-bit MMIO write, no REX.W).
#[allow(dead_code)]
unsafe fn emit_mmio_write32(dst: *mut u8, off: usize, base: u8, disp: u32, src_reg: u8) -> usize {
    let mut p = off;
    let rex = (if src_reg > 7 { 1 << 2 } else { 0 }) | (if base > 7 { 1 << 0 } else { 0 });
    if rex != 0 { dst.add(p).write(0x40 | rex); p += 1; }
    dst.add(p).write(0x89); p += 1;
    dst.add(p).write(0x80 | ((src_reg & 7) << 3) | (base & 7)); p += 1;
    for j in 0..4 { dst.add(p+j).write((disp >> (j*8)) as u8); } p += 4;
    p - off
}
/// Emit `and eax, imm32` (5-byte short form 25).
#[allow(dead_code)]
unsafe fn emit_and_eax_imm32(dst: *mut u8, off: usize, val: u32) -> usize {
    dst.add(off).write(0x25);
    for j in 0..4 { dst.add(off+1+j).write((val >> (j*8)) as u8); }
    5
}
/// Emit `or eax, imm32` (5-byte short form 0D).
#[allow(dead_code)]
unsafe fn emit_or_eax_imm32(dst: *mut u8, off: usize, val: u32) -> usize {
    dst.add(off).write(0x0D);
    for j in 0..4 { dst.add(off+1+j).write((val >> (j*8)) as u8); }
    5
}
/// Emit `test eax, imm32` (5-byte short form A9).
#[allow(dead_code)]
unsafe fn emit_test_eax_imm32(dst: *mut u8, off: usize, val: u32) -> usize {
    dst.add(off).write(0xA9);
    for j in 0..4 { dst.add(off+1+j).write((val >> (j*8)) as u8); }
    5
}
/// Emit `mov r/m64, r64` — `mov dst, src`.
/// - `d` = destination register (r/m field, 0..15)
/// - `s` = source register (reg field, 0..15)
unsafe fn emit_mov_r64(dst: *mut u8, off: usize, d: u8, s: u8) -> usize {
    let rex = 0x48 // REX.W
        | (if s > 7 { 1 << 2 } else { 0 }) // REX.R → reg (source)
        | (if d > 7 { 1 << 0 } else { 0 }); // REX.B → r/m (dest)
    dst.add(off).write(rex);
    dst.add(off+1).write(0x89);
    dst.add(off+2).write(0xC0 | ((s & 7) << 3) | (d & 7));
    3
}

// ── SYS_LOG_WRITE helper ──────────────────────────────────────────────
/// Emit a LOG_WRITE syscall (RAX=20, RDX=str, R8=len).
unsafe fn emit_print(dst: *mut u8, off: usize, str_addr: u64, len: u32) -> usize {
    let mut p = off;
    p += emit_mov_imm64(dst, p, 2, str_addr); // RDX = string pointer (arg1)
    p += emit_mov_imm32(dst, p, 8, len);      // R8  = length (arg2)
    p += emit_mov_imm32(dst, p, 0, 20);       // RAX = SYS_LOG_WRITE
    p += emit_syscall(dst, p);
    p - off
}
/// Emit a syscall with 3 user args (RAX=num, RDX=arg1, R8=arg2, R9=arg3).
unsafe fn emit_syscall3(dst: *mut u8, off: usize, num: u64, arg1: u64, arg2: u64, arg3: u64) -> usize {
    let mut p = off;
    p += emit_mov_imm64(dst, p, 8, arg2); // R8 = arg2
    p += emit_mov_imm64(dst, p, 9, arg3); // R9 = arg3
    p += emit_mov_imm64(dst, p, 2, arg1); // RDX = arg1
    p += emit_mov_imm64(dst, p, 0, num);  // RAX = num
    p += emit_syscall(dst, p);
    p - off
}

const ENTRY_A: u64 = 0x100000000;
const ENTRY_B: u64 = 0x100001000;
const ENTRY_E1000: u64 = 0x100002000;
const ENTRY_CONSOLE: u64 = 0x100003000;

// ── Existing ring-3 test tasks ─────────────────────────────────────────
pub unsafe fn write_user_code_sender(dst: *mut u8) {
    let mut off: usize = 0;
    let msg: &[u8] = b"[A] ring3 sender!\r\n";
    let str_off: usize = 0x40;
    off += emit_print(dst, off, ENTRY_A + str_off as u64, msg.len() as u32);
    dst.add(off).write(0xEB); dst.add(off+1).write(0xFE); // jmp $
    for i in 0..msg.len() { dst.add(str_off + i).write(msg[i]); }
}
pub unsafe fn write_user_code_receiver(dst: *mut u8) {
    let mut off: usize = 0;
    let msg: &[u8] = b"[B] ring3 receiver!\r\n";
    let str_off: usize = 0x40;
    off += emit_print(dst, off, ENTRY_B + str_off as u64, msg.len() as u32);
    dst.add(off).write(0xEB); dst.add(off+1).write(0xFE); // jmp $
    for i in 0..msg.len() { dst.add(str_off + i).write(msg[i]); }
}

// ── e1000 userspace driver code generator ──────────────────────────────
const MMIO_VIRT: u64 = 0x30000000;
const E1000_BDF: u64 = 0x0010; // bus=0 dev=2 func=0: (0<<8)|(2<<3)|0 = 0x10
const E1000_BAR0_OFF: u64 = 0x10;

pub unsafe fn write_user_code_e1000(dst: *mut u8) {
    let mut p: usize = 0;

    // 1. Read BAR0 via SYS_PCI_READ(18): RAX = PCI config read(BDF, 0x10)
    p += emit_syscall3(dst, p, 18, E1000_BDF, E1000_BAR0_OFF, 0);
    p += emit_mov_r64(dst, p, 3, 0); // RBX = RAX (BAR0 raw)

    // 2. AND RBX with 0xFFFFFFF0 → phys addr in RBX
    p += emit_mov_imm64(dst, p, 1, 0xFFFFFFF0);
    dst.add(p).write(0x48); p += 1; // REX.W
    dst.add(p).write(0x21); p += 1; // AND r/m64, r64
    dst.add(p).write(0xCB); p += 1; // ModRM: reg=1(RCX), r/m=3(RBX) = 11.001.011 = 0xCB

    // 3. SYS_MMIO_MAP(17, phys=RBX, size=0x20000, virt=MMIO_VIRT)
    p += emit_mov_imm64(dst, p, 8, 0x20000);     // R8 = size
    p += emit_mov_imm64(dst, p, 9, MMIO_VIRT);   // R9 = virt
    p += emit_mov_r64(dst, p, 2, 3);              // RDX = RBX (phys)
    p += emit_mov_imm32(dst, p, 0, 17);           // RAX = SYS_MMIO_MAP
    p += emit_syscall(dst, p);

    // 4. RDX = MMIO_VIRT (MMIO base for subsequent reads)
    p += emit_mov_imm64(dst, p, 2, MMIO_VIRT);

    // 5. Read MAC registers
    p += emit_mmio_read32(dst, p, 0, 2, 0x5400);  // EAX = MAC_LO
    p += emit_mmio_read32(dst, p, 1, 2, 0x5404);  // ECX = MAC_HI
    p += emit_mmio_read32(dst, p, 8, 2, 0x0008);  // R8d = STATUS (link, etc.)

    // 6. Print success message
    let ok_str = b"[E1000] driver OK: BAR+MMIO+MAC+STATUS read from ring3\r\n";
    let str_off: usize = 0x100;
    for i in 0..ok_str.len() { dst.add(str_off + i).write(ok_str[i]); }
    p += emit_print(dst, p, ENTRY_E1000 + str_off as u64, ok_str.len() as u32);

    // 7. Idle loop: jmp $ (infinite, never re-execute)
    dst.add(p).write(0xEB); dst.add(p+1).write(0xFE);
}

/// Allocate a page + stack + GDT/TSS and spawn the e1000 ring-3 driver task.
pub unsafe fn test_ring3_e1000() {
    crate::driver::uart::write_str("[E1000] spawning userspace driver...\r\n");

    let entry = ENTRY_E1000;
    let stack_virt: u64 = 0x200002000; // 8 GB + 8 KB

    let code_phys = match crate::memory::palloc() { 0 => return, v => v };
    let stack_phys = match crate::memory::palloc() { 0 => { crate::memory::pfree(code_phys); return } v => v };
    let gdt_phys = match crate::memory::palloc() { 0 => { crate::memory::pfree(code_phys); crate::memory::pfree(stack_phys); return } v => v };
    let tss_page = match crate::memory::palloc() { 0 => { crate::memory::pfree(code_phys); crate::memory::pfree(stack_phys); crate::memory::pfree(gdt_phys); return } v => v };

    // Each ring-3 task gets its own PML4 so exit() can free it safely.
    let pml4 = match prepare_user_pml4() { Some(v) => v, None => { return; } };

    write_user_code_e1000(code_phys as *mut u8);

    if crate::vm::map_page(pml4, code_phys, entry,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 {
        crate::driver::uart::write_str("[CONSOLE] FAIL map code\r\n"); return;
    }
    if crate::vm::map_page(pml4, stack_phys, stack_virt,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 {
        crate::driver::uart::write_str("[CONSOLE] FAIL map stack\r\n"); return;
    }

    // Write GDT/TSS in this task's own PML4
    let orig = crate::vm::current_pml4() as *mut u64;
    crate::vm::switch_to(pml4);
    let sys_krsp_val = core::ptr::addr_of!(sys_krsp).read();
    setup_user_gdt_tss(gdt_phys, tss_page, sys_krsp_val, false);
    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: (8*8-1) as u16, base: gdt_phys } as *const _ as u64);
    crate::vm::switch_to(orig);

    // Restore kernel GDT (switched away temporarily)
    let mut pd: crate::interrupts::GdtPacked = core::mem::zeroed();
    asm!("sgdt [{}]", in(reg) (&mut pd as *mut crate::interrupts::GdtPacked) as u64);

    sys_kret = crate::syscall::ring3_done as *const () as u64;
    let id = crate::scheduler::spawn_user(entry, stack_virt + 4096, pml4, gdt_phys, tss_page, code_phys, stack_phys);
    if let Some(tid) = id {
        crate::driver::uart::write_str("[E1000] spawned task id=");
        let hex = b"0123456789ABCDEF";
        crate::driver::uart::putchar(hex[(tid >> 4) as usize]);
        crate::driver::uart::putchar(hex[(tid & 0xF) as usize]);
        crate::driver::uart::write_str("\r\n");
    } else {
        crate::driver::uart::write_str("[E1000] spawn FAILED\r\n");
    }
}

pub unsafe fn test_ring3_console() {
    crate::driver::uart::write_str("[CONSOLE] spawning server...\r\n");

    let entry = ENTRY_CONSOLE;
    let stack_virt: u64 = 0x200003000;

    // Manually generate console server code in syscall.rs (avoid cross-crate issues)
    let code_phys = match crate::memory::palloc() { 0 => return, v => v };
    let stack_phys = match crate::memory::palloc() { 0 => return, v => v };
    let gdt_phys = match crate::memory::palloc() { 0 => return, v => v };
    let tss_page = match crate::memory::palloc() { 0 => return, v => v };
    let pml4 = match prepare_user_pml4() { Some(v) => v, None => return };

    // ── Generate code: print startup, then loop: IPC_RECV; LOG_WRITE ──
    let code = code_phys as *mut u8;
    let mut off: usize = 0;
    let str_off: usize = 0x100;
    let str_virt: u64 = entry + str_off as u64;
    let startup = b"[CONSOLE] server ready\r\n";
    for (i, &b) in startup.iter().enumerate() { code.add(str_off + i).write(b); }
    off += emit_print(code, off, str_virt, startup.len() as u32);
    // Loop: sub rsp,80; SYS_IPC_RECV(cap_idx, rsp, 0); SYS_LOG_WRITE(rsp, 80); add rsp,80; jmp loop
    let loop_off = off;
    // sub rsp, 80: 48 81 EC 50 00 00 00
    code.add(off).write(0x48); off += 1;
    code.add(off).write(0x81); off += 1;
    code.add(off).write(0xEC); off += 1;
    for i in 0..4 { code.add(off+i).write((80u32 >> (i*8)) as u8); }
    off += 4;
    // mov rdx, cap_idx (placeholder 0)
    let cap_off = off;
    off += emit_mov_imm64(code, off, 2, 0);
    // mov r8, rsp
    off += emit_mov_r64(code, off, 8, 4);
    // mov r9, 0
    off += emit_mov_imm64(code, off, 9, 0);
    // mov eax, 12 (SYS_IPC_RECV)
    off += emit_mov_imm32(code, off, 0, 12);
    off += emit_syscall(code, off);
    // mov rdx, rsp + 12 (data field); mov r8d, 64; mov eax, 20; syscall
    off += emit_mov_r64(code, off, 2, 4); // RDX = RSP
    code.add(off).write(0x48); off += 1; // REX.W
    code.add(off).write(0x83); off += 1; // ADD r/m64, imm8
    code.add(off).write(0xC2); off += 1; // ModRM: r/m=RDX
    code.add(off).write(12); off += 1;   // +12 (offset of data field)
    off += emit_mov_imm32(code, off, 8, 64); // length = PAYLOAD_SIZE
    // mov eax, 20 (SYS_LOG_WRITE)
    off += emit_mov_imm32(code, off, 0, 20);
    off += emit_syscall(code, off);
    // add rsp, 80: 48 81 C4 50 00 00 00
    code.add(off).write(0x48); off += 1;
    code.add(off).write(0x81); off += 1;
    code.add(off).write(0xC4); off += 1;
    for i in 0..4 { code.add(off+i).write((80u32 >> (i*8)) as u8); }
    off += 4;
    // jmp loop
    let disp = (loop_off as i64 - off as i64 - 2) as i8;
    code.add(off).write(0xEB); off += 1;
    code.add(off).write(disp as u8);
    // cap_idx offset in code page for later patching
    let cap_idx_code_off = cap_off + 3; // imm32 starts at byte 3 (48 C7 C2 <imm32>)

    // Map pages
    if crate::vm::map_page(pml4, code_phys, entry,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 { return; }
    if crate::vm::map_page(pml4, stack_phys, stack_virt,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 { return; }

    // GDT/TSS setup
    let orig = crate::vm::current_pml4() as *mut u64;
    crate::vm::switch_to(pml4);
    let sys_krsp_val = core::ptr::addr_of!(sys_krsp).read();
    setup_user_gdt_tss(gdt_phys, tss_page, sys_krsp_val, false);
    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: (8*8-1) as u16, base: gdt_phys } as *const _ as u64);
    crate::vm::switch_to(orig);
    let mut pd: crate::interrupts::GdtPacked = core::mem::zeroed();
    asm!("sgdt [{}]", in(reg) (&mut pd as *mut crate::interrupts::GdtPacked) as u64);

    sys_kret = crate::syscall::ring3_done as *const () as u64;
    // Disable interrupts to prevent task preemption during cap setup
    asm!("cli");
    let id = crate::scheduler::spawn_user(entry, stack_virt + 4096, pml4, gdt_phys, tss_page, code_phys, stack_phys);
    if let Some(tid) = id {
        if let Some(cap_idx) = crate::ipc::create_server_cap(tid, dbsos_abi::ipc::PORT_CONSOLE) {
            let patch_addr = code_phys + cap_idx_code_off as u64;
            // emit_mov_imm64 uses short form (imm32) for small values like cap_idx
            core::ptr::write_unaligned(patch_addr as *mut u32, cap_idx as u32);
            asm!("sti");
            crate::driver::uart::write_str("[CONSOLE] spawned id=");
            let hex = b"0123456789ABCDEF";
            crate::driver::uart::putchar(hex[(tid >> 4) as usize]);
            crate::driver::uart::putchar(hex[(tid & 0xF) as usize]);
            crate::driver::uart::write_str(" cap=");
            crate::driver::uart::putchar(hex[(cap_idx >> 4) as usize]);
            crate::driver::uart::putchar(hex[(cap_idx & 0xF) as usize]);
            crate::driver::uart::write_str("\r\n");
            crate::ipc::init_console_client(tid);
        } else {
            asm!("sti");
        }
    } else {
        asm!("sti");
    }
}

/// Update the SYSCALL kernel stack (used by the asm stub).
/// Must be called before returning to each ring-3 task.
pub unsafe fn set_sys_krsp(stack_top: u64) {
    core::ptr::addr_of_mut!(sys_krsp).write(stack_top);
}

pub unsafe fn set_sys_ursave(ursave: u64) {
    core::ptr::addr_of_mut!(sys_ursave).write(ursave);
}

pub unsafe fn read_sys_ursave() -> u64 {
    core::ptr::addr_of!(sys_ursave).read()
}

/// Helper: write GDT + TSS into a task's PML4, then loadlgdt and switch back.
unsafe fn setup_task_pml4(pml4: *mut u64, gdt_phys: u64, tss_page: u64, kstk: u64) {
    let orig = crate::vm::current_pml4() as *mut u64;
    crate::vm::switch_to(pml4);
    setup_user_gdt_tss(gdt_phys, tss_page, kstk, false);
    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: (8*8-1) as u16, base: gdt_phys } as *const _ as u64);
    crate::vm::switch_to(orig);
}

pub unsafe fn test_ring3() {
    crate::driver::uart::write_str("[RING3] test start\r\n");

    let entry_a: u64 = 0x100000000u64;
    let entry_b: u64 = 0x100001000u64;
    let stack_a_virt: u64 = 0x200000000u64;
    let stack_b_virt: u64 = 0x200001000u64;

    // ── Task A (sender) ────────────────────────────────────────────
    let (code_phys_a, stack_phys_a, pml4_a, gdt_a, tss_a) =
        match create_user_task_env() { Some(v) => v, None => return };
    write_user_code_sender(code_phys_a as *mut u8);

    // ── Task B (receiver) — own PML4 ───────────────────────────────
    let code_phys_b = match crate::memory::palloc() { 0 => return, v => v };
    let stack_phys_b = match crate::memory::palloc() { 0 => return, v => v };
    let gdt_b = match crate::memory::palloc() { 0 => return, v => v };
    let tss_b = match crate::memory::palloc() { 0 => return, v => v };
    let pml4_b = match prepare_user_pml4() { Some(v) => v, None => return };

    write_user_code_receiver(code_phys_b as *mut u8);
    if crate::vm::map_page(pml4_b, code_phys_b, entry_b,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 { return; }
    if crate::vm::map_page(pml4_b, stack_phys_b, stack_b_virt,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 { return; }

    // Save original GDT before switching
    let mut pd: crate::interrupts::GdtPacked = core::mem::zeroed();
    asm!("sgdt [{}]", in(reg) (&mut pd as *mut crate::interrupts::GdtPacked) as u64);
    let orig_gdt_base = pd.base;
    let orig_gdt_limit = pd.limit;
    let sys_krsp_val = core::ptr::addr_of!(sys_krsp).read();

    // Write GDT/TSS for each task in its own PML4
    setup_task_pml4(pml4_a, gdt_a, tss_a, sys_krsp_val);
    setup_task_pml4(pml4_b, gdt_b, tss_b, sys_krsp_val);

    // Restore kernel GDT
    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: orig_gdt_limit, base: orig_gdt_base } as *const _ as u64);

    // Spawn both tasks
    sys_kret = crate::syscall::ring3_done as *const () as u64;
    let id_a = crate::scheduler::spawn_user(entry_a, stack_a_virt + 4096, pml4_a, gdt_a, tss_a, code_phys_a, stack_phys_a);
    let id_b = crate::scheduler::spawn_user(entry_b, stack_b_virt + 4096, pml4_b, gdt_b, tss_b, code_phys_b, stack_phys_b);
    crate::driver::uart::write_str("[RING3] spawned: A=");
    let hex = b"0123456789ABCDEF";
    crate::driver::uart::putchar(hex[(id_a.unwrap_or(0) >> 4) as usize]);
    crate::driver::uart::putchar(hex[(id_a.unwrap_or(0) & 0xF) as usize]);
    crate::driver::uart::write_str(" B=");
    crate::driver::uart::putchar(hex[(id_b.unwrap_or(0) >> 4) as usize]);
    crate::driver::uart::putchar(hex[(id_b.unwrap_or(0) & 0xF) as usize]);
    crate::driver::uart::write_str("\r\n");

    // Spawn e1000 userspace driver
    test_ring3_e1000();

    // Spawn console server
    test_ring3_console();
}

/// Called after user mode exits via syscall(0)
pub unsafe fn ring3_done() {
    crate::driver::uart::write_str("[RING3] back to kernel\r\n");
    crate::scheduler::exit();
}


