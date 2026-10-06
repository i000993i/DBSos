/// Linux process management — LinuxTask struct and process operations.
///
/// Each Linux process has its own address space, file descriptors,
/// and a mapping to a DBSos task slot.

use super::{*, mm::LinuxMM};
use crate::vm;
use crate::memory::{self, PAGE_SIZE};

fn uart_print(s: &str) { crate::driver::uart::write_str(s); }

fn uart_dec(mut v: u64) {
    if v == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut buf = [0u8; 20]; let mut i = 0;
    while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; crate::driver::uart::putchar(buf[i]); }
}

fn uart_hex(mut v: u64) {
    if v == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut buf = [0u8; 16]; let mut i = 0;
    while v > 0 { let n = (v & 0xF) as u8; buf[i] = if n < 10 { b'0' + n } else { b'A' + n - 10 }; v >>= 4; i += 1; }
    while i > 0 { i -= 1; crate::driver::uart::putchar(buf[i]); }
}

pub const MAX_LINUX_FDS: usize = 32;
pub const MAX_LINUX_TASKS: usize = 16;
pub const MAX_SIGNALS: usize = 32;

/// Linux signal numbers
pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;
pub const SIG_ERR: u64 = !0u64;

pub const SIGHUP: usize = 1;
pub const SIGINT: usize = 2;
pub const SIGQUIT: usize = 3;
pub const SIGKILL: usize = 9;
pub const SIGSEGV: usize = 11;
pub const SIGTERM: usize = 15;

/// Linux sigaction structure (simplified)
#[derive(Clone, Copy)]
pub struct SigAction {
    pub handler: u64,    // SA_RESTART etc in flags; handler fn ptr
    pub flags: u32,
    pub mask: u64,       // signals to block during handler
    pub restorer: u64,   // sigreturn trampoline
}

impl SigAction {
    pub const fn empty() -> Self {
        Self { handler: 0, flags: 0, mask: 0, restorer: 0 }
    }
}

/// Signal frame pushed on user stack for signal delivery
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SignalFrame {
    pub rdi: u64, pub rsi: u64, pub rdx: u64, pub rcx: u64,
    pub r8: u64, pub r9: u64, pub rax: u64, pub rbx: u64,
    pub rbp: u64, pub r10: u64, pub r11: u64, pub r12: u64,
    pub r13: u64, pub r14: u64, pub r15: u64,
    pub rflags: u64, pub rip: u64, pub rsp: u64,
    pub signal_num: u64, pub siginfo_pad: [u8; 128],
}

#[derive(Clone, Copy)]
pub struct LinuxFd {
    pub in_use: bool,
    pub path: [u8; 128],
    pub path_len: usize,
    pub offset: u64,
    pub size: u64,
    pub flags: i32,
}

impl LinuxFd {
    pub const fn empty() -> Self {
        Self {
            in_use: false, path: [0u8; 128], path_len: 0,
            offset: 0, size: 0, flags: 0,
        }
    }
}

pub struct LinuxTask {
    pub pid: u64,
    pub ppid: u64,
    pub dbsos_task: u64,          // DBSos scheduler task ID
    pub pml4_phys: u64,           // Physical PML4
    pub entry: u64,               // Entry point
    pub brk: u64,                 // Current brk
    pub mmap_base: u64,           // Next mmap address
    pub stack_top: u64,           // User stack top
    pub stack_size: u64,          // Stack size
    pub fds: [LinuxFd; MAX_LINUX_FDS],
    pub cwd: [u8; 128],
    pub cwd_len: usize,
    pub mm: LinuxMM,
    pub alive: bool,
    pub exit_status: i32,
    pub uid: u32,
    pub gid: u32,
    // Signal handling
    pub signal_handlers: [SigAction; MAX_SIGNALS],  // per-signal handlers
    pub signal_pending: u32,                          // bitmask of pending signals
    pub signal_mask: u64,                             // blocked signals mask
    pub signal_frame: SignalFrame,                    // saved context during handler
    pub in_signal_handler: bool,                      // currently inside signal handler
    pub saved_rsp: u64,                               // user RSP before signal frame push
    pub saved_rip: u64,                               // user RIP before signal frame
}

impl LinuxTask {
    pub const fn empty() -> Self {
        Self {
            pid: 0, ppid: 0, dbsos_task: 0,
            pml4_phys: 0, entry: 0, brk: 0,
            mmap_base: 0, stack_top: 0, stack_size: 0,
            fds: {
                const INIT: LinuxFd = LinuxFd::empty();
                [INIT; MAX_LINUX_FDS]
            },
            cwd: [0u8; 128], cwd_len: 0,
            mm: LinuxMM::empty(),
            alive: false, exit_status: 0,
            uid: 0, gid: 0,
            signal_handlers: {
                const INIT: SigAction = SigAction::empty();
                [INIT; MAX_SIGNALS]
            },
            signal_pending: 0,
            signal_mask: 0,
            signal_frame: SignalFrame {
                rdi: 0, rsi: 0, rdx: 0, rcx: 0,
                r8: 0, r9: 0, rax: 0, rbx: 0,
                rbp: 0, r10: 0, r11: 0, r12: 0,
                r13: 0, r14: 0, r15: 0,
                rflags: 0, rip: 0, rsp: 0,
                signal_num: 0, siginfo_pad: [0; 128],
            },
            in_signal_handler: false,
            saved_rsp: 0, saved_rip: 0,
        }
    }

