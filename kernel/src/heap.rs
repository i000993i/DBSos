// Kernel heap allocator (slab + free-list)
//
// Allocations come from a contiguous virtual region backed by physical pages
// obtained from the physical page allocator. The region starts at HEAP_BASE
// and can grow up to HEAP_MAX by calling into the physical allocator.
//
// Strategy: first-fit free-list with 16-byte alignment. Each block has a
// 16-byte header: { size: u32, used: u16, _pad: u16 }. Payload follows
// immediately after the header. Free blocks contain a next/prev pointer pair
// in the payload area. Allocations <= 4096 go through the free-list; larger
// allocations are whole-page runs from the physical allocator.



const PAGE_SIZE: u64 = 4096;
const HEAP_BASE: u64 = 0x0000_1000_0000_0000; // 4 TiB — high canonical, no conflict with kernel
const HEAP_MAX_SIZE: u64 = 256 * 1024 * 1024; // 256 MiB soft limit

// Minimum block payload size (must hold two u64 pointers when free)
const MIN_ALIGN: usize = 16;
const MIN_BLOCK: usize = 32; // 16 header + 16 payload minimum

// Block header (16 bytes, at the start of each allocation or free block)
#[repr(C)]
struct BlockHeader {
    size: u32,   // payload size (excluding header)
    used: u16,   // 0 = free, 1 = used
    _pad: u16,
}

// Free-list node (stored in the payload area of free blocks)
#[repr(C)]
struct FreeNode {
    next: *mut FreeNode,
    prev: *mut FreeNode,
}

static mut HEAP_READY: bool = false;
static mut HEAP_START: u64 = 0;
static mut HEAP_END: u64 = 0;
static mut FREE_LIST: *mut FreeNode = core::ptr::null_mut();

fn uart_print(s: &str) { crate::driver::uart::write_str(s); }
fn uart_hex(mut v: u64) {
    if v == 0 { uart_print("0"); return; }
    let h = b"0123456789ABCDEF";
    let mut buf = [0u8; 16]; let mut i = 0;
    while v > 0 { buf[i] = h[(v & 0xF) as usize]; v >>= 4; i += 1; }
    while i > 0 { i -= 1; uart_print(core::str::from_utf8(&buf[i..i+1]).unwrap_or("?")); }
}
fn uart_dec(mut v: u64) {
    if v == 0 { uart_print("0"); return; }
    let mut buf = [0u8; 20]; let mut i = 0;
    while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart_print(core::str::from_utf8(&buf[i..i+1]).unwrap_or("?")); }
}

/// Round up to MIN_ALIGN
fn align_up(n: usize) -> usize {
    (n + MIN_ALIGN - 1) & !(MIN_ALIGN - 1)
}

/// Total block size (header + aligned payload)
fn block_total(size: usize) -> usize {
    align_up(size) + core::mem::size_of::<BlockHeader>()
}

/// Pointer to the header from a payload pointer
unsafe fn payload_to_header(ptr: *mut u8) -> *mut BlockHeader {
    unsafe { (ptr as *mut BlockHeader).sub(1) }
}

/// Pointer to the payload from a header
unsafe fn header_to_payload(hdr: *mut BlockHeader) -> *mut u8 {
    unsafe { (hdr.add(1)) as *mut u8 }
}

/// Pointer to the next block (adjacent in memory)
unsafe fn block_next(hdr: *mut BlockHeader) -> *mut BlockHeader {
    let total = block_total((*hdr).size as usize);
    unsafe { (hdr as *mut u8).add(total) as *mut BlockHeader }
}

/// Remove a node from the free list
unsafe fn free_list_remove(node: *mut FreeNode) {
    unsafe {
        if node.is_null() { return; }
        let n = &mut *node;
        if !n.prev.is_null() {
            (*n.prev).next = n.next;
        } else {
            FREE_LIST = n.next;
        }
        if !n.next.is_null() {
            (*n.next).prev = n.prev;
        }
    }
}

/// Insert a node at the head of the free list
unsafe fn free_list_insert(node: *mut FreeNode) {
    unsafe {
        let n = &mut *node;
        n.prev = core::ptr::null_mut();
        n.next = FREE_LIST;
        if !FREE_LIST.is_null() {
            (*FREE_LIST).prev = node;
        }
        FREE_LIST = node;
    }
}

