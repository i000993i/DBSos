/// Capability-based IPC: типы и права

/// Тип capability
#[repr(u64)]
pub enum CapType {
    /// Право отправлять/получать сообщения указанному серверу
    IpcTarget = 1,
    /// Право маппить shared memory (data = физический адрес страницы)
    SharedMem = 2,
    /// Право принимать прерывание (data = номер IRQ)
    Irq = 3,
    /// Право отвечать на IPC (data = id отправителя)
    IpcReply = 4,
}

/// Флаги прав
pub const CAP_SEND: u64 = 1 << 0;
pub const CAP_RECV: u64 = 1 << 1;
pub const CAP_READ: u64 = 1 << 2;
pub const CAP_WRITE: u64 = 1 << 3;

/// Слот capability (16 байт)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Cap {
    pub cap_type: u64,  // CapType как u64
    pub server_id: u64, // id сервера/порта
    pub rights: u64,    // битовая маска прав
    pub data: u64,      // доп. данные (phys page, irq num...)
}

impl Cap {
    pub const fn null() -> Self {
        Self { cap_type: 0, server_id: 0, rights: 0, data: 0 }
    }

    pub fn is_valid(&self) -> bool {
        self.cap_type != 0
    }
}

/// Максимум слотов на процесс
pub const MAX_CAPS_PER_PROCESS: usize = 64;

/// Коды ошибок IPC
pub const IPC_OK: i64 = 0;
pub const IPC_ERR_BAD_CAP: i64 = -1;
pub const IPC_ERR_NO_SERVER: i64 = -2;
pub const IPC_ERR_TIMEOUT: i64 = -3;
pub const IPC_ERR_NO_MEM: i64 = -4;
pub const IPC_ERR_DENIED: i64 = -5;
