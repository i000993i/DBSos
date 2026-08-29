/// Linux Binary Compatibility Layer
///
/// Allows running statically-linked Linux x86_64 ELF binaries on DBSos.
/// Translates Linux syscalls → DBSos syscalls, provides Linux-compatible
/// memory layout, and manages Linux processes.

pub mod elf;
pub mod syscall;
pub mod task;
pub mod mm;

/// Linux x86_64 ELF constants
pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
pub const EI_CLASS: usize = 4;
pub const EI_DATA: usize = 5;
pub const ELFCLASS64: u8 = 2;
pub const ELFDATA2LSB: u8 = 1;
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;
pub const EM_X86_64: u16 = 62;
pub const PT_LOAD: u32 = 1;
pub const PT_INTERP: u32 = 3;
pub const PT_GNU_STACK: u32 = 0x6474e551;
pub const PT_GNU_RELRO: u32 = 0x6474e552;
pub const PT_GNU_EH_FRAME: u32 = 0x6474e550;

/// Linux x86_64 memory layout
pub const LINUX_MMAP_BASE: u64 = 0x0000_0040_0000;  // Traditional mmap start
pub const LINUX_STACK_TOP: u64 = 0x0000_7FFF_FFF0;   // Stack grows down
pub const LINUX_BRK_START: u64 = 0x0000_0060_0000;   // Initial brk
pub const LINUX_VSYSCALL: u64 = 0xFFFF_FFFF_FF60_0000; // vsyscall page (compat)

/// Linux x86_64 syscall numbers (most important ones)
pub const LINUX_SYS_READ: u64 = 0;
pub const LINUX_SYS_WRITE: u64 = 1;
pub const LINUX_SYS_OPEN: u64 = 2;
pub const LINUX_SYS_CLOSE: u64 = 3;
pub const LINUX_SYS_STAT: u64 = 4;
pub const LINUX_SYS_FSTAT: u64 = 5;
pub const LINUX_SYS_LSTAT: u64 = 6;
pub const LINUX_SYS_LSEEK: u64 = 8;
pub const LINUX_SYS_MMAP: u64 = 9;
pub const LINUX_SYS_MPROTECT: u64 = 10;
pub const LINUX_SYS_MUNMAP: u64 = 11;
pub const LINUX_SYS_BRK: u64 = 12;
pub const LINUX_SYS_RT_SIGACTION: u64 = 13;
pub const LINUX_SYS_RT_SIGPROCMASK: u64 = 14;
pub const LINUX_SYS_IOCTL: u64 = 16;
pub const LINUX_SYS_PREAD64: u64 = 17;
pub const LINUX_SYS_PWRITE64: u64 = 18;
pub const LINUX_SYS_READV: u64 = 19;
pub const LINUX_SYS_WRITEV: u64 = 20;
pub const LINUX_SYS_ACCESS: u64 = 21;
pub const LINUX_SYS_PIPE: u64 = 22;
pub const LINUX_SYS_SELECT: u64 = 23;
pub const LINUX_SYS_SCHED_YIELD: u64 = 24;
pub const LINUX_SYS_MSYNC: u64 = 26;
pub const LINUX_SYS_MADVISE: u64 = 28;
pub const LINUX_SYS_DUP: u64 = 32;
pub const LINUX_SYS_DUP2: u64 = 33;
pub const LINUX_SYS_NANOSLEEP: u64 = 35;
pub const LINUX_SYS_GETPID: u64 = 39;
pub const LINUX_SYS_FORK: u64 = 57;
pub const LINUX_SYS_VFORK: u64 = 58;
pub const LINUX_SYS_EXECVE: u64 = 59;
pub const LINUX_SYS_EXIT: u64 = 60;
pub const LINUX_SYS_WAIT4: u64 = 61;
pub const LINUX_SYS_KILL: u64 = 62;
pub const LINUX_SYS_UNAME: u64 = 63;
pub const LINUX_SYS_FCNTL: u64 = 72;
pub const LINUX_SYS_FLOCK: u64 = 73;
pub const LINUX_SYS_FSYNC: u64 = 74;
pub const LINUX_SYS_FTRUNCATE: u64 = 77;
pub const LINUX_SYS_GETDENTS: u64 = 78;
pub const LINUX_SYS_GETCWD: u64 = 79;
pub const LINUX_SYS_CHDIR: u64 = 80;
pub const LINUX_SYS_RENAME: u64 = 82;
pub const LINUX_SYS_MKDIR: u64 = 83;
pub const LINUX_SYS_RMDIR: u64 = 84;
pub const LINUX_SYS_LINK: u64 = 86;
pub const LINUX_SYS_UNLINK: u64 = 87;
pub const LINUX_SYS_SYMLINK: u64 = 88;
pub const LINUX_SYS_CHMOD: u64 = 90;
pub const LINUX_SYS_CHOWN: u64 = 92;
pub const LINUX_SYS_UMASK: u64 = 95;
pub const LINUX_SYS_GETTIMEOFDAY: u64 = 96;
pub const LINUX_SYS_GETRUSAGE: u64 = 98;
pub const LINUX_SYS_SYSINFO: u64 = 99;
pub const LINUX_SYS_GETUID: u64 = 102;
pub const LINUX_SYS_GETGID: u64 = 104;
pub const LINUX_SYS_GETEUID: u64 = 107;
pub const LINUX_SYS_GETEGID: u64 = 108;
pub const LINUX_SYS_GETPPID: u64 = 110;
pub const LINUX_SYS_SETPGID: u64 = 109;
pub const LINUX_SYS_GETPGID: u64 = 121;
pub const LINUX_SYS_GETSID: u64 = 124;
pub const LINUX_SYS_SETSID: u64 = 112;
pub const LINUX_SYS_SETUID: u64 = 105;
pub const LINUX_SYS_SETGID: u64 = 106;
pub const LINUX_SYS_GETGROUPS: u64 = 115;
pub const LINUX_SYS_SETGROUPS: u64 = 116;
pub const LINUX_SYS_SIGALTSTACK: u64 = 131;
pub const LINUX_SYS_RT_SIGRETURN: u64 = 15;
pub const LINUX_SYS_MMAP2: u64 = 192;  // i386 compat, not used on x86_64
pub const LINUX_SYS_MREMAP: u64 = 25;
pub const LINUX_SYS_CLONE: u64 = 56;
pub const LINUX_SYS_FUTEX: u64 = 202;
pub const LINUX_SYS_SET_TID_ADDRESS: u64 = 218;
pub const LINUX_SYS_CLOCK_GETTIME: u64 = 228;
pub const LINUX_SYS_CLOCK_GETRES: u64 = 229;
pub const LINUX_SYS_TGKILL: u64 = 234;
pub const LINUX_SYS_OPENAT: u64 = 257;
pub const LINUX_SYS_MKDIRAT: u64 = 258;
pub const LINUX_SYS_NEWFSTATAT: u64 = 262;
pub const LINUX_SYS_UNLINKAT: u64 = 263;
pub const LINUX_SYS_RENAMEAT: u64 = 264;
pub const LINUX_SYS_READLINKAT: u64 = 267;
pub const LINUX_SYS_FACCESSAT: u64 = 269;
pub const LINUX_SYS_DUP3: u64 = 292;
pub const LINUX_SYS_PIPE2: u64 = 293;
pub const LINUX_SYS_GETRANDOM: u64 = 318;
pub const LINUX_SYS_RSEQ: u64 = 334;