    // ── File operations ───────────────────────────────────────────

    pub fn sys_open(&mut self, path_ptr: *const u8, flags: i32) -> i64 {
        let path = match copy_path_from_user(path_ptr) {
            Some(p) => p,
            None => return -(LINUX_EFAULT as i64),
        };

        // Find free FD
        let fd_idx = match self.alloc_fd() {
            Some(i) => i,
            None => return -(LINUX_EMFILE as i64),
        };

        // Open via VFS
        let fd = crate::vfs::open(&path, if flags & 0x1 != 0 { 1 } else { 0 });
        if fd < 0 {
            self.fds[fd_idx].in_use = false;
            return -(LINUX_ENOENT as i64);
        }

        // Get file size
        let size = crate::vfs::fstat(fd);

        self.fds[fd_idx].in_use = true;
        self.fds[fd_idx].path[..path.len()].copy_from_slice(&path);
        self.fds[fd_idx].path_len = path.len();
        self.fds[fd_idx].offset = 0;
        self.fds[fd_idx].size = size as u64;
        self.fds[fd_idx].flags = flags;

        fd as i64
    }

    pub fn sys_openat(&mut self, _dirfd: i32, path_ptr: *const u8, flags: i32) -> i64 {
        // Simplified: ignore dirfd, just open the path
        self.sys_open(path_ptr, flags)
    }

    pub fn sys_close(&mut self, fd: i32) -> i64 {
        if fd < 0 || fd >= MAX_LINUX_FDS as i32 { return -(LINUX_EBADF as i64); }
        let idx = fd as usize;
        if !self.fds[idx].in_use { return -(LINUX_EBADF as i64); }

        // Close via VFS
        crate::vfs::close(fd);
        self.fds[idx].in_use = false;
        0
    }

    pub fn sys_read(&mut self, fd: i32, buf: *mut u8, count: usize) -> i64 {
        if fd < 0 || fd >= MAX_LINUX_FDS as i32 { return -(LINUX_EBADF as i64); }
        let idx = fd as usize;
        if !self.fds[idx].in_use { return -(LINUX_EBADF as i64); }

        // Reopen, seek, read, close (simple approach)
        let vfd = crate::vfs::open(&self.fds[idx].path, 0);
        if vfd < 0 { return -(LINUX_ENOENT as i64); }

        crate::vfs::lseek(vfd, self.fds[idx].offset as i64, 0);

        let mut tmp_buf = [0u8; 4096];
        let to_read = count.min(4096);
        let n = crate::vfs::read(vfd, &mut tmp_buf[..to_read]);
        crate::vfs::close(vfd);

        if n <= 0 { return n as i64; }

        // Copy to user buffer
        unsafe {
            core::ptr::copy_nonoverlapping(tmp_buf.as_ptr(), buf, n as usize);
        }

        self.fds[idx].offset += n as u64;
        n as i64
    }

    pub fn sys_write(&mut self, fd: i32, buf: *const u8, count: usize) -> i64 {
        if fd == 1 || fd == 2 {
            unsafe {
                let slice = core::slice::from_raw_parts(buf, count);
                for &b in slice {
                    crate::driver::uart::putchar(b);
                }
            }
            return count as i64;
        }

        if fd < 0 || fd >= MAX_LINUX_FDS as i32 { return -(LINUX_EBADF as i64); }
        let idx = fd as usize;
        if !self.fds[idx].in_use { return -(LINUX_EBADF as i64); }

        // Write via VFS
        let vfd = crate::vfs::open(&self.fds[idx].path, 1);
        if vfd < 0 { return -(LINUX_ENOENT as i64); }

        crate::vfs::lseek(vfd, self.fds[idx].offset as i64, 0);

        let tmp_buf = unsafe { core::slice::from_raw_parts(buf, count.min(4096)) };
        let n = crate::vfs::write(vfd, tmp_buf);
        crate::vfs::close(vfd);

        if n > 0 { self.fds[idx].offset += n as u64; }
        n as i64
    }

    pub fn sys_lseek(&mut self, fd: i32, offset: i64, whence: i32) -> i64 {
        if fd < 0 || fd >= MAX_LINUX_FDS as i32 { return -(LINUX_EBADF as i64); }
        let idx = fd as usize;
        if !self.fds[idx].in_use { return -(LINUX_EBADF as i64); }

        let new_offset = match whence {
            0 => offset as u64,              // SEEK_SET
            1 => self.fds[idx].offset + offset as u64, // SEEK_CUR
            2 => self.fds[idx].size + offset as u64,   // SEEK_END
            _ => return -(LINUX_EINVAL as i64),
        };

        self.fds[idx].offset = new_offset;
        new_offset as i64
    }

