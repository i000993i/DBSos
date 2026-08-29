/// Linux memory management — tracks virtual memory areas for Linux processes.
///
/// Provides mmap/brk tracking and demand paging support.

use crate::memory::{palloc, PAGE_SIZE};
use crate::vm;

pub const MAX_VMAS: usize = 32;

#[derive(Clone, Copy, PartialEq)]
pub enum VmaKind {
    None,
    Code,
    Data,
    Heap,
    Stack,
    Mmap,
    Vdso,
    Vsyscall,
}

#[derive(Clone, Copy)]
pub struct Vma {
    pub start: u64,
    pub end: u64,
    pub kind: VmaKind,
    pub flags: u64,  // PTE flags (u64 to match vm module)
    pub mapped: bool,
}

impl Vma {
    pub const fn empty() -> Self {
        Self { start: 0, end: 0, kind: VmaKind::None, flags: 0, mapped: false }
    }
}

pub struct LinuxMM {
    pub vmas: [Vma; MAX_VMAS],
    pub vma_count: usize,
    pub brk: u64,
    pub mmap_base: u64,
    pub stack_bottom: u64,
    pub stack_top: u64,
}

impl LinuxMM {
    pub const fn empty() -> Self {
        Self {
            vmas: {
                const INIT: Vma = Vma::empty();
                [INIT; MAX_VMAS]
            },
            vma_count: 0,
            brk: 0,
            mmap_base: 0,
            stack_bottom: 0,
            stack_top: 0,
        }
    }

    pub fn add_vma(&mut self, start: u64, end: u64, kind: VmaKind, flags: u64) -> bool {
        if self.vma_count >= MAX_VMAS { return false; }
        self.vmas[self.vma_count] = Vma { start, end, kind, flags, mapped: false };
        self.vma_count += 1;
        true
    }

    pub fn find_vma(&self, addr: u64) -> Option<&Vma> {
        for i in 0..self.vma_count {
            if addr >= self.vmas[i].start && addr < self.vmas[i].end {
                return Some(&self.vmas[i]);
            }
        }
        None
    }

    pub fn handle_page_fault(&mut self, pml4: *mut u64, addr: u64, write: bool) -> bool {
        // Find the VMA for this address
        if let Some(vma) = self.find_vma(addr).cloned() {
            // Allocate and map a page
            let page = palloc();
            if page == 0 { return false; }
            crate::memory::memset_phys(page, 0, PAGE_SIZE);

            let page_addr = addr & !0xFFF;
            let mut flags = vma.flags | vm::PTE_USER;
            if write { flags |= vm::PTE_WRITABLE; }

            if unsafe { vm::map_page(pml4, page, page_addr, flags) } == 0 {
                return true;
            }
        }

        false
    }
}
