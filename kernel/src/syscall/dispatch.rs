// Syscall dispatch: syscall_rust_entry

use dbsos_abi::syscall::*;
use dbsos_abi::cap::*;
use dbsos_abi::ipc::Message;

const EXIT_MAGIC: u64 = !0u64;

#[no_mangle]
unsafe extern "C" fn syscall_rust_entry(num: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    match num {
        SYS_EXIT => {
            crate::driver::uart::write_str("[SYSCALL] exit\r\n");
            EXIT_MAGIC
        }

        SYS_IPC_SEND_LEGACY => {
            let dst_id = arg1;
            let val = arg2;
            crate::scheduler::ipc_send_u64(dst_id, val)
        }
        SYS_IPC_RECV_LEGACY => {
            let _src_id = arg1;
            crate::scheduler::ipc_recv_u64()
        }

        SYS_IPC_SEND => {
            let cap_idx = arg1 as u16;
            let msg_ptr = arg2 as *const Message;
            if crate::cap::validate(cap_idx, CAP_SEND) {
                crate::ipc::send_with_cap(cap_idx, &*msg_ptr) as u64
            } else {
                IPC_ERR_DENIED as u64
            }
        }
        SYS_IPC_RECV => {
            let cap_idx = arg1 as u16;
            let buf_ptr = arg2 as *mut Message;
            if crate::cap::validate(cap_idx, CAP_RECV) {
                crate::ipc::recv_with_cap(cap_idx, &mut *buf_ptr) as u64
            } else {
                IPC_ERR_DENIED as u64
            }
        }

        SYS_LOG_WRITE => {
            let ptr = arg1 as *const u8;
            let len = arg2 as usize;
            for i in 0..len {
                crate::driver::uart::putchar(core::ptr::read_volatile(ptr.add(i)));
            }
            0
        }

        SYS_CAP_GRANT => {
            let dst_task_id = arg1;
            let cap_idx = arg2 as u16;
            crate::cap::duplicate(dst_task_id, cap_idx) as u64
        }

        SYS_SHMEM_MAP => {
            let cap_idx = arg1 as u16;
            let virt = arg2;
            if let Some(cap) = crate::cap::get(cap_idx) {
                if cap.cap_type == CapType::SharedMem as u64 && (cap.rights & CAP_WRITE) != 0 {
                    let phys = cap.data;
                    let cur = crate::scheduler::CURRENT;
                    let pml4 = crate::scheduler::TASKS[cur].pml4;
                    if pml4.is_null() { return IPC_ERR_DENIED as u64; }
                    if crate::vm::map_page(pml4, phys, virt,
                        crate::vm::PTE_WRITABLE | crate::vm::PTE_USER) != 0 {
                        return IPC_ERR_NO_MEM as u64;
                    }
                    0
                } else {
                    IPC_ERR_DENIED as u64
                }
            } else {
                IPC_ERR_BAD_CAP as u64
            }
        }

        SYS_SHMEM_CREATE => {
            let _pages = arg1;
            let phys = crate::memory::palloc();
            if phys == 0 { return IPC_ERR_NO_MEM as u64; }
            match crate::cap::alloc(
                CapType::SharedMem as u64, 0,
                CAP_READ | CAP_WRITE, phys)
            {
                Some(idx) => idx as u64,
                None => { crate::memory::pfree(phys); IPC_ERR_NO_MEM as u64 }
            }
        }

        SYS_MMIO_MAP => {
            let phys_addr = arg1;
            let size = arg2;
            let virt = arg3;
            if !crate::driver::pci::validate_mmio(phys_addr, size) {
                return IPC_ERR_DENIED as u64;
            }
            let cur = crate::scheduler::CURRENT;
            let pml4 = crate::scheduler::TASKS[cur].pml4;
            if pml4.is_null() { return IPC_ERR_DENIED as u64; }
            let page_count = ((size + 0xFFF) / 0x1000) as usize;
            for i in 0..page_count {
                let pa = phys_addr + (i as u64) * 0x1000;
                let va = virt + (i as u64) * 0x1000;
                if crate::vm::map_page(pml4, pa, va,
                    crate::vm::PTE_WRITABLE | crate::vm::PTE_USER
                    | crate::vm::PTE_CACHE_DISABLE) != 0
                {
                    return IPC_ERR_NO_MEM as u64;
                }
            }
            0
        }

        SYS_PCI_READ => {
            let bdf = arg1;
            let offset = arg2 as u8;
            let bus = ((bdf >> 8) & 0xFF) as u8;
            let dev = ((bdf >> 3) & 0x1F) as u8;
            let func = (bdf & 0x7) as u8;
            crate::driver::pci::read32(bus, dev, func, offset) as u64
        }

        SYS_PCI_WRITE => {
            let bdf = arg1;
            let offset = arg2 as u8;
            let val = arg3 as u32;
            let bus = ((bdf >> 8) & 0xFF) as u8;
            let dev = ((bdf >> 3) & 0x1F) as u8;
            let func = (bdf & 0x7) as u8;
            crate::driver::pci::write32(bus, dev, func, offset, val);
            0
        }

        SYS_CAP_GET_DATA => {
            let cap_idx = arg1 as u16;
            match crate::cap::get(cap_idx) {
                Some(c) => c.data,
                None => IPC_ERR_BAD_CAP as u64,
            }
        }

        _ => {
            crate::driver::uart::write_str("[SYSCALL] num=");
            let hex = b"0123456789ABCDEF";
            crate::driver::uart::putchar(hex[((num >> 4) & 0xF) as usize]);
            crate::driver::uart::putchar(hex[(num & 0xF) as usize]);
            crate::driver::uart::write_str("\r\n");
            0
        }
    }
}