    pub fn sys_dup(&mut self, old_fd: i32) -> i64 {
        if old_fd < 0 || old_fd >= MAX_LINUX_FDS as i32 { return -(LINUX_EBADF as i64); }
        let old_idx = old_fd as usize;
        if !self.fds[old_idx].in_use { return -(LINUX_EBADF as i64); }

        let new_idx = match self.alloc_fd() {
            Some(i) => i,
            None => return -(LINUX_EMFILE as i64),
        };

        self.fds[new_idx] = LinuxFd {
            in_use: true,
            path: self.fds[old_idx].path,
            path_len: self.fds[old_idx].path_len,
            offset: self.fds[old_idx].offset,
            size: self.fds[old_idx].size,
            flags: self.fds[old_idx].flags,
        };

        new_idx as i64
    }

    pub fn sys_dup2(&mut self, old_fd: i32, new_fd: i32) -> i64 {
        if old_fd < 0 || old_fd >= MAX_LINUX_FDS as i32 { return -(LINUX_EBADF as i64); }
        if new_fd < 0 || new_fd >= MAX_LINUX_FDS as i32 { return -(LINUX_EBADF as i64); }

        let old_idx = old_fd as usize;
        let new_idx = new_fd as usize;

        if !self.fds[old_idx].in_use { return -(LINUX_EBADF as i64); }

        // Close existing if open
        if self.fds[new_idx].in_use {
            crate::vfs::close(new_fd);
        }

        self.fds[new_idx] = LinuxFd {
            in_use: true,
            path: self.fds[old_idx].path,
            path_len: self.fds[old_idx].path_len,
            offset: self.fds[old_idx].offset,
            size: self.fds[old_idx].size,
            flags: self.fds[old_idx].flags,
        };

        new_fd as i64
    }

    pub fn sys_pipe(&mut self, fds_ptr: *mut i32) -> i64 {
        let idx0 = match self.alloc_fd() {
            Some(i) => i,
            None => return -(LINUX_EMFILE as i64),
        };
        let idx1 = match self.alloc_fd() {
            Some(i) => i,
            None => {
                self.fds[idx0].in_use = false;
                return -(LINUX_EMFILE as i64);
            }
        };

        // Create a pipe buffer in shared memory
        let pipe_buf = memory::palloc();
        if pipe_buf == 0 {
            self.fds[idx0].in_use = false;
            self.fds[idx1].in_use = false;
            return -(LINUX_ENOMEM as i64);
        }
        memory::memset_phys(pipe_buf, 0, memory::PAGE_SIZE);

        self.fds[idx0].in_use = true;
        let pipe_r = b"pipe_r";
        self.fds[idx0].path[..6].copy_from_slice(pipe_r);
        self.fds[idx0].path_len = 6;
        self.fds[idx0].offset = pipe_buf;  // Abuse offset for pipe buffer phys
        self.fds[idx0].size = 0;

        self.fds[idx1].in_use = true;
        let pipe_w = b"pipe_w";
        self.fds[idx1].path[..6].copy_from_slice(pipe_w);
        self.fds[idx1].path_len = 6;
        self.fds[idx1].offset = pipe_buf;
        self.fds[idx1].size = 0;

        unsafe {
            *fds_ptr = idx0 as i32;
            *fds_ptr.add(1) = idx1 as i32;
        }

        0
    }

    pub fn sys_ioctl(&mut self, _fd: i32, request: u64, arg: u64) -> i64 {
        // TIOCGWINSZ = 0x5413
        if request == 0x5413 {
            unsafe {
                let winsize = arg as *mut u16;
                *winsize = 80;           // ws_col
                *winsize.add(1) = 24;    // ws_row
                *winsize.add(2) = 800;   // ws_xpixel
                *winsize.add(3) = 600;   // ws_ypixel
            }
            return 0;
        }
        0
    }

    // ── Memory operations ─────────────────────────────────────────

    pub fn sys_brk(&mut self, new_brk: u64) -> i64 {
        if new_brk == 0 {
            return self.brk as i64;
        }

        if new_brk > self.brk {
            // Extend brk — allocate pages
            let old_brk = self.brk;
            let new_brk_aligned = (new_brk + 0xFFF) & !0xFFF;
            let pages = ((new_brk_aligned - old_brk) / PAGE_SIZE as u64) as usize;

            for i in 0..pages {
                let page = memory::palloc();
                if page == 0 { break; }
                memory::memset_phys(page, 0, PAGE_SIZE);

                let virt = old_brk + (i as u64) * PAGE_SIZE as u64;
                unsafe { vm::map_page(self.pml4_phys as *mut u64, page, virt,
                    vm::PTE_WRITABLE | vm::PTE_USER); }
            }

            self.brk = new_brk_aligned;
        }

        self.brk as i64
    }

