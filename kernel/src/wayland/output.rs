//! wl_output — вывод (разрешение, scale)

use crate::driver::uart;

pub fn handle_output(op: u32) -> u64 {
    match op {
        0 => { uart::write_str("[WAYLAND] output geometry\n"); 0 }
        1 => {
            let w=crate::display::width();
            let h=crate::display::height();
            uart::write_str("[WAYLAND] output mode ");
            let mut v=w; if v==0{ uart::putchar(b'0');} else { let mut b=[0u8;10]; let mut i=0; while v>0{b[i]=b'0'+(v%10)as u8;v/=10;i+=1;} while i>0{i-=1;uart::putchar(b[i]);}}
            uart::write_str("x");
            let mut v2=h; if v2==0{ uart::putchar(b'0');} else { let mut b=[0u8;10]; let mut i=0; while v2>0{b[i]=b'0'+(v2%10)as u8;v2/=10;i+=1;} while i>0{i-=1;uart::putchar(b[i]);}}
            uart::write_str("\n");
            0
        }
        _=>0,
    }
}
pub fn geometry() -> (u32,u32) { (crate::display::width(), crate::display::height()) }