/// Grow the heap by at least `min_pages` pages. Maps them in the heap region.
unsafe fn grow_heap(min_pages: usize) -> bool {
    unsafe {
        let current_size = HEAP_END - HEAP_BASE;
        let mut pages = min_pages;
        // Grow by at least 1 page or 25% of current size, whichever is larger
        let quarter = (current_size / PAGE_SIZE as u64 / 4) as usize;
        if quarter > pages { pages = quarter; }
        if pages < 4 { pages = 4; } // minimum 16 KiB growth

        if current_size + (pages as u64) * PAGE_SIZE > HEAP_MAX_SIZE {
            pages = ((HEAP_MAX_SIZE - current_size) / PAGE_SIZE as u64) as usize;
            if pages == 0 { return false; }
        }

        let mut allocated = 0u64;
        for _ in 0..pages {
            let phys = crate::memory::palloc();
            if phys == 0 { break; }
            // Map the page into the heap virtual region
            let ret = crate::vm::map_page(
                crate::vm::KERNEL_PML4 as *mut u64,
                phys, HEAP_END + allocated,
                crate::vm::PTE_WRITABLE | crate::vm::PTE_GLOBAL,
            );
            if ret != 0 {
                // map_page failed — free the physical page, don't count it
                crate::memory::pfree(phys);
                break;
            }
            allocated += 1;
        }

        if allocated == 0 { return false; }

        let new_end = HEAP_END + allocated * PAGE_SIZE;
        let grow_start = HEAP_END;

        // Initialize all new memory as one large free block
        let offset = grow_start;
        // Один большой свободный блок на всю выросшую область (тело всегда
        // завершалось break — while здесь был эквивалентен if).
        if offset + block_total(0) as u64 <= new_end {
            let hdr = offset as *mut BlockHeader;
            let payload_size = (new_end - offset - core::mem::size_of::<BlockHeader>() as u64) as usize;
            if payload_size >= MIN_BLOCK {
                (*hdr).size = payload_size as u32;
                (*hdr).used = 0;
                (*hdr)._pad = 0;
                let node = header_to_payload(hdr) as *mut FreeNode;
                free_list_insert(node);
            }
        }

        HEAP_END = new_end;
        true
    }
}

/// Initialize the heap. Called once during boot after the physical allocator is ready.
pub unsafe fn init(heap_pages: usize) {
    unsafe {
        if heap_pages == 0 { return; }

        let mut base = HEAP_BASE;
        for _ in 0..heap_pages {
            let phys = crate::memory::palloc();
            if phys == 0 {
                uart_print("[HEAP] palloc failed during init\r\n");
                break;
            }
            crate::vm::map_page(
                crate::vm::KERNEL_PML4 as *mut u64,
                phys, base,
                crate::vm::PTE_WRITABLE | crate::vm::PTE_GLOBAL,
            );
            base += PAGE_SIZE;
        }

        HEAP_START = HEAP_BASE;
        HEAP_END = base;
        FREE_LIST = core::ptr::null_mut();

        // Create one big free block covering the entire initial heap
        let total = (HEAP_END - HEAP_START) as usize;
        let payload = total - core::mem::size_of::<BlockHeader>();
        let hdr = HEAP_START as *mut BlockHeader;
        (*hdr).size = payload as u32;
        (*hdr).used = 0;
        (*hdr)._pad = 0;
        let node = header_to_payload(hdr) as *mut FreeNode;
        free_list_insert(node);

        HEAP_READY = true;

        uart_print("[HEAP] init: ");
        uart_dec((HEAP_END - HEAP_START) / 1024);
        uart_print(" KiB at 0x");
        uart_hex(HEAP_START);
        uart_print("\r\n");
    }
}

#[inline]
unsafe fn heap_irq_save() -> u64 { let f: u64; core::arch::asm!("pushfq; pop {}", out(reg) f); core::arch::asm!("cli"); f }
#[inline]
unsafe fn heap_irq_restore(f: u64) { if f & (1 << 9) != 0 { core::arch::asm!("sti"); } }