    pub fn sys_mmap(&mut self, addr: u64, length: u64, prot: u32,
                     _flags: u32, _fd: u64, _offset: u64) -> i64 {
        let size = (length + 0xFFF) & !0xFFF;
        let pages = (size / PAGE_SIZE as u64) as usize;

        let virt = if addr == 0 {
            // Kernel chooses address
            let v = self.mmap_base;
            self.mmap_base += size;
            v
        } else {
            addr
        };

        for i in 0..pages {
            let page = memory::palloc();
            if page == 0 { return -(LINUX_ENOMEM as i64); }
            memory::memset_phys(page, 0, PAGE_SIZE);

            let v = virt + (i as u64) * PAGE_SIZE as u64;
            let mut flags = vm::PTE_WRITABLE | vm::PTE_USER;
            if prot & 1 == 0 { flags |= vm::PTE_NX; }  // No execute if PROT_EXEC not set

            unsafe { vm::map_page(self.pml4_phys as *mut u64, page, v, flags); }
        }

        virt as i64
    }

    // ── Process operations ────────────────────────────────────────

    pub fn sys_fork(&mut self) -> i64 {
        // Find free Linux task slot
        let child_slot = unsafe {
            let mut found = None;
            for i in 0..MAX_LINUX_TASKS {
                if !LINUX_TASKS[i].alive {
                    found = Some(i);
                    break;
                }
            }
            match found {
                Some(s) => s,
                None => return -(LINUX_ENOMEM as i64),
            }
        };

        // Clone address space: copy PML4 entries (sharing page table pages — CoW simplified)
        let new_pml4_phys = memory::palloc();
        if new_pml4_phys == 0 { return -(LINUX_ENOMEM as i64); }

        unsafe {
            // Copy parent's PML4
            core::ptr::copy_nonoverlapping(
                self.pml4_phys as *const u8,
                new_pml4_phys as *mut u8,
                4096,
            );

            // Create child task
            let child = &mut *LINUX_TASKS.as_mut_ptr().add(child_slot);
            child.alive = true;
            child.pid = crate::scheduler::next_task_id();
            child.ppid = self.pid;
            child.pml4_phys = new_pml4_phys;
            child.entry = self.entry;
            child.brk = self.brk;
            child.mmap_base = self.mmap_base;
            child.stack_top = self.stack_top;
            child.stack_size = self.stack_size;
            child.uid = self.uid;
            child.gid = self.gid;

            // Copy file descriptors
            for i in 0..MAX_LINUX_FDS {
                child.fds[i] = self.fds[i];
            }

            // Copy CWD
            child.cwd = self.cwd;
            child.cwd_len = self.cwd_len;

            // Copy signal handlers
            for i in 0..MAX_SIGNALS {
                child.signal_handlers[i] = self.signal_handlers[i];
            }
            child.signal_mask = self.signal_mask;
            child.signal_pending = 0;

            // Create scheduler task for child
            let parent_dbsos_task = self.dbsos_task;
            let child_dbsos_task = crate::scheduler::fork(parent_dbsos_task);
            if child_dbsos_task == 0 {
                child.alive = false;
                memory::pfree(new_pml4_phys);
                return -(LINUX_ENOMEM as i64);
            }
            child.dbsos_task = child_dbsos_task;

            uart_print("[LINUX] fork: parent pid=");
            uart_dec(self.pid);
            uart_print(" child pid=");
            uart_dec(child.pid);
            uart_print("\r\n");

            child.pid as i64
        }
    }

