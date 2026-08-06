// Ring-3 test tasks

use core::arch::asm;
use super::env::{prepare_user_pml4, create_user_task_env, setup_user_gdt_tss};

const ENTRY_A: u64 = 0x100000000;
const ENTRY_B: u64 = 0x100001000;
const ENTRY_E1000: u64 = 0x100002000;
const ENTRY_CONSOLE: u64 = 0x100003000;
const ENTRY_DEMAND: u64 = 0x100004000;
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

/// Emit ring-3 code that exercises demand paging:
///  * lazy heap at 0x300000000 — 16 pages, none mapped up front
///  * stack growth from 0x200103000 down to 0x200100000 (3 pages below the
///    single initially-mapped stack page)
/// Every page crossing faults once; the kernel maps a zeroed page and retries.
/// On success prints "[DM] heap+stack demand paging OK" and exits.
pub unsafe fn write_user_code_demand(dst: *mut u8) {
    use super::emit::*;
    let ok_str = b"[DM] heap+stack demand paging OK\r\n";
    let fail_str = b"[DM] VERIFY FAIL\r\n";
    let ok_off: usize = 0x200;
    let fail_off: usize = 0x240;
    let mut p: usize = 0;

    // --- lazy heap: write marker to every 8 bytes across 16 pages ---
    p += emit_mov_imm64(dst, p, 2, 0x300000000u64);        // rdx = heap base
    p += emit_mov_imm64(dst, p, 8, 0x300100000u64);        // r8  = heap end
    p += emit_mov_imm64(dst, p, 1, 0x1111u64);             // rcx = marker
    let hloop = p;
    dst.add(p).write(0x48); dst.add(p+1).write(0x89); dst.add(p+2).write(0x0A); p += 3; // mov [rdx], rcx
    dst.add(p).write(0x48); dst.add(p+1).write(0x83); dst.add(p+2).write(0xC2); dst.add(p+3).write(0x08); p += 4; // add rdx, 8
    dst.add(p).write(0x4C); dst.add(p+1).write(0x39); dst.add(p+2).write(0xC2); p += 3; // cmp rdx, r8
    let j_h = p; dst.add(p).write(0x75); dst.add(p+1).write(0x00); p += 2; // jne hloop
    dst.add(j_h+1).write((hloop as i64 - (j_h as i64 + 2)) as u8);

    // --- verify first heap page ---
    p += emit_mov_imm64(dst, p, 2, 0x300000000u64);
    dst.add(p).write(0x48); dst.add(p+1).write(0x8B); dst.add(p+2).write(0x0A); p += 3; // mov rcx, [rdx]
    p += emit_mov_imm32(dst, p, 0, 0x1111);                 // rax = 0x1111
    dst.add(p).write(0x48); dst.add(p+1).write(0x39); dst.add(p+2).write(0xC1); p += 3; // cmp rcx, rax
    let j_hv = p; dst.add(p).write(0x75); dst.add(p+1).write(0x00); p += 2; // jne fail (patched below)

    // --- verify last heap page ---
    p += emit_mov_imm64(dst, p, 2, 0x3000FF000u64);
    dst.add(p).write(0x48); dst.add(p+1).write(0x8B); dst.add(p+2).write(0x0A); p += 3;
    dst.add(p).write(0x48); dst.add(p+1).write(0x39); dst.add(p+2).write(0xC1); p += 3; // cmp rcx, rax
    let j_hv2 = p; dst.add(p).write(0x75); dst.add(p+1).write(0x00); p += 2;

    // --- stack growth: walk 0x200103000 down to 0x200100000 ---
    p += emit_mov_imm64(dst, p, 2, 0x200103000u64);
    p += emit_mov_imm64(dst, p, 8, 0x200100000u64);
    let sloop = p;
    dst.add(p).write(0x48); dst.add(p+1).write(0x83); dst.add(p+2).write(0xEA); dst.add(p+3).write(0x08); p += 4; // sub rdx, 8
    dst.add(p).write(0x48); dst.add(p+1).write(0x89); dst.add(p+2).write(0x12); p += 3; // mov [rdx], rdx
    dst.add(p).write(0x4C); dst.add(p+1).write(0x39); dst.add(p+2).write(0xC2); p += 3; // cmp rdx, r8
    let j_s = p; dst.add(p).write(0x75); dst.add(p+1).write(0x00); p += 2; // jne sloop
    dst.add(j_s+1).write((sloop as i64 - (j_s as i64 + 2)) as u8);

    // --- verify stack bottom page (marker == address) ---
    p += emit_mov_imm64(dst, p, 2, 0x200100000u64);
    dst.add(p).write(0x48); dst.add(p+1).write(0x8B); dst.add(p+2).write(0x0A); p += 3; // mov rcx, [rdx]
    p += emit_mov_imm64(dst, p, 0, 0x200100000u64);        // rax = expected marker
    dst.add(p).write(0x48); dst.add(p+1).write(0x39); dst.add(p+2).write(0xC1); p += 3; // cmp rcx, rax
    let j_sv = p; dst.add(p).write(0x75); dst.add(p+1).write(0x00); p += 2;

    // --- success path (fall-through): print OK, then exit ---
    for (i, &b) in ok_str.iter().enumerate() { dst.add(ok_off + i).write(b); }
    p += emit_print(dst, p, ENTRY_DEMAND + ok_off as u64, ok_str.len() as u32);
    p += emit_mov_imm32(dst, p, 0, 0);                      // SYS_EXIT
    p += emit_syscall(dst, p);
    dst.add(p).write(0xEB); dst.add(p+1).write(0xFE); p += 2; // jmp $ (unreachable)

    // --- fail path ---
    let fail = p;
    for (i, &b) in fail_str.iter().enumerate() { dst.add(fail_off + i).write(b); }
    p += emit_print(dst, p, ENTRY_DEMAND + fail_off as u64, fail_str.len() as u32);
    dst.add(p).write(0xEB); dst.add(p+1).write(0xFE);

    // Patch forward jne targets to the fail label.
    dst.add(j_hv+1).write((fail as i64 - (j_hv as i64 + 2)) as u8);
    dst.add(j_hv2+1).write((fail as i64 - (j_hv2 as i64 + 2)) as u8);
    dst.add(j_sv+1).write((fail as i64 - (j_sv as i64 + 2)) as u8);
}

