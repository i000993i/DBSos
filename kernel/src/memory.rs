// Физический page allocator (bitmap) через UEFI memory map

use uefi::boot;
use uefi::mem::memory_map::{MemoryType, MemoryMap};
use core::ptr;

pub const PAGE_SIZE: usize = 4096;

static mut ALLOC: PhysAllocator = PhysAllocator {
    bitmap_addr: ptr::null_mut(),
    phys_start: 0,
    total_pages: 0,
    free_pages: 0,
    first_page: 0,
};

struct PhysAllocator {
    bitmap_addr: *mut u8,
    phys_start: u64,
    total_pages: usize,
    free_pages: usize,
    first_page: usize,
}

pub fn init() {
    let mmap = match boot::memory_map(MemoryType::LOADER_DATA) {
        Ok(m) => m,
        Err(_) => return,
    };

    let mut best_start = 0u64;
    let mut best_pages = 0u64;

    for entry in mmap.entries() {
        if entry.ty == MemoryType::CONVENTIONAL && entry.page_count > best_pages {
            best_pages = entry.page_count;
            best_start = entry.phys_start;
        }
    }

    if best_pages == 0 || best_start == 0 {
        return;
    }

    let bitmap_bytes = ((best_pages as usize + 7) / 8) as u64;
    let bitmap_pages = ((bitmap_bytes + (PAGE_SIZE as u64) - 1) / (PAGE_SIZE as u64)) as u64;

    let bitmap_phys = best_start;
    let region_start = best_start + bitmap_pages * (PAGE_SIZE as u64);
    let region_pages = best_pages - bitmap_pages;

    unsafe {
        ALLOC = PhysAllocator {
            bitmap_addr: bitmap_phys as *mut u8,
            phys_start: region_start,
            total_pages: region_pages as usize,
            free_pages: region_pages as usize,
            first_page: 0,
        };
        ptr::write_bytes(ALLOC.bitmap_addr, 0, bitmap_bytes as usize);
    }

    uart_print("[MEM] phys allocator: ");
    uart_dec(best_pages * (PAGE_SIZE as u64) / (1024 * 1024));
    uart_print(" MB free, bitmap ");
    uart_dec(bitmap_bytes);
    uart_print(" bytes\r\n");
}

unsafe fn alloc_mut() -> &'static mut PhysAllocator {
    &mut *(&raw mut ALLOC)
}

/// Run `f` with interrupts disabled, then restore the previous IF state.
/// Unlike a bare `cli`/`sti` pair, this does NOT re-enable interrupts if the
/// caller already had them masked (which would break outer critical sections).
fn with_irqs_off<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let flags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) flags);
        core::arch::asm!("cli");
        let r = f();
        if flags & (1 << 9) != 0 {
            core::arch::asm!("sti");
        }
        r
    }
}

pub fn palloc() -> u64 {
    with_irqs_off(|| {
        unsafe {
        let a = alloc_mut();
        if a.free_pages == 0 || a.bitmap_addr.is_null() {
            return 0;
        }
        for i in a.first_page..a.total_pages {
            if bit_test(a.bitmap_addr, i) == 0 {
                bit_set(a.bitmap_addr, i, 1);
                a.free_pages -= 1;
                a.first_page = i + 1;
                return a.phys_start + (i as u64) * (PAGE_SIZE as u64);
            }
        }
        for i in 0..a.first_page {
            if bit_test(a.bitmap_addr, i) == 0 {
                bit_set(a.bitmap_addr, i, 1);
                a.free_pages -= 1;
                return a.phys_start + (i as u64) * (PAGE_SIZE as u64);
            }
        }
        0
        }
    })
}

pub fn palloc_n(n: usize) -> u64 {
    with_irqs_off(|| {
        unsafe {
        let a = alloc_mut();
        if a.free_pages < n || a.bitmap_addr.is_null() {
            return 0;
        }
        let mut run_start = a.first_page;
        let mut run_len = 0;
        for i in a.first_page..(a.total_pages + a.first_page) {
            let idx = if i >= a.total_pages { i - a.total_pages } else { i };
            if bit_test(a.bitmap_addr, idx) == 0 {
                if run_len == 0 { run_start = idx; }
                run_len += 1;
                if run_len >= n {
                    for j in run_start..(run_start + n) {
                        bit_set(a.bitmap_addr, j, 1);
                    }
                    a.free_pages -= n;
                    a.first_page = run_start + n;
                    return a.phys_start + (run_start as u64) * (PAGE_SIZE as u64);
                }
            } else {
                run_len = 0;
            }
        }
        0
        }
    })
}

pub fn pfree(phys: u64) {
    with_irqs_off(|| {
        unsafe {
        let a = alloc_mut();
        let offset = match phys.checked_sub(a.phys_start) {
            Some(o) => o,
            None => { return; }
        };
        let page = (offset / (PAGE_SIZE as u64)) as usize;
        if page >= a.total_pages || a.bitmap_addr.is_null() {
            return;
        }
        if bit_test(a.bitmap_addr, page) == 0 {
            return;
        }
        bit_set(a.bitmap_addr, page, 0);
        a.free_pages -= 1;
        if page < a.first_page {
            a.first_page = page;
        }
        }
    })
}

pub fn pfree_n(phys: u64, n: usize) {
    for i in 0..n {
        pfree(phys + (i as u64) * (PAGE_SIZE as u64));
    }
}

pub fn free_count() -> usize {
    unsafe { (*&raw const ALLOC).free_pages }
}

fn bit_test(bitmap: *const u8, bit: usize) -> u8 {
    unsafe { (*bitmap.add(bit >> 3) >> (bit & 7)) & 1 }
}

fn bit_set(bitmap: *mut u8, bit: usize, val: u8) {
    unsafe {
        let p = bitmap.add(bit >> 3);
        if val != 0 {
            *p |= 1 << (bit & 7);
        } else {
            *p &= !(1 << (bit & 7));
        }
    }
}

fn uart_print(s: &str) {
    crate::driver::uart::write_str(s);
}

fn uart_dec(mut val: u64) {
    if val == 0 {
        crate::driver::uart::putchar(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = 0;
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        crate::driver::uart::putchar(buf[i]);
    }
}
