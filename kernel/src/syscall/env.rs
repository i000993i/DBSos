// Syscall environment: MSR setup, PML4 creation, GDT/TSS setup

use core::arch::asm;

pub unsafe fn init() {
    let efer_lo: u32;
    let efer_hi: u32;
    asm!("rdmsr", out("eax") efer_lo, out("edx") efer_hi, in("ecx") 0xC0000080u32);
    asm!("wrmsr", in("ecx") 0xC0000080u32, in("eax") efer_lo | 1, in("edx") efer_hi);

    let stack_phys = crate::memory::palloc();
    if stack_phys != 0 {
        asm!("lea rcx, [rip + sys_krsp]",
             "mov [rcx], {0}",
             in(reg) (stack_phys + 4096));
    }

    let star_hi: u32 = 0x0008_0020;
    asm!("wrmsr",
         in("ecx") 0xC0000081u32,
         in("eax") 0u32,
         in("edx") star_hi);

    let lstar = super::syscall_stub as *const () as u64;
    asm!("wrmsr",
         in("ecx") 0xC0000082u32,
         in("eax") lstar as u32,
         in("edx") (lstar >> 32) as u32);

    asm!("wrmsr",
         in("ecx") 0xC0000102u32,
         in("eax") 0u32,
         in("edx") 0u32);

    asm!("wrmsr",
         in("ecx") 0xC0000084u32,
         in("eax") (1u32 << 9) | (1u32 << 10) | (1u32 << 8),
         in("edx") 0u32);

    crate::driver::uart::write_str("[SYSCALL] MSRs configured\r\n");
}

pub unsafe fn prepare_user_pml4() -> Option<*mut u64> {
    let pml4 = crate::vm::create_address_space();
    if pml4.is_null() { return None; }
    let cur = crate::vm::current_pml4() as *mut u64;
    crate::vm::clone_high_half(cur, pml4);
    if !crate::vm::identity_map_2mb(pml4, 0, 0x100000000,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_GLOBAL) { return None; }
    let pml4e0 = *(pml4.add(0));
    if pml4e0 & 1 != 0 {
        *(pml4.add(0)) = pml4e0 | crate::vm::PTE_USER;
        let pdpt = (pml4e0 & 0xFFFF_FFFF_F000) as *mut u64;
        let pdpte0 = *pdpt.add(0);
        if pdpte0 & 1 != 0 { *pdpt.add(0) = pdpte0 | crate::vm::PTE_USER; }
    }
    Some(pml4)
}

pub unsafe fn create_user_task_env() -> Option<(u64, u64, *mut u64, u64, u64)> {
    let gdt_phys = crate::memory::palloc(); if gdt_phys == 0 { return None; }
    let tss_page = crate::memory::palloc(); if tss_page == 0 { return None; }
    let user_code_phys = crate::memory::palloc(); if user_code_phys == 0 { return None; }
    let user_stack_phys = crate::memory::palloc(); if user_stack_phys == 0 { return None; }

    let pml4 = match prepare_user_pml4() { Some(v) => v, None => return None };

    let user_entry = 0x100000000u64;
    let user_stack_virt = 0x200000000u64;
    crate::vm::map_page(pml4, user_code_phys, user_entry,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER);
    crate::vm::map_page(pml4, user_stack_phys, user_stack_virt,
        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER);

    Some((user_code_phys, user_stack_phys, pml4, gdt_phys, tss_page))
}

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

#[allow(dead_code)]
unsafe fn setup_task_pml4(pml4: *mut u64, gdt_phys: u64, tss_page: u64, kstk: u64) {
    let orig = crate::vm::current_pml4() as *mut u64;
    crate::vm::switch_to(pml4);
    setup_user_gdt_tss(gdt_phys, tss_page, kstk, false);
    asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
        limit: (8*8-1) as u16, base: gdt_phys } as *const _ as u64);
    crate::vm::switch_to(orig);
}
