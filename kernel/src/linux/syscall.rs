/// Linux syscall translation — translates Linux x86_64 syscalls to DBSos syscalls.
///
/// Handles the most common syscalls needed by GNOME/X11/Wayland components.
/// Unsupported syscalls return -ENOSYS.

use super::{*, task::LinuxTask};

/// Translate a Linux syscall number into a DBSos action.
/// Returns: positive value = success (returned to user), 0 = handled internally,
/// negative = error (will be converted to Linux errno)
pub fn handle_syscall(task: &mut LinuxTask, nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> i64 {
    // Check for pending signals before handling syscall
    if let Some(handler) = task.check_signals() {
        // Signal handler needs to be called — return special value
        // The interrupt/syscall return path will push signal frame and jump to handler
        return -(handler as i64);
    }

    match nr {
        // ── I/O ───────────────────────────────────────────────────
        LINUX_SYS_READ => {
            let fd = a1 as i32;
            let buf = a2 as *mut u8;
            let count = a3 as usize;
            task.sys_read(fd, buf, count)
        }
        LINUX_SYS_WRITE => {
            let fd = a1 as i32;
            let buf = a2 as *const u8;
            let count = a3 as usize;
            task.sys_write(fd, buf, count)
        }
        LINUX_SYS_OPEN => {
            let path = a1 as *const u8;
            let flags = a2 as i32;
            task.sys_open(path, flags)
        }
        LINUX_SYS_OPENAT => {
            let dirfd = a1 as i32;
            let path = a2 as *const u8;
            let flags = a3 as i32;
            task.sys_openat(dirfd, path, flags)
        }
        LINUX_SYS_CLOSE => {
            task.sys_close(a1 as i32)
        }
        LINUX_SYS_LSEEK => {
            task.sys_lseek(a1 as i32, a2 as i64, a3 as i32)
        }
        LINUX_SYS_DUP => {
            task.sys_dup(a1 as i32)
        }
        LINUX_SYS_DUP2 | LINUX_SYS_DUP3 => {
            task.sys_dup2(a1 as i32, a2 as i32)
        }
        LINUX_SYS_PIPE | LINUX_SYS_PIPE2 => {
            task.sys_pipe(a1 as *mut i32)
        }
        LINUX_SYS_FCNTL => {
            // Most fcntl calls are no-ops (non-blocking, etc.)
            0
        }
        LINUX_SYS_IOCTL => {
            // TIOCGWINSZ, etc. — return dummy terminal size
            task.sys_ioctl(a1 as i32, a2, a3)
        }

        // ── Memory ───────────────────────────────────────────────
        LINUX_SYS_MMAP => {
            task.sys_mmap(a1, a2, a3 as u32, a4 as u32, a5, a6)
        }
        LINUX_SYS_MPROTECT => {
            0 // No-op for now (all pages already RWX)
        }
        LINUX_SYS_MUNMAP => {
            0 // No-op for now
        }
        LINUX_SYS_MADVISE => {
            0 // No-op
        }
        LINUX_SYS_BRK => {
            task.sys_brk(a1)
        }
        LINUX_SYS_MREMAP => {
            0 // No-op, return original address
            // TODO: proper remap
        }

        // ── Process ──────────────────────────────────────────────
        LINUX_SYS_GETPID => {
            task.pid as i64
        }
        LINUX_SYS_GETTID => {
            // Однопоточно: tid == pid (настоящие CLONE_THREAD — следующий этап)
            task.pid as i64
        }
        LINUX_SYS_GETPPID => {
            task.ppid as i64
        }
        LINUX_SYS_GETUID | LINUX_SYS_GETEUID => { 0 }
        LINUX_SYS_GETGID | LINUX_SYS_GETEGID => { 0 }
        LINUX_SYS_GETGROUPS => {
            // Return empty group list
            0
        }
        LINUX_SYS_SETUID | LINUX_SYS_SETGID |
        LINUX_SYS_SETPGID | LINUX_SYS_SETGROUPS => {
            0 // Silently succeed
        }
        LINUX_SYS_GETPGID | LINUX_SYS_GETSID => {
            task.pid as i64
        }
        LINUX_SYS_SETSID => {
            task.pid as i64
        }
        LINUX_SYS_UMASK => {
            0o022 // Default umask
        }
        LINUX_SYS_FORK | LINUX_SYS_VFORK => {
            task.sys_fork()
        }
        LINUX_SYS_CLONE => {
            // fork-семантика; честно предупреждаем про requested VM-sharing
            if a1 & (super::LINUX_CLONE_VM | super::LINUX_CLONE_THREAD) != 0 {
                static mut WARNED: bool = false;
                unsafe {
                    if !WARNED {
                        WARNED = true;
                        crate::driver::uart::write_str("[LINUX] clone: VM/thread sharing -> fork copy (TODO)\r\n");
                    }
                }
            }
            task.sys_fork()
        }
        LINUX_SYS_EXECVE => {
            task.sys_execve(a1 as *const u8)
        }
        LINUX_SYS_EXIT => {
            task.sys_exit(a1 as i32)
        }
        LINUX_SYS_WAIT4 => {
            task.sys_wait4(a1 as i32, a2 as *mut i32, a3 as i32)
        }
        LINUX_SYS_KILL => {
            task.sys_kill(a1 as i32, a2 as i32)
        }
        LINUX_SYS_TGKILL => {
            task.sys_tgkill(a1 as i32, a2 as i32, a3 as i32)
        }
        LINUX_SYS_SCHED_YIELD => {
            crate::scheduler::context::yield_now();
            0
        }

        // ── Filesystem ───────────────────────────────────────────
        LINUX_SYS_STAT | LINUX_SYS_LSTAT | LINUX_SYS_FSTAT => {
            task.sys_fstat(a1 as i32, a2 as *mut u8)
        }
        LINUX_SYS_NEWFSTATAT => {
            task.sys_fstatat(a1 as i32, a2 as *const u8, a3 as *mut u8, a4 as i32)
        }
        LINUX_SYS_GETCWD => {
            task.sys_getcwd(a1 as *mut u8, a2 as usize)
        }
        LINUX_SYS_CHDIR => {
            task.sys_chdir(a1 as *const u8)
        }
        LINUX_SYS_MKDIR | LINUX_SYS_MKDIRAT => {
            task.sys_mkdir(a1 as *const u8, a2 as u32)
        }
        LINUX_SYS_RMDIR => {
            task.sys_rmdir(a1 as *const u8)
        }
        LINUX_SYS_UNLINK | LINUX_SYS_UNLINKAT => {
            task.sys_unlink(a1 as *const u8)
        }
        LINUX_SYS_RENAME | LINUX_SYS_RENAMEAT => {
            0 // No-op
        }
        LINUX_SYS_ACCESS | LINUX_SYS_FACCESSAT => {
            0 // Silently succeed
        }
        LINUX_SYS_GETDENTS => {
            task.sys_getdents(a1 as i32, a2 as *mut u8, a3 as usize)
        }
        LINUX_SYS_CHMOD | LINUX_SYS_CHOWN => {
            0 // No-op
        }

        // ── Time ─────────────────────────────────────────────────
        LINUX_SYS_GETTIMEOFDAY => {
            task.sys_gettimeofday(a1 as *mut u8)
        }
        LINUX_SYS_CLOCK_GETTIME => {
            task.sys_clock_gettime(a1 as u32, a2 as *mut u8)
        }
        LINUX_SYS_CLOCK_GETRES => {
            if a2 != 0 {
                // resolution = 100ns (HPET 10MHz)
                unsafe {
                    let tv = a2 as *mut u64;
                    *tv = 0;           // tv_sec
                    *tv.add(1) = 100;  // tv_nsec (100ns)
                }
            }
            0
        }
        LINUX_SYS_NANOSLEEP => {
            task.sys_nanosleep(a1 as *const u8)
        }

        // ── Signals ──────────────────────────────────────────────
        LINUX_SYS_RT_SIGACTION => {
            task.sys_rt_sigaction(a1 as usize, a2 as *const u8, a3 as *mut u8, a4 as usize)
        }
        LINUX_SYS_RT_SIGPROCMASK => {
            task.sys_rt_sigprocmask(a1 as i32, a2 as *const u64, a3 as *mut u64, a4 as usize)
        }
        LINUX_SYS_SIGALTSTACK => {
            0 // Ignore for now
        }
        LINUX_SYS_RT_SIGRETURN => {
            // Signal handler returned — restore context
            // In real implementation this restores registers from signal frame
            0
        }

        // ── System info ──────────────────────────────────────────
        LINUX_SYS_UNAME => {
            task.sys_uname(a1 as *mut u8)
        }
        LINUX_SYS_SYSINFO => {
            task.sys_sysinfo(a1 as *mut u8)
        }
        LINUX_SYS_GETRUSAGE => {
            // Zero out rusage
            unsafe {
                let ptr = a2 as *mut u8;
                for i in 0..112 {  // sizeof(struct rusage) ≈ 112
                    *ptr.add(i) = 0;
                }
            }
            0
        }
        LINUX_SYS_GETRANDOM => {
            task.sys_getrandom(a1 as *mut u8, a2 as usize)
        }

        // ── Misc ─────────────────────────────────────────────────
        LINUX_SYS_FUTEX => {
            // Simplified: just return 0 (no contention)
            0
        }
        LINUX_SYS_SET_TID_ADDRESS => {
            task.pid as i64
        }
        LINUX_SYS_FSYNC | LINUX_SYS_FDATASYNC => {
            0
        }
        LINUX_SYS_FTRUNCATE => {
            0
        }
        LINUX_SYS_FLOCK => {
            0
        }
        LINUX_SYS_MSYNC => {
            0
        }
        LINUX_SYS_RSEQ => {
            0 // Ignore restartable sequences
        }
        LINUX_SYS_SELECT | LINUX_SYS_PSELECT6 => {
            // Simplified: always ready
            1
        }
        LINUX_SYS_POLL | LINUX_SYS_PPOLL => {
            1
        }
        LINUX_SYS_EPOLL_CREATE | LINUX_SYS_EPOLL_CTL | LINUX_SYS_EPOLL_WAIT => {
            0
        }
        LINUX_SYS_EVENTFD2 | LINUX_SYS_EVENTFD => {
            0
        }
        LINUX_SYS_SIGNALFD4 | LINUX_SYS_TIMERFD_CREATE |
        LINUX_SYS_TIMERFD_SETTIME | LINUX_SYS_TIMERFD_GETTIME => {
            0
        }
        LINUX_SYS_INOTIFY_INIT | LINUX_SYS_INOTIFY_ADD_WATCH |
        LINUX_SYS_INOTIFY_RM_WATCH => {
            0
        }

        // ── Unknown ──────────────────────────────────────────────
        _ => {
            //uart_print("[LINUX] Unhandled syscall: ");
            //uart_print_hex(nr);
            //uart_print("\r\n");
            -(LINUX_ENOSYS as i64)
        }
    }
}