    /// execve: загрузить ELF через VFS и заменить образ текущего процесса.
    /// Возвращает 0 при успехе (RIP переключается планировщиком на self.entry),
    /// -ENOENT/-ENOMEM/-ENOEXEC при ошибке. Старый pml4 намеренно не освобождаем
    /// чтобы не уронить running CR3 — утечка одной таблицы до exit (безопасно).
    pub fn sys_execve(&mut self, path_ptr: *const u8) -> i64 {
        // 1. Прочитать C-string путь из user-памяти (до 128 байт)
        let mut path = [0u8; 128];
        let mut plen = 0usize;
        unsafe {
            for i in 0..128 {
                let c = core::ptr::read_volatile(path_ptr.add(i));
                path[i] = c;
                if c == 0 { break; }
                plen += 1;
            }
        }
        if plen == 0 { return -(LINUX_ENOENT as i64); }
        let upath = &path[..plen];

        // 2. Открыть через VFS и узнать размер
        let fd = crate::vfs::open(upath, 0);
        if fd < 0 { return -(LINUX_ENOENT as i64); }
        let mut st = crate::vfs::StatInfo { size: 0, is_dir: false, cluster: 0 };
        if crate::vfs::stat(upath, &mut st) != 0 { crate::vfs::close(fd); return -(LINUX_ENOENT as i64); }
        if st.is_dir { crate::vfs::close(fd); return -(LINUX_ENOEXEC as i64); }
        let fsize = st.size as usize;
        if fsize < 64 || fsize > 256 * 1024 { crate::vfs::close(fd); return -(LINUX_ENOEXEC as i64); }

        // 3. Прочитать файл в heap-буфер (FAT/tmpfs отдают всё за один read)
        let buf = unsafe { crate::heap::kmalloc(fsize) };
        if buf.is_null() { crate::vfs::close(fd); return -(LINUX_ENOMEM as i64); }
        let slice = unsafe { core::slice::from_raw_parts_mut(buf, fsize) };
        let n = crate::vfs::read(fd, slice);
        crate::vfs::close(fd);
        if n <= 0 || (n as usize) < 64 { unsafe { crate::heap::kfree(buf); } return -(LINUX_ENOEXEC as i64); }
        let read_total = n as usize;
        let elf_data = unsafe { core::slice::from_raw_parts(buf as *const u8, read_total) };

        // 4. Быстрая проверка ELF magic до тяжёлой загрузки
        if elf_data.len() < 4 || elf_data[0..4] != [0x7F, b'E', b'L', b'F'] {
            unsafe { crate::heap::kfree(buf); }
            return -(LINUX_ENOEXEC as i64);
        }

        // 5. Загрузить в новое адресное пространство
        let prog = match super::elf::load_linux_elf(elf_data) {
            Ok(p) => p,
            Err(_) => { unsafe { crate::heap::kfree(buf); } return -(LINUX_ENOEXEC as i64); }
        };
        unsafe { crate::heap::kfree(buf); }

        // 6. Заменить образ: entry/brk/mmap/stack/pml4. Сигналы сбросить, fds оставить (POSIX).
        // Старый pml4 намеренно leaked (не трогаем running CR3).
        let old_pml4 = self.pml4_phys;
        self.entry = prog.entry;
        self.pml4_phys = prog.pml4 as u64;
        self.brk = prog.brk;
        self.mmap_base = prog.mmap_base;
        self.stack_top = prog.stack_top;
        self.stack_size = prog.stack_size;
        for i in 0..MAX_SIGNALS { self.signal_handlers[i] = SigAction::empty(); }
        self.signal_pending = 0;
        self.signal_mask = 0;
        self.in_signal_handler = false;
        // 7. Синхронизация с планировщиком: следующий context switch должен
        // загрузить НОВЫЙ pml4, иначе задача продолжит жить в старом адресном пространстве.
        unsafe {
            if let Some(slot) = crate::scheduler::find_task(self.dbsos_task) {
                crate::scheduler::TASKS[slot].pml4 = prog.pml4;
                crate::scheduler::TASKS[slot].pml4_phys = prog.pml4 as u64;
            }
        }
        // 8. RIP-редирект: syscall_stub вернётся не в вызывающий код, а в entry
        // нового образа (с его стеком и CR3). Регистры обнулены стабом.
        unsafe {
            crate::syscall::request_exec_redirect(prog.entry, prog.stack_top, prog.pml4 as u64);
        }
        let _ = old_pml4;
        uart_print("[LINUX] execve ok entry=0x");
        uart_hex(prog.entry);
        uart_print(" (redirect armed)\r\n");
        0
    }

    pub fn sys_exit(&mut self, status: i32) -> i64 {
        self.exit_status = status;
        self.alive = false;

        // Close all FDs
        for i in 0..MAX_LINUX_FDS {
            if self.fds[i].in_use {
                crate::vfs::close(i as i32);
                self.fds[i].in_use = false;
            }
        }

        // Return special value to indicate exit
        -1  // Will be caught by the syscall handler
    }

    pub fn sys_wait4(&mut self, _pid: i32, _status_ptr: *mut i32, _options: i32) -> i64 {
        // Simplified: no children to wait for
        -(LINUX_ECHILD as i64)
    }

    // ── Info operations ───────────────────────────────────────────

    pub fn sys_fstat(&mut self, fd: i32, stat_buf: *mut u8) -> i64 {
        if fd < 0 || fd >= MAX_LINUX_FDS as i32 { return -(LINUX_EBADF as i64); }
        let idx = fd as usize;
        if !self.fds[idx].in_use { return -(LINUX_EBADF as i64); }

        // Fill in a minimal Linux stat structure
        unsafe {
            let buf = stat_buf as *mut u64;
            *buf = 0;                    // st_dev
            *buf.add(1) = 0;             // st_ino
            *buf.add(2) = 0o100_644;     // st_mode (regular file, rw-r--r--)
            *buf.add(3) = 1;             // st_nlink
            *buf.add(4) = self.uid as u64;  // st_uid
            *buf.add(5) = self.gid as u64;  // st_gid
            *buf.add(6) = 0;             // st_rdev
            *buf.add(7) = self.fds[idx].size;  // st_size
            *buf.add(8) = 4096;          // st_blksize
            *buf.add(9) = 0;             // st_blocks
            // atime, mtime, ctime = 0
        }

        0
    }

