//! DBS-Browser — базовый текстовый браузер DBS-GR (HTTP/1.0 via e1000)
#![allow(dead_code, unused_variables, unused_mut, unused_assignments)]

use crate::display;
use crate::driver::ps2;
use crate::timer;
use crate::font::FONT_8X16;

fn sw()->u32{ display::width() } fn sh()->u32{ display::height() }
fn draw_char(x:u32,y:u32,ch:u8,fg:u32,bg:u32){
    let f=display::framebuffer(); let s=display::stride() as i32;
    let idx=ch as usize*16; if idx+16>FONT_8X16.len(){return;}
    let g=&FONT_8X16[idx..idx+16];
    let is_bgr=display::is_bgr();
    let fr=((fg>>16)&0xFF)as u8; let fg2=((fg>>8)&0xFF)as u8; let fb=(fg&0xFF)as u8;
    let br=((bg>>16)&0xFF)as u8; let bg2=((bg>>8)&0xFF)as u8; let bb=(bg&0xFF)as u8;
    for r in 0..16{ let bits=g[r]; for c in 0..8{ let px=x+c; let py=y+r as u32; if px>=sw()||py>=sh(){continue;} let off=(py as i32*s+px as i32) as usize*4; let use_fg=bits&(0x80>>c)!=0; unsafe{if is_bgr{ if use_fg{*f.add(off)=fb;*f.add(off+1)=fg2;*f.add(off+2)=fr;}else{*f.add(off)=bb;*f.add(off+1)=bg2;*f.add(off+2)=br;}}else{ if use_fg{*f.add(off)=fr;*f.add(off+1)=fg2;*f.add(off+2)=fb;}else{*f.add(off)=br;*f.add(off+1)=bg2;*f.add(off+2)=bb;}}}}}
}
fn draw_text(x:u32,y:u32,s:&[u8],fg:u32,bg:u32){ let mut cx=x; for &c in s{ draw_char(cx,y,c,fg,bg); cx+=8; }}
fn fill_rect(x:u32,y:u32,w:u32,h:u32,col:u32){ let f=display::framebuffer(); let s=display::stride() as i32; let is_bgr=display::is_bgr(); let fr=((col>>16)&0xFF)as u8; let fg=((col>>8)&0xFF)as u8; let fb=(col&0xFF)as u8; for dy in 0..h{ let py=y+dy; if py>=sh(){break;} for dx in 0..w{ let px=x+dx; if px>=sw(){break;} let off=(py as i32*s+px as i32) as usize*4; unsafe{if is_bgr{*f.add(off)=fb;*f.add(off+1)=fg;*f.add(off+2)=fr;}else{*f.add(off)=fr;*f.add(off+1)=fg;*f.add(off+2)=fb;}}}}}
fn present(){ display::mark_dirty(0,0,sw(),sh()); unsafe{ display::present(); } }

fn http_get(host: &[u8], path: &[u8], out: &mut [u8]) -> Option<usize> {
    // use existing wget logic via tcp
    // Do DNS, then TCP connect 80, send GET
    let ip = crate::driver::dns::resolve(host, 3000)?;
    let conn = crate::driver::tcp::connect(ip, 80)?;
    let mut req=[0u8; 256]; let mut p=0;
    let get=b"GET "; req[p..p+4].copy_from_slice(get); p+=4;
    let pl=path.len().min(200); req[p..p+pl].copy_from_slice(&path[..pl]); p+=pl;
    let hdr=b" HTTP/1.0\r\nHost: "; req[p..p+15].copy_from_slice(hdr); p+=15;
    let hl=host.len().min(64); req[p..p+hl].copy_from_slice(&host[..hl]); p+=hl;
    let end=b"\r\nUser-Agent: DBS-Browser/0.1\r\n\r\n"; req[p..p+end.len()].copy_from_slice(end); p+=end.len();
    crate::driver::tcp::send(conn, &req[..p]);
    let start=timer::millis();
    let mut total=0;
    while timer::millis()-start < 5000 && total < out.len() {
        let mut tmp=[0u8; 1024];
        let n=crate::driver::tcp::recv(conn, &mut tmp);
        if n>0 {
            let copy=n.min(out.len()-total);
            out[total..total+copy].copy_from_slice(&tmp[..copy]);
            total+=copy;
        }
        crate::driver::tcp::pump();
        if total>0 && n==0 { timer::usleep(10000); }
    }
    crate::driver::tcp::close(conn);
    if total==0 { None } else { Some(total) }
}
fn strip_tags(input: &[u8], out: &mut [u8]) -> usize {
    let mut oi=0; let mut in_tag=false;
    for &c in input {
        if c==b'<' { in_tag=true; continue; }
        if c==b'>' { in_tag=false; continue; }
        if in_tag { continue; }
        if c==b'&' { // &amp; etc skip
            // simple: skip until ;
            continue;
        }
        if oi < out.len() { out[oi]=c; oi+=1; }
    }
    oi
}