// Additional syscall numbers used in match patterns
pub const LINUX_SYS_POLL: u64 = 7;
pub const LINUX_SYS_PPOLL: u64 = 271;
pub const LINUX_SYS_PSELECT6: u64 = 270;
pub const LINUX_SYS_EPOLL_CREATE: u64 = 213;
pub const LINUX_SYS_EPOLL_CTL: u64 = 233;
pub const LINUX_SYS_EPOLL_WAIT: u64 = 232;
pub const LINUX_SYS_EVENTFD: u64 = 284;
pub const LINUX_SYS_EVENTFD2: u64 = 290;
pub const LINUX_SYS_FDATASYNC: u64 = 75;
pub const LINUX_SYS_SIGNALFD4: u64 = 289;
pub const LINUX_SYS_TIMERFD_CREATE: u64 = 283;
pub const LINUX_SYS_TIMERFD_SETTIME: u64 = 286;
pub const LINUX_SYS_TIMERFD_GETTIME: u64 = 287;
pub const LINUX_SYS_INOTIFY_INIT: u64 = 253;
pub const LINUX_SYS_INOTIFY_ADD_WATCH: u64 = 254;
pub const LINUX_SYS_INOTIFY_RM_WATCH: u64 = 255;

/// Linux error codes
pub const LINUX_EPERM: u64 = 1;
pub const LINUX_ENOENT: u64 = 2;
pub const LINUX_EINTR: u64 = 4;
pub const LINUX_EBADF: u64 = 9;
pub const LINUX_ENOMEM: u64 = 12;
pub const LINUX_EACCES: u64 = 13;
pub const LINUX_EFAULT: u64 = 14;
pub const LINUX_EEXIST: u64 = 17;
pub const LINUX_EINVAL: u64 = 22;
pub const LINUX_EMFILE: u64 = 24;
pub const LINUX_ENOSYS: u64 = 38;
pub const LINUX_ENOBUFS: u64 = 105;
pub const LINUX_ECHILD: u64 = 10;

/// Map a DBSos VFS error to a Linux errno
pub fn to_linux_errno(dbsos_err: i32) -> u64 {
    match dbsos_err {
        -1 => LINUX_ENOENT,
        -2 => LINUX_EACCES,
        -3 => LINUX_EBADF,
        -4 => LINUX_ENOMEM,
        -5 => LINUX_EINVAL,
        _ => LINUX_ENOSYS,
    }
}
