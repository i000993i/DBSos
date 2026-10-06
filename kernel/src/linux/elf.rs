/// Linux ELF loader — loads statically-linked Linux x86_64 ELF binaries
/// into a new address space with Linux-compatible memory layout.

use super::{ELF_MAGIC, EI_CLASS, EI_DATA, ELFCLASS64, ELFDATA2LSB,
            ET_EXEC, ET_DYN, EM_X86_64, PT_LOAD, PT_INTERP,
            randomize_mmap_base, randomize_stack_top};
use crate::memory::{self, palloc, palloc_n, PAGE_SIZE};
use crate::vm;

fn uart_print(s: &str) { crate::driver::uart::write_str(s); }

#[repr(C)]
struct Elf64Ehdr {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
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

pub struct LinuxProgram {
    pub entry: u64,
    pub pml4: *mut u64,
    pub code_phys: u64,
    pub stack_phys: u64,
    pub brk: u64,
    pub mmap_base: u64,
    pub phdr_vaddr: u64,
    pub interp_vaddr: u64,
    pub interp_size: u64,
    pub stack_size: u64,
    pub stack_top: u64,
}

pub fn load_linux_elf(elf_data: &[u8]) -> Result<LinuxProgram, &'static str> {
    if elf_data.len() < 64 { return Err("ELF too small"); }
    if elf_data[..4] != ELF_MAGIC { return Err("Bad ELF magic"); }
    if elf_data[EI_CLASS] != ELFCLASS64 { return Err("Not 64-bit"); }
    if elf_data[EI_DATA] != ELFDATA2LSB { return Err("Not little-endian"); }

    let hdr = unsafe { &*(elf_data.as_ptr() as *const Elf64Ehdr) };
    if hdr.e_type != ET_EXEC && hdr.e_type != ET_DYN { return Err("Not EXEC or DYN"); }
    if hdr.e_machine != EM_X86_64 { return Err("Not x86_64"); }
    if hdr.e_phnum == 0 || hdr.e_phentsize != 56 { return Err("Bad program headers"); }

    // Check for interpreter (dynamic linking)
    let mut needs_interp = false;
    let mut interp_offset = 0u64;
    let mut interp_filesz = 0u64;
    for i in 0..hdr.e_phnum {
        let off = hdr.e_phoff as usize + i as usize * hdr.e_phentsize as usize;
        if off + 56 > elf_data.len() { break; }
        let phdr = unsafe { &*(elf_data[off..].as_ptr() as *const Elf64Phdr) };
        if phdr.p_type == PT_INTERP {
            needs_interp = true;
            interp_offset = phdr.p_offset;
            interp_filesz = phdr.p_filesz;
        }
    }
    if needs_interp {
        uart_print("[LINUX-ELF] Dynamic binary — loading statically\r\n");
    }

    // Create new address space
    let pml4 = unsafe { vm::create_address_space() };
    if pml4.is_null() { return Err("Failed to create address space"); }

    unsafe {
        let current = vm::current_pml4() as *mut u64;
        vm::clone_high_half(current, pml4);
        // Map only the low 1MB for BIOS/video structures — NOT the entire 4GB.
        // Each ELF segment gets its own specific physical pages mapped below.
        vm::identity_map_2mb(pml4, 0, 0x100_000,
            vm::PTE_WRITABLE | vm::PTE_USER);
    }

    let mut max_vaddr: u64 = 0;
    let mut first_code_phys: u64 = 0;

