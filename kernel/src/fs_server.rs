//! FS Server: файловая система, выставленная как capability-based IPC-сервер.
//!
//! Архитектурный скелет: операции FAT-слоя (`crate::fs`) обслуживаются
//! отдельной задачей-сервером, которая владеет capability на PORT_FILESYSTEM
//! и циклом `recv -> обработать -> reply`. Shell и ядро ходят в FS через IPC
//! (send_with_cap/recv_with_cap), а не напрямую.
//!
//! Протокол (см. dbsos-abi::ipc):
//!   request : msg_type=FsRequest, data[0]=op, data[1..] = path [NUL] [payload]
//!   reply   : msg_type=FsReply,   data[0]=FS_OK|FS_ERR, data[1..] = payload

use crate::driver::uart;
use crate::ipc;
use crate::scheduler;
use crate::cap;
use dbsos_abi::cap::{CapType, CAP_SEND, CAP_RECV};
use dbsos_abi::ipc::*;

/// Cap клиента (задача 0 / ядро) на FS-сервер.
static mut FS_CLIENT_CAP: u16 = 0;
/// Cap приёма сервера (индекс в таблице cap серверной задачи).
static mut FS_SERVER_RECV_CAP: u16 = 0;
/// Cap сервера для ответа задаче 0.
static mut FS_SERVER_REPLY_CAP: u16 = 0;
/// Идентификатор серверной задачи.
static mut FS_SERVER_TID: u64 = 0;

fn uart_print(s: &str) { uart::write_str(s); }

// ── Построители протокола ───────────────────────────────────────────

fn build_request(op: u8, path: &[u8], payload: &[u8]) -> Message {
    let mut m = Message::empty();
    m.msg_type = MsgType::FsRequest as u16;
    m.dst_port = PORT_FILESYSTEM;
    m.data[0] = op;
    let mut i = 1;
    for &b in path.iter().take(PAYLOAD_SIZE - 1) {
        m.data[i] = b; i += 1;
        if i >= PAYLOAD_SIZE { break; }
    }
    if i < PAYLOAD_SIZE { m.data[i] = 0; i += 1; }
    for &b in payload.iter().take(PAYLOAD_SIZE.saturating_sub(i)) {
        m.data[i] = b; i += 1;
    }
    m.length = i as u16;
    m
}

fn request_path(data: &[u8]) -> &[u8] {
    data[1..].split(|&c| c == 0).next().unwrap_or(&data[1..])
}

fn request_payload(data: &[u8]) -> &[u8] {
    let path = request_path(data);
    let path_end = 1 + path.len();
    let after = &data[path_end..];
    if after.first() == Some(&0) {
        after[1..].split(|&c| c == 0).next().unwrap_or(&after[1..])
    } else {
        &[]
    }
}

fn set_status(m: &mut Message, ok: bool) {
    m.msg_type = MsgType::FsReply as u16;
    m.data[0] = if ok { FS_OK } else { FS_ERR };
    m.length = 1;
}

fn reply_payload(msg: &mut Message, payload: &[u8]) {
    let n = payload.len().min(PAYLOAD_SIZE - 1);
    for i in 0..n { msg.data[i + 1] = payload[i]; }
    msg.length = (n + 1) as u16;
}

fn ok_reply(buff: &mut Message, payload: &[u8]) {
    set_status(buff, true);
    reply_payload(buff, payload);
}

fn err_reply(buff: &mut Message, text: &[u8]) {
    set_status(buff, false);
    reply_payload(buff, text);
}

// ── Обработка одной операции ────────────────────────────────────────

