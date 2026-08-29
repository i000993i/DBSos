/// Linux process management — LinuxTask struct and process operations.
///
/// Each Linux process has its own address space, file descriptors,
/// and a mapping to a DBSos task slot.

use super::{*, mm::LinuxMM};
use crate::vm;
use crate::memory::{self, PAGE_SIZE};

fn uart_print(s: &str) { crate::driver::uart::write_str(s); }

pub const MAX_LINUX_FDS: usize = 32;
pub const MAX_LINUX_TASKS: usize = 16;

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
        // Simplified fork: create a new Linux task with cloned address space
        // For now, return error (proper fork requires deep clone)
        uart_print("[LINUX] fork() called — returning -ENOSYS\r\n");
        -(LINUX_ENOSYS as i64)
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
        let ms = crate::timer::millis();
        let secs = (ms / 1000) as i64;
        let nsecs = ((ms % 1000) * 1_000_000) as i64;
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
