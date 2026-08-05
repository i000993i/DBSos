// Virtual memory: 4-level paging (x86-64)
// Page table format: PML4 → PDPT → PD → PT → 4KB page

use crate::memory::{palloc, PAGE_SIZE};

pub const PAGE_MASK: u64 = 0xFFFF_FFFF_F000; // 4KB-aligned physical address mask

// Page table entry flags
pub const PTE_PRESENT: u64   = 1 << 0;
pub const PTE_WRITABLE: u64  = 1 << 1;
pub const PTE_USER: u64      = 1 << 2;
pub const PTE_WRITE_THROUGH: u64 = 1 << 3;
pub const PTE_CACHE_DISABLE: u64 = 1 << 4;
pub const PTE_ACCESSED: u64  = 1 << 5;
pub const PTE_DIRTY: u64     = 1 << 6;
pub const PTE_HUGE: u64      = 1 << 7;
pub const PTE_GLOBAL: u64    = 1 << 8;
pub const PTE_NX: u64        = 1 << 63;

fn phys_from_pte(entry: u64) -> u64 {
    entry & PAGE_MASK
}

fn pte_from_phys(phys: u64) -> u64 {
    phys & PAGE_MASK
}

unsafe fn pte_addr(table: *mut u64, index: usize) -> *mut u64 {
    table.add(index)
}

unsafe fn get_pte(table: *mut u64, index: usize) -> u64 {
    *pte_addr(table, index)
}

unsafe fn set_pte(table: *mut u64, index: usize, value: u64) {
    *pte_addr(table, index) = value;
}

unsafe fn table_from_phys(phys: u64) -> *mut u64 {
    phys as *mut u64
}

fn pml4_index(virt: u64) -> usize {
    ((virt >> 39) & 0x1FF) as usize
}

fn pdpt_index(virt: u64) -> usize {
    ((virt >> 30) & 0x1FF) as usize
}

fn pd_index(virt: u64) -> usize {
    ((virt >> 21) & 0x1FF) as usize
}

fn pt_index(virt: u64) -> usize {
    ((virt >> 12) & 0x1FF) as usize
}