fn handle(op: u8, data: &[u8], buff: &mut Message) {
    let path = request_path(data);
    match op {
        FS_OP_LS => {
            if path.is_empty() {
                crate::fs::ls();
                ok_reply(buff, b"");
            } else {
                crate::fs::ls_path(path);
                ok_reply(buff, b"");
            }
        }
        FS_OP_CAT => {
            let mut file_buf = [0u8; PAYLOAD_SIZE];
            match crate::fs::read_file_path(path, &mut file_buf) {
                Some(sz) => ok_reply(buff, &file_buf[..sz]),
                None => err_reply(buff, b"not found"),
            }
        }
        FS_OP_WRITE => {
            let payload = request_payload(data);
            if crate::fs::write_file(path, payload) {
                ok_reply(buff, b"");
            } else {
                err_reply(buff, b"write failed");
            }
        }
        FS_OP_MKDIR => {
            if crate::fs::mkdir(path) { ok_reply(buff, b"") } else { err_reply(buff, b"mkdir failed") }
        }
        FS_OP_RM => {
            if crate::fs::rm(path) { ok_reply(buff, b"") } else { err_reply(buff, b"rm failed") }
        }
        FS_OP_RMDIR => {
            if crate::fs::rmdir(path) { ok_reply(buff, b"") } else { err_reply(buff, b"rmdir failed") }
        }
        FS_OP_IS_DIR => {
            let is = crate::fs::is_dir(path);
            set_status(buff, true);
            reply_payload(buff, if is { &[1] } else { &[0] });
        }
        FS_OP_SIZE => {
            match crate::fs::file_size(path) {
                Some(sz) => {
                    set_status(buff, true);
                    let mut p = [0u8; 4];
                    p[0] = sz as u8; p[1] = (sz >> 8) as u8;
                    p[2] = (sz >> 16) as u8; p[3] = (sz >> 24) as u8;
                    reply_payload(buff, &p);
                }
                None => err_reply(buff, b"not found"),
            }
        }
        FS_OP_EXISTS => {
            let found = crate::fs::find_file(path).is_some();
            set_status(buff, true);
            reply_payload(buff, if found { &[1] } else { &[0] });
        }
        _ => err_reply(buff, b"bad op"),
    }
}

/// Цикл сервера: блокирующий recv -> обработка -> reply.
extern "C" fn server_entry() {
    loop {
        let cap = unsafe { FS_SERVER_RECV_CAP };
        let mut req = Message::empty();
        let r = ipc::recv_with_cap(cap, &mut req);
        if r != 0 { continue; }
        let op = req.data[0];
        let mut data = [0u8; PAYLOAD_SIZE];
        data.copy_from_slice(&req.data);
        handle(op, &data, &mut req);
        let reply_cap = unsafe { FS_SERVER_REPLY_CAP };
        let _ = ipc::send_with_cap(reply_cap, &req);
    }
}

/// Запустить FS-сервер как kernel task и привязать capabilities.
/// Вызывается из ядра (задача 0) после scheduler::init().
pub fn init() -> Option<u16> {
    let tid = scheduler::spawn(server_entry)?;
    unsafe { FS_SERVER_TID = tid; }
    uart_print("[FSS] spawned server tid=");
    scheduler::uart_hex(tid);
    uart_print("\r\n");

    // Cap приёма — в таблице серверной задачи.
    let recv_cap = match ipc::create_server_cap(tid, PORT_FILESYSTEM) {
        Some(c) => c,
        None => { return None; }
    };
    unsafe { FS_SERVER_RECV_CAP = recv_cap; }

    // Cap reply (сервер -> задача 0) в таблице сервера.
    let reply_cap = ipc::create_client_cap(tid, 0);
    let reply_cap = match reply_cap {
        Some(c) => c,
        None => { return None; }
    };
    unsafe { FS_SERVER_REPLY_CAP = reply_cap; }

    // Cap клиента (задача 0 -> сервер). Нужен и SEND, и RECV (запрос+ответ).
    let client_cap = cap::alloc_for(0, CapType::IpcTarget as u64, tid,
        CAP_SEND | CAP_RECV, PORT_FILESYSTEM as u64)?;
    unsafe { FS_CLIENT_CAP = client_cap; }
    Some(client_cap)
}

// ── Клиентский API (задача 0 / ядро) ────────────────────────────────

