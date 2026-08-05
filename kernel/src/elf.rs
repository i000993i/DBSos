fn uart_print(s: &str) { crate::driver::uart::write_str(s); }

fn uart_hex(mut val: u64) {
    if val == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut b = [0u8; 16]; let mut i = 0;
    while val > 0 { let n = (val & 0xF) as u8; b[i] = if n < 10 { b'0' + n } else { b'A' + n - 10 }; val >>= 4; i += 1; }
    while i > 0 { i -= 1; crate::driver::uart::putchar(b[i]); }
}

#[repr(C, packed)]
struct Elf64Ehdr {
    ident: [u8; 16],
    type_: u16,
    machine: u16,
    version: u32,
    entry: u64,
    phoff: u64,
    shoff: u64,
    flags: u32,
    ehsize: u16,
    phentsize: u16,
    phnum: u16,
    shentsize: u16,
    shnum: u16,
    shstrndx: u16,
}

#[repr(C, packed)]
struct Elf64Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

const PT_LOAD: u32 = 1;
const PF_W: u32 = 2;

// User-space virtual addresses
const USER_STACK_VIRT: u64 = 0x00007FFFFFF000;
const USER_STACK_SIZE: u64 = 0x1000;
// From scheduler
use crate::scheduler;

pub fn load_and_spawn(name: &[u8]) -> u64 {
    // Read ELF file
    let mut elf_buf = [0u8; 4096];
    let _file_size = match crate::fs::read_file_path(name, &mut elf_buf) {
        Some(sz) => sz,
        None => { uart_print("[ELF] file not found or too large\r\n"); return 0; }
    };

    let ehdr = unsafe { &*(elf_buf.as_ptr() as *const Elf64Ehdr) };

    if &ehdr.ident[0..4] != b"\x7fELF" || ehdr.ident[4] != 2 || ehdr.ident[5] != 1 {
        uart_print("[ELF] bad magic/class/endian\r\n");
        return 0;
    }
    if ehdr.machine != 62 || (ehdr.type_ != 2 && ehdr.type_ != 3) {
        uart_print("[ELF] not x86_64 exe/dyn\r\n");
        return 0;
    }

    let entry = ehdr.entry;
    let phoff = ehdr.phoff as usize;
    let phnum = ehdr.phnum as usize;
    let phentsize = ehdr.phentsize as usize;

    uart_print("[ELF] entry=0x");
    uart_hex(entry);
    uart_print(" phnum=");
    let mut v = phnum as u64;
    let mut b = [0u8; 20]; let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; crate::driver::uart::putchar(b[i]); }
    uart_print("\r\n");

    let code_phys = match crate::memory::palloc() { 0 => { uart_print("[ELF] alloc failed\r\n"); return 0; } p => p };
    let stack_phys = match crate::memory::palloc() { 0 => { crate::memory::pfree(code_phys); return 0; } p => p };
    let gdt_phys = match crate::memory::palloc() { 0 => { crate::memory::pfree(code_phys); crate::memory::pfree(stack_phys); return 0; } p => p };
    let tss_page = match crate::memory::palloc() { 0 => { crate::memory::pfree(code_phys); crate::memory::pfree(stack_phys); crate::memory::pfree(gdt_phys); return 0; } p => p };

    let pml4 = match unsafe { crate::syscall::prepare_user_pml4() } {
        Some(v) => v,
        None => { uart_print("[ELF] pml4 failed\r\n"); return 0; }
    };

    unsafe {
        if crate::vm::map_page(pml4, stack_phys, USER_STACK_VIRT,
            crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 {
            uart_print("[ELF] stack map failed\r\n"); return 0;
        }
    }

    for i in 0..phnum {
        let phdr = unsafe { &*((elf_buf.as_ptr() as u64 + phoff as u64 + (i * phentsize) as u64) as *const Elf64Phdr) };
        if phdr.p_type != PT_LOAD { continue; }

        let vaddr = phdr.p_vaddr;
        let filesz = phdr.p_filesz as usize;
        let memsz = phdr.p_memsz as usize;
        let offset = phdr.p_offset as usize;

        let seg_start = vaddr & !0xFFF;
        let seg_end = (vaddr + memsz as u64 + 0xFFF) & !0xFFF;
        let num_pages = ((seg_end - seg_start) / 0x1000) as usize;

        for p in 0..num_pages {
            let page_phys = match crate::memory::palloc() {
                0 => { uart_print("[ELF] page alloc failed\r\n"); return 0; }
                p => p
            };
            let page_virt = seg_start + (p as u64) * 0x1000;
            let mut flags = crate::vm::PTE_USER;
            if phdr.p_flags & PF_W != 0 { flags |= crate::vm::PTE_WRITABLE; }
            unsafe {
                if crate::vm::map_page(pml4, page_phys, page_virt, flags) != 0 {
                    uart_print("[ELF] map failed\r\n"); return 0;
                }
            }
            // Write segment data directly into the physical page (kernel identity
            // map), so the user PTE keeps its final RX/RW flags.
            let rel = page_virt - vaddr;
            if rel < filesz as u64 {
                let n = core::cmp::min(filesz as u64 - rel, 0x1000) as usize;
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        elf_buf.as_ptr().add(offset + rel as usize),
                        page_phys as *mut u8, n);
                }
            }
            let z_lo = if rel >= filesz as u64 { rel } else { filesz as u64 };
            let z_hi = core::cmp::min(rel + 0x1000, memsz as u64);
            if z_hi > z_lo {
                unsafe {
                    core::ptr::write_bytes(
                        (page_phys as *mut u8).add((z_lo - rel) as usize),
                        0, (z_hi - z_lo) as usize);
                }
            }
        }
    }

    let orig = unsafe { crate::vm::current_pml4() as *mut u64 };
    unsafe { crate::vm::switch_to(pml4); }
    let sys_krsp_val = unsafe { crate::syscall::sys_krsp };
    unsafe { crate::syscall::setup_user_gdt_tss(gdt_phys, tss_page, sys_krsp_val, false); }
    unsafe {
        core::arch::asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
            limit: (8*8-1) as u16, base: gdt_phys } as *const _ as u64);
        crate::vm::switch_to(orig);
        let mut pd: crate::interrupts::GdtPacked = core::mem::zeroed();
        core::arch::asm!("sgdt [{}]", in(reg) (&mut pd as *mut crate::interrupts::GdtPacked) as u64);
        crate::syscall::sys_kret = crate::syscall::ring3_done as *const () as u64;
    }

    let user_rsp = USER_STACK_VIRT + USER_STACK_SIZE;
    let id = unsafe {
        scheduler::spawn_user(entry, user_rsp, pml4, gdt_phys, tss_page, code_phys, stack_phys)
    };

    match id {
        Some(tid) => {
            uart_print("[ELF] spawned task id=");
            uart_hex(tid);
            uart_print("\r\n");
            tid
        }
        None => {
            uart_print("[ELF] spawn failed\r\n");
            0
        }
    }
}
