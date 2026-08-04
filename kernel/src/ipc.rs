/// IPC: capability-based межпроцессное взаимодействие (zero-copy ready)
///
/// Ядро только маршрутизирует сообщения и проверяет capabilities.
/// Данные не копируются — сообщение содержит inline payload + опциональный
/// shared memory capability для zero-copy передачи больших объёмов.

use dbsos_abi::cap::*;
use dbsos_abi::ipc::*;
use crate::scheduler::{self, TaskState, TASKS, MAX_TASKS};
use crate::cap;

/// 0 = "accept from any" (проще чем IPC_ANY = !0)
const ANY_PARTNER: u64 = 0;

/// Pending messages для self-send (когда отправитель == получатель)
static mut PENDING: [u64; 32] = [0; 32];

/// Разослать широковещательное уведомление всем задачам с данным capability
pub fn broadcast(msg: &Message) {
    unsafe {
        for i in 0..MAX_TASKS {
            if TASKS[i].state != TaskState::Free && TASKS[i].id != 0 {
                // TODO: проверить, есть ли у задачи capability на приём этого типа
                // Пока просто шлём всем (для теста)
                let _ = send_inner(i, msg);
            }
        }
    }
}

/// Отправить сообщение, используя capability
pub fn send_with_cap(cap_idx: u16, msg: &Message) -> i64 {
    let cap = match cap::get(cap_idx) {
        Some(c) => c,
        None => return IPC_ERR_BAD_CAP,
    };
    if cap.cap_type != CapType::IpcTarget as u64 || cap.rights & CAP_SEND == 0 {
        return IPC_ERR_DENIED;
    }
    let target_id = cap.server_id;

    unsafe {
        let cur = scheduler::CURRENT;
        let cur_id = TASKS[cur].id;

        let dst_slot = match (0..MAX_TASKS).find(|&i| TASKS[i].id == target_id && TASKS[i].state != TaskState::Free) {
            Some(s) => s,
            None => return IPC_ERR_NO_SERVER,
        };

        // Self-send: кладём в pending, не блокируемся
        if dst_slot == cur {
            PENDING[cur] = msg as *const Message as u64;
            return IPC_OK;
        }

        // Пробуем мгновенный матчинг
        if TASKS[dst_slot].state == TaskState::BlockedRecv
            && (TASKS[dst_slot].ipc_partner == cur_id || TASKS[dst_slot].ipc_partner == ANY_PARTNER)
        {
            TASKS[dst_slot].state = TaskState::Ready;
            deliver_message(dst_slot, cur_id, msg);
            return IPC_OK;
        }

        // Блокируем отправителя
        TASKS[cur].state = TaskState::BlockedSend;
        TASKS[cur].ipc_partner = target_id;
        let msg_ptr = msg as *const Message as u64;
        TASKS[cur].ipc_val = msg_ptr;
        if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
        scheduler::yield_now();
        IPC_OK
    }
}

/// Получить сообщение, используя capability
pub fn recv_with_cap(cap_idx: u16, buf: &mut Message) -> i64 {
    let cap = match cap::get(cap_idx) {
        Some(c) => c,
        None => return IPC_ERR_BAD_CAP,
    };
    if cap.cap_type != CapType::IpcTarget as u64 || cap.rights & CAP_RECV == 0 {
        return IPC_ERR_DENIED;
    }

    unsafe {
        let cur = scheduler::CURRENT;
        let cur_id = TASKS[cur].id;

        // Self-recv: проверяем pending
        if PENDING[cur] != 0 {
            let msg_ptr = PENDING[cur] as *const Message;
            *buf = *msg_ptr;
            PENDING[cur] = 0;
            return IPC_OK;
        }

        // Ищем отправителя, который ждёт нас
        for i in 0..MAX_TASKS {
            if TASKS[i].state == TaskState::BlockedSend
                && TASKS[i].ipc_partner == cur_id
            {
                let msg_ptr = TASKS[i].ipc_val as *const Message;
                *buf = *msg_ptr;
                TASKS[i].state = TaskState::Ready;
                return IPC_OK;
            }
        }

        // Проверяем, не доставлено ли уже сообщение через fast path (deliver_message)
        if TASKS[cur].ipc_val != 0 {
            *buf = TASKS[cur].pending_msg;
            TASKS[cur].ipc_val = 0;
            return IPC_OK;
        }

        // Блокируем получателя
        TASKS[cur].state = TaskState::BlockedRecv;
        TASKS[cur].ipc_partner = ANY_PARTNER;
        if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
        scheduler::yield_now();

        if TASKS[cur].ipc_val != 0 {
            *buf = TASKS[cur].pending_msg;
            TASKS[cur].ipc_val = 0;
            IPC_OK
        } else {
            IPC_ERR_TIMEOUT
        }
    }
}

