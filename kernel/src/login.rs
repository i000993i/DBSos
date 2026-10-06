//! Login manager — mandatory authentication before desktop/shell
//! Uses framebuffer + UART + PS/2 for I/O, runs before LEVEL_2/1.

use crate::display;
use crate::driver::uart;
use crate::driver::ps2;
use crate::timer;
use crate::font::FONT_8X16;

const CHAR_W: u32 = 8;
const CHAR_H: u32 = 16;

fn sw() -> u32 { display::width() }
fn sh() -> u32 { display::height() }

fn fill_rect(x: u32, y: u32, w: u32, h: u32, c: u32) {
    let fb = display::framebuffer();
    let stride = display::stride();
    let wi = sw(); let hi = sh();
    let r = ((c>>16)&0xFF) as u8;
    let g = ((c>>8)&0xFF) as u8;
    let b = (c&0xFF) as u8;
    for dy in 0..h {
        let py = y+dy;
        if py>=hi {break;}
        for dx in 0..w {
            let px = x+dx;
            if px>=wi {break;}
            let off=(py*stride+px) as usize*4;
            unsafe{ *fb.add(off)=b; *fb.add(off+1)=g; *fb.add(off+2)=r; }
        }
    }
}
fn draw_char(x: u32, y: u32, ch: u8, col: u32) {
    if ch < 32 || ch > 126 { return; }
    let idx = ch as usize * 16;
    if idx+16 > FONT_8X16.len() { return; }
    let glyph = &FONT_8X16[idx..idx+16];
    let fb = display::framebuffer();
    let stride = display::stride();
    let r = ((col>>16)&0xFF) as u8;
    let g = ((col>>8)&0xFF) as u8;
    let b = (col&0xFF) as u8;
    for row in 0..16u32 {
        let bits = glyph[row as usize];
        for colb in 0..8u32 {
            if bits & (0x80>>colb)==0 {continue;}
            let px=x+colb;
            let py=y+row;
            if px>=sw()||py>=sh(){continue;}
            let off=(py*stride+px) as usize*4;
            unsafe{ *fb.add(off)=b; *fb.add(off+1)=g; *fb.add(off+2)=r; }
        }
    }
}
fn draw_text(mut x: u32, y: u32, s: &[u8], col: u32) -> u32 {
    for &c in s { draw_char(x,y,c,col); x+=8; }
    x
}
fn present() { unsafe{ display::present(); } }

static mut CUR_X: u32 = 0;
static mut CUR_Y: u32 = 0;
fn term_init() {
    unsafe{ CUR_X=0; CUR_Y=0; }
    display::clear_screen(0x1b,0x1e,0x20);
    present();
}
fn term_print(s: &[u8], col: u32) {
    unsafe{
        for &c in s {
            if c==b'\n' { CUR_X=0; CUR_Y+=CHAR_H; if CUR_Y+CHAR_H>sh(){ term_init(); } continue; }
            if c==b'\r' { CUR_X=0; continue; }
            draw_char(CUR_X, CUR_Y, c, col);
            CUR_X+=CHAR_W;
            if CUR_X+CHAR_W>sw(){ CUR_X=0; CUR_Y+=CHAR_H; if CUR_Y+CHAR_H>sh(){ term_init(); } }
        }
        present();
    }
}
fn term_print_str(s: &str, col: u32){ term_print(s.as_bytes(), col); }

// Poll keyboard: PS/2 or UART
fn poll_char() -> Option<u8> {
    if let Some(c)=ps2::poll_char(){ return Some(c); }
    if let Some(c)=uart::poll_char(){ return Some(c); }
    None
}

