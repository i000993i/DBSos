//! xdg_shell — XDG window management (Ubuntu Yaru decorations)

use crate::driver::uart;

static mut NEXT_XDG_ID: u32 = 100;

pub fn handle_wm_base(op: u32) -> u64 {
    match op {
        2 => { // get_xdg_surface
            let id=unsafe{ let v=NEXT_XDG_ID; NEXT_XDG_ID+=1; v };
            uart::write_str("[WAYLAND] xdg_surface create\n");
            id as u64
        }
        3 => { uart::write_str("[WAYLAND] xdg pong\n"); 0 }
        _ => 0,
    }
}
pub fn handle_xdg_surface(op: u32) -> u64 {
    match op {
        1 => { // get_toplevel
            let id=unsafe{ let v=NEXT_XDG_ID; NEXT_XDG_ID+=1; v };
            uart::write_str("[WAYLAND] xdg_toplevel create\n");
            id as u64
        }
        4 => { uart::write_str("[WAYLAND] xdg ack_configure\n"); 0 }
        _ => 0,
    }
}
pub fn handle_toplevel(op: u32, _arg: u64) -> u64 {
    match op {
        2 => { uart::write_str("[WAYLAND] set_title\n"); 0 }
        3 => { uart::write_str("[WAYLAND] set_app_id\n"); 0 }
        _ => 0,
    }
}

// ── Состояние xdg-shell (для Plasma-клиентов и RS-Kernel-Test) ───────
// Сервер помнит: xdg_surface id, toplevel id, title/app_id, сериалы
// configure→ack (протокол требует ack последнего configure перед коммитом).

const MAX_XDG: usize = 16;

#[derive(Clone, Copy)]
struct XdgSurf {
    used: bool,
    xdg_id: u32,
    toplevel_id: u32,
    has_toplevel: bool,
    title: [u8; 64],
    title_len: u8,
    app_id: [u8; 32],
    app_len: u8,
    pending_serial: u32,
    acked_serial: u32,
}

impl XdgSurf {
    const fn empty() -> Self {
        XdgSurf {
            used: false, xdg_id: 0, toplevel_id: 0, has_toplevel: false,
            title: [0; 64], title_len: 0, app_id: [0; 32], app_len: 0,
            pending_serial: 0, acked_serial: 0,
        }
    }
}

static mut XDG: [XdgSurf; MAX_XDG] = [XdgSurf::empty(); MAX_XDG];
static mut XDG_SERIAL: u32 = 1;

fn find_slot(xdg_id: u32) -> Option<usize> {
    unsafe {
        for i in 0..MAX_XDG {
            if XDG[i].used && XDG[i].xdg_id == xdg_id {
                return Some(i);
            }
        }
        None
    }
}

/// Создать xdg_surface. Возвращает id.
pub fn surface_new() -> Option<u32> {
    unsafe {
        for i in 0..MAX_XDG {
            if !XDG[i].used {
                XDG[i] = XdgSurf::empty();
                XDG[i].used = true;
                let id = NEXT_XDG_ID;
                NEXT_XDG_ID += 1;
                XDG[i].xdg_id = id;
                return Some(id);
            }
        }
        None
    }
}

/// get_toplevel + немедленный configure (сериал). Возвращает (toplevel_id, serial).
pub fn toplevel_new(xdg_id: u32) -> Option<(u32, u32)> {
    let s = find_slot(xdg_id)?;
    unsafe {
        let top = NEXT_XDG_ID;
        NEXT_XDG_ID += 1;
        let serial = XDG_SERIAL;
        XDG_SERIAL += 1;
        XDG[s].toplevel_id = top;
        XDG[s].has_toplevel = true;
        XDG[s].pending_serial = serial;
        Some((top, serial))
    }
}

pub fn set_title(xdg_id: u32, title: &[u8]) -> bool {
    let s = match find_slot(xdg_id) {
        Some(s) => s,
        None => return false,
    };
    unsafe {
        let n = title.len().min(64);
        XDG[s].title[..n].copy_from_slice(&title[..n]);
        XDG[s].title_len = n as u8;
        true
    }
}

pub fn set_app_id(xdg_id: u32, app: &[u8]) -> bool {
    let s = match find_slot(xdg_id) {
        Some(s) => s,
        None => return false,
    };
    unsafe {
        let n = app.len().min(32);
        XDG[s].app_id[..n].copy_from_slice(&app[..n]);
        XDG[s].app_len = n as u8;
        true
    }
}

/// ack_configure: клиент подтверждает сериал. true если совпал с pending.
pub fn ack_configure(xdg_id: u32, serial: u32) -> bool {
    let s = match find_slot(xdg_id) {
        Some(s) => s,
        None => return false,
    };
    unsafe {
        if XDG[s].pending_serial != serial {
            return false;
        }
        XDG[s].acked_serial = serial;
        true
    }
}

pub fn surface_count() -> usize {
    unsafe { XDG.iter().filter(|s| s.used).count() }
}

pub fn title_of(xdg_id: u32, out: &mut [u8; 64]) -> Option<usize> {
    let s = find_slot(xdg_id)?;
    unsafe {
        let n = XDG[s].title_len as usize;
        out[..n].copy_from_slice(&XDG[s].title[..n]);
        Some(n)
    }
}

/// KAT: create → toplevel → title/app_id → ack → verify. 0 = ok.
pub fn self_test() -> u32 {
    let x = match surface_new() {
        Some(id) => id,
        None => return 1,
    };
    let (top, serial) = match toplevel_new(x) {
        Some(v) => v,
        None => return 2,
    };
    if top == 0 || serial == 0 {
        return 4;
    }
    if !set_title(x, b"Plasma-Test") {
        return 8;
    }
    if !set_app_id(x, b"org.dbos.test") {
        return 16;
    }
    // чужой сериал — отказ
    if ack_configure(x, serial.wrapping_add(999)) {
        return 32;
    }
    if !ack_configure(x, serial) {
        return 64;
    }
    let mut t = [0u8; 64];
    match title_of(x, &mut t) {
        Some(n) if n == 11 && &t[..n] == b"Plasma-Test" => {}
        _ => return 128,
    }
    if surface_count() == 0 {
        return 256;
    }
    0
}