/// Отправить с блокировкой (для внутреннего использования ядром)
unsafe fn send_inner(dst_slot: usize, msg: &Message) -> i64 {
    if dst_slot >= MAX_TASKS { return IPC_ERR_NO_SERVER; }
    if TASKS[dst_slot].state == TaskState::Free { return IPC_ERR_NO_SERVER; }

    let cur = scheduler::CURRENT;
    let cur_id = TASKS[cur].id;
    let dst_id = TASKS[dst_slot].id;

    if TASKS[dst_slot].state == TaskState::BlockedRecv
        && (TASKS[dst_slot].ipc_partner == cur_id || TASKS[dst_slot].ipc_partner == ANY_PARTNER)
    {
        TASKS[dst_slot].state = TaskState::Ready;
        deliver_message(dst_slot, cur_id, msg);
        IPC_OK
    } else {
        TASKS[cur].state = TaskState::BlockedSend;
        TASKS[cur].ipc_partner = dst_id;
        TASKS[cur].ipc_val = msg as *const Message as u64;
        if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
        scheduler::yield_now();
        IPC_OK
    }
}

/// Доставить сообщение в буфер получателя
unsafe fn deliver_message(dst_slot: usize, src_id: u64, msg: &Message) {
    TASKS[dst_slot].ipc_partner = src_id;
    // Копируем сообщение в стабильный буфер (pending_msg — часть Task, не сборщика)
    TASKS[dst_slot].pending_msg = *msg;
    TASKS[dst_slot].ipc_val = 1; // флаг: сообщение в pending_msg
}

/// Создать capability для сервера (вызывается при bind)
pub fn create_server_cap(task_id: u64, server_port: u16) -> Option<u16> {
    cap::alloc_for(task_id, CapType::IpcTarget as u64, task_id, CAP_SEND | CAP_RECV, server_port as u64)
}

/// Создать capability для клиента на подключение к серверу
pub fn create_client_cap(client_task_id: u64, server_task_id: u64) -> Option<u16> {
    cap::alloc_for(client_task_id, CapType::IpcTarget as u64, server_task_id, CAP_SEND, 0)
}

pub fn init() {
    cap::init();
    // Kernel task (ID=0) получает capability на все порты
    create_server_cap(0, PORT_KERNEL);
}

