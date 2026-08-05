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
