/// IPC: протокол межпроцессного взаимодействия (capability-based, zero-copy)

/// Размер inline-данных сообщения (68 байт)
pub const PAYLOAD_SIZE: usize = 64;

/// Максимальное количество серверов/портов в системе
pub const MAX_PORTS: usize = 128;

/// Зарезервированные порты
pub const PORT_KERNEL: u16 = 0;
pub const PORT_CONSOLE: u16 = 1;
pub const PORT_FILESYSTEM: u16 = 2;
pub const PORT_NETWORK: u16 = 3;
pub const PORT_AHCI: u16 = 4;
pub const PORT_PCI: u16 = 5;
pub const PORT_DISPLAY: u16 = 6;

/// Типы сообщений
#[repr(u16)]
pub enum MsgType {
    Ping = 0x0001,
    Pong = 0x0002,
    Log = 0x0010,
    DataSend = 0x0100,
    DataRecv = 0x0101,
    IrqNotification = 0x0200,
    /// Файловая система: запрос клиента -> серверу
    FsRequest = 0x0300,
    /// Файловая система: ответ сервера -> клиенту
    FsReply = 0x0301,
    Shutdown = 0xFFFF,
}

// ── FS protocol (PORT_FILESYSTEM) ────────────────────────────────────
/// Операции файловой системы (data[0]). Ответ кладёт статус в data[1]
/// (0 = OK, 1 = ошибка) и payload в data[2..].
pub const FS_OP_LS: u8 = 1;
pub const FS_OP_CAT: u8 = 2;
pub const FS_OP_READ: u8 = 3;
pub const FS_OP_WRITE: u8 = 4;
pub const FS_OP_MKDIR: u8 = 5;
pub const FS_OP_RM: u8 = 6;
pub const FS_OP_RMDIR: u8 = 7;
pub const FS_OP_IS_DIR: u8 = 8;
pub const FS_OP_SIZE: u8 = 9;
pub const FS_OP_EXISTS: u8 = 10;

pub const FS_OK: u8 = 0;
pub const FS_ERR: u8 = 1;

/// Сообщение IPC (80 байт)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Message {
    pub src_port: u16,
    pub dst_port: u16,
    pub msg_type: u16,
    pub flags: u16,
    /// Длина данных в data (0-64)
    pub length: u16,
    /// Индекс shared-mem capability (0 = нет)
    pub shmem_cap: u16,
    /// Inline payload (64 байт)
    pub data: [u8; PAYLOAD_SIZE],
}

impl Message {
    pub const fn empty() -> Self {
        Self {
            src_port: 0, dst_port: 0, msg_type: 0, flags: 0,
            length: 0, shmem_cap: 0,
            data: [0u8; PAYLOAD_SIZE],
        }
    }

    pub fn ping(from: u16, to: u16) -> Self {
        Self {
            src_port: from, dst_port: to,
            msg_type: MsgType::Ping as u16,
            flags: 0, length: 0, shmem_cap: 0,
            data: [0u8; PAYLOAD_SIZE],
        }
    }

    pub fn pong(from: u16, to: u16) -> Self {
        Self {
            src_port: from, dst_port: to,
            msg_type: MsgType::Pong as u16,
            flags: 0, length: 0, shmem_cap: 0,
            data: [0u8; PAYLOAD_SIZE],
        }
    }
}

/// Результат IPC вызова
#[repr(C)]
pub struct IpcResult {
    pub error: i64,
    pub msg: Message,
}

impl IpcResult {
    pub const fn ok(msg: Message) -> Self {
        Self { error: 0, msg }
    }

    pub const fn err(e: i64) -> Self {
        Self { error: e, msg: Message::empty() }
    }
}