/// Allocate `size` bytes from the kernel heap. Returns a pointer or null.
/// Result is aligned to 16 bytes.
pub unsafe fn kmalloc(size: usize) -> *mut u8 {
    if size == 0 { return core::ptr::null_mut(); }
    if !HEAP_READY { return core::ptr::null_mut(); }
    let flags = heap_irq_save();

    let total = block_total(size);

    unsafe {
        // First-fit search
        let mut node = FREE_LIST;
        while !node.is_null() {
            let hdr = payload_to_header(node as *mut u8);
            let blk_total = block_total((*hdr).size as usize);
            if blk_total >= total {
                // Found a block large enough
                free_list_remove(node);

                let remaining = blk_total - total;
                if remaining >= block_total(0) + MIN_BLOCK {
                    // Split: carve off the tail as a new free block
                    let new_hdr = (hdr as *mut u8).add(total) as *mut BlockHeader;
                    (*new_hdr).size = (remaining - core::mem::size_of::<BlockHeader>()) as u32;
                    (*new_hdr).used = 0;
                    (*new_hdr)._pad = 0;
                    (*hdr).size = size as u32;

                    let new_node = header_to_payload(new_hdr) as *mut FreeNode;
                    free_list_insert(new_node);
                }

                (*hdr).used = 1;
                let out = header_to_payload(hdr);
                heap_irq_restore(flags);
                return out;
            }
            node = (*node).next;
        }

        // No suitable block — grow the heap
        let pages_needed = (total + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;
        if !grow_heap(pages_needed) {
            heap_irq_restore(flags);
            return core::ptr::null_mut(); // OOM
        }

        // Retry allocation after growth — avoid recursion to keep irq flags correct
        heap_irq_restore(flags);
        return kmalloc(size);
    }
    // Unreachable, but restore for safety
    #[allow(unreachable_code)]
    { heap_irq_restore(flags); core::ptr::null_mut() }
}

/// Allocate zeroed memory. Same as kmalloc but memory is zero-filled.
pub unsafe fn kzalloc(size: usize) -> *mut u8 {
    let ptr = unsafe { kmalloc(size) };
    if !ptr.is_null() {
        unsafe { core::ptr::write_bytes(ptr, 0, size); }
    }
    ptr
}

/// Free a previous kmalloc/kzalloc allocation. Safe to call with null or double-free.
pub unsafe fn kfree(ptr: *mut u8) {
    if ptr.is_null() { return; }
    if !HEAP_READY { return; }
    let flags = heap_irq_save();
    unsafe {
        let hdr = payload_to_header(ptr);

        // Sanity check
        if (*hdr).used == 0 { heap_irq_restore(flags); return; } // double free — ignore
        if (hdr as u64) < HEAP_START || (hdr as u64) >= HEAP_END { heap_irq_restore(flags); return; }

        (*hdr).used = 0;
        let node = ptr as *mut FreeNode;
        free_list_insert(node);

        // Coalesce with next block if free — preserve alignment
        let next = block_next(hdr);
        if (next as u64) < HEAP_END && (*next).used == 0 {
            let next_node = header_to_payload(next) as *mut FreeNode;
            free_list_remove(next_node);
            let cur_total = block_total((*hdr).size as usize);
            let next_total = block_total((*next).size as usize);
            let combined = cur_total + next_total;
            (*hdr).size = (combined - core::mem::size_of::<BlockHeader>()) as u32;
        }
        // Coalesce with previous block (scan free list for block ending at hdr)
        let mut prev_hdr: *mut BlockHeader = core::ptr::null_mut();
        let mut n = FREE_LIST;
        while !n.is_null() {
            let h = payload_to_header(n as *mut u8);
            if block_next(h) as *mut u8 == hdr as *mut u8 && (*h).used == 0 && h != hdr {
                prev_hdr = h;
                break;
            }
            n = (*n).next;
        }
        if !prev_hdr.is_null() {
            let prev_node = header_to_payload(prev_hdr) as *mut FreeNode;
            free_list_remove(node);
            free_list_remove(prev_node);
            let prev_total = block_total((*prev_hdr).size as usize);
            let cur_total = block_total((*hdr).size as usize);
            (*prev_hdr).size = (prev_total + cur_total - core::mem::size_of::<BlockHeader>()) as u32;
            free_list_insert(header_to_payload(prev_hdr) as *mut FreeNode);
        }
    }
    heap_irq_restore(flags);
}

/// Query heap statistics (free bytes / total bytes).
pub fn heap_stats() -> (usize, usize) {
    unsafe {
        if !HEAP_READY { return (0, 0); }
        let total = (HEAP_END - HEAP_START) as usize;
        let mut free_bytes = 0usize;
        let mut node = FREE_LIST;
        while !node.is_null() {
            let hdr = payload_to_header(node as *mut u8);
            free_bytes += (*hdr).size as usize + core::mem::size_of::<BlockHeader>();
            node = (*node).next;
        }
        (free_bytes, total)
    }
}
