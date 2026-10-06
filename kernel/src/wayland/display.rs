//! wl_display — центральный объект Wayland, registry

use super::protocol::*;
use crate::driver::uart;

static mut NEXT_ID: u32 = 6;
static mut REGISTRY_DONE: bool = false;

pub fn next_id() -> u32 {
    unsafe{ let id=NEXT_ID; NEXT_ID+=1; id }
}

#[derive(Clone, Copy)]
pub struct WlGlobal {
    pub id: u32,
    pub interface: &'static str,
    pub version: u32,
}

pub const GLOBALS: &[WlGlobal] = &[
    WlGlobal{ id: WL_COMPOSITOR_ID, interface: "wl_compositor", version: 4 },
    WlGlobal{ id: WL_SHM_ID, interface: "wl_shm", version: 1 },
    WlGlobal{ id: WL_SEAT_ID, interface: "wl_seat", version: 7 },
    WlGlobal{ id: WL_OUTPUT_ID, interface: "wl_output", version: 3 },
    WlGlobal{ id: XDG_WM_BASE_ID, interface: "xdg_wm_base", version: 2 },
];

pub fn handle_display(op: u32, _arg: u64) -> u64 {
    match op {
        WL_DISPLAY_GET_REGISTRY => {
            unsafe{ REGISTRY_DONE=false; }
            uart::write_str("[WAYLAND] get_registry\n");
            // client will then receive globals via registry events
            // we synthesize them via next calls to registry_global
            WL_COMPOSITOR_ID as u64 // return registry id
        }
        WL_DISPLAY_SYNC => {
            uart::write_str("[WAYLAND] sync\n");
            next_id() as u64
        }
        _ => { uart::write_str("[WAYLAND] unknown display op\n"); 0 }
    }
}

pub fn registry_next_global() -> Option<WlGlobal> {
    static mut IDX: usize = 0;
    unsafe{
        if IDX < GLOBALS.len() {
            let g=GLOBALS[IDX];
            IDX+=1;
            Some(g)
        } else { None }
    }
}

pub fn handle_registry(op: u32, arg1: u64, arg2: u64) -> u64 {
    match op {
        WL_REGISTRY_BIND => {
            let id=arg1 as u32;
            let _iface_ptr=arg2 as *const u8;
            // arg2 is interface string ptr (from client), we just log
            uart::write_str("[WAYLAND] registry bind id=");
            let mut v=id; if v==0{ uart::putchar(b'0');} else { let mut b=[0u8;10]; let mut i=0; while v>0{b[i]=b'0'+(v%10)as u8; v/=10; i+=1;} while i>0{i-=1; uart::putchar(b[i]);}}
            uart::write_str("\n");
            id as u64
        }
        _ => 0,
    }
}
