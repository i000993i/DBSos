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
/// fork(): создать копию текущего процесса. Ребёнок получает 0, родитель — pid
/// ребёнка, при ошибке — -1 (u64::MAX). Ребёнок продолжает с той же инструкции.
pub const SYS_FORK: u64 = 6;
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
/// Установить пользовательский обработчик сигнала. arg1 = sig, arg2 = handler.
/// Возвращает предыдущий обработчик (0 = не было). SIGKILL перехватить нельзя.
pub const SYS_SIGNAL: u64 = 22;
/// Вернуться из обработчика сигнала в прерванный контекст.
pub const SYS_SIGRETURN: u64 = 23;
/// Заснуть текущий процесс. arg1 = миллисекунды. Возвращает 0.
pub const SYS_SLEEP: u64 = 24;

/// Сигналы
pub const SIG_KILL: u64 = 9;
pub const SIG_USR1: u64 = 10;
pub const SIG_TERM: u64 = 15;

/// Флаги IPC
pub const IPC_NONBLOCK: u64 = 1 << 0;
pub const IPC_SHMEM: u64 = 1 << 1;