    pub fn sys_fstatat(&mut self, _dirfd: i32, path: *const u8, stat_buf: *mut u8, _flags: i32) -> i64 {
        // Simplified: just do fstat on a temporary fd
        let fd = self.sys_open(path, 0);
        if fd < 0 { return fd; }
        let result = self.sys_fstat(fd as i32, stat_buf);
        self.sys_close(fd as i32);
        result
    }

    pub fn sys_getcwd(&mut self, buf: *mut u8, size: usize) -> i64 {
        if self.cwd_len == 0 {
            unsafe { *buf = b'/'; }
            return 1;
        }
        let copy = self.cwd_len.min(size - 1);
        unsafe {
            core::ptr::copy_nonoverlapping(self.cwd.as_ptr(), buf, copy);
            *buf.add(copy) = 0;
        }
        buf as i64
    }

    pub fn sys_chdir(&mut self, path: *const u8) -> i64 {
        match copy_path_from_user(path) {
            Some(p) => {
                let len = p.len().min(127);
                self.cwd[..len].copy_from_slice(&p[..len]);
                self.cwd_len = len;
                0
            }
            None => -(LINUX_ENOENT as i64),
        }
    }

    pub fn sys_mkdir(&mut self, path: *const u8, _mode: u32) -> i64 {
        match copy_path_from_user(path) {
            Some(p) => {
                if crate::vfs::mkdir(&p) { 0 } else { -(LINUX_EEXIST as i64) }
            }
            None => -(LINUX_ENOENT as i64),
        }
    }

    pub fn sys_rmdir(&mut self, path: *const u8) -> i64 {
        match copy_path_from_user(path) {
            Some(p) => {
                if crate::vfs::unlink(&p) { 0 } else { -(LINUX_ENOENT as i64) }
            }
            None => -(LINUX_ENOENT as i64),
        }
    }

    pub fn sys_unlink(&mut self, path: *const u8) -> i64 {
        match copy_path_from_user(path) {
            Some(p) => {
                if crate::vfs::unlink(&p) { 0 } else { -(LINUX_ENOENT as i64) }
            }
            None => -(LINUX_ENOENT as i64),
        }
    }

    pub fn sys_getdents(&mut self, fd: i32, dirp: *mut u8, count: usize) -> i64 {
        if fd < 0 || fd >= MAX_LINUX_FDS as i32 { return -(LINUX_EBADF as i64); }
        let idx = fd as usize;
        if !self.fds[idx].in_use { return -(LINUX_EBADF as i64); }

        let path = match copy_path_from_user(self.fds[idx].path.as_ptr()) {
            Some(p) => p,
            None => return -(LINUX_ENOENT as i64),
        };

        let mut entries = [crate::vfs::DirEntry {
            name: [0u8; crate::vfs::MAX_NAME], is_dir: false, size: 0,
        }; 32];
        let n = crate::vfs::readdir(&path, &mut entries);

        let mut offset = 0usize;
        for i in 0..n as usize {
            let name_len = entries[i].name.iter().position(|&c| c == 0).unwrap_or(32);
            // Linux linux_dirent64 structure
            let reclen = 19 + name_len;  // minimal reclen
            if offset + reclen > count { break; }

            unsafe {
                let entry = dirp.add(offset) as *mut u64;
                *entry = (i as u64 + 1) as u64;  // d_ino
                *entry.add(1) = offset as u64 + reclen as u64;  // d_off
                *(entry.add(2) as *mut u16) = reclen as u16;  // d_reclen
                *(entry.add(2) as *mut u8).add(2) = if entries[i].is_dir { 4 } else { 8 }; // d_type
                core::ptr::copy_nonoverlapping(
                    entries[i].name.as_ptr(),
                    entry.add(3) as *mut u8,
                    name_len,
                );
                *(((entry.add(3) as *mut u8).add(name_len))) = 0;
            }
            offset += reclen;
        }

        if offset == 0 { 0 } else { offset as i64 }
    }

    pub fn sys_gettimeofday(&self, tv_ptr: *mut u8) -> i64 {
        let ms = crate::timer::millis();
        let secs = (ms / 1000) as i64;
        let usecs = ((ms % 1000) * 1000) as i64;
        unsafe {
            let tv = tv_ptr as *mut i64;
            *tv = secs;
            *tv.add(1) = usecs;
        }
        0
    }

    pub fn sys_clock_gettime(&self, _clock_id: u32, tp_ptr: *mut u8) -> i64 {
        let ns = crate::timer::nanos();
        let secs = (ns / 1_000_000_000) as i64;
        let nsecs = (ns % 1_000_000_000) as i64;
        unsafe {
            let tp = tp_ptr as *mut i64;
            *tp = secs;
            *tp.add(1) = nsecs;
        }
        0
    }

    pub fn sys_nanosleep(&self, req_ptr: *const u8) -> i64 {
        unsafe {
            let req = req_ptr as *const u64;
            let secs = *req;
            let nsecs = *req.add(1);
            let total_us = secs * 1_000_000 + nsecs / 1000;
            crate::timer::usleep(total_us as u64);
        }
        0
    }

