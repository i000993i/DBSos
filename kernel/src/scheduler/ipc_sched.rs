// IPC scheduling: send/recv blocking + matching

use super::{TaskState, MAX_TASKS, CURRENT, TASKS, IPC_ERR, IPC_ANY};

unsafe fn try_match_send(cur: usize, dst_id: u64, msg_ptr: u64) -> bool {
    if let Some(dst_slot) = super::find_task(dst_id) {
        if TASKS[dst_slot].state == TaskState::BlockedRecv
            && (TASKS[dst_slot].ipc_partner == IPC_ANY
                || TASKS[dst_slot].ipc_partner == TASKS[cur].id)
        {
            TASKS[dst_slot].ipc_val = msg_ptr;
            TASKS[dst_slot].state = TaskState::Ready;
            return true;
        }
    }
    false
}

unsafe fn try_match_send_u64(cur: usize, dst_id: u64, val: u64) -> bool {
    if let Some(dst_slot) = super::find_task(dst_id) {
        if TASKS[dst_slot].state == TaskState::BlockedRecv
            && (TASKS[dst_slot].ipc_partner == IPC_ANY
                || TASKS[dst_slot].ipc_partner == TASKS[cur].id)
        {
            TASKS[dst_slot].ipc_val = val;
            TASKS[dst_slot].state = TaskState::Ready;
            return true;
        }
    }
    false
}

unsafe fn try_match_recv(cur: usize, src_id: u64) -> Option<u64> {
    let maybe_slot = if src_id == 0 {
        (0..MAX_TASKS).find(|&i| {
            TASKS[i].state == TaskState::BlockedSend
                && (TASKS[i].ipc_partner == IPC_ANY || TASKS[i].ipc_partner == TASKS[cur].id)
        })
    } else {
        super::find_task(src_id).filter(|&s| {
            TASKS[s].state == TaskState::BlockedSend
                && (TASKS[s].ipc_partner == IPC_ANY || TASKS[s].ipc_partner == TASKS[cur].id)
        })
    };
    if let Some(src_slot) = maybe_slot {
        let msg_ptr = TASKS[src_slot].ipc_val;
        TASKS[src_slot].state = TaskState::Ready;
        Some(msg_ptr)
    } else {
        None
    }
}

unsafe fn try_match_recv_u64(cur: usize) -> Option<u64> {
    (0..MAX_TASKS).find(|&i| {
        TASKS[i].state == TaskState::BlockedSend
            && (TASKS[i].ipc_partner == IPC_ANY || TASKS[i].ipc_partner == TASKS[cur].id)
    }).map(|slot| {
        let val = TASKS[slot].ipc_val;
        TASKS[slot].state = TaskState::Ready;
        val
    })
}

pub unsafe fn ipc_send(dst_id: u64, msg_ptr: u64) -> u64 {
    let cur = CURRENT;
    if super::find_task(dst_id).is_none() { return IPC_ERR; }
    if try_match_send(cur, dst_id, msg_ptr) { return 0; }
    TASKS[cur].state = TaskState::BlockedSend;
    TASKS[cur].ipc_partner = dst_id;
    TASKS[cur].ipc_val = msg_ptr;
    if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
    super::yield_now();
    0
}

pub unsafe fn ipc_send_u64(dst_id: u64, val: u64) -> u64 {
    let cur = CURRENT;
    if super::find_task(dst_id).is_none() { return IPC_ERR; }
    if try_match_send_u64(cur, dst_id, val) { return 0; }
    TASKS[cur].state = TaskState::BlockedSend;
    TASKS[cur].ipc_partner = dst_id;
    TASKS[cur].ipc_val = val;
    if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
    super::yield_now();
    0
}

pub unsafe fn ipc_recv(src_id: u64) -> u64 {
    let cur = CURRENT;
    if let Some(msg_ptr) = try_match_recv(cur, src_id) { return msg_ptr; }
    TASKS[cur].state = TaskState::BlockedRecv;
    TASKS[cur].ipc_partner = if src_id == 0 { IPC_ANY } else { src_id };
    if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
    super::yield_now();
    TASKS[cur].ipc_val
}

pub unsafe fn ipc_recv_u64() -> u64 {
    let cur = CURRENT;
    if let Some(msg_ptr) = try_match_recv_u64(cur) { return msg_ptr; }
    TASKS[cur].state = TaskState::BlockedRecv;
    TASKS[cur].ipc_partner = IPC_ANY;
    if TASKS[cur].ring3 { TASKS[cur].sys_ursave = crate::syscall::read_sys_ursave(); }
    super::yield_now();
    TASKS[cur].ipc_val
}
