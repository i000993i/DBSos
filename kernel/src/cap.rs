/// Capability Manager: права доступа для IPC, IRQ, shared memory
///
/// Каждый процесс имеет таблицу capabilities (64 слота).
/// Capabilities — единственный способ получить доступ к ресурсам.

use dbsos_abi::cap::*;

/// Таблица capabilities для одного процесса
struct CapTable {
    caps: [Cap; MAX_CAPS_PER_PROCESS],
}

impl CapTable {
    const fn new() -> Self {
        Self { caps: [Cap::null(); MAX_CAPS_PER_PROCESS] }
    }

    fn alloc(&mut self, cap_type: u64, server_id: u64, rights: u64, data: u64) -> Option<u16> {
        for (i, slot) in self.caps.iter_mut().enumerate() {
            if !slot.is_valid() {
                *slot = Cap { cap_type, server_id, rights, data };
                return Some(i as u16);
            }
        }
        None
    }

    fn get(&self, idx: u16) -> Option<&Cap> {
        let i = idx as usize;
        if i >= MAX_CAPS_PER_PROCESS { return None; }
        if !self.caps[i].is_valid() { return None; }
        Some(&self.caps[i])
    }

    fn free(&mut self, idx: u16) {
        let i = idx as usize;
        if i < MAX_CAPS_PER_PROCESS {
            self.caps[i] = Cap::null();
        }
    }

    fn validate(&self, idx: u16, required_rights: u64) -> bool {
        self.get(idx).map(|c| c.rights & required_rights == required_rights).unwrap_or(false)
    }
}

/// Глобальная таблица: индекс слота задачи в TASKS → её CapTable
static mut CAP_TABLES: [CapTable; 32] = [const { CapTable::new() }; 32];

fn slot_for_task(task_id: u64) -> Option<usize> {
    // Ищем слот задачи по её ID
    unsafe {
        for i in 0..crate::scheduler::MAX_TASKS {
            if crate::scheduler::TASKS[i].state != crate::scheduler::TaskState::Free
                && crate::scheduler::TASKS[i].id == task_id
            {
                return Some(i);
            }
        }
    }
    None
}

fn current_slot() -> usize {
    unsafe { crate::scheduler::CURRENT }
}

/// Alloc: создать capability для текущей задачи
pub fn alloc(cap_type: u64, server_id: u64, rights: u64, data: u64) -> Option<u16> {
    unsafe { CAP_TABLES[current_slot()].alloc(cap_type, server_id, rights, data) }
}

/// Alloc для указанной задачи (используется при создании процесса)
pub fn alloc_for(task_id: u64, cap_type: u64, server_id: u64, rights: u64, data: u64) -> Option<u16> {
    slot_for_task(task_id).and_then(|slot| unsafe { CAP_TABLES[slot].alloc(cap_type, server_id, rights, data) })
}

/// Get capability по индексу для текущей задачи
pub fn get(idx: u16) -> Option<&'static Cap> {
    unsafe { CAP_TABLES[current_slot()].get(idx) }
}

/// Validate: проверить права для текущей задачи
pub fn validate(idx: u16, required_rights: u64) -> bool {
    unsafe { CAP_TABLES[current_slot()].validate(idx, required_rights) }
}

/// Передать capability от текущей задачи к указанной
pub fn transfer(dst_task_id: u64, cap_idx: u16) -> i64 {
    let src_slot = current_slot();
    let cap = unsafe { CAP_TABLES[src_slot].get(cap_idx) };
    let cap = match cap { Some(c) => *c, None => return IPC_ERR_BAD_CAP };

    let dst_slot = match slot_for_task(dst_task_id) {
        Some(s) => s,
        None => return IPC_ERR_NO_SERVER,
    };

    unsafe {
        match CAP_TABLES[dst_slot].alloc(cap.cap_type, cap.server_id, cap.rights, cap.data) {
            Some(_) => {
                // Отзываем у себя (передача владения)
                CAP_TABLES[src_slot].free(cap_idx);
                IPC_OK
            }
            None => IPC_ERR_NO_MEM,
        }
    }
}

/// Копировать capability (оба процесса имеют копию)
pub fn duplicate(dst_task_id: u64, cap_idx: u16) -> i64 {
    let cap = match get(cap_idx) {
        Some(c) => *c,
        None => return IPC_ERR_BAD_CAP,
    };
    let dst_slot = match slot_for_task(dst_task_id) {
        Some(s) => s,
        None => return IPC_ERR_NO_SERVER,
    };
    unsafe {
        match CAP_TABLES[dst_slot].alloc(cap.cap_type, cap.server_id, cap.rights, cap.data) {
            Some(_) => IPC_OK,
            None => IPC_ERR_NO_MEM,
        }
    }
}

/// Освободить все capabilities задачи
pub fn free_all(task_id: u64) {
    if let Some(slot) = slot_for_task(task_id) {
        unsafe { CAP_TABLES[slot] = CapTable::new(); }
    }
}

/// Инициализация: добавить базовые capabilities для задачи 0 (kernel)
pub fn init() {
    // У задачи 0 будут все права
    // Kernel серверу доступны все порты
    unsafe {
        // Capability на самого себя (IPC)
        CAP_TABLES[0].alloc(CapType::IpcTarget as u64, 0, CAP_SEND | CAP_RECV, 0);
    }
}
