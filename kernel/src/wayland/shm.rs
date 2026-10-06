//! wl_shm — shared memory buffers for Wayland
//!
//! Buffers are allocated via `memory::palloc` and shared via capabilities.
//! Format: WL_SHM_FORMAT_XRGB8888 (0) / ARGB8888 (1)

use crate::memory::PAGE_SIZE;
use crate::driver::uart;

pub const WL_SHM_FORMAT_XRGB8888: u32 = 0;
pub const WL_SHM_FORMAT_ARGB8888: u32 = 1;

#[derive(Clone, Copy)]
pub struct ShmPool {
    pub phys: u64,
    pub size: usize,
    pub in_use: bool,
}
const MAX_POOLS: usize = 8;
static mut POOLS: [ShmPool; MAX_POOLS] = [ShmPool{phys:0,size:0,in_use:false}; MAX_POOLS];

pub fn init() {
    uart::write_str("[WAYLAND-SHM] init\r\n");
}

/// Create a shm pool of `size` bytes (rounded to pages). Returns pool index or None.
pub fn pool_create(size: usize) -> Option<usize> {
    let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
    if pages==0 { return None; }
    let phys = crate::memory::palloc_n(pages);
    if phys==0 { return None; }
    // map into kernel for compositor access
    unsafe {
        crate::vm::identity_map_2mb(crate::vm::KERNEL_PML4 as *mut u64, phys, phys + (pages*PAGE_SIZE) as u64, crate::vm::PTE_WRITABLE);
    }
    let slot = unsafe{ (0..MAX_POOLS).find(|&i| !POOLS[i].in_use)? };
    unsafe{
        POOLS[slot]=ShmPool{phys, size: pages*PAGE_SIZE, in_use:true};
    }
    Some(slot)
}
pub fn pool_destroy(idx: usize) -> bool {
    if idx>=MAX_POOLS { return false; }
    unsafe{
        if !POOLS[idx].in_use { return false; }
        let p=POOLS[idx];
        crate::memory::pfree_n(p.phys, p.size / PAGE_SIZE);
        POOLS[idx].in_use=false;
        return true;
    }
}
pub fn pool_phys(idx: usize) -> Option<u64> {
    if idx>=MAX_POOLS { return None; }
    unsafe{ if POOLS[idx].in_use { Some(POOLS[idx].phys) } else { None } }
}