    // Load PT_LOAD segments
    for i in 0..hdr.e_phnum {
        let off = hdr.e_phoff as usize + i as usize * hdr.e_phentsize as usize;
        if off + 56 > elf_data.len() { break; }
        let phdr = unsafe { &*(elf_data[off..].as_ptr() as *const Elf64Phdr) };
        if phdr.p_type != PT_LOAD { continue; }

        let vaddr = phdr.p_vaddr;
        let memsz = phdr.p_memsz;
        let filesz = phdr.p_filesz;
        if memsz == 0 { continue; }

        let vaddr_page = vaddr & !0xFFF;
        let end_page = ((vaddr + memsz + 0xFFF) & !0xFFF) as u64;
        let pages = ((end_page - vaddr_page) / PAGE_SIZE as u64) as usize;
        if pages == 0 { continue; }

        let pages_phys = palloc_n(pages);
        if pages_phys == 0 { return Err("Out of memory loading ELF segment"); }

        for i in 0..pages {
            memory::memset_phys(pages_phys + (i as u64) * PAGE_SIZE as u64, 0, PAGE_SIZE);
        }

        // Copy segment data
        let file_start = phdr.p_offset as usize;
        let file_end = file_start + filesz as usize;
        if file_end <= elf_data.len() {
            let mut written = 0u64;
            while written < filesz {
                let page_idx = (written / PAGE_SIZE as u64) as usize;
                let page_off = (written % PAGE_SIZE as u64) as usize;
                let phys = pages_phys + (page_idx as u64) * PAGE_SIZE as u64;
                let dst = phys as *mut u8;
                let src = elf_data[(file_start + written as usize)..].as_ptr();
                let copy = (filesz - written).min(PAGE_SIZE as u64 - page_off as u64);
                unsafe {
                    core::ptr::copy_nonoverlapping(src, dst.add(page_off), copy as usize);
                }
                written += copy;
            }
        }

        let mut flags = vm::PTE_USER | vm::PTE_WRITABLE;
        if phdr.p_flags & 1 == 0 { flags |= vm::PTE_NX; }

        for i in 0..pages {
            let phys = pages_phys + (i as u64) * PAGE_SIZE as u64;
            let virt = vaddr_page + (i as u64) * PAGE_SIZE as u64;
            if unsafe { vm::map_page(pml4, phys, virt, flags) } != 0 {
                return Err("Failed to map ELF page");
            }
        }

        if first_code_phys == 0 && (phdr.p_flags & 1) != 0 { first_code_phys = pages_phys; }
        if vaddr + memsz > max_vaddr { max_vaddr = vaddr + memsz; }
    }

    // Allocate user stack (64KB) — randomized location (ASLR)
    let stack_pages = 16;
    let stack_phys = palloc_n(stack_pages);
    if stack_phys == 0 { return Err("Failed to allocate stack"); }

    for i in 0..stack_pages {
        memory::memset_phys(stack_phys + (i as u64) * PAGE_SIZE as u64, 0, PAGE_SIZE);
    }

    let stack_top = randomize_stack_top();
    let stack_virt_bottom = stack_top - (stack_pages as u64 * PAGE_SIZE as u64);
    for i in 0..stack_pages {
        let phys = stack_phys + (i as u64) * PAGE_SIZE as u64;
        let virt = stack_virt_bottom + (i as u64) * PAGE_SIZE as u64;
        if unsafe { vm::map_page(pml4, phys, virt, vm::PTE_WRITABLE | vm::PTE_USER) } != 0 {
            return Err("Failed to map stack page");
        }
    }

    // Interpreter path
    let mut interp_vaddr = 0u64;
    let mut interp_size = 0u64;
    if needs_interp && interp_filesz > 0 && interp_filesz < 256 {
        let start = interp_offset as usize;
        let end = start + interp_filesz as usize;
        if end <= elf_data.len() {
            let interp_page = palloc();
            if interp_page != 0 {
                memory::memset_phys(interp_page, 0, PAGE_SIZE);
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        elf_data[start..].as_ptr(), interp_page as *mut u8, interp_filesz as usize);
                }
                interp_vaddr = 0x0000_7FFF_0000;
                if unsafe { vm::map_page(pml4, interp_page, interp_vaddr, vm::PTE_USER | vm::PTE_WRITABLE) } == 0 {
                    interp_size = interp_filesz;
                }
            }
        }
    }

    let brk = (max_vaddr + 0xFFF) & !0xFFF;

    // Store program headers for auxv
    let phdr_count = hdr.e_phnum as usize;
    let phdr_size = phdr_count * 56;
    let mut phdr_vaddr = 0u64;
    let phdr_page = palloc();
    if phdr_page != 0 {
        memory::memset_phys(phdr_page, 0, PAGE_SIZE);
        unsafe {
            core::ptr::copy_nonoverlapping(
                elf_data[hdr.e_phoff as usize..].as_ptr(),
                phdr_page as *mut u8,
                phdr_size.min(PAGE_SIZE),
            );
        }
        phdr_vaddr = 0x0000_7FFE_0000;
        unsafe { vm::map_page(pml4, phdr_page, phdr_vaddr, vm::PTE_USER | vm::PTE_WRITABLE); }
    }

    Ok(LinuxProgram {
        entry: hdr.e_entry,
        pml4,
        code_phys: first_code_phys,
        stack_phys,
        brk,
        mmap_base: randomize_mmap_base(),
        phdr_vaddr,
        interp_vaddr,
        interp_size,
        stack_size: stack_pages as u64 * PAGE_SIZE as u64,
        stack_top,
    })
}
