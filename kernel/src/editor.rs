//! DBS-Edit — простой редактор DBS-GR
#![allow(dead_code, unused_variables, unused_mut)]

use crate::display;
use crate::driver::ps2;
use crate::timer;
use crate::font::FONT_8X16;

fn sw()->u32{ display::width() }
fn sh()->u32{ display::height() }
fn draw_char(x:u32,y:u32,ch:u8,fg:u32,bg:u32){
    let f=display::framebuffer(); let s=display::stride() as i32;
    let idx=ch as usize*16;
    if idx+16>FONT_8X16.len(){return;}
    let g=&FONT_8X16[idx..idx+16];
    let is_bgr=display::is_bgr();
    let fr=((fg>>16)&0xFF)as u8; let fg2=((fg>>8)&0xFF)as u8; let fb=(fg&0xFF)as u8;
    let br=((bg>>16)&0xFF)as u8; let bg2=((bg>>8)&0xFF)as u8; let bb=(bg&0xFF)as u8;
    for r in 0..16{ let bits=g[r]; for c in 0..8{ let px=x+c; let py=y+r as u32; if px>=sw()||py>=sh(){continue;} let off=(py as i32*s+px as i32) as usize*4; let use_fg=bits&(0x80>>c)!=0; unsafe{if is_bgr{ if use_fg{*f.add(off)=fb;*f.add(off+1)=fg2;*f.add(off+2)=fr;}else{*f.add(off)=bb;*f.add(off+1)=bg2;*f.add(off+2)=br;}}else{ if use_fg{*f.add(off)=fr;*f.add(off+1)=fg2;*f.add(off+2)=fb;}else{*f.add(off)=br;*f.add(off+1)=bg2;*f.add(off+2)=bb;}}}}}
}
fn draw_text(x:u32,y:u32,s:&[u8],fg:u32,bg:u32){ let mut cx=x; for &c in s{ draw_char(cx,y,c,fg,bg); cx+=8; }}
fn fill_rect(x:u32,y:u32,w:u32,h:u32,col:u32){ let f=display::framebuffer(); let s=display::stride() as i32; let is_bgr=display::is_bgr(); let fr=((col>>16)&0xFF)as u8; let fg=((col>>8)&0xFF)as u8; let fb=(col&0xFF)as u8; for dy in 0..h{ let py=y+dy; if py>=sh(){break;} for dx in 0..w{ let px=x+dx; if px>=sw(){break;} let off=(py as i32*s+px as i32) as usize*4; unsafe{if is_bgr{*f.add(off)=fb;*f.add(off+1)=fg;*f.add(off+2)=fr;}else{*f.add(off)=fr;*f.add(off+1)=fg;*f.add(off+2)=fb;}}}}}
fn present(){ display::mark_dirty(0,0,sw(),sh()); unsafe{ display::present(); } }