pub fn shmem_test() {
    unsafe {
        // 1. Allocate a physical page
        let phys = crate::memory::palloc();
        if phys == 0 { crate::driver::uart::write_str("[SHMEM] palloc FAIL\r\n"); return; }

        // 2. Create SharedMem cap
        let cap_idx = match cap::alloc(CapType::SharedMem as u64, 0, CAP_READ | CAP_WRITE, phys) {
            Some(idx) => idx,
            None => { crate::memory::pfree(phys); crate::driver::uart::write_str("[SHMEM] cap alloc FAIL\r\n"); return; }
        };

        // 3. Verify cap data
        let cap = cap::get(cap_idx).unwrap();
        if cap.data == phys && cap.cap_type == CapType::SharedMem as u64 {
            crate::driver::uart::write_str("[SHMEM] cap OK\r\n");
        } else {
            crate::driver::uart::write_str("[SHMEM] cap verify FAIL\r\n");
            return;
        }

        // 4. Map into a user PML4 with identity map for kernel access
        let pml4 = crate::vm::create_address_space();
        if pml4.is_null() { crate::driver::uart::write_str("[SHMEM] create_pml4 FAIL\r\n"); return; }
        crate::vm::identity_map_2mb(pml4, 0, 0x100000000,
            crate::vm::PTE_WRITABLE | crate::vm::PTE_GLOBAL);
        // Use 0x40000000 which is within the identity-mapped range (triggers huge page split)
        let r = crate::vm::map_page(pml4, phys, 0x40000000u64, crate::vm::PTE_WRITABLE);
        if r != 0 { crate::driver::uart::write_str("[SHMEM] map FAIL\r\n"); return; }
        crate::driver::uart::write_str("[SHMEM] mapped OK\r\n");

        // 5. Write via virtual address
        crate::vm::switch_to(pml4);
        *(0x40000000u64 as *mut u64) = 0xDEADBEEF_CAFEBABEu64;
        crate::vm::switch_to(crate::vm::KERNEL_PML4 as *mut u64);

        // 6. Read back via physical address (kernel identity-mapped)
        let readback = *(phys as *const u64);
        if readback == 0xDEADBEEF_CAFEBABEu64 {
            crate::driver::uart::write_str("[SHMEM] zero-copy OK\r\n");
        } else {
            crate::driver::uart::write_str("[SHMEM] verify FAIL\r\n");
        }

        crate::vm::unmap_page(pml4, 0x40000000u64);
    }
}

pub mod tests {
    use super::*;

    pub fn run_test() {
        // IPC test: kernel отправляет сам себе через capability
        if let Some(cap) = cap::alloc(CapType::IpcTarget as u64, 0, CAP_SEND | CAP_RECV, 0) {
            let mut msg = Message::ping(PORT_KERNEL, PORT_KERNEL);
            msg.data[0] = 42;
            msg.length = 1;

            let result = send_with_cap(cap, &msg);
            if result == IPC_OK {
                crate::driver::uart::write_str("[IPC] send_with_cap OK\r\n");
            } else {
                crate::driver::uart::write_str("[IPC] send_with_cap FAIL: ");
                crate::driver::uart::putchar(b'0' + (-result) as u8);
                crate::driver::uart::write_str("\r\n");
                return;
            }

            let mut recv_buf = Message::empty();
            let r = recv_with_cap(cap, &mut recv_buf);
            if r == IPC_OK && recv_buf.data[0] == 42 {
                crate::driver::uart::write_str("[IPC] recv_with_cap OK (data=42)\r\n");
            } else {
                crate::driver::uart::write_str("[IPC] recv FAIL\r\n");
            }
        } else {
            crate::driver::uart::write_str("[IPC] cap alloc FAIL\r\n");
        }
    }
}

// ── Console server IPC helper ─────────────────────────────────────────
static mut CONSOLE_CLIENT_CAP: u16 = 0;

pub fn init_console_client(server_tid: u64) -> Option<u16> {
    let cap = create_client_cap(0, server_tid)?;
    unsafe { CONSOLE_CLIENT_CAP = cap; }
    Some(cap)
}

pub fn console_write(text: &[u8]) -> i64 {
    unsafe {
        let cap = CONSOLE_CLIENT_CAP;
        if cap == 0 {
            // Fall back to direct UART if console server not available
            crate::driver::uart::write_str(core::str::from_utf8(text).unwrap_or("<bin>"));
            return IPC_OK;
        }
        let mut msg = Message::empty();
        msg.msg_type = MsgType::Log as u16;
        msg.dst_port = PORT_CONSOLE;
        let len = text.len().min(PAYLOAD_SIZE);
        msg.data[..len].copy_from_slice(&text[..len]);
        msg.length = len as u16;
        send_with_cap(cap, &msg)
    }
}
