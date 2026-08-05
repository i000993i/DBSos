// Syscall dispatch: syscall_rust_entry

use dbsos_abi::syscall::*;
use dbsos_abi::cap::*;
use dbsos_abi::ipc::Message;

const EXIT_MAGIC: u64 = !0u64;

/// SMAP-safe: temporarily disable SMAP to read/write user memory
/// Currently SMAP is disabled, so these are no-ops.
/// Re-enable when all user pointer accesses use these wrappers.
#[inline(always)]
unsafe fn smap_disable() {}

#[inline(always)]
unsafe fn smap_enable() {}

/// Copy data from user pointer to kernel buffer (SMAP-safe)
#[allow(dead_code)]
unsafe fn copy_from_user(dst: *mut u8, src: *const u8, len: usize) {
    smap_disable();
    for i in 0..len {
        *dst.add(i) = core::ptr::read_volatile(src.add(i));
    }
    smap_enable();
}

/// Read a Message from user pointer (SMAP-safe)
unsafe fn read_user_msg(ptr: *const Message) -> Message {
    smap_disable();
    let msg = core::ptr::read_volatile(ptr);
    smap_enable();
    msg
}

/// Write a Message to user pointer (SMAP-safe)
unsafe fn write_user_msg(ptr: *mut Message, msg: &Message) {
    smap_disable();
    core::ptr::write_volatile(ptr, *msg);
    smap_enable();
}

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
                let msg = read_user_msg(msg_ptr);
                crate::ipc::send_with_cap(cap_idx, &msg) as u64
            } else {
                IPC_ERR_DENIED as u64
            }
        }
        SYS_IPC_RECV => {
            let cap_idx = arg1 as u16;
            let buf_ptr = arg2 as *mut Message;
            if crate::cap::validate(cap_idx, CAP_RECV) {
                let mut msg = Message::empty();
                let result = crate::ipc::recv_with_cap(cap_idx, &mut msg);
                write_user_msg(buf_ptr, &msg);
                result as u64
            } else {
                IPC_ERR_DENIED as u64
            }
        }

        SYS_LOG_WRITE => {
            let ptr = arg1 as *const u8;
            let len = arg2 as usize;
            let mut buf = [0u8; 256];
            let copy_len = len.min(256);
            smap_disable();
            for i in 0..copy_len {
                buf[i] = core::ptr::read_volatile(ptr.add(i));
            }
            smap_enable();
            for i in 0..copy_len {
                crate::driver::uart::putchar(buf[i]);
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

        // ── File I/O syscalls ──────────────────────────────────────
        SYS_OPEN => {
            // arg1 = path ptr, arg2 = path len, arg3 = flags
            let path_ptr = arg1 as *const u8;
            let path_len = arg2 as usize;
            let flags = arg3;
            if path_len == 0 || path_len > 127 { return !0u64; }
            // Read path from user (SMAP-safe)
            smap_disable();
            let mut path = [0u8; 128];
            for i in 0..path_len {
                path[i] = core::ptr::read_volatile(path_ptr.add(i));
            }
            smap_enable();
            // Find free FD
            let cur = crate::scheduler::CURRENT;
            let fds = &mut crate::scheduler::TASKS[cur].fds;
            let fd_idx = match fds.iter().position(|f| !f.in_use) {
                Some(i) => i,
                None => { return !0u64; } // too many open files
            };
            // Get file size
            let size = match crate::fs::find_file(&path[..path_len]) {
                Some((_, sz)) => sz,
                None => { return !0u64; } // file not found
            };
            fds[fd_idx] = crate::scheduler::FdEntry {
                in_use: true,
                path: {
                    let mut p = [0u8; 128];
                    p[..path_len].copy_from_slice(&path[..path_len]);
                    p
                },
                offset: 0,
                size,
                flags,
            };
            fd_idx as u64
        }

        SYS_READ => {
            // arg1 = fd, arg2 = buf ptr, arg3 = count
            let fd_idx = arg1 as usize;
            let buf_ptr = arg2 as *mut u8;
            let count = arg3 as usize;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            let fd = &fds[fd_idx];
            let remaining = (fd.size as usize).saturating_sub(fd.offset as usize);
            let to_read = count.min(remaining);
            if to_read == 0 { return 0; }
            // Read file data
            let mut kernel_buf = [0u8; 4096];
            let read_len = to_read.min(4096);
            match crate::fs::read_file(&fd.path[..fd.path.iter().position(|&c| c == 0).unwrap_or(128)], &mut kernel_buf) {
                Some(sz) => {
                    let actual = read_len.min(sz.saturating_sub(fd.offset as usize));
                    if actual == 0 { return 0; }
                    // Copy to user buffer (SMAP-safe)
                    smap_disable();
                    for i in 0..actual {
                        core::ptr::write_volatile(buf_ptr.add(i), kernel_buf[fd.offset as usize + i]);
                    }
                    smap_enable();
                    // Update offset
                    let fds_mut = &mut crate::scheduler::TASKS[cur].fds;
                    fds_mut[fd_idx].offset += actual as u32;
                    actual as u64
                }
                None => !0u64
            }
        }

        SYS_WRITE => {
            // arg1 = fd, arg2 = buf ptr, arg3 = count
            let fd_idx = arg1 as usize;
            let buf_ptr = arg2 as *const u8;
            let count = arg3 as usize;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            let fd = &fds[fd_idx];
            if fd.flags & O_WRONLY == 0 && fd.flags & O_RDWR == 0 { return !0u64; }
            // Read data from user (SMAP-safe)
            let mut kernel_buf = [0u8; 4096];
            let write_len = count.min(4096);
            smap_disable();
            for i in 0..write_len {
                kernel_buf[i] = core::ptr::read_volatile(buf_ptr.add(i));
            }
            smap_enable();
            // Write to file
            let path_len = fd.path.iter().position(|&c| c == 0).unwrap_or(128);
            if crate::fs::write_file(&fd.path[..path_len], &kernel_buf[..write_len]) {
                let fds_mut = &mut crate::scheduler::TASKS[cur].fds;
                fds_mut[fd_idx].offset += write_len as u32;
                write_len as u64
            } else {
                !0u64
            }
        }

        SYS_CLOSE => {
            let fd_idx = arg1 as usize;
            let cur = crate::scheduler::CURRENT;
            let fds = &mut crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            fds[fd_idx] = crate::scheduler::FdEntry::empty();
            0
        }

        SYS_FSTAT => {
            let fd_idx = arg1 as usize;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            fds[fd_idx].size as u64
        }

        // ── Process management syscalls ──────────────────────────────
        SYS_FORK => {
            use crate::scheduler::{TaskState, CURRENT, NEXT_ID, TASKS, STACK_SIZE, STACK_CANARY, FdEntry};
            use dbsos_abi::syscall::MAX_FDS;
            unsafe {
                let cur = CURRENT;
                let cur_id = TASKS[cur].id;

                // Find free task slot
                let slot = match (0..crate::scheduler::MAX_TASKS).find(|&i| TASKS[i].state == TaskState::Free) {
                    Some(s) => s,
                    None => return !0u64, // no free slots
                };

                // Allocate new kernel stack and copy parent's
                let new_kstack = crate::memory::palloc_n(STACK_SIZE / crate::memory::PAGE_SIZE);
                if new_kstack == 0 { return !0u64; }
                crate::vm::identity_map_2mb(crate::vm::KERNEL_PML4 as *mut u64,
                    new_kstack, new_kstack + STACK_SIZE as u64, crate::vm::PTE_WRITABLE);
                let parent_sp = TASKS[cur].sp;
                let parent_stack_base = TASKS[cur].stack_base as u64;
                let sp_offset = parent_sp - parent_stack_base;
                core::ptr::copy_nonoverlapping(
                    parent_stack_base as *const u8,
                    new_kstack as *mut u8,
                    STACK_SIZE,
                );
                let new_sp = new_kstack + sp_offset;

                // Write canary at bottom
                *(new_kstack as *mut u64) = STACK_CANARY;

                // Allocate new kernel stack for user processes
                let new_kstack_phys = if TASKS[cur].ring3 {
                    let ks = crate::memory::palloc_n(STACK_SIZE / crate::memory::PAGE_SIZE);
                    if ks != 0 {
                        crate::vm::identity_map_2mb(crate::vm::KERNEL_PML4 as *mut u64,
                            ks, ks + STACK_SIZE as u64, crate::vm::PTE_WRITABLE);
                    }
                    ks
                } else { 0 };

                // Clone address space (simplified: share kernel mappings)
                let new_pml4_phys = crate::memory::palloc();
                if new_pml4_phys == 0 {
                    crate::memory::pfree_n(new_kstack, STACK_SIZE / crate::memory::PAGE_SIZE);
                    return !0u64;
                }
                crate::vm::identity_map_2mb(crate::vm::KERNEL_PML4 as *mut u64,
                    new_pml4_phys, new_pml4_phys + 4096,
                    crate::vm::PTE_WRITABLE);
                let new_pml4 = new_pml4_phys as *mut u64;
                core::ptr::write_bytes(new_pml4 as *mut u8, 0, 4096);

                // Clone user-space mappings if parent has them
                if !TASKS[cur].pml4.is_null() && TASKS[cur].ring3 {
                    // Simple approach: copy all low-half (user) PML4 entries
                    // Note: this shares the actual page table pages (CoW would be better)
                    let parent_pml4 = TASKS[cur].pml4;
                    for i in 0..256 {
                        let e = *((parent_pml4 as *const u64).add(i));
                        if e & crate::vm::PTE_PRESENT != 0 {
                            *((new_pml4).add(i)) = e;
                        }
                    }
                }
                // Copy kernel mappings
                for i in 256..512 {
                    let e = *((crate::vm::KERNEL_PML4 as *const u64).add(i));
                    if e & crate::vm::PTE_PRESENT != 0 {
                        *((new_pml4).add(i)) = e;
                    }
                }

                // Copy file descriptors
                let mut new_fds = [FdEntry::empty(); MAX_FDS];
                for i in 0..MAX_FDS {
                    new_fds[i] = TASKS[cur].fds[i];
                }

                let new_id = NEXT_ID;
                NEXT_ID += 1;

                // Create child task
                TASKS[slot] = crate::scheduler::Task {
                    state: TaskState::Ready,
                    stack_base: new_kstack as *mut u8,
                    sp: new_sp,
                    id: new_id,
                    parent_id: cur_id,
                    exit_status: 0,
                    ipc_partner: 0, ipc_val: 0, ipc_msg: Message::empty(),
                    pml4: new_pml4,
                    pml4_phys: new_pml4_phys,
                    gdt_phys: 0, tss_phys: 0,
                    ring3: TASKS[cur].ring3,
                    sys_ursave: TASKS[cur].sys_ursave,
                    code_phys: TASKS[cur].code_phys,
                    user_stack_phys: TASKS[cur].user_stack_phys,
                    kstack_phys: new_kstack_phys,
                    fpu_buf_phys: crate::scheduler::task::fpu_alloc_buf(),
                    pending_msg: Message::empty(),
                    fds: new_fds,
                };

                // Copy FPU state
                if TASKS[cur].fpu_buf_phys != 0 && TASKS[slot].fpu_buf_phys != 0 {
                    let parent_fpu = TASKS[cur].fpu_buf_phys;
                    let child_fpu = TASKS[slot].fpu_buf_phys;
                    core::ptr::copy_nonoverlapping(
                        parent_fpu as *const u8,
                        child_fpu as *mut u8,
                        512,
                    );
                }

                new_id // return child PID to parent
            }
        }

        SYS_EXEC => {
            // arg1 = path ptr, arg2 = path len
            let path_ptr = arg1 as *const u8;
            let path_len = arg2 as usize;
            if path_len == 0 || path_len > 127 { return !0u64; }
            // Read path from user (SMAP-safe)
            smap_disable();
            let mut path = [0u8; 128];
            for i in 0..path_len {
                path[i] = core::ptr::read_volatile(path_ptr.add(i));
            }
            smap_enable();
            // Load ELF and replace current process image
            // For now, delegate to the existing ELF loader which spawns a new process
            // A proper exec would replace the current process, but this is simpler
            let result = crate::elf::load_and_spawn(&path[..path_len]);
            if result == 0 { !0u64 } else { result }
        }

        SYS_WAITPID => {
            // arg1 = child pid (0 = any), arg2 = status ptr
            let child_pid = arg1;
            let status_ptr = arg2 as *mut i32;
            use crate::scheduler::{TaskState, TASKS};
            unsafe {
                let cur = crate::scheduler::CURRENT;
                let cur_id = TASKS[cur].id;

                // Look for an exited child
                let mut found = None;
                for i in 0..crate::scheduler::MAX_TASKS {
                    if TASKS[i].parent_id == cur_id
                        && (child_pid == 0 || TASKS[i].id == child_pid)
                    {
                        if TASKS[i].state == TaskState::Exited {
                            found = Some(i);
                            break;
                        }
                    }
                }

                if let Some(slot) = found {
                    let child_id = TASKS[slot].id;
                    let status = TASKS[slot].exit_status;
                    TASKS[slot].state = TaskState::Free; //回收 slot
                    if status_ptr as u64 != 0 {
                        smap_disable();
                        core::ptr::write_volatile(status_ptr, status);
                        smap_enable();
                    }
                    child_id
                } else {
                    // No exited child found — block until one exits
                    // Mark ourselves as waiting
                    TASKS[cur].state = TaskState::BlockedRecv;
                    TASKS[cur].ipc_partner = 0; // accept from any
                    crate::scheduler::yield_now();
                    // When we wake up, ipc_val contains the child slot
                    let child_slot = TASKS[cur].ipc_val as usize;
                    if child_slot < crate::scheduler::MAX_TASKS && TASKS[child_slot].state == TaskState::Exited {
                        let child_id = TASKS[child_slot].id;
                        let status = TASKS[child_slot].exit_status;
                        TASKS[child_slot].state = TaskState::Free;
                        if status_ptr as u64 != 0 {
                            smap_disable();
                            core::ptr::write_volatile(status_ptr, status);
                            smap_enable();
                        }
                        child_id
                    } else {
                        !0u64
                    }
                }
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