    pub fn sys_uname(&self, buf: *mut u8) -> i64 {
        // Linux utsname: each field is 65 bytes
        let fields: [&[u8]; 6] = [
            b"DBSos",         // sysname
            b"localhost",     // nodename
            b"0.1.0",         // release
            b"#1 SMP",        // version
            b"x86_64",        // machine
            b"DBSos",         // domainname
        ];
        unsafe {
            let mut offset = 0usize;
            for s in &fields {
                let ptr = buf.add(offset);
                let len = s.len().min(64);
                for (i, &ch) in s.iter().take(len).enumerate() {
                    *ptr.add(i) = ch;
                }
                *ptr.add(len) = 0;
                offset += 65;
            }
        }
        0
    }

    pub fn sys_sysinfo(&self, buf: *mut u8) -> i64 {
        unsafe {
            let info = buf as *mut u64;
            *info = crate::timer::millis() / 1000;  // uptime (seconds)
            *info.add(1) = 0;   // loads[0]
            *info.add(2) = 0;   // loads[1]
            *info.add(3) = 0;   // loads[2]
            let total_ram = crate::memory::total_pages() as u64 * 4096;
            let free_ram = crate::memory::free_count() as u64 * 4096;
            *info.add(4) = total_ram;   // totalram
            *info.add(5) = free_ram;    // freeram
            *info.add(6) = 0;   // sharedram
            *info.add(7) = 0;   // bufferram
            *info.add(8) = 0;   // totalswap
            *info.add(9) = 0;   // freeswap
            *info.add(10) = 128;  // procs (number of processes)
        }
        0
    }

    pub fn sys_getrandom(&self, buf: *mut u8, count: usize) -> i64 {
        // Simple PRNG based on timer
        let ms = crate::timer::millis();
        unsafe {
            for i in 0..count {
                *buf.add(i) = ((ms >> (i % 8)) ^ (i as u64 * 0x9E3779B9)) as u8;
            }
        }
        count as i64
    }

    // ── Signal operations ──────────────────────────────────────────

    /// Register a signal handler: rt_sigaction(signum, act, oldact, sigsetsize)
    pub fn sys_rt_sigaction(&mut self, signum: usize, act_ptr: *const u8, old_ptr: *mut u8, _sigsetsize: usize) -> i64 {
        if signum >= MAX_SIGNALS || signum == 0 { return -(LINUX_EINVAL as i64); }

        // Return old handler if requested
        if !old_ptr.is_null() {
            unsafe {
                let old = old_ptr as *mut SigAction;
                *old = self.signal_handlers[signum];
            }
        }

        // Set new handler
        if !act_ptr.is_null() {
            unsafe {
                let act = &*(act_ptr as *const SigAction);
                self.signal_handlers[signum] = *act;
                uart_print("[SIGNAL] sigaction(");
                uart_dec(signum as u64);
                uart_print(") handler=0x");
                uart_hex(act.handler);
                uart_print("\r\n");
            }
        }

        0
    }

    /// Modify signal mask: rt_sigprocmask(how, set, oldset, sigsetsize)
    pub fn sys_rt_sigprocmask(&mut self, how: i32, set_ptr: *const u64, old_ptr: *mut u64, _sigsetsize: usize) -> i64 {
        // Return old mask if requested
        if !old_ptr.is_null() {
            unsafe { *old_ptr = self.signal_mask; }
        }

        if !set_ptr.is_null() {
            unsafe {
                let new_mask = *set_ptr;
                match how {
                    0 => self.signal_mask |= new_mask,   // SIG_BLOCK
                    1 => self.signal_mask &= !new_mask,  // SIG_UNBLOCK
                    2 => self.signal_mask = new_mask,     // SIG_SETMASK
                    _ => {}
                }
            }
        }
        0
    }

    /// Send signal to process: kill(pid, sig)
    pub fn sys_kill(&mut self, target_pid: i32, sig: i32) -> i64 {
        if sig < 0 || sig >= MAX_SIGNALS as i32 { return -(LINUX_EINVAL as i64); }
        let sig_bit = 1u32 << (sig as u32);

        if target_pid == 0 {
            // Signal to all processes in current group
            for i in 0..MAX_LINUX_TASKS {
                unsafe {
                    if LINUX_TASKS[i].alive {
                        LINUX_TASKS[i].signal_pending |= sig_bit;
                    }
                }
            }
        } else if target_pid == -1 {
            // Signal to all processes
            for i in 0..MAX_LINUX_TASKS {
                unsafe {
                    if LINUX_TASKS[i].alive {
                        LINUX_TASKS[i].signal_pending |= sig_bit;
                    }
                }
            }
        } else if target_pid > 0 {
            // Signal to specific PID
            let mut found = false;
            unsafe {
                for i in 0..MAX_LINUX_TASKS {
                    if LINUX_TASKS[i].alive && LINUX_TASKS[i].pid == target_pid as u64 {
                        LINUX_TASKS[i].signal_pending |= sig_bit;
                        found = true;
                    }
                }
            }
            if !found { return -(LINUX_ESRCH as i64); }
        }

        // SIGKILL and SIGTERM always terminate
        if sig == SIGKILL as i32 || sig == SIGTERM as i32 {
            self.sys_exit(sig);
        }

        0
    }