/// Отправить запрос и получить reply.
fn request(op: u8, path: &[u8], payload: &[u8], out: &mut [u8]) -> i64 {
    let cap = unsafe { FS_CLIENT_CAP };
    if cap == 0 { return -2; }
    let req = build_request(op, path, payload);
    let r = ipc::send_with_cap(cap, &req);
    if r != 0 { return r; }
    let mut reply = Message::empty();
    let rr = ipc::recv_with_cap(cap, &mut reply);
    if rr != 0 { return rr; }
    let ok = reply.data[0] == FS_OK;
    if ok {
        let n = reply.length as usize;
        let payload = &reply.data[1..n];
        let n = payload.len().min(out.len());
        out[..n].copy_from_slice(&payload[..n]);
    }
    if ok { 0 } else { -1 }
}

/// Получить payload последнего ответа (для извлечения результата).
pub fn is_dir(path: &[u8]) -> bool {
    let mut out = [0u8; 4];
    if request(FS_OP_IS_DIR, path, &[], &mut out) == 0 {
        out[0] == 1
    } else {
        false
    }
}

pub fn exists(path: &[u8]) -> bool {
    let mut out = [0u8; 4];
    if request(FS_OP_EXISTS, path, &[], &mut out) == 0 {
        out[0] == 1
    } else {
        false
    }
}

pub fn size(path: &[u8]) -> Option<u32> {
    let mut out = [0u8; 4];
    if request(FS_OP_SIZE, path, &[], &mut out) == 0 {
        Some(out[0] as u32 | (out[1] as u32) << 8 | (out[2] as u32) << 16 | (out[3] as u32) << 24)
    } else {
        None
    }
}

pub fn read(path: &[u8], buf: &mut [u8]) -> Option<usize> {
    let mut out = [0u8; PAYLOAD_SIZE];
    match request(FS_OP_CAT, path, &[], &mut out) {
        0 => {
            let n = out.iter().position(|&c| c == 0).unwrap_or(out.len());
            if n > buf.len() { return None; }
            buf[..n].copy_from_slice(&out[..n]);
            Some(n)
        }
        _ => None,
    }
}

/// Прочитать содержимое для cat (как есть).
pub fn cat(path: &[u8]) {
    let mut out = [0u8; PAYLOAD_SIZE];
    if request(FS_OP_CAT, path, &[], &mut out) == 0 {
        for &b in out.iter() { if b == 0 { break; } uart::putchar(b); }
    } else {
        uart_print("[FS] not found\r\n");
    }
    uart_print("\r\n");
}

pub fn write(path: &[u8], data: &[u8]) -> bool {
    let mut out = [0u8; 1];
    request(FS_OP_WRITE, path, data, &mut out) == 0
}

pub fn mkdir(path: &[u8]) -> bool {
    let mut out = [0u8; 1];
    request(FS_OP_MKDIR, path, &[], &mut out) == 0
}

pub fn rm(path: &[u8]) -> bool {
    let mut out = [0u8; 1];
    request(FS_OP_RM, path, &[], &mut out) == 0
}

pub fn rmdir(path: &[u8]) -> bool {
    let mut out = [0u8; 1];
    request(FS_OP_RMDIR, path, &[], &mut out) == 0
}

pub fn ls(path: &[u8]) {
    let mut out = [0u8; 1];
    if request(FS_OP_LS, path, &[], &mut out) == 0 {
        // Возврат не нужен — сервер печатает листинг в uart.
    }
}

/// Сквозной self-test: write -> read через IPC-сервер.
pub fn roundtrip_test() {
    uart_print("\r\n[FSS] roundtrip (write->read via IPC):\r\n");
    let p = b"/test/fsipc.txt";
    let _ = mkdir(b"/test");
    let _ = rm(p);
    if write(p, b"FS IPC!\r\n") {
        uart_print("[FSS] write OK\r\n");
    } else {
        uart_print("[FSS] write FAIL\r\n");
        return;
    }
    let mut buf = [0u8; PAYLOAD_SIZE];
    match read(p, &mut buf) {
        Some(n) => {
            uart_print("[FSS] read OK: ");
            for &b in buf[..n].iter() { uart::putchar(b); }
            uart_print("\r\n");
            let _ = rm(p);
        }
        None => uart_print("[FSS] read FAIL\r\n"),
    }
    uart_print("[FSS] roundtrip done\r\n");
}