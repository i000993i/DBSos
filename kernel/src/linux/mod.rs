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

/// ASLR entropy: 28 bits → 256 MB randomization range for mmap/stack
const ASLR_BITS: u32 = 28;
const ASLR_MASK: u64 = (1 << ASLR_BITS) - 1;

/// Simple xorshift64 PRNG — seeded from HPET timer + CSPRNG-like entropy
static mut ASLR_SEED: u64 = 0;

pub fn aslr_init() {
    let t = crate::timer::ticks();
    let t2 = crate::timer::millis();
    let rdtsc: u64;
    unsafe {
        let low: u32;
        let high: u32;
        core::arch::asm!("rdtsc", out("eax") low, out("edx") high);
        rdtsc = (high as u64) << 32 | low as u64;
    }
    unsafe {
        ASLR_SEED = t ^ (t2 << 17) ^ rdtsc ^ 0xDEAD_BEEF_CAFE_BABE;
        if ASLR_SEED == 0 { ASLR_SEED = 1; }
    }
}

fn aslr_next() -> u64 {
    unsafe {
        // xorshift64
        ASLR_SEED ^= ASLR_SEED << 13;
        ASLR_SEED ^= ASLR_SEED >> 7;
        ASLR_SEED ^= ASLR_SEED << 17;
        ASLR_SEED & ASLR_MASK
    }
}

/// Randomize mmap base for a new process (ASLR)
pub fn randomize_mmap_base() -> u64 {
    LINUX_MMAP_BASE + (aslr_next() << 12)  // page-aligned, up to 256MB above default
}

/// Randomize stack top for a new process (ASLR)
pub fn randomize_stack_top() -> u64 {
    // Stack: 0x7FFF_FFF0 minus random offset (up to 256MB)
    LINUX_STACK_TOP - (aslr_next() << 12)
}

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
pub const LINUX_SYS_GETTID: u64 = 186;

// clone(2) флаги: пока поддерживается fork-семантика (копия), VM-sharing — TODO.
pub const LINUX_CLONE_VM: u64 = 0x100;
pub const LINUX_CLONE_THREAD: u64 = 0x10000;
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
pub const LINUX_ESRCH: u64 = 3;
pub const LINUX_ENOEXEC: u64 = 8;

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

/// Spawn a Linux ELF binary from VFS path as a new Ring3 task.
/// Читает файл (до 256KB), load_linux_elf, GDT/TSS, spawn_user + LinuxTask.
/// Возвращает pid или 0 при ошибке. Безопасный: при любой ошибке чистит ресурсы.
pub fn spawn_file(path: &[u8]) -> u64 {
    fn up(s: &str) { crate::driver::uart::write_str(s); }
    if path.is_empty() || path.len() > 127 { return 0; }
    let fd = crate::vfs::open(path, 0);
    if fd < 0 { up("[linux-spawn] not found\r\n"); return 0; }
    let mut st = crate::vfs::StatInfo { size: 0, is_dir: false, cluster: 0 };
    if crate::vfs::stat(path, &mut st) != 0 { crate::vfs::close(fd); return 0; }
    if st.is_dir { crate::vfs::close(fd); return 0; }
    let fsize = st.size as usize;
    if fsize < 64 || fsize > 256 * 1024 { crate::vfs::close(fd); up("[linux-spawn] bad size\r\n"); return 0; }
    let buf = unsafe { crate::heap::kmalloc(fsize) };
    if buf.is_null() { crate::vfs::close(fd); return 0; }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, fsize) };
    let n = crate::vfs::read(fd, slice);
    crate::vfs::close(fd);
    if n <= 0 || (n as usize) < 64 { unsafe { crate::heap::kfree(buf); } return 0; }
    let elf_data = unsafe { core::slice::from_raw_parts(buf as *const u8, n as usize) };
    if elf_data[0..4] != [0x7F, b'E', b'L', b'F'] {
        unsafe { crate::heap::kfree(buf); }
        up("[linux-spawn] not ELF\r\n");
        return 0;
    }
    let prog = match elf::load_linux_elf(elf_data) {
        Ok(p) => p,
        Err(e) => {
            unsafe { crate::heap::kfree(buf); }
            up("[linux-spawn] load: "); up(e); up("\r\n");
            return 0;
        }
    };
    unsafe { crate::heap::kfree(buf); }

    // GDT/TSS для Ring3 (как в native elf::load_and_spawn)
    let gdt_phys = crate::memory::palloc();
    let tss_page = crate::memory::palloc();
    if gdt_phys == 0 || tss_page == 0 {
        if gdt_phys != 0 { crate::memory::pfree(gdt_phys); }
        if tss_page != 0 { crate::memory::pfree(tss_page); }
        return 0;
    }
    unsafe {
        let orig = crate::vm::current_pml4() as *mut u64;
        crate::vm::switch_to(prog.pml4);
        let ksp = crate::syscall::sys_krsp;
        crate::syscall::setup_user_gdt_tss(gdt_phys, tss_page, ksp, false);
        core::arch::asm!("lgdt [{ptr}]", ptr = in(reg) &crate::interrupts::GdtPacked {
            limit: (8*8-1) as u16, base: gdt_phys } as *const _ as u64);
        crate::vm::switch_to(orig);
        crate::syscall::sys_kret = crate::syscall::ring3_done as *const () as u64;
    }
    let tid = unsafe {
        crate::scheduler::spawn_user(
            prog.entry, prog.stack_top, prog.pml4,
            gdt_phys, tss_page, prog.code_phys, prog.stack_phys,
        )
    };
    let tid = match tid { Some(t) => t, None => {
        crate::memory::pfree(gdt_phys); crate::memory::pfree(tss_page);
        up("[linux-spawn] no task slot\r\n"); return 0;
    }};
    // LinuxTask метаданные
    let lt = match task::create_task() {
        Some(t) => t,
        None => { up("[linux-spawn] no linux slot\r\n"); return 0; }
    };
    lt.entry = prog.entry;
    lt.pml4_phys = prog.pml4 as u64;
    lt.brk = prog.brk;
    lt.mmap_base = prog.mmap_base;
    lt.stack_top = prog.stack_top;
    lt.stack_size = prog.stack_size;
    lt.dbsos_task = tid;
    let pid = lt.pid;
    up("[linux-spawn] pid="); {
        let mut v = pid; let mut b = [0u8; 20]; let mut i = 0;
        if v == 0 { crate::driver::uart::putchar(b'0'); }
        while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
        while i > 0 { i -= 1; crate::driver::uart::putchar(b[i]); }
    }
    up("\r\n");
    pid
}