pub unsafe fn test_ring3_demand() {
    crate::driver::uart::write_str("[DM] demand paging test...\r\n");

    let entry = ENTRY_DEMAND;
    let stack_virt: u64 = 0x200103000;
    let stack_top = stack_virt + 4096;
    let stack_low: u64 = 0x200100000;
    let heap_virt: u64 = 0x300000000;
    let heap_end: u64 = 0x300100000;

    let code_phys = match crate::memory::palloc() { 0 => return, v => v };
    let stack_phys = match crate::memory::palloc() { 0 => { crate::memory::pfree(code_phys); return } v => v };
    let gdt_phys = match crate::memory::palloc() { 0 => return, v => v };
    let tss_page = match crate::memory::palloc() { 0 => return, v => v };
    let pml4 = match prepare_user_pml4() { Some(v) => v, None => return };

    write_user_code_demand(code_phys as *mut u8);

    if crate::vm::map_page(pml4, code_phys, entry,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 { return; }
    // Map only the initial stack page; the pages below are demand-paged.
    if crate::vm::map_page(pml4, stack_phys, stack_virt,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 { return; }

    // Bring up GDT/TSS with interrupts masked so VMAs can be registered
    // atomically before the task can ever be scheduled.
    asm!("cli");
    let orig = crate::vm::current_pml4() as *mut u64;
    crate::vm::switch_to(pml4);
    let sys_krsp_val = core::ptr::addr_of!(super::sys_krsp).read();
    setup_user_gdt_tss(gdt_phys, tss_page, sys_krsp_val, false);
    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: (8*8-1) as u16, base: gdt_phys } as *const _ as u64);
    crate::vm::switch_to(orig);

    super::sys_kret = super::ring3_done as *const () as u64;
    let id = crate::scheduler::spawn_user(entry, stack_top, pml4, gdt_phys, tss_page, code_phys, stack_phys);
    let hex = b"0123456789ABCDEF";
    if let Some(tid) = id {
        crate::driver::uart::write_str("[DM] spawned id=");
        crate::driver::uart::putchar(hex[(tid >> 4) as usize]);
        crate::driver::uart::putchar(hex[(tid & 0xF) as usize]);
        if let Some(slot) = crate::scheduler::find_task(tid) {
            crate::driver::uart::write_str(" slot=");
            crate::driver::uart::putchar(hex[(slot >> 4) as usize]);
            crate::driver::uart::putchar(hex[(slot & 0xF) as usize]);
            let ok1 = crate::scheduler::vma::add(slot, stack_low, stack_top,
                crate::scheduler::vma::VMA_STACK,
                crate::vm::PTE_WRITABLE | crate::vm::PTE_USER);
            let ok2 = crate::scheduler::vma::add(slot, heap_virt, heap_end,
                crate::scheduler::vma::VMA_HEAP,
                crate::vm::PTE_WRITABLE | crate::vm::PTE_USER);
            crate::driver::uart::write_str(" vma=");
            crate::driver::uart::putchar(if ok1 { b'1' } else { b'0' });
            crate::driver::uart::putchar(if ok2 { b'1' } else { b'0' });
        } else {
            crate::driver::uart::write_str(" find=NONE");
        }
        crate::driver::uart::write_str("\r\n");
    } else {
        crate::driver::uart::write_str("[DM] spawn FAILED\r\n");
    }
    asm!("sti");
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
    test_ring3_demand();
    asm!("sti");
}

pub use super::ring3_done;
