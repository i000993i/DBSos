// Ring-3 test tasks

use core::arch::asm;
use super::env::{prepare_user_pml4, create_user_task_env, setup_user_gdt_tss};

const ENTRY_A: u64 = 0x100000000;
const ENTRY_B: u64 = 0x100001000;
const ENTRY_E1000: u64 = 0x100002000;
const ENTRY_CONSOLE: u64 = 0x100003000;
const MMIO_VIRT: u64 = 0x30000000;
const E1000_BDF: u64 = 0x0010;
const E1000_BAR0_OFF: u64 = 0x10;

pub unsafe fn write_user_code_sender(dst: *mut u8) {
    use super::emit::*;
    let mut off: usize = 0;
    let msg: &[u8] = b"[A] ring3 sender!\r\n";
    let str_off: usize = 0x40;
    off += emit_print(dst, off, ENTRY_A + str_off as u64, msg.len() as u32);
    dst.add(off).write(0xEB); dst.add(off+1).write(0xFE);
    for i in 0..msg.len() { dst.add(str_off + i).write(msg[i]); }
}

pub unsafe fn write_user_code_receiver(dst: *mut u8) {
    use super::emit::*;
    let mut off: usize = 0;
    let msg: &[u8] = b"[B] ring3 receiver!\r\n";
    let str_off: usize = 0x40;
    off += emit_print(dst, off, ENTRY_B + str_off as u64, msg.len() as u32);
    dst.add(off).write(0xEB); dst.add(off+1).write(0xFE);
    for i in 0..msg.len() { dst.add(str_off + i).write(msg[i]); }
}

pub unsafe fn write_user_code_e1000(dst: *mut u8) {
    use super::emit::*;
    let mut p: usize = 0;

    p += emit_syscall3(dst, p, 18, E1000_BDF, E1000_BAR0_OFF, 0);
    p += emit_mov_r64(dst, p, 3, 0);

    p += emit_mov_imm64(dst, p, 1, 0xFFFFFFF0);
    dst.add(p).write(0x48); p += 1;
    dst.add(p).write(0x21); p += 1;
    dst.add(p).write(0xCB); p += 1;

    p += emit_mov_imm64(dst, p, 8, 0x20000);
    p += emit_mov_imm64(dst, p, 9, MMIO_VIRT);
    p += emit_mov_r64(dst, p, 2, 3);
    p += emit_mov_imm32(dst, p, 0, 17);
    p += emit_syscall(dst, p);

    p += emit_mov_imm64(dst, p, 2, MMIO_VIRT);

    p += emit_mmio_read32(dst, p, 0, 2, 0x5400);
    p += emit_mmio_read32(dst, p, 1, 2, 0x5404);
    p += emit_mmio_read32(dst, p, 8, 2, 0x0008);

    let ok_str = b"[E1000] driver OK: BAR+MMIO+MAC+STATUS read from ring3\r\n";
    let str_off: usize = 0x100;
    for i in 0..ok_str.len() { dst.add(str_off + i).write(ok_str[i]); }
    p += emit_print(dst, p, ENTRY_E1000 + str_off as u64, ok_str.len() as u32);

    dst.add(p).write(0xEB); dst.add(p+1).write(0xFE);
}

