/// Системные вызовы DBSos (capability-based IPC)

/// Номера syscall (RAX)
pub const SYS_EXIT: u64 = 0;
/// Возвращает PID текущего процесса (RAX)
pub const SYS_GETPID: u64 = 3;
/// Ожидание завершения дочернего процесса.
/// arg1 = pid ребёнка (0 = любой), arg2 = ptr на u64-статус.
/// Возвращает PID дочернего процесса (0 = нет такого/живых детей).
pub const SYS_WAIT: u64 = 4;
/// Убить процесс. arg1 = pid, arg2 = код выхода/сигнал.
/// Возвращает 1 при успехе, 0 если процесса нет (или это сам вызывающий).
pub const SYS_KILL: u64 = 5;
/// Legacy u64-based IPC (для ring-3 теста)
pub const SYS_IPC_SEND_LEGACY: u64 = 1;
pub const SYS_IPC_RECV_LEGACY: u64 = 2;
/// Capability-based IPC
pub const SYS_IPC_SEND: u64 = 11;
pub const SYS_IPC_RECV: u64 = 12;
pub const SYS_CAP_GRANT: u64 = 13;
pub const SYS_CAP_ATTACH_IRQ: u64 = 14;
pub const SYS_SHMEM_MAP: u64 = 15;
pub const SYS_SHMEM_CREATE: u64 = 16;
pub const SYS_MMIO_MAP: u64 = 17;
pub const SYS_PCI_READ: u64 = 18;
pub const SYS_PCI_WRITE: u64 = 19;
pub const SYS_CAP_GET_DATA: u64 = 21;
pub const SYS_LOG_WRITE: u64 = 20;
/// Процессная группа: получить pgid процесса. arg1 = pid (0 = текущий).
/// Возвращает pgid, 0 если процесса нет.
pub const SYS_GETPGID: u64 = 7;
/// Процессная группа: установить pgid. arg1 = pid (0 = текущий), arg2 = pgid (0 = pid).
/// Разрешено только для себя или прямого ребёнка. Возвращает 0 или IPC_ERR.
pub const SYS_SETPGID: u64 = 8;
/// Послать сигнал всем процессам группы. arg1 = pgid, arg2 = сигнал.
/// Возвращает количество процессов, получивших сигнал.
pub const SYS_KILLPG: u64 = 9;

// ── File I/O syscalls ──────────────────────────────────────────────
/// Open a file. arg1 = path ptr, arg2 = path len, arg3 = flags (O_RDONLY=0, O_WRONLY=1, O_RDWR=2).
/// Returns file descriptor (>= 0) or -1 on error.
pub const SYS_OPEN: u64 = 30;
/// Read from file descriptor. arg1 = fd, arg2 = buf ptr, arg3 = count.
/// Returns bytes read or -1 on error.
pub const SYS_READ: u64 = 31;
/// Write to file descriptor. arg1 = fd, arg2 = buf ptr, arg3 = count.
/// Returns bytes written or -1 on error.
pub const SYS_WRITE: u64 = 32;
/// Close file descriptor. arg1 = fd.
/// Returns 0 on success or -1 on error.
pub const SYS_CLOSE: u64 = 33;
/// Get file size. arg1 = fd.
/// Returns file size or -1 on error.
pub const SYS_FSTAT: u64 = 34;
/// Seek in file. arg1 = fd, arg2 = offset, arg3 = whence (0=SET, 1=CUR, 2=END).
/// Returns new offset or -1 on error.
pub const SYS_LSEEK: u64 = 35;
/// Read directory entries. arg1 = path ptr, arg2 = path len, arg3 = entries buf ptr, arg4 = max entries.
/// Returns number of entries or -1 on error.
pub const SYS_READDIR: u64 = 36;
/// Create directory. arg1 = path ptr, arg2 = path len.
/// Returns 0 on success or -1 on error.
pub const SYS_MKDIR: u64 = 37;
/// Remove directory. arg1 = path ptr, arg2 = path len.
/// Returns 0 on success or -1 on error.
pub const SYS_RMDIR: u64 = 38;
/// Delete file. arg1 = path ptr, arg2 = path len.
/// Returns 0 on success or -1 on error.
pub const SYS_UNLINK: u64 = 39;
/// Duplicate file descriptor. arg1 = old fd.
/// Returns new fd or -1 on error.
pub const SYS_DUP: u64 = 43;
/// Duplicate file descriptor to specific number. arg1 = old fd, arg2 = new fd.
/// Returns new fd or -1 on error.
pub const SYS_DUP2: u64 = 44;
/// Create a pipe. arg1 = fds array ptr (2 u32s: [read_fd, write_fd]).
/// Returns 0 on success or -1 on error.
pub const SYS_PIPE: u64 = 45;
/// Get current working directory. arg1 = buf ptr, arg2 = buf size.
/// Returns 0 on success or -1 on error.
pub const SYS_GETCWD: u64 = 46;
/// Change working directory. arg1 = path ptr, arg2 = path len.
/// Returns 0 on success or -1 on error.
pub const SYS_CHDIR: u64 = 47;
// ── Socket API syscalls ────────────────────────────────────────────
/// Create a socket. arg1 = domain (AF_INET=2), arg2 = type (SOCK_STREAM=1), arg3 = protocol (0).
/// Returns socket fd (>= 0) or -1 on error.
pub const SYS_SOCKET: u64 = 50;
/// Bind a socket. arg1 = fd, arg2 = sockaddr ptr, arg3 = addrlen.
/// Returns 0 on success or -1 on error.
pub const SYS_BIND: u64 = 51;
/// Listen on a socket. arg1 = fd, arg2 = backlog.
/// Returns 0 on success or -1 on error.
pub const SYS_LISTEN: u64 = 52;
/// Accept a connection. arg1 = fd, arg2 = sockaddr ptr, arg3 = addrlen ptr.
/// Returns new socket fd or -1 on error.
pub const SYS_ACCEPT: u64 = 53;
/// Connect to a remote. arg1 = fd, arg2 = sockaddr ptr, arg3 = addrlen.
/// Returns 0 on success or -1 on error.
pub const SYS_CONNECT: u64 = 54;
/// Send data. arg1 = fd, arg2 = buf ptr, arg3 = len, arg4 = flags.
/// Returns bytes sent or -1 on error.
pub const SYS_SEND: u64 = 55;
/// Receive data. arg1 = fd, arg2 = buf ptr, arg3 = len, arg4 = flags.
/// Returns bytes received or -1 on error.
pub const SYS_RECV: u64 = 56;
/// Shutdown a socket. arg1 = fd, arg2 = how (0=RD, 1=WR, 2=BOTH).
/// Returns 0 on success or -1 on error.
pub const SYS_SHUTDOWN: u64 = 57;

