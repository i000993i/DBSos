//! DBS-FM — файловый менеджер DBS-GR (TUI)
//! Навигация: стрелки Up/Down, Enter — войти, Backspace — вверх, q — выход, d — удалить, n — создать
#![allow(dead_code, unused_variables, unused_mut, unused_assignments)]

use crate::display;
use crate::driver::ps2;
use crate::timer;
use crate::font::FONT_8X16;

const FM_MAX_ENTRIES: usize = 64;
const FM_MAX_ROWS: usize = 96;
const FM_MAX_CHILD_DIRS: usize = 8;   // сколько папок раскрываем на уровень
const FM_MAX_CHILDREN: usize = 8;     // сколько детей показываем в папке
const FM_W: u32 = 80;
const FM_H: u32 = 25;

// Серый для папок, тусклый для линий иерархии (ASCII: шрифт без бокс-графики)
const FM_DIR_FG: u32 = 0x9E9E9E;
const FM_LINE_FG: u32 = 0x5A5A5A;
const FM_FILE_FG: u32 = 0xFFFFFF;
const FM_SEL_BG: u32 = 0xE95420;

/// Строка дерева: depth 0 — cwd, depth 1 — дети раскрытых папок.
#[derive(Clone, Copy)]
struct FmRow {
    depth: u8,
    last: bool,        // последний среди сиблингов (`-- вместо |-- )
    parent_next: bool, // у родителя есть следующий сиблинг (|   вместо пробелов)
    is_dir: bool,
    size: u32,
    parent: u8,        // индекс строки-родителя (только depth 1)
    name: [u8; 32],
}

fn zlen(n: &[u8]) -> usize { n.iter().position(|&c| c == 0).unwrap_or(n.len()) }

/// Построить полный путь строки: cwd[/parent]/name. Возвращает длину.
fn row_path(cwd: &[u8], rows: &[FmRow], idx: usize, out: &mut [u8; 128]) -> usize {
    let mut p = 0usize;
    let cl = cwd.len().min(120);
    out[..cl].copy_from_slice(&cwd[..cl]); p = cl;
    if rows[idx].depth == 1 {
        let pr = &rows[rows[idx].parent as usize];
        let pl = zlen(&pr.name);
        if p > 1 && p < 127 { out[p] = b'/'; p += 1; }
        let cn = pl.min(127 - p);
        out[p..p + cn].copy_from_slice(&pr.name[..cn]); p += cn;
    }
    let nl = zlen(&rows[idx].name);
    if p > 1 && p < 127 { out[p] = b'/'; p += 1; }
    let cn = nl.min(127 - p);
    out[p..p + cn].copy_from_slice(&rows[idx].name[..cn]); p += cn;
    p
}

fn sw() -> u32 { crate::display::width() }
fn sh() -> u32 { crate::display::height() }

fn draw_char(x: u32, y: u32, ch: u8, fg: u32, bg: u32) {
    let f = display::framebuffer(); let s = display::stride() as i32;
    let idx = ch as usize * 16;
    if idx+16 > FONT_8X16.len() { return; }
    let g=&FONT_8X16[idx..idx+16];
    let fr=((fg>>16)&0xFF)as u8; let fg2=((fg>>8)&0xFF)as u8; let fb=(fg&0xFF)as u8;
    let br=((bg>>16)&0xFF)as u8; let bg2=((bg>>8)&0xFF)as u8; let bb=(bg&0xFF)as u8;
    let is_bgr=display::is_bgr();
    for r in 0..16 {
        let bits=g[r];
        for c in 0..8 {
            let px=x+c; let py=y+r as u32;
            if px>=sw()||py>=sh(){continue;}
            let off=(py as i32*s + px as i32) as usize*4;
            let use_fg = bits & (0x80>>c) !=0;
            unsafe{
                if is_bgr {
                    if use_fg{*f.add(off)=fb;*f.add(off+1)=fg2;*f.add(off+2)=fr;}
                    else{*f.add(off)=bb;*f.add(off+1)=bg2;*f.add(off+2)=br;}
                } else {
                    if use_fg{*f.add(off)=fr;*f.add(off+1)=fg2;*f.add(off+2)=fb;}
                    else{*f.add(off)=br;*f.add(off+1)=bg2;*f.add(off+2)=bb;}
                }
            }
        }
    }
}
fn draw_text(x: u32, y: u32, s: &[u8], fg:u32, bg:u32){ let mut cx=x; for &c in s{ draw_char(cx,y,c,fg,bg); cx+=8; } }
fn fill_rect(x:u32,y:u32,w:u32,h:u32,col:u32){
    let f=display::framebuffer(); let s=display::stride() as i32;
    let fr=((col>>16)&0xFF)as u8; let fg=((col>>8)&0xFF)as u8; let fb=(col&0xFF)as u8;
    let is_bgr=display::is_bgr();
    for dy in 0..h{ let py=y+dy; if py>=sh(){break;} for dx in 0..w{ let px=x+dx; if px>=sw(){break;} let off=(py as i32*s+px as i32) as usize*4; unsafe{if is_bgr{*f.add(off)=fb;*f.add(off+1)=fg;*f.add(off+2)=fr;}else{*f.add(off)=fr;*f.add(off+1)=fg;*f.add(off+2)=fb;}}}}
}
fn present() { display::mark_dirty(0,0,sw(),sh()); unsafe{ display::present(); } }