fn read_line(echo: bool, buf: &mut [u8]) -> usize {
    let mut len=0usize;
    let start_x = unsafe{ CUR_X };
    let start_y = unsafe{ CUR_Y };
    loop{
        if let Some(c)=poll_char(){
            match c {
                b'\r'|b'\n' => {
                    term_print(b"\n", 0xfcfcfc);
                    if len<buf.len(){buf[len]=0;}
                    return len;
                }
                0x08 | 0x7F => {
                    if len>0 {
                        len-=1;
                        // erase char
                        unsafe{
                            if CUR_X>=CHAR_W { CUR_X-=CHAR_W; } else if CUR_Y>=CHAR_H { CUR_Y-=CHAR_H; CUR_X=sw()-CHAR_W; }
                            fill_rect(CUR_X, CUR_Y, CHAR_W, CHAR_H, 0x1b1e20);
                            present();
                        }
                    }
                }
                0x03 => { // Ctrl+C clear
                    len=0;
                    unsafe{ CUR_X=start_x; CUR_Y=start_y; }
                    fill_rect(start_x, start_y, (buf.len() as u32)*CHAR_W, CHAR_H, 0x1b1e20);
                    present();
                }
                32..=126 => {
                    if len < buf.len()-1 {
                        buf[len]=c; len+=1;
                        if echo {
                            let tmp=[c];
                            term_print(&tmp, 0xfcfcfc);
                        } else {
                            term_print(b"*", 0xfcfcfc);
                        }
                    }
                }
                _=>{}
            }
        } else {
            timer::usleep(1000);
        }
    }
}

fn draw_banner() {
    display::clear_screen(0x1b,0x1e,0x20);
    unsafe{ CUR_X=0; CUR_Y=0; }
    // centered title
    let title=b"DBSos - Login";
    let w = sw();
    let tx = (w - (title.len() as u32)*CHAR_W)/2;
    // big accent bar
    fill_rect(0, 0, w, 32, 0x232629);
    draw_text(tx, 8, title, 0x3daee9);
    // users hint
    let hint=b"Users: root/root, guest (no password), user/user";
    let hx = (w - (hint.len() as u32)*CHAR_W)/2;
    draw_text(hx, 48, hint, 0x9ca0a4);
    unsafe{ CUR_X=20; CUR_Y=90; }
    present();
}

/// Mandatory login — blocks until success. Called from lib.rs init.
pub fn require_login() {
    uart::write_str("[LOGIN] mandatory login...\r\n");
    term_init();
    draw_banner();
    let mut attempts=0u32;
    loop {
        term_print_str("login: ", 0xfcfcfc);
        let mut ubuf=[0u8;32];
        let ulen=read_line(true, &mut ubuf);
        if ulen==0 { continue; }
        let user=&ubuf[..ulen];
        term_print_str("Password: ", 0xfcfcfc);
        let mut pbuf=[0u8;32];
        let plen=read_line(false, &mut pbuf);
        let pass=&pbuf[..plen];
        if crate::user::login(user, pass) {
            let name=crate::user::current_name();
            term_print_str("Welcome ", 0x3daee9);
            term_print(name.as_bytes(), 0x3daee9);
            term_print_str("!\n", 0xfcfcfc);
            uart::write_str("[LOGIN] success as ");
            uart::write_str(name);
            uart::write_str("\r\n");
            // set cwd to home if exists
            {
                let mut home=[0u8;64];
                let pre=b"/home/";
                // guest has /home/guest, root has /root — handle root special
                if user==b"root" {
                    let root=b"/root";
                    if crate::vfs::is_dir(root){
                        crate::shell::cwd_set_public(root);
                    }
                } else {
                    home[..pre.len()].copy_from_slice(pre);
                    let nl=user.len().min(64-pre.len());
                    home[pre.len()..pre.len()+nl].copy_from_slice(&user[..nl]);
                    if crate::vfs::is_dir(&home[..pre.len()+nl]){
                        crate::shell::cwd_set_public(&home[..pre.len()+nl]);
                    }
                }
            }
            timer::usleep(500_000);
            display::clear_screen(0,0,0);
            present();
            return;
        } else {
            attempts+=1;
            term_print_str("Login incorrect\n", 0xe81123);
            uart::write_str("[LOGIN] failed attempt\r\n");
            if attempts>=3 {
                term_print_str("Too many attempts, waiting 2s...\n", 0x9ca0a4);
                timer::usleep(2_000_000);
                attempts=0;
                draw_banner();
            }
        }
    }
}
