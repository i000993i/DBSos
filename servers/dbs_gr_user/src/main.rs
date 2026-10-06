#![no_std]
// ring-3 _start скрыт при cargo test — остальной код становится «мертвым».
#![cfg_attr(test, allow(dead_code))]
// При cargo test (host) харнесс генерирует свой main.
#![cfg_attr(not(test), no_main)]

use core::arch::asm;
#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! { loop { unsafe { asm!("hlt"); } } }

#[repr(C)]
struct FbInfo {
    width: u64,
    height: u64,
    stride: u64,
    phys: u64,
    is_bgr: u64,
    size: u64,
}

unsafe fn syscall3(n: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    asm!(
        "syscall",
        in("rax") n,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") 0,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
    );
    ret
}
unsafe fn syscall4(n: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    asm!(
        "syscall",
        in("rax") n,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
    );
    ret
}
unsafe fn log(s: &[u8]) {
    syscall3(20, s.as_ptr() as u64, s.len() as u64, 0);
}

// Точка входа ring-3-образа. При `cargo test` (host) не компилируется:
// с host crt уже есть свой _start.
#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        let msg = b"[DBS-GR-USER] Ring3 start\n";
        log(msg);
        // Get FB info
        let mut info = FbInfo{ width:0,height:0,stride:0,phys:0,is_bgr:0,size:0 };
        let r = syscall3(77, &mut info as *mut _ as u64, 0, 0);
        if r != 0 {
            let e = b"[DBS-GR-USER] FB_INFO failed\n";
            log(e);
            loop { asm!("hlt"); }
        }
        let virt: u64 = 0x500000;
        let r2 = syscall4(78, info.phys, virt, info.size, 0);
        if r2 != 0 {
            let e = b"[DBS-GR-USER] FB_MAP failed\n";
            log(e);
            loop { asm!("hlt"); }
        }
        let ok = b"[DBS-GR-USER] FB mapped, drawing\n";
        log(ok);
        // Draw gradient black->gray directly to mapped FB at virt
        let w = info.width as usize;
        let h = info.height as usize;
        let stride = info.stride as usize;
        let is_bgr = info.is_bgr != 0;
        let fb = virt as *mut u8;
        for y in 0..h {
            let t = (y * 255 / h.max(1)) as u32;
            let v = if t < 85 { 0x40 * t * 3 / 255 } else if t < 170 { 0x40 + 0x40 * (t-85)*3/255 } else { 0x80 - 0x80*(t-170)*3/255 };
            let r = v as u8; let g = v as u8; let b = v as u8;
            let row_off = y * stride * 4;
            for x in 0..w {
                let off = row_off + x*4;
                if is_bgr { *fb.add(off)=b; *fb.add(off+1)=g; *fb.add(off+2)=r; } else { *fb.add(off)=r; *fb.add(off+1)=g; *fb.add(off+2)=b; }
                *fb.add(off+3)=0;
            }
        }
        // Draw red prompt text at top-left via simple pixel blocks (8x16 font would need font data; just draw white pixels for demo)
        // For demo, draw a red rectangle where prompt would be
        for y in 10..26 {
            for x in 10..200 {
                let off = (y*stride + x)*4;
                if is_bgr { *fb.add(off)=0x30 as u8; *fb.add(off+1)=0x3B as u8; *fb.add(off+2)=0xFF as u8; } else { *fb.add(off)=0xFF; *fb.add(off+1)=0x3B; *fb.add(off+2)=0x30; }
            }
        }
        let done = b"[DBS-GR-USER] drawn, hlt\n";
        log(done);
        loop { asm!("hlt"); }
    }
}