pub fn run(start_path: &[u8]) {
    let mut cwd=[0u8;128]; let mut cwd_len=0usize;
    let sp = if start_path.is_empty(){ b"/" } else { start_path };
    let l=sp.len().min(127); cwd[..l].copy_from_slice(&sp[..l]); cwd_len=l;
    if cwd_len==0 { cwd[0]=b'/'; cwd_len=1; }
    let mut sel=0usize;
    loop{
        // draw background gradient (reuse dbs_gr gradient via display clear)
        // simple dark bg
        fill_rect(0,0,sw(),sh(),0x202020);
        // top bar
        fill_rect(0,0,sw(),16,0xE95420);
        draw_text(4,0,b"DBS-FM  File Manager  [Up/Down Enter Bksp q]",0xFFFFFF,0xE95420);
        // path
        fill_rect(0,16,sw(),16,0x303030);
        draw_text(4,16,&cwd[..cwd_len],0xFFFFFF,0x303030);
        // entries -> дерево: cwd (depth 0) + дети папок (depth 1)
        let mut entries=[crate::vfs::DirEntry{name:[0;32], is_dir:false, size:0}; FM_MAX_ENTRIES];
        let n = crate::vfs::readdir(&cwd[..cwd_len], &mut entries);
        let n = if n<0 {0} else {n as usize}.min(FM_MAX_ENTRIES);
        let mut rows=[FmRow{depth:0,last:false,parent_next:false,is_dir:false,size:0,parent:0,name:[0;32]}; FM_MAX_ROWS];
        let mut rn=0usize;
        let mut expanded_dirs=0usize;
        for i in 0..n {
            if rn >= FM_MAX_ROWS { break; }
            let e=&entries[i];
            let last = i + 1 >= n;
            rows[rn]=FmRow{depth:0,last,parent_next:false,is_dir:e.is_dir,size:e.size,parent:0,name:e.name};
            let row_idx=rn; rn+=1;
            // Раскрыть папку на уровень: дети с линиями иерархии
            if e.is_dir && expanded_dirs < FM_MAX_CHILD_DIRS && rn + 1 < FM_MAX_ROWS {
                let nl=zlen(&e.name);
                let mut child_path=[0u8;128]; let mut cp=0usize;
                let cc=cwd_len.min(90);
                child_path[..cc].copy_from_slice(&cwd[..cc]); cp=cc;
                if cp>1 && cp<127 { child_path[cp]=b'/'; cp+=1; }
                let cn=nl.min(127-cp);
                child_path[cp..cp+cn].copy_from_slice(&e.name[..cn]); cp+=cn;
                let mut cent=[crate::vfs::DirEntry{name:[0;32], is_dir:false, size:0}; 16];
                let cn2 = crate::vfs::readdir(&child_path[..cp], &mut cent);
                let cn2 = if cn2<0 {0} else {cn2 as usize}.min(FM_MAX_CHILDREN);
                if cn2>0 { expanded_dirs+=1; }
                for j in 0..cn2 {
                    if rn >= FM_MAX_ROWS { break; }
                    rows[rn]=FmRow{depth:1,last:j+1>=cn2,parent_next:!last,
                        is_dir:cent[j].is_dir,size:cent[j].size,parent:row_idx as u8,name:cent[j].name};
                    rn+=1;
                }
            }
        }
        // handle empty
        if rn==0 {
            draw_text(4, 40, b"<empty>",0xAEA79F,0x202020);
        } else {
            if sel>=rn { sel=rn-1; }
            // окно прокрутки чтобы sel всегда виден
            let vis_rows=((sh().saturating_sub(64))/16).max(1) as usize;
            let start=if sel+1>vis_rows{sel+1-vis_rows}else{0};
            let mut vi=0usize;
            for i in start..rn {
                let y=40 + vi as u32*16;
                if y+16>=sh(){break;}
                let r=&rows[i];
                let is_sel = i==sel;
                let bg=if is_sel{FM_SEL_BG}else{0x202020};
                let fg=if is_sel{0xFFFFFF}else{if r.is_dir{FM_DIR_FG}else{FM_FILE_FG}};
                fill_rect(0,y,sw(),16,bg);
                // префикс иерархии: `-- / |--  (+ родительская полоска на depth 1)
                let mut pre=[0u8;8]; let mut pl=0usize;
                if r.depth==1 {
                    let pp=if r.parent_next{b"|"}else{b" "};
                    pre[0]=pp[0]; pre[1]=b' '; pre[2]=b' '; pre[3]=b' '; pl=4;
                }
                let tail=if r.last{b"`-- "}else{b"|-- "};
                pre[pl]=tail[0]; pre[pl+1]=tail[1]; pre[pl+2]=tail[2]; pre[pl+3]=tail[3]; pl+=4;
                draw_text(4,y,&pre[..pl],FM_LINE_FG,bg);
                let nx=4 + pl as u32*8;
                // имя: папка — просто её название серым, без иконок и суффиксов
                let nl=zlen(&r.name).min(40);
                draw_text(nx,y,&r.name[..nl],fg,bg);
                if !r.is_dir {
                    // size
                    let mut sbuf=[0u8;12]; let mut sl=0;
                    let mut v=r.size;
                    if v==0{ sbuf[0]=b'0'; sl=1;} else { while v>0{ sbuf[sl]=b'0'+(v%10) as u8; v/=10; sl+=1; } // reverse
                        for j in 0..sl/2{ let t=sbuf[j]; sbuf[j]=sbuf[sl-1-j]; sbuf[sl-1-j]=t; }
                    }
                    draw_text(sw()-80,y,&sbuf[..sl],0xAEA79F,bg);
                }
                vi+=1;
            }
        }
        // footer
        fill_rect(0, sh()-16, sw(),16,0x303030);
        draw_text(4, sh()-16, b"Enter:open  Bksp:up  n:new  d:del  q:quit",0xAEA79F,0x303030);
        // layout indicator
        let layout = crate::driver::ps2::layout_name();
        draw_text(sw()-40, 0, layout.as_bytes(),0xFFFFFF,0xE95420);
        present();
        // input
        let mut waited=0;
        loop{
            if let Some(c)=ps2::poll_char(){
                match c {
                    ps2::KEY_UP => { if sel>0{sel-=1;} break; }
                    ps2::KEY_DOWN => { if sel+1<rn{sel+=1;} break; }
                    b'\n'|b'\r' => {
                        if rn>0 {
                            let r=&rows[sel];
                            let nl=zlen(&r.name);
                            if r.is_dir {
                                let nm=&r.name[..nl];
                                if nm == b"." {
                                    // остаться: ничего не делать
                                } else if nm == b".." {
                                    // вверх как Backspace
                                    if cwd_len>1 {
                                        let mut end=cwd_len;
                                        while end>1 && cwd[end-1]==b'/'{end-=1;}
                                        while end>1 && cwd[end-1]!=b'/'{end-=1;}
                                        if end==1{ cwd_len=1; } else { cwd_len=end-1; }
                                        sel=0;
                                    }
                                } else {
                                    // cd в полный путь строки (учитывает depth)
                                    let mut np=[0u8;128];
                                    let pl=row_path(&cwd[..cwd_len], &rows, sel, &mut np);
                                    cwd_len=pl.min(127); cwd[..cwd_len].copy_from_slice(&np[..cwd_len]);
                                    sel=0;
                                }
                            } else {
                                // preview via cat
                                let mut full=[0u8;128];
                                let fl=row_path(&cwd[..cwd_len], &rows, sel, &mut full);
                                // flash preview (resume after key)
                                fill_rect(0, sh()/2-40, sw(),80,0x000000);
                                draw_text(4, sh()/2-32, b"File: ",0xE95420,0x000000);
                                draw_text(4+6*8, sh()/2-32, &r.name[..nl],0xFFFFFF,0x000000);
                                draw_text(4, sh()/2-16, b"Press any key...",0xAEA79F,0x000000);
                                present();
                                while ps2::poll_char().is_none() && crate::driver::uart::poll_char().is_none(){ timer::usleep(10000); }
                                let _ = ps2::poll_char(); // consume
                                let _ = fl;
                            }
                        }
                        break;
                    }
                    0x08|0x7F|ps2::KEY_LEFT => { // Backspace -> up
                        // go to parent
                        if cwd_len>1 {
                            let mut end=cwd_len;
                            while end>1 && cwd[end-1]==b'/'{end-=1;}
                            while end>1 && cwd[end-1]!=b'/'{end-=1;}
                            if end==1{ cwd_len=1; } else { cwd_len=end-1; }
                            sel=0;
                        }
                        break;
                    }
                    b'q'|b'Q'|27 => { return; }
                    b'n'|b'N' => {
                        // create file (simple)
                        let mut name=[0u8;32]; let mut nl=0;
                        fill_rect(sw()/2-100, sh()/2-20, 200, 40,0x303030);
                        draw_text(sw()/2-80, sh()/2-8, b"New name:",0xFFFFFF,0x303030);
                        present();
                        // reuse simple input (poll)
                        let mut tmp=[0u8;32]; let mut tl=0;
                        loop{
                            if let Some(ch)=ps2::poll_char(){
                                if ch==b'\n'{ break; }
                                if ch==0x08 && tl>0{ tl-=1; }
                                else if ch>=32 && ch<127 && tl<31{ tmp[tl]=ch; tl+=1; }
                                // redraw input line
                                fill_rect(sw()/2-100, sh()/2+10, 200, 16,0x202020);
                                draw_text(sw()/2-80, sh()/2+10, &tmp[..tl],0xFFFFFF,0x202020);
                                present();
                            } else { timer::usleep(10000); }
                        }
                        if tl>0 {
                            let mut full=[0u8;128]; let mut fl=0;
                            full[..cwd_len].copy_from_slice(&cwd[..cwd_len]);
                            fl=cwd_len;
                            if fl>1{ full[fl]=b'/'; fl+=1; }
                            full[fl..fl+tl].copy_from_slice(&tmp[..tl]);
                            fl+=tl;
                            let _ = crate::vfs::open(&full[..fl], 0x100|1); // create empty
                        }
                        break;
                    }
                    b'd'|b'D'|ps2::KEY_DELETE => {
                        if rn>0 {
                            let r=&rows[sel];
                            let mut full=[0u8;128];
                            let fl=row_path(&cwd[..cwd_len], &rows, sel, &mut full);
                            if r.is_dir{ let _=crate::vfs::rmdir(&full[..fl]); } else { let _=crate::vfs::unlink(&full[..fl]); }
                        }
                        break;
                    }
                    _=>{}
                }
                // also check uart
            } else if let Some(ch)=crate::driver::uart::poll_char(){
                if ch==b'q'{return;}
                if ch==b'\n'{ }
            } else {
                timer::usleep(10000);
                waited+=1;
                if waited>200 { // allow gradient refresh? just continue
                }
            }
            if waited> 1000 { break; } // avoid hang
        }
    }
}