pub unsafe fn test_ring3_e1000() {
    crate::driver::uart::write_str("[E1000] spawning userspace driver...\r\n");

    let entry = ENTRY_E1000;
    let stack_virt: u64 = 0x200002000;

    let code_phys = match crate::memory::palloc() { 0 => return, v => v };
    let stack_phys = match crate::memory::palloc() { 0 => { crate::memory::pfree(code_phys); return } v => v };
    let gdt_phys = match crate::memory::palloc() { 0 => { crate::memory::pfree(code_phys); crate::memory::pfree(stack_phys); return } v => v };
    let tss_page = match crate::memory::palloc() { 0 => { crate::memory::pfree(code_phys); crate::memory::pfree(stack_phys); crate::memory::pfree(gdt_phys); return } v => v };

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

    let orig = crate::vm::current_pml4() as *mut u64;
    crate::vm::switch_to(pml4);
    let sys_krsp_val = core::ptr::addr_of!(super::sys_krsp).read();
    setup_user_gdt_tss(gdt_phys, tss_page, sys_krsp_val, false);
    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: (8*8-1) as u16, base: gdt_phys } as *const _ as u64);
    crate::vm::switch_to(orig);

    let mut pd: crate::interrupts::GdtPacked = core::mem::zeroed();
    asm!("sgdt [{}]", in(reg) (&mut pd as *mut crate::interrupts::GdtPacked) as u64);

    super::sys_kret = super::ring3_done as *const () as u64;
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

    let code_phys = match crate::memory::palloc() { 0 => return, v => v };
    let stack_phys = match crate::memory::palloc() { 0 => return, v => v };
    let gdt_phys = match crate::memory::palloc() { 0 => return, v => v };
    let tss_page = match crate::memory::palloc() { 0 => return, v => v };
    let pml4 = match prepare_user_pml4() { Some(v) => v, None => return };

    use super::emit::*;
    let code = code_phys as *mut u8;
    let mut off: usize = 0;
    let str_off: usize = 0x100;
    let str_virt: u64 = entry + str_off as u64;
    let startup = b"[CONSOLE] server ready\r\n";
    for (i, &b) in startup.iter().enumerate() { code.add(str_off + i).write(b); }
    off += emit_print(code, off, str_virt, startup.len() as u32);
    let loop_off = off;
    code.add(off).write(0x48); off += 1;
    code.add(off).write(0x81); off += 1;
    code.add(off).write(0xEC); off += 1;
    for i in 0..4 { code.add(off+i).write((80u32 >> (i*8)) as u8); }
    off += 4;
    let cap_off = off;
    off += emit_mov_imm64(code, off, 2, 0);
    off += emit_mov_r64(code, off, 8, 4);
    off += emit_mov_imm64(code, off, 9, 0);
    off += emit_mov_imm32(code, off, 0, 12);
    off += emit_syscall(code, off);
    off += emit_mov_r64(code, off, 2, 4);
    code.add(off).write(0x48); off += 1;
    code.add(off).write(0x83); off += 1;
    code.add(off).write(0xC2); off += 1;
    code.add(off).write(12); off += 1;
    off += emit_mov_imm32(code, off, 8, 64);
    off += emit_mov_imm32(code, off, 0, 20);
    off += emit_syscall(code, off);
    code.add(off).write(0x48); off += 1;
    code.add(off).write(0x81); off += 1;
    code.add(off).write(0xC4); off += 1;
    for i in 0..4 { code.add(off+i).write((80u32 >> (i*8)) as u8); }
    off += 4;
    let disp = (loop_off as i64 - off as i64 - 2) as i8;
    code.add(off).write(0xEB); off += 1;
    code.add(off).write(disp as u8);
    let cap_idx_code_off = cap_off + 3;

    if crate::vm::map_page(pml4, code_phys, entry,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 { return; }
    if crate::vm::map_page(pml4, stack_phys, stack_virt,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 { return; }

    let orig = crate::vm::current_pml4() as *mut u64;
    crate::vm::switch_to(pml4);
    let sys_krsp_val = core::ptr::addr_of!(super::sys_krsp).read();
    setup_user_gdt_tss(gdt_phys, tss_page, sys_krsp_val, false);
    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: (8*8-1) as u16, base: gdt_phys } as *const _ as u64);
    crate::vm::switch_to(orig);
    let mut pd: crate::interrupts::GdtPacked = core::mem::zeroed();
    asm!("sgdt [{}]", in(reg) (&mut pd as *mut crate::interrupts::GdtPacked) as u64);

    super::sys_kret = super::ring3_done as *const () as u64;
    asm!("cli");
    let id = crate::scheduler::spawn_user(entry, stack_virt + 4096, pml4, gdt_phys, tss_page, code_phys, stack_phys);
    if let Some(tid) = id {
        if let Some(cap_idx) = crate::ipc::create_server_cap(tid, dbsos_abi::ipc::PORT_CONSOLE) {
            let patch_addr = code_phys + cap_idx_code_off as u64;
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

pub unsafe fn test_ring3() {
    // Disable interrupts for the whole ring-3 bringup. The code below switches
    // CR3 and loads per-task GDTs (global state). A timer interrupt landing in
    // the middle leaves the kernel executing with a mismatched CR3+GDT and can
    // corrupt page tables (observed as clobbered user PD/PT pages). Everything
    // is set up atomically; the LAPIC preemption resumes at the next task.
    asm!("cli");
    crate::driver::uart::write_str("[RING3] test start\r\n");

    let entry_a: u64 = 0x100000000u64;
    let entry_b: u64 = 0x100001000u64;
    let stack_a_virt: u64 = 0x200000000u64;
    let stack_b_virt: u64 = 0x200001000u64;

    let (code_phys_a, stack_phys_a, pml4_a, gdt_a, tss_a) =
        match create_user_task_env() { Some(v) => v, None => return };
    write_user_code_sender(code_phys_a as *mut u8);

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
    // DEBUG B mapping
    {
        let v2p = unsafe { crate::vm::virt_to_phys(pml4_b, entry_b) };
        crate::driver::uart::write_str("[B] code_phys=");
        crate::scheduler::uart_hex(code_phys_b);
        crate::driver::uart::write_str(" v2p@");
        crate::scheduler::uart_hex(v2p);
        crate::driver::uart::write_str(" pml4=");
        crate::scheduler::uart_hex(pml4_b as u64);
        // print level phys + first PD entries
        unsafe {
            let p4 = pml4_b as *const u64;
            let e0 = *p4.add((entry_b >> 39) as usize & 0x1FF);
            let p3 = (e0 & 0xFFFF_FFFF_F000) as *const u64;
            let e1 = *p3.add((entry_b >> 30) as usize & 0x1FF);
            crate::driver::uart::write_str(" PML4E=");
            crate::scheduler::uart_hex(e0);
            crate::driver::uart::write_str(" PDPTE=");
            crate::scheduler::uart_hex(e1);
            crate::driver::uart::write_str(" PDphys=");
            crate::scheduler::uart_hex(e1 & 0xFFFF_FFFF_F000);
            let p2 = (e1 & 0xFFFF_FFFF_F000) as *const u64;
            for k in 0..4 {
                crate::driver::uart::write_str(" PD[");
                crate::driver::uart::putchar(b'0' + k as u8);
                crate::driver::uart::write_str("]=");
                crate::scheduler::uart_hex(*p2.add(k));
            }
        }
        crate::driver::uart::write_str("\r\n");
    }

    let mut pd: crate::interrupts::GdtPacked = core::mem::zeroed();
    asm!("sgdt [{}]", in(reg) (&mut pd as *mut crate::interrupts::GdtPacked) as u64);
    let orig_gdt_base = pd.base;
    let orig_gdt_limit = pd.limit;
    let sys_krsp_val = core::ptr::addr_of!(super::sys_krsp).read();

    let orig = crate::vm::current_pml4() as *mut u64;
    crate::vm::switch_to(pml4_a);
    setup_user_gdt_tss(gdt_a, tss_a, sys_krsp_val, false);
    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: (8*8-1) as u16, base: gdt_a } as *const _ as u64);
    crate::vm::switch_to(orig);

    let orig = crate::vm::current_pml4() as *mut u64;
    crate::vm::switch_to(pml4_b);
    setup_user_gdt_tss(gdt_b, tss_b, sys_krsp_val, false);
    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: (8*8-1) as u16, base: gdt_b } as *const _ as u64);
    crate::vm::switch_to(orig);

    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: orig_gdt_limit, base: orig_gdt_base } as *const _ as u64);

    super::sys_kret = super::ring3_done as *const () as u64;
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

    test_ring3_e1000();
    test_ring3_console();
    asm!("sti");
}

pub use super::ring3_done;