    /// Send signal to thread: tgkill(tgid, tid, sig)
    pub fn sys_tgkill(&mut self, _tgid: i32, tid: i32, sig: i32) -> i64 {
        if sig < 0 || sig >= MAX_SIGNALS as i32 { return -(LINUX_EINVAL as i64); }
        let sig_bit = 1u32 << (sig as u32);

        unsafe {
            for i in 0..MAX_LINUX_TASKS {
                if LINUX_TASKS[i].alive && LINUX_TASKS[i].pid == tid as u64 {
                    LINUX_TASKS[i].signal_pending |= sig_bit;
                    if sig == SIGKILL as i32 || sig == SIGTERM as i32 {
                        LINUX_TASKS[i].alive = false;
                    }
                    return 0;
                }
            }
        }
        -(LINUX_ESRCH as i64)
    }

    /// Check and deliver pending signals (called from syscall entry/return).
    /// Returns Some(handler_rip) if a signal handler should be called, None otherwise.
    pub fn check_signals(&mut self) -> Option<u64> {
        if self.signal_pending == 0 { return None; }

        for sig in 1..MAX_SIGNALS {
            let sig_bit = 1u32 << sig;
            if self.signal_pending & sig_bit == 0 { continue; }
            if self.signal_mask & (1u64 << sig) != 0 { continue; } // blocked

            // Clear pending
            self.signal_pending &= !sig_bit;

            let handler = self.signal_handlers[sig].handler;
            match handler {
                SIG_DFL => {
                    // Default action: terminate for fatal signals
                    match sig {
                        SIGHUP | SIGINT | SIGQUIT | SIGTERM | SIGSEGV => {
                            self.sys_exit(128 + sig as i32);
                            return None;
                        }
                        _ => {} // ignore others
                    }
                }
                SIG_IGN => {}
                _ => {
                    // Custom handler — deliver signal
                    uart_print("[SIGNAL] delivering sig=");
                    uart_dec(sig as u64);
                    uart_print(" to pid=");
                    uart_dec(self.pid);
                    uart_print("\r\n");
                    return Some(handler);
                }
            }
        }
        None
    }

    // ── Helpers ───────────────────────────────────────────────────

    fn alloc_fd(&mut self) -> Option<usize> {
        for i in 0..MAX_LINUX_FDS {
            if !self.fds[i].in_use {
                return Some(i);
            }
        }
        None
    }
}

fn copy_path_from_user(ptr: *const u8) -> Option<[u8; 128]> {
    let mut path = [0u8; 128];
    unsafe {
        for i in 0..128 {
            let b = *ptr.add(i);
            if b == 0 { return Some(path); }
            path[i] = b;
        }
    }
    None
}

/// Static Linux task table
static mut LINUX_TASKS: [LinuxTask; MAX_LINUX_TASKS] = {
    const INIT: LinuxTask = LinuxTask::empty();
    [INIT; MAX_LINUX_TASKS]
};

/// Find or create a Linux task for a given PID
pub fn find_task(pid: u64) -> Option<&'static mut LinuxTask> {
    unsafe {
        for i in 0..MAX_LINUX_TASKS {
            if LINUX_TASKS[i].alive && LINUX_TASKS[i].pid == pid {
                return Some(&mut LINUX_TASKS[i]);
            }
        }
        None
    }
}

/// Create a new Linux task
pub fn create_task() -> Option<&'static mut LinuxTask> {
    unsafe {
        for i in 0..MAX_LINUX_TASKS {
            if !LINUX_TASKS[i].alive {
                LINUX_TASKS[i].alive = true;
                LINUX_TASKS[i].pid = crate::scheduler::next_task_id();
                return Some(&mut LINUX_TASKS[i]);
            }
        }
        None
    }
}

/// Итератор для `top`: (pid, entry, alive) по всем слотам.
pub fn for_each_task<F: FnMut(usize, u64, u64, bool)>(mut f: F) {
    unsafe {
        for i in 0..MAX_LINUX_TASKS {
            f(i, LINUX_TASKS[i].pid, LINUX_TASKS[i].entry, LINUX_TASKS[i].alive);
        }
    }
}

pub fn get_current() -> Option<&'static mut LinuxTask> {
    let task_id = crate::scheduler::current_task_id();
    unsafe {
        for i in 0..MAX_LINUX_TASKS {
            if LINUX_TASKS[i].alive && LINUX_TASKS[i].dbsos_task == task_id {
                return Some(&mut LINUX_TASKS[i]);
            }
        }
    }
    None
}