pub fn run(url: &[u8]) {
    fill_rect(0,0,sw(),sh(),0x1E1E1E);
    fill_rect(0,0,sw(),16,0xE95420);
    draw_text(4,0,b"DBS-Browser  Enter:URL  q:quit  Up/Down scroll",0xFFFFFF,0xE95420);
    present();
    let mut target=[0u8;128]; let mut tlen=0;
    if !url.is_empty(){ let l=url.len().min(127); target[..l].copy_from_slice(&url[..l]); tlen=l; }
    else { let def=b"example.com"; target[..def.len()].copy_from_slice(def); tlen=def.len(); }
    // parse host/path
    let mut host_end=tlen;
    let mut path_start=tlen;
    for i in 0..tlen{ if target[i]==b'/'{ host_end=i; path_start=i; break; } }
    let host=&target[..host_end];
    let path=if path_start<tlen{ &target[path_start..tlen]} else { b"/" };
    fill_rect(0,16,sw(),16,0x303030);
    draw_text(4,16, b"URL: ",0xAEA79F,0x303030);
    draw_text(4+40,16, &target[..tlen],0xFFFFFF,0x303030);
    draw_text(4,32, b"Loading...",0xE95420,0x1E1E1E);
    present();
    let mut raw=[0u8; 8192];
    let mut text=[0u8; 8192];
    let n = http_get(host, path, &mut raw);
    let rendered_len = if let Some(sz)=n {
        // find body after \r\n\r\n
        let mut body_start=0;
        for i in 0..sz-3{ if raw[i]==b'\r' && raw[i+1]==b'\n' && raw[i+2]==b'\r' && raw[i+3]==b'\n'{ body_start=i+4; break; } }
        if body_start==0 { for i in 0..sz-1{ if raw[i]==b'\n'&&raw[i+1]==b'\n'{body_start=i+2; break;}} }
        let body=&raw[body_start..sz];
        strip_tags(body, &mut text)
    } else { let msg=b"Failed to fetch (check net/dhcp/dns)"; text[..msg.len()].copy_from_slice(msg); msg.len() };
    // render text with word wrap and scroll
    let mut scroll=0usize;
    loop{
        fill_rect(0,48,sw(),sh()-64,0x1E1E1E);
        let mut y=48u32; let mut x=4u32;
        let mut pos=scroll;
        let mut line_start=pos;
        while y+16 < sh()-16 && pos < rendered_len {
            // find next line break or wrap at  sw/8 chars
            let max_chars=(sw()-8)/8;
            let mut line_end=pos;
            while line_end < rendered_len && line_end-pos < max_chars as usize && text[line_end]!=b'\n' { line_end+=1; }
            // if no \n, break at word
            let slice=&text[pos..line_end.min(rendered_len)];
            draw_text(x,y,slice,0xFFFFFF,0x1E1E1E);
            y+=16;
            pos=if line_end<rendered_len && text[line_end]==b'\n' { line_end+1 } else { line_end+1 };
            if pos>=rendered_len{ break; }
        }
        fill_rect(0, sh()-16, sw(),16,0x303030);
        draw_text(4, sh()-16, b"Up/Down scroll  q:quit  g:go",0xAEA79F,0x303030);
        draw_text(sw()-80, sh()-16, b"DBS-Browser",0xFFFFFF,0x303030);
        present();
        // input
        let c=loop{
            if let Some(ch)=ps2::poll_char(){ break ch; }
            if let Some(ch)=crate::driver::uart::poll_char(){ break ch; }
            timer::usleep(10000);
        };
        match c {
            b'q'|27 => { return; }
            ps2::KEY_UP => { if scroll>=80 { scroll-=80; } else { scroll=0; } }
            ps2::KEY_DOWN => { if scroll+80 < rendered_len { scroll+=80; } }
            b'g'|b'G' => {
                // prompt for new URL
                fill_rect(sw()/2-120, sh()/2-20, 240, 40,0x303030);
                draw_text(sw()/2-100, sh()/2-8, b"URL:",0xFFFFFF,0x303030);
                present();
                let mut nbuf=[0u8;64]; let mut nl=0;
                loop{
                    if let Some(ch)=ps2::poll_char(){
                        if ch==b'\n'{ break; }
                        if ch==0x08 && nl>0{ nl-=1; }
                        else if ch>=32&&ch<127&&nl<63{ nbuf[nl]=ch; nl+=1; }
                        fill_rect(sw()/2-100, sh()/2+10, 200,16,0x202020);
                        draw_text(sw()/2-80, sh()/2+10, &nbuf[..nl],0xFFFFFF,0x202020);
                        present();
                    } else { timer::usleep(10000); }
                }
                if nl>0 { return run(&nbuf[..nl]); }
                break;
            }
            _=>{}
        }
    }
}