/// File open flags
pub const O_RDONLY: u64 = 0;
pub const O_WRONLY: u64 = 1;
pub const O_RDWR: u64 = 2;
pub const O_CREAT: u64 = 0x100;
pub const O_TRUNC: u64 = 0x200;

/// Maximum open file descriptors per process
pub const MAX_FDS: usize = 16;
/// Установить пользовательский обработчик сигнала. arg1 = sig, arg2 = handler.
/// Возвращает предыдущий обработчик (0 = не было). SIGKILL перехватить нельзя.
pub const SYS_SIGNAL: u64 = 22;
/// Вернуться из обработчика сигнала в прерванный контекст.
pub const SYS_SIGRETURN: u64 = 23;
/// Заснуть текущий процесс. arg1 = миллисекунды. Возвращает 0.
pub const SYS_SLEEP: u64 = 24;

// ── Process management syscalls ────────────────────────────────────
/// fork(): duplicate current process. Returns child PID to parent, 0 to child.
pub const SYS_FORK: u64 = 40;
/// exec(): replace process image with ELF file. arg1 = path ptr, arg2 = path len.
/// Does not return on success. Returns -1 on error.
pub const SYS_EXEC: u64 = 41;
/// waitpid(): wait for child process. arg1 = child pid (0 = any), arg2 = status ptr.
/// Returns child PID or -1 on error.
pub const SYS_WAITPID: u64 = 42;

/// Сигналы
pub const SIG_KILL: u64 = 9;
pub const SIG_USR1: u64 = 10;
pub const SIG_TERM: u64 = 15;

/// Флаги IPC
pub const IPC_NONBLOCK: u64 = 1 << 0;
pub const IPC_SHMEM: u64 = 1 << 1;

/// Multi-user syscalls
pub const SYS_GETUID: u64 = 60;
pub const SYS_GETGID: u64 = 61;
pub const SYS_SETUID: u64 = 62;
pub const SYS_CHMOD: u64 = 63;
pub const SYS_CHOWN: u64 = 64;

/// Wayland syscalls (syscall-based, no socket)
pub const SYS_WL_SHM_CREATE: u64 = 70;
pub const SYS_WL_SHM_DESTROY: u64 = 71;
pub const SYS_WL_SURFACE_CREATE: u64 = 72;
pub const SYS_WL_SURFACE_DESTROY: u64 = 73;
pub const SYS_WL_SURFACE_ATTACH: u64 = 74;
pub const SYS_WL_SURFACE_COMMIT: u64 = 75;
pub const SYS_WL_SURFACE_SET_POS: u64 = 76;

/// DBS-GR Ring3 framebuffer
pub const SYS_FB_INFO: u64 = 77;
pub const SYS_FB_MAP: u64 = 78;