// Walk page tables: find the PTE for a virtual address, creating intermediate tables if needed.
// Returns (PTE phys addr, PTE value, entry_type) where entry_type:
//   0 = leaf page (4KB), 1 = huge page (2MB), 2 = not mapped, 3 = error
unsafe fn walk_or_create(pml4_ptr: *mut u64, virt: u64, create: bool) -> (u64, u64) {
    let i0 = pml4_index(virt);
    let i1 = pdpt_index(virt);
    let i2 = pd_index(virt);
    let i3 = pt_index(virt);

    // PML4 → PDPT
    let e0 = get_pte(pml4_ptr, i0);
    if e0 & PTE_PRESENT == 0 {
        if !create { return (0, 0); }
        let page = palloc();
        if page == 0 { return (0, 0); }
        core::ptr::write_bytes(page as *mut u64, 0, PAGE_SIZE as usize / 8);
        set_pte(pml4_ptr, i0, page | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
    }

    let phys0 = phys_from_pte(get_pte(pml4_ptr, i0));
    let pdpt_ptr = table_from_phys(phys0);

    // PDPT → PD
    let e1 = get_pte(pdpt_ptr, i1);
    if e1 & PTE_PRESENT == 0 {
        if !create { return (0, 0); }
        let page = palloc();
        if page == 0 { return (0, 0); }
        core::ptr::write_bytes(page as *mut u64, 0, PAGE_SIZE as usize / 8);
        set_pte(pdpt_ptr, i1, page | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
    }
    if e1 & PTE_HUGE != 0 {
        return (0, 0);
    }

    let phys1 = phys_from_pte(get_pte(pdpt_ptr, i1));
    let pd_ptr = table_from_phys(phys1);

    // PD → PT (handle 2MB Huge → PT split)
    let e2 = get_pte(pd_ptr, i2);
    if e2 & PTE_HUGE != 0 {
        if !create {
            let pte_addr = pd_ptr.add(i2) as u64;
            return (pte_addr, e2);
        }
        // Split 2MB huge page into 512 4KB PT entries
        let pt_page = palloc();
        if pt_page == 0 { return (0, 0); }
        let pt_new = table_from_phys(pt_page);
        core::ptr::write_bytes(pt_new, 0, 512);
        let huge_base = phys_from_pte(e2);
        let huge_flags = e2 & !(PAGE_MASK | PTE_HUGE | PTE_ACCESSED | PTE_DIRTY);
        for i in 0..512 {
            set_pte(pt_new, i, (huge_base + (i as u64) * 4096) | huge_flags);
        }
        set_pte(pd_ptr, i2, pt_page | PTE_PRESENT | PTE_WRITABLE | PTE_USER
            | (e2 & PTE_GLOBAL));
        // Fall through to read PT entry from the new PT page
    } else if e2 & PTE_PRESENT == 0 {
        if !create { return (0, 0); }
        let page = palloc();
        if page == 0 { return (0, 0); }
        core::ptr::write_bytes(page as *mut u64, 0, PAGE_SIZE as usize / 8);
        set_pte(pd_ptr, i2, page | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
    }

    let phys2 = phys_from_pte(get_pte(pd_ptr, i2));
    let pt_ptr = table_from_phys(phys2);

    // PT → Page
    let pte_addr = pt_ptr.add(i3) as u64;
    let pte_val = get_pte(pt_ptr, i3);
    (pte_addr, pte_val)
}

/// Map a 4KB page (phys → virt) in the given address space.
/// walk_or_create handles 2MB huge page → 4KB PT split when needed.
pub unsafe fn map_page(pml4: *mut u64, phys: u64, virt: u64, flags: u64) -> u64 {
    let (pte_addr, _existing) = walk_or_create(pml4, virt, true);
    if pte_addr == 0 { return !0; }
    let entry = pte_from_phys(phys) | PTE_PRESENT | flags;
    *(pte_addr as *mut u64) = entry;
    core::arch::asm!("invlpg [{}]", in(reg) virt);
    0
}

/// Unmap a virtual address.
pub unsafe fn unmap_page(pml4: *mut u64, virt: u64) {
    let (pte_addr, existing) = walk_or_create(pml4, virt, false);
    if pte_addr == 0 || existing & PTE_PRESENT == 0 { return; }
    *(pte_addr as *mut u64) = 0;
    core::arch::asm!("invlpg [{}]", in(reg) virt);
}

/// Lookup physical address for a virtual address.
pub unsafe fn virt_to_phys(pml4: *mut u64, virt: u64) -> u64 {
    let (pte_addr, pte) = walk_or_create(pml4, virt, false);
    if pte_addr == 0 || pte & PTE_PRESENT == 0 { return 0; }

    if pte & PTE_HUGE != 0 {
        // 2MB page
        let base = phys_from_pte(pte);
        let offset = virt & 0x1FFFFF;
        return base | offset;
    } else {
        // 4KB page
        let base = phys_from_pte(pte);
        let offset = virt & 0xFFF;
        return base | offset;
    }
}

/// Create a new address space with identity map for kernel.
pub unsafe fn create_address_space() -> *mut u64 {
    let phys = palloc();
    if phys == 0 { return core::ptr::null_mut(); }
    let pml4 = table_from_phys(phys);

    // Zero out the new PML4
    core::ptr::write_bytes(pml4, 0, PAGE_SIZE as usize / 8);
    pml4
}

/// Switch to a new address space (load CR3).
pub unsafe fn switch_to(pml4_ptr: *mut u64) {
    let phys = pml4_ptr as u64;
    core::arch::asm!("mov cr3, {}", in(reg) phys);
}

/// Get current PML4 physical address.
pub unsafe fn current_pml4() -> u64 {
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3);
    cr3
}

/// Clone kernel mappings from one address space to another.
/// Assumes both are identity-mapped for low 4GB or share a page table skeleton.
pub unsafe fn clone_kernel_mappings(src_pml4: *mut u64, dst_pml4: *mut u64) {
    // Copy PML4 entries for high half (kernel space)
    // User entries are indices 0-255, kernel entries 256-511
    for i in 256..512 {
        let src_pte = get_pte(src_pml4, i);
        if src_pte & PTE_PRESENT != 0 {
            set_pte(dst_pml4, i, src_pte);
        }
    }
}

/// Copy high-half PML4 entries (256-511) from source to destination.
pub unsafe fn clone_high_half(src: *mut u64, dst: *mut u64) {
    for i in 256..512 {
        let v = get_pte(src, i);
        if v & PTE_PRESENT != 0 { set_pte(dst, i, v); }
    }
}

/// Identity map a physical region using 2MB huge pages, creating new page table pages.
/// No PTE_USER — kernel-only access.
pub unsafe fn identity_map_2mb(pml4: *mut u64, start: u64, end: u64, extra_flags: u64) -> bool {
    let i0 = pml4_index(start);
    // Assumes start & end are within the same PML4 entry, which is true for ranges < 512GB.
    let mut addr = start & !(0x1FFFFFu64); // align to 2MB
    while addr < end {
        // Ensure PML4 entry exists
        let e0 = get_pte(pml4, i0);
        let pdpt_ptr = if e0 & PTE_PRESENT == 0 {
            let page = palloc(); if page == 0 { return false; }
            core::ptr::write_bytes(page as *mut u64, 0, 512);
            set_pte(pml4, i0, page | PTE_PRESENT | PTE_WRITABLE);
            table_from_phys(page)
        } else {
            table_from_phys(phys_from_pte(e0))
        };

        // Ensure PDPT entry exists (non-huge)
        let i1 = pdpt_index(addr);
        let e1 = get_pte(pdpt_ptr, i1);
        let pd_ptr = if e1 & PTE_PRESENT == 0 {
            let page = palloc(); if page == 0 { return false; }
            core::ptr::write_bytes(page as *mut u64, 0, 512);
            set_pte(pdpt_ptr, i1, page | PTE_PRESENT | PTE_WRITABLE);
            table_from_phys(page)
        } else {
            if e1 & PTE_HUGE != 0 { addr += 0x40000000; continue; } // skip 1GB page range
            table_from_phys(phys_from_pte(e1))
        };

        // Set PD entry as 2MB huge page
        let i2 = pd_index(addr);
        let entry = (addr & 0xFFFFFFE00000) | PTE_PRESENT | PTE_HUGE | extra_flags;
        set_pte(pd_ptr, i2, entry);
        addr += 0x200000;
    }
    true
}

/// Free all page table pages and mapped data pages in the low half (indices 0-255)
/// of a user address space.
pub unsafe fn destroy_address_space(pml4: *mut u64) {
    use crate::memory::pfree;
    for i in 0..256 {
        let e0 = get_pte(pml4, i);
        if e0 & PTE_PRESENT == 0 { continue; }
        let pdpt = table_from_phys(phys_from_pte(e0));
        for j in 0..512 {
            let e1 = get_pte(pdpt, j);
            if e1 & PTE_PRESENT == 0 { continue; }
            if e1 & PTE_HUGE != 0 { pfree(phys_from_pte(e1)); continue; }
            let pd = table_from_phys(phys_from_pte(e1));
            for k in 0..512 {
                let e2 = get_pte(pd, k);
                if e2 & PTE_PRESENT == 0 { continue; }
                if e2 & PTE_HUGE != 0 { pfree(phys_from_pte(e2)); continue; }
                let pt = table_from_phys(phys_from_pte(e2));
                for l in 0..512 {
                    let e3 = get_pte(pt, l);
                    if e3 & PTE_PRESENT != 0 {
                        pfree(phys_from_pte(e3));
                    }
                }
                pfree(phys_from_pte(e2));
            }
            pfree(phys_from_pte(e1));
        }
        pfree(phys_from_pte(e0));
    }
    pfree(pml4 as u64);
}

/// Saved kernel PML4 physical address — restored when exiting a ring-3 task.
pub static mut KERNEL_PML4: u64 = 0;

/// Initialize virtual memory: set up kernel's address space.
pub unsafe fn init() {
    let phys = current_pml4();
    KERNEL_PML4 = phys;
    uart_print("[VM] CR3=");
    uart_hex(phys);
    uart_print("\r\n");

    // Ensure kernel can access high-half mappings later
    // For now, we work with identity-mapped page tables (as set up by firmware)
}

fn uart_print(s: &str) {
    crate::driver::uart::write_str(s);
}

fn uart_hex(mut val: u64) {
    if val == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut buf = [0u8; 16];
    let mut i = 0;
    while val > 0 {
        let nib = (val & 0xF) as u8;
        buf[i] = if nib < 10 { b'0' + nib } else { b'A' + nib - 10 };
        val >>= 4;
        i += 1;
    }
    while i > 0 { i -= 1; crate::driver::uart::putchar(buf[i]); }
}