pub fn edit(path: &[u8]) {
    // load file
    let mut buf=[0u8; 4096]; let mut len=0usize;
    let fd=crate::vfs::open(path, 0);
    if fd>=0 {
        let n=crate::vfs::read(fd, &mut buf);
        crate::vfs::close(fd);
        if n>0{ len=n as usize; }
    }
    let mut cursor=len.min(4096-1);
    let mut scroll=0usize; // first visible line offset?
    loop{
        // draw
        fill_rect(0,0,sw(),sh(),0x1E1E1E);
        fill_rect(0,0,sw(),16,0xE95420);
        draw_text(4,0,b"DBS-Edit  Ctrl+S save  Ctrl+Q quit  EN/RU Alt+Shift",0xFFFFFF,0xE95420);
        fill_rect(0,16,sw(),16,0x303030);
        draw_text(4,16,path,0xFFFFFF,0x303030);
        // layout
        draw_text(sw()-40,0,crate::driver::ps2::layout_name().as_bytes(),0xFFFFFF,0xE95420);
        // text area: split buf into lines
        let mut y=32u32; let mut line_start=0usize;
        let mut line_no=0;
        // find scroll offset: we show from scroll
        // For simplicity, show from start, with scroll as byte offset
        // Find line containing scroll
        let mut pos=0usize;
        // Render up to 40 lines
        let mut cur_x=0u32; let mut cur_y=0u32;
        let mut rendered=0;
        let mut lines: [(usize,usize); 64]=[(0,0);64];
        let mut line_cnt=0;
        let mut i=0;
        while i<=len && line_cnt<64 {
            let mut j=i;
            while j<len && buf[j]!=b'\n' {j+=1;}
            lines[line_cnt]=(i, j);
            line_cnt+=1;
            i=j+1;
        }
        // find cursor line
        let mut cur_line=0; let mut cur_col=0;
        for l in 0..line_cnt {
            let (s,e)=lines[l];
            if cursor>=s && cursor<=e { cur_line=l; cur_col=cursor-s; break; }
            if l==line_cnt-1 && cursor>e { cur_line=l; cur_col=e-s; }
        }
        // adjust scroll to keep cursor visible (show 30 lines)
        let visible= (sh()-48)/16;
        if cur_line < scroll { scroll=cur_line; }
        if cur_line >= scroll+visible as usize { scroll=cur_line - visible as usize +1; }
        for idx in scroll..(scroll+visible as usize).min(line_cnt) {
            let (s,e)=lines[idx];
            let y_cur=32 + (idx-scroll) as u32*16;
            // line number
            let mut lno=[b'0';4];
            let mut v=idx; for k in (0..3).rev(){ lno[k]=b'0'+(v%10) as u8; v/=10; }
            draw_text(4,y_cur,&lno,0xAEA79F,0x1E1E1E);
            // content
            let mut cx=40;
            for p in s..e.min(s+80) {
                draw_char(cx,y_cur,buf[p],0xFFFFFF,0x1E1E1E);
                cx+=8;
                if p==cursor-1 && idx==cur_line { /* cursor after */ }
            }
            // cursor
            if idx==cur_line {
                let cx_cur=40 + cur_col as u32*8;
                // blink
                if timer::millis()/400%2==0 {
                    fill_rect(cx_cur, y_cur+14, 8,2,0xE95420);
                }
            }
        }
        fill_rect(0, sh()-16, sw(),16,0x303030);
        draw_text(4, sh()-16, b"Ctrl+S Save  Ctrl+Q Quit",0xAEA79F,0x303030);
        present();
        // input
        let c=loop{
            if let Some(ch)=ps2::poll_char(){ break ch; }
            if let Some(ch)=crate::driver::uart::poll_char(){ break ch; }
            timer::usleep(5000);
        };
        match c {
            0x11 => { // Ctrl+Q (0x11)
                return;
            }
            0x13 => { // Ctrl+S
                // save
                let fd2=crate::vfs::open(path, 0x100|0x200|1);
                if fd2>=0 {
                    let _=crate::vfs::write(fd2, &buf[..len]);
                    crate::vfs::close(fd2);
                } else {
                    // try create via write
                    let _=crate::vfs::open(path, 1);
                }
                fill_rect(sw()/2-60, sh()/2-8,120,16,0x4CD964);
                draw_text(sw()/2-40, sh()/2-8, b"Saved",0x000000,0x4CD964);
                present(); timer::usleep(500000);
            }
            0x08|0x7F => { if cursor>0 && len>0 { // backspace
                for i in cursor..len { buf[i-1]=buf[i]; }
                len-=1; cursor-=1;
            } }
            ps2::KEY_DELETE => { if cursor < len { // сдвиг хвоста влево
                for i in cursor..len-1 { buf[i]=buf[i+1]; } len-=1;
            }}
            ps2::KEY_LEFT => { if cursor>0 {cursor-=1;} }
            ps2::KEY_RIGHT => { if cursor < len {cursor+=1;} }
            ps2::KEY_UP => {
                // move to previous line same col
                // find cur_line and go up
                let mut l=cur_line;
                if l>0 { let (ps,pe)=lines[l-1]; let new_col=cur_col.min(pe-ps); cursor=ps+new_col; }
            }
            ps2::KEY_DOWN => {
                if cur_line+1 < line_cnt { let (ns,ne)=lines[cur_line+1]; let new_col=cur_col.min(ne-ns); cursor=ns+new_col; }
                else if cursor<len { cursor=len; }
            }
            b'\r'|b'\n' => {
                if len < 4095 {
                    for i in (cursor..len).rev(){ buf[i+1]=buf[i]; }
                    buf[cursor]=b'\n'; len+=1; cursor+=1;
                }
            }
            27 => { return; } // ESC
            c if c>=32 && c<127 || c>=0xC0 => { // allow RU bytes
                if len < 4095 {
                    for i in (cursor..len).rev(){ buf[i+1]=buf[i]; }
                    buf[cursor]=c; len+=1; cursor+=1;
                }
            }
            _=>{}
        }
        if c==0x11 { break; }
    }
}
