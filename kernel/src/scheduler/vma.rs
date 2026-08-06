// VMA (virtual memory area) registry for ring-3 tasks.
//
// A task can declare which user ranges are "valid but not yet backed".
// The page-fault handler consults this table: a fault inside a VMA is
// serviced by allocating a zeroed physical page and mapping it, then the
// faulting instruction is retried (demand paging / lazy stack growth).

use super::{TaskState, TASKS};

/// VMA kinds
pub const VMA_NONE: u8 = 0;
pub const VMA_CODE: u8 = 1;
pub const VMA_STACK: u8 = 2;
pub const VMA_HEAP: u8 = 3;
pub const VMA_DATA: u8 = 4;

/// Maximum number of VMAs a single task may register.
pub const MAX_VMAS: usize = 8;

#[derive(Clone, Copy)]
pub struct Vma {
    pub start: u64,
    pub end: u64,
    pub kind: u8,
    /// PTE flags applied when the fault handler lazily maps a page.
    pub flags: u64,
}

impl Vma {
    pub const fn empty() -> Self {
        Vma { start: 0, end: 0, kind: VMA_NONE, flags: 0 }
    }
}

/// Register a VMA for a task slot. Returns true on success.
pub fn add(slot: usize, start: u64, end: u64, kind: u8, flags: u64) -> bool {
    unsafe {
        if slot >= super::MAX_TASKS || TASKS[slot].state == TaskState::Free { return false; }
        if start >= end { return false; }
        let count = TASKS[slot].vma_count as usize;
        if count >= MAX_VMAS { return false; }
        TASKS[slot].vmas[count] = Vma { start, end, kind, flags };
        TASKS[slot].vma_count = (count + 1) as u8;
        true
    }
}

/// Find the VMA containing an address for a task slot.
pub fn find(slot: usize, addr: u64) -> Option<&'static Vma> {
    unsafe {
        if slot >= super::MAX_TASKS { return None; }
        let count = TASKS[slot].vma_count as usize;
        for i in 0..count {
            let v = &TASKS[slot].vmas[i];
            if addr >= v.start && addr < v.end { return Some(v); }
        }
        None
    }
}
