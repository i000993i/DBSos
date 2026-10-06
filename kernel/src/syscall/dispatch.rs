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

/// Проверка user-буфера перед копированием: EFAULT вместо #PF.
/// Для ядерных задач (pml4 null) — пропуск (старое поведение).
#[inline(always)]
unsafe fn user_range_ok(ptr: u64, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let cur = crate::scheduler::CURRENT;
    let pml4 = crate::scheduler::TASKS[cur].pml4;
    if pml4.is_null() {
        return true;
    }
    crate::vm::is_user_range_mapped(pml4, ptr, len)
}

#[no_mangle]
unsafe extern "C" fn syscall_rust_entry(num: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64) -> u64 {
    // Linux ABI роутинг: задача, привязанная к живому LinuxTask, говорит
    // Linux-номерами (0=read, 1=write, 60=exit...), а не DBSos-номерами.
    // Раньше такие вызовы падали в native match и ломались.
    // Регистры совпадают для первых 4 аргументов (rdi,rsi,rdx,r10); a5/a6=0
    // (все реализованные хендлеры их игнорируют: mmap anon, clone=fork).
    if let Some(lt) = crate::linux::task::get_current() {
        return crate::linux::syscall::handle_syscall(lt, num, arg1, arg2, arg3, arg4, 0, 0) as u64;
    }
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
            if !user_range_ok(ptr as u64, len) { return !0u64; }
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

        // ── File I/O syscalls (via VFS) ────────────────────────────
        SYS_OPEN => {
            // arg1 = path ptr, arg2 = path len, arg3 = flags
            let path_ptr = arg1 as *const u8;
            let path_len = arg2 as usize;
            let flags = arg3;
            if path_len == 0 || path_len > 127 { return !0u64; }
            smap_disable();
            let mut path = [0u8; 128];
            for i in 0..path_len {
                path[i] = core::ptr::read_volatile(path_ptr.add(i));
            }
            smap_enable();
            let fd = crate::vfs::open(&path[..path_len], flags);
            if fd < 0 { return !0u64; }
            // Store VFS fd in the task's FD table
            let cur = crate::scheduler::CURRENT;
            let task_fds = &mut crate::scheduler::TASKS[cur].fds;
            let slot = match task_fds.iter().position(|f| !f.in_use) {
                Some(i) => i,
                None => { crate::vfs::close(fd); return !0u64; }
            };
            // Get size from VFS
            let mut info = crate::vfs::StatInfo { size: 0, is_dir: false, cluster: 0 };
            let _ = crate::vfs::stat(&path[..path_len], &mut info);
            task_fds[slot] = crate::scheduler::FdEntry {
                in_use: true,
                path: {
                    let mut p = [0u8; 128];
                    p[..path_len].copy_from_slice(&path[..path_len]);
                    p
                },
                offset: 0,
                size: info.size,
                flags,
            };
            // Encode VFS fd into the path field's last bytes (hack: use offset to store it)
            // Actually, store it separately — extend FdEntry or use a separate mapping.
            // For now, re-open via VFS on each read/write (simple, correct).
            slot as u64
        }

        SYS_READ => {
            // arg1 = fd, arg2 = buf ptr, arg3 = count
            let fd_idx = arg1 as usize;
            let buf_ptr = arg2 as *mut u8;
            let count = arg3 as usize;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            let path_len = fds[fd_idx].path.iter().position(|&c| c == 0).unwrap_or(128);
            let offset = fds[fd_idx].offset as u64;
            let size = fds[fd_idx].size as u64;
            let remaining = size.saturating_sub(offset) as usize;
            let to_read = count.min(remaining);
            if to_read == 0 { return 0; }
            // EFAULT вместо #PF на кривом user-буфере
            if !user_range_ok(buf_ptr as u64, to_read) { return !0u64; }
            // Read via VFS
            let mut kernel_buf = [0u8; 4096];
            let read_chunk = to_read.min(4096);
            // Open file via VFS, read at offset, close
            let vfs_fd = crate::vfs::open(&fds[fd_idx].path[..path_len], 0);
            if vfs_fd < 0 { return !0u64; }
            // Seek to offset
            crate::vfs::lseek(vfs_fd, offset as i64, 0);
            let n = crate::vfs::read(vfs_fd, &mut kernel_buf[..read_chunk]);
            crate::vfs::close(vfs_fd);
            if n <= 0 { return if n == 0 { 0 } else { !0u64 }; }
            let actual = n as usize;
            // Copy to user buffer
            smap_disable();
            for i in 0..actual {
                core::ptr::write_volatile(buf_ptr.add(i), kernel_buf[i]);
            }
            smap_enable();
            // Update offset
            let fds_mut = &mut crate::scheduler::TASKS[cur].fds;
            fds_mut[fd_idx].offset += actual as u32;
            actual as u64
        }

        SYS_WRITE => {
            // arg1 = fd, arg2 = buf ptr, arg3 = count
            let fd_idx = arg1 as usize;
            let buf_ptr = arg2 as *const u8;
            let count = arg3 as usize;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            if fds[fd_idx].flags & O_WRONLY == 0 && fds[fd_idx].flags & O_RDWR == 0 { return !0u64; }
            // EFAULT вместо #PF на кривом user-буфере
            if !user_range_ok(buf_ptr as u64, count) { return !0u64; }
            let path_len = fds[fd_idx].path.iter().position(|&c| c == 0).unwrap_or(128);
            let offset = fds[fd_idx].offset as u64;
            // Read data from user
            let mut kernel_buf = [0u8; 4096];
            let write_len = count.min(4096);
            smap_disable();
            for i in 0..write_len {
                kernel_buf[i] = core::ptr::read_volatile(buf_ptr.add(i));
            }
            smap_enable();
            // Write via VFS
            let vfs_fd = crate::vfs::open(&fds[fd_idx].path[..path_len], fds[fd_idx].flags);
            if vfs_fd < 0 { return !0u64; }
            crate::vfs::lseek(vfs_fd, offset as i64, 0);
            let n = crate::vfs::write(vfs_fd, &kernel_buf[..write_len]);
            crate::vfs::close(vfs_fd);
            if n <= 0 { return !0u64; }
            let actual = n as usize;
            let fds_mut = &mut crate::scheduler::TASKS[cur].fds;
            fds_mut[fd_idx].offset += actual as u32;
            if fds_mut[fd_idx].offset > fds_mut[fd_idx].size {
                fds_mut[fd_idx].size = fds_mut[fd_idx].offset;
            }
            actual as u64
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

        // ── lseek ─────────────────────────────────────────────────
        SYS_LSEEK => {
            let fd_idx = arg1 as usize;
            let offset = arg2 as i64;
            let whence = arg3 as u32;
            let cur = crate::scheduler::CURRENT;
            let fds = &mut crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            let new_off = match whence {
                0 => offset as u64,                           // SEEK_SET
                1 => (fds[fd_idx].offset as i64 + offset) as u64, // SEEK_CUR
                2 => (fds[fd_idx].size as i64 + offset) as u64,   // SEEK_END
                _ => return !0u64,
            };
            fds[fd_idx].offset = new_off as u32;
            new_off
        }

        // ── Process management syscalls ──────────────────────────────
        SYS_FORK => {
            use crate::scheduler::{TaskState, CURRENT, NEXT_ID, TASKS, STACK_SIZE, STACK_CANARY, FdEntry};
            use dbsos_abi::syscall::MAX_FDS;
            unsafe {
                let cur = CURRENT;
                let cur_id = TASKS[cur].id;
                let cur_uid = TASKS[cur].uid;
                if cur_uid != 0 && crate::scheduler::task_count_for_uid(cur_uid) >= crate::scheduler::MAX_TASKS_PER_USER {
                    return !0u64;
                }

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

                // Create child task — inherit uid/gid/cwd
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
                    vmas: [const { crate::scheduler::vma::Vma::empty() }; crate::scheduler::vma::MAX_VMAS],
                    vma_count: 0,
                    uid: TASKS[cur].uid,
                    gid: TASKS[cur].gid,
                    cwd: TASKS[cur].cwd,
                    cwd_len: TASKS[cur].cwd_len,
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

        // ── readdir syscall ────────────────────────────────────────
        SYS_READDIR => {
            let path_ptr = arg1 as *const u8;
            let path_len = arg2 as usize;
            let entries_ptr = arg3 as *mut u8;
            let max_entries = arg4 as usize;
            if path_len == 0 || path_len > 127 { return !0u64; }
            smap_disable();
            let mut path = [0u8; 128];
            for i in 0..path_len { path[i] = core::ptr::read_volatile(path_ptr.add(i)); }
            smap_enable();
            let mut entries = [crate::vfs::DirEntry {
                name: [0; crate::vfs::MAX_NAME],
                is_dir: false,
                size: 0,
            }; 64];
            let n = crate::vfs::readdir(&path[..path_len], &mut entries);
            if n < 0 { return !0u64; }
            let n = n as usize;
            let write_n = n.min(max_entries).min(64);
            // Write entries to user buffer (each entry: 32 bytes: 24 name + 1 is_dir + 3 size + 4 padding)
            smap_disable();
            for i in 0..write_n {
                let e = &entries[i];
                let base = entries_ptr.add(i * 32);
                let name_len = e.name.iter().position(|&c| c == 0).unwrap_or(crate::vfs::MAX_NAME);
                for j in 0..24 {
                    let v = if j < name_len { e.name[j] } else { 0 };
                    core::ptr::write_volatile(base.add(j), v);
                }
                core::ptr::write_volatile(base.add(24), if e.is_dir { 1 } else { 0 });
                let sz = e.size;
                core::ptr::write_volatile(base.add(25), sz as u8);
                core::ptr::write_volatile(base.add(26), (sz >> 8) as u8);
                core::ptr::write_volatile(base.add(27), (sz >> 16) as u8);
            }
            smap_enable();
            write_n as u64
        }

        // ── mkdir syscall ─────────────────────────────────────────
        SYS_MKDIR => {
            let path_ptr = arg1 as *const u8;
            let path_len = arg2 as usize;
            if path_len == 0 || path_len > 127 { return !0u64; }
            smap_disable();
            let mut path = [0u8; 128];
            for i in 0..path_len { path[i] = core::ptr::read_volatile(path_ptr.add(i)); }
            smap_enable();
            if crate::vfs::mkdir(&path[..path_len]) { 0 } else { !0u64 }
        }

        // ── rmdir syscall ─────────────────────────────────────────
        SYS_RMDIR => {
            let path_ptr = arg1 as *const u8;
            let path_len = arg2 as usize;
            if path_len == 0 || path_len > 127 { return !0u64; }
            smap_disable();
            let mut path = [0u8; 128];
            for i in 0..path_len { path[i] = core::ptr::read_volatile(path_ptr.add(i)); }
            smap_enable();
            if crate::vfs::rmdir(&path[..path_len]) { 0 } else { !0u64 }
        }

        // ── unlink syscall ────────────────────────────────────────
        SYS_UNLINK => {
            let path_ptr = arg1 as *const u8;
            let path_len = arg2 as usize;
            if path_len == 0 || path_len > 127 { return !0u64; }
            smap_disable();
            let mut path = [0u8; 128];
            for i in 0..path_len { path[i] = core::ptr::read_volatile(path_ptr.add(i)); }
            smap_enable();
            if crate::vfs::unlink(&path[..path_len]) { 0 } else { !0u64 }
        }

        // ── dup syscall ───────────────────────────────────────────
        SYS_DUP => {
            let old_fd = arg1 as usize;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if old_fd >= fds.len() || !fds[old_fd].in_use { return !0u64; }
            // Find free slot
            let new_slot = match fds.iter().position(|f| !f.in_use) {
                Some(s) => s,
                None => return !0u64,
            };
            let cur_fds = &mut crate::scheduler::TASKS[cur].fds;
            cur_fds[new_slot] = cur_fds[old_fd];
            new_slot as u64
        }

        // ── dup2 syscall ──────────────────────────────────────────
        SYS_DUP2 => {
            let old_fd = arg1 as usize;
            let new_fd = arg2 as usize;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if old_fd >= fds.len() || !fds[old_fd].in_use { return !0u64; }
            if new_fd >= dbsos_abi::syscall::MAX_FDS { return !0u64; }
            let entry = fds[old_fd];
            let cur_fds = &mut crate::scheduler::TASKS[cur].fds;
            cur_fds[new_fd] = entry;
            new_fd as u64
        }

        // ── pipe syscall ──────────────────────────────────────────
        SYS_PIPE => {
            let fds_ptr = arg1 as *mut u32;
            // Allocate a pipe buffer (4KB from heap)
            let buf = unsafe { crate::heap::kmalloc(4096) };
            if buf.is_null() { return !0u64; }
            unsafe { core::ptr::write_bytes(buf, 0, 4096); }
            // Create two FDs: read and write ends
            let cur = crate::scheduler::CURRENT;
            let task_fds = &mut crate::scheduler::TASKS[cur].fds;
            let read_slot = match task_fds.iter().position(|f| !f.in_use) {
                Some(s) => s,
                None => { unsafe { crate::heap::kfree(buf); } return !0u64; }
            };
            task_fds[read_slot] = crate::scheduler::FdEntry {
                in_use: true,
                path: {
                    let mut p = [0u8; 128];
                    // Store pipe buffer pointer in first 8 bytes of path (hack)
                    let ptr_val = buf as u64;
                    p[0] = ptr_val as u8;
                    p[1] = (ptr_val >> 8) as u8;
                    p[2] = (ptr_val >> 16) as u8;
                    p[3] = (ptr_val >> 24) as u8;
                    p[4] = (ptr_val >> 32) as u8;
                    p[5] = (ptr_val >> 40) as u8;
                    p[6] = (ptr_val >> 48) as u8;
                    p[7] = (ptr_val >> 56) as u8;
                    p[8] = b'P'; p[9] = b'I'; p[10] = b'P'; p[11] = b'E';
                    p
                },
                offset: 0, // read position
                size: 0,   // write position (stored separately)
                flags: 0,  // O_RDONLY for read end
            };
            let write_slot = match task_fds.iter().position(|f| !f.in_use) {
                Some(s) => s,
                None => {
                    unsafe { crate::heap::kfree(buf); }
                    task_fds[read_slot] = crate::scheduler::FdEntry::empty();
                    return !0u64;
                }
            };
            task_fds[write_slot] = crate::scheduler::FdEntry {
                in_use: true,
                path: {
                    let mut p = [0u8; 128];
                    let ptr_val = buf as u64;
                    p[0] = ptr_val as u8;
                    p[1] = (ptr_val >> 8) as u8;
                    p[2] = (ptr_val >> 16) as u8;
                    p[3] = (ptr_val >> 24) as u8;
                    p[4] = (ptr_val >> 32) as u8;
                    p[5] = (ptr_val >> 40) as u8;
                    p[6] = (ptr_val >> 48) as u8;
                    p[7] = (ptr_val >> 56) as u8;
                    p[8] = b'P'; p[9] = b'I'; p[10] = b'P'; p[11] = b'E';
                    p
                },
                offset: 0, // write position
                size: 4096,
                flags: 1,  // O_WRONLY for write end
            };
            smap_disable();
            core::ptr::write_volatile(fds_ptr, read_slot as u32);
            core::ptr::write_volatile(fds_ptr.add(1), write_slot as u32);
            smap_enable();
            0
        }

        // ── getcwd syscall ────────────────────────────────────────
        SYS_GETCWD => {
            let buf_ptr = arg1 as *mut u8;
            let buf_size = arg2 as usize;
            let cwd = crate::shell::cwd_get_public();
            let cwd_len = cwd.len().min(buf_size - 1);
            smap_disable();
            for i in 0..cwd_len {
                core::ptr::write_volatile(buf_ptr.add(i), cwd[i]);
            }
            core::ptr::write_volatile(buf_ptr.add(cwd_len), 0);
            smap_enable();
            0
        }

        // ── chdir syscall ─────────────────────────────────────────
        SYS_CHDIR => {
            let path_ptr = arg1 as *const u8;
            let path_len = arg2 as usize;
            if path_len == 0 || path_len > 127 { return !0u64; }
            smap_disable();
            let mut path = [0u8; 128];
            for i in 0..path_len { path[i] = core::ptr::read_volatile(path_ptr.add(i)); }
            smap_enable();
            if crate::vfs::is_dir(&path[..path_len]) {
                crate::shell::cwd_set_public(&path[..path_len]);
                0
            } else {
                !0u64
            }
        }

        // ── Socket API syscalls ───────────────────────────────────
        SYS_SOCKET => {
            let domain = arg1 as u32;
            let sock_type = arg2 as u32;
            let _protocol = arg3 as u32;
            if domain != 2 /* AF_INET */ || sock_type != 1 /* SOCK_STREAM */ { return !0u64; }
            // Allocate a TCP connection slot
            let slot = match crate::driver::tcp::alloc_conn() {
                Some(i) => i,
                None => return !0u64,
            };
            let cur = crate::scheduler::CURRENT;
            let task_fds = &mut crate::scheduler::TASKS[cur].fds;
            let fd_slot = match task_fds.iter().position(|f| !f.in_use) {
                Some(s) => s,
                None => { crate::driver::tcp::close(slot); return !0u64; }
            };
            task_fds[fd_slot] = crate::scheduler::FdEntry {
                in_use: true,
                path: {
                    let mut p = [0u8; 128];
                    p[0] = b'S'; p[1] = b'O'; p[2] = b'C'; p[3] = b'K';
                    p
                },
                offset: slot as u32,
                size: 0,
                flags: 0,
            };
            fd_slot as u64
        }

        SYS_BIND => {
            // TCP API: bind + listen combined via tcp::listen(port).
            // We store the port in the fd's size field; actual listen happens in LISTEN.
            let fd_idx = arg1 as usize;
            let sockaddr_ptr = arg2 as *const u8;
            let _addrlen = arg3 as usize;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            smap_disable();
            let sin_port = core::ptr::read_volatile(sockaddr_ptr.add(2)) as u16
                         | (core::ptr::read_volatile(sockaddr_ptr.add(3)) as u16) << 8;
            smap_enable();
            // Store port for LISTEN to use
            let fds_mut = &mut crate::scheduler::TASKS[cur].fds;
            fds_mut[fd_idx].size = sin_port as u32;
            0
        }

        SYS_LISTEN => {
            let fd_idx = arg1 as usize;
            let _backlog = arg2 as i32;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            let conn_idx = fds[fd_idx].offset as usize;
            let port = fds[fd_idx].size as u16;
            // Close the placeholder connection and re-allocate via listen(port)
            crate::driver::tcp::close(conn_idx);
            match crate::driver::tcp::listen(port) {
                Some(new_idx) => {
                    let fds_mut = &mut crate::scheduler::TASKS[cur].fds;
                    fds_mut[fd_idx].offset = new_idx as u32;
                    0
                }
                None => !0u64,
            }
        }

        SYS_ACCEPT => {
            let fd_idx = arg1 as usize;
            let sockaddr_ptr = arg2 as *mut u8;
            let addrlen_ptr = arg3 as *mut u32;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            let _listen_conn = fds[fd_idx].offset as usize;
            let port = fds[fd_idx].size as u16;
            let accepted = match crate::driver::tcp::find_accepted(port) {
                Some(i) => i,
                None => return !0u64,
            };
            // Map to process-local fd
            let task_fds = &mut crate::scheduler::TASKS[cur].fds;
            let fd_slot = match task_fds.iter().position(|f| !f.in_use) {
                Some(s) => s,
                None => return !0u64,
            };
            task_fds[fd_slot] = crate::scheduler::FdEntry {
                in_use: true,
                path: {
                    let mut p = [0u8; 128];
                    p[0] = b'S'; p[1] = b'O'; p[2] = b'C'; p[3] = b'K';
                    p
                },
                offset: accepted as u32,
                size: 0,
                flags: 0,
            };
            // Fill sockaddr_in if provided
            if !sockaddr_ptr.is_null() {
                let c = crate::driver::tcp::conn_ref(accepted);
                smap_disable();
                core::ptr::write_volatile(sockaddr_ptr, 2);       // sin_family = AF_INET
                core::ptr::write_volatile(sockaddr_ptr.add(1), 0);
                let rp = c.remote_port;
                core::ptr::write_volatile(sockaddr_ptr.add(2), rp as u8);
                core::ptr::write_volatile(sockaddr_ptr.add(3), (rp >> 8) as u8);
                for i in 0..4 {
                    core::ptr::write_volatile(sockaddr_ptr.add(4 + i), c.peer_ip[i]);
                }
                for i in 8..16 { core::ptr::write_volatile(sockaddr_ptr.add(i), 0); }
                if !addrlen_ptr.is_null() {
                    core::ptr::write_volatile(addrlen_ptr, 16);
                }
                smap_enable();
            }
            fd_slot as u64
        }

        SYS_CONNECT => {
            let fd_idx = arg1 as usize;
            let sockaddr_ptr = arg2 as *const u8;
            let _addrlen = arg3 as usize;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            let conn_idx = fds[fd_idx].offset as usize;
            smap_disable();
            let sin_port = core::ptr::read_volatile(sockaddr_ptr.add(2)) as u16
                         | (core::ptr::read_volatile(sockaddr_ptr.add(3)) as u16) << 8;
            let mut sin_addr = [0u8; 4];
            for i in 0..4 { sin_addr[i] = core::ptr::read_volatile(sockaddr_ptr.add(4 + i)); }
            smap_enable();
            // Close the placeholder and re-allocate via connect
            crate::driver::tcp::close(conn_idx);
            match crate::driver::tcp::connect(sin_addr, sin_port) {
                Some(new_idx) => {
                    let fds_mut = &mut crate::scheduler::TASKS[cur].fds;
                    fds_mut[fd_idx].offset = new_idx as u32;
                    0
                }
                None => !0u64,
            }
        }

        SYS_SEND => {
            let fd_idx = arg1 as usize;
            let buf_ptr = arg2 as *const u8;
            let len = arg3 as usize;
            let _flags = arg4 as u32;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            let conn_idx = fds[fd_idx].offset as usize;
            let mut buf = [0u8; 1400];
            let chunk = len.min(1400);
            smap_disable();
            for i in 0..chunk { buf[i] = core::ptr::read_volatile(buf_ptr.add(i)); }
            smap_enable();
            if crate::driver::tcp::send(conn_idx, &buf[..chunk]) {
                chunk as u64
            } else {
                !0u64
            }
        }

        SYS_RECV => {
            let fd_idx = arg1 as usize;
            let buf_ptr = arg2 as *mut u8;
            let len = arg3 as usize;
            let _flags = arg4 as u32;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            let conn_idx = fds[fd_idx].offset as usize;
            let mut out = [0u8; 2048];
            let n = crate::driver::tcp::recv(conn_idx, &mut out);
            if n == 0 { return 0; }
            let actual = n.min(len);
            smap_disable();
            for i in 0..actual { core::ptr::write_volatile(buf_ptr.add(i), out[i]); }
            smap_enable();
            actual as u64
        }

        SYS_SHUTDOWN => {
            let fd_idx = arg1 as usize;
            let _how = arg2 as u32;
            let cur = crate::scheduler::CURRENT;
            let fds = &crate::scheduler::TASKS[cur].fds;
            if fd_idx >= fds.len() || !fds[fd_idx].in_use { return !0u64; }
            let conn_idx = fds[fd_idx].offset as usize;
            crate::driver::tcp::close(conn_idx);
            let cur_fds = &mut crate::scheduler::TASKS[cur].fds;
            cur_fds[fd_idx] = crate::scheduler::FdEntry::empty();
            0
        }

        SYS_GETUID => {
            crate::user::current_uid() as u64
        }
        SYS_GETGID => {
            crate::user::current_gid() as u64
        }
        SYS_SETUID => {
            let uid = arg1 as u32;
            // Only root can setuid
            if crate::user::current_uid() != 0 { return !0u64; }
            if crate::user::find_by_uid(uid).is_none() && uid != 0 { return !0u64; }
            crate::user::set_current_uid(uid);
            0
        }
        SYS_CHMOD => {
            let path_ptr = arg1 as *const u8;
            let path_len = arg2 as usize;
            let mode = arg3 as u16;
            if path_len == 0 || path_len > 127 { return !0u64; }
            if crate::user::current_uid() != 0 {
                // Only owner or root can chmod
                let mut path = [0u8; 128];
                smap_disable();
                for i in 0..path_len { path[i] = core::ptr::read_volatile(path_ptr.add(i)); }
                smap_enable();
                let perm = crate::permissions::get(&path[..path_len]);
                if perm.owner != crate::user::current_uid() { return !0u64; }
                crate::permissions::set(&path[..path_len], crate::permissions::Perm { owner: perm.owner, group: perm.group, mode: mode & 0o777 });
                return 0;
            }
            smap_disable();
            let mut path = [0u8; 128];
            for i in 0..path_len { path[i] = core::ptr::read_volatile(path_ptr.add(i)); }
            smap_enable();
            let perm = crate::permissions::get(&path[..path_len]);
            crate::permissions::set(&path[..path_len], crate::permissions::Perm { owner: perm.owner, group: perm.group, mode: mode & 0o777 });
            0
        }
        SYS_CHOWN => {
            let path_ptr = arg1 as *const u8;
            let path_len = arg2 as usize;
            let owner = arg3 as u32;
            let group = arg4 as u32;
            if path_len == 0 || path_len > 127 { return !0u64; }
            if crate::user::current_uid() != 0 { return !0u64; }
            smap_disable();
            let mut path = [0u8; 128];
            for i in 0..path_len { path[i] = core::ptr::read_volatile(path_ptr.add(i)); }
            smap_enable();
            let perm = crate::permissions::get(&path[..path_len]);
            crate::permissions::set(&path[..path_len], crate::permissions::Perm { owner, group, mode: perm.mode });
            0
        }

        SYS_WL_SHM_CREATE => {
            let size = arg1 as usize;
            match crate::wayland::shm::pool_create(size) {
                Some(idx) => idx as u64,
                None => !0u64,
            }
        }
        SYS_WL_SHM_DESTROY => {
            let idx = arg1 as usize;
            if crate::wayland::shm::pool_destroy(idx) { 0 } else { !0u64 }
        }
        SYS_WL_SURFACE_CREATE => {
            let x = arg1 as i32;
            let y = arg2 as i32;
            let w = arg3 as u32;
            let h = arg4 as u32;
            if w==0 || h==0 || w>2048 || h>2048 { return !0u64; }
            match crate::wayland::compositor::surface_create(x, y, w, h) {
                Some(id) => id as u64,
                None => !0u64,
            }
        }
        SYS_WL_SURFACE_DESTROY => {
            let id = arg1 as u32;
            if crate::wayland::compositor::surface_destroy(id) { 0 } else { !0u64 }
        }
        SYS_WL_SURFACE_ATTACH => {
            let id = arg1 as u32;
            let pool = arg2 as usize;
            let offset = arg3 as u32;
            let wh = arg4;
            let w = (wh >> 32) as u32;
            let h = (wh & 0xFFFFFFFF) as u32;
            if w==0 || h==0 { return !0u64; }
            let stride = w * 4;
            if crate::wayland::compositor::surface_attach(id, pool, offset, w, h, stride) { 0 } else { !0u64 }
        }
        SYS_WL_SURFACE_COMMIT => {
            let id = arg1 as u32;
            if crate::wayland::compositor::surface_commit(id) { 0 } else { !0u64 }
        }
        SYS_WL_SURFACE_SET_POS => {
            let id = arg1 as u32;
            let x = arg2 as i32;
            let y = arg3 as i32;
            if crate::wayland::compositor::surface_set_pos(id, x, y) { 0 } else { !0u64 }
        }
        SYS_FB_INFO => {
            // arg1 = *mut FbInfo { width, height, stride, phys, is_bgr, size }
            let out_ptr = arg1 as *mut u64;
            if out_ptr.is_null() { return !0u64; }
            let w = crate::display::width() as u64;
            let h = crate::display::height() as u64;
            let s = crate::display::stride() as u64;
            let phys = crate::display::fb_phys();
            let is_bgr = if crate::display::is_bgr() {1u64} else {0u64};
            let size = s * h * 4;
            unsafe{
                smap_disable();
                core::ptr::write_volatile(out_ptr.add(0), w);
                core::ptr::write_volatile(out_ptr.add(1), h);
                core::ptr::write_volatile(out_ptr.add(2), s);
                core::ptr::write_volatile(out_ptr.add(3), phys);
                core::ptr::write_volatile(out_ptr.add(4), is_bgr);
                core::ptr::write_volatile(out_ptr.add(5), size);
                smap_enable();
            }
            0
        }
        SYS_FB_MAP => {
            // arg1 = phys, arg2 = virt, arg3 = size
            let phys = arg1;
            let virt = arg2;
            let size = arg3 as usize;
            if phys==0 || virt==0 || size==0 { return !0u64; }
            // only allow mapping the framebuffer phys range
            let fb_phys = crate::display::fb_phys();
            let fb_size = (crate::display::stride() as usize) * (crate::display::height() as usize) * 4;
            if phys < fb_phys || phys + size as u64 > fb_phys + fb_size as u64 {
                // allow also shm pools? For now only FB
                return !0u64;
            }
            let pages = (size + 0xFFF) / 0x1000;
            let cur = crate::scheduler::CURRENT;
            let pml4 = crate::scheduler::TASKS[cur].pml4;
            if pml4.is_null() { return !0u64; }
            for i in 0..pages {
                let pa = phys + (i as u64)*0x1000;
                let va = virt + (i as u64)*0x1000;
                let flags = crate::vm::PTE_WRITABLE | crate::vm::PTE_USER;
                if unsafe{ crate::vm::map_page(pml4, pa, va, flags)} !=0 { return !0u64; }
            }
            0
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
