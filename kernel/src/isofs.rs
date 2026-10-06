//! ISO9660 read-only (CD-ROM через IDE ATAPI).
//!
//! Для VirtualBox: загрузочный CD виден только через IDE — без этого
//! модуля на VBox нет вообще никакой ФС (NVMe-диска там нет).
//! Подмножество: PVD → root extent → обход каталогов, файлы одним
//! экстентом (наш mk_iso.py такие и кладёт), версии ";1" отрезаются.
//! Запись/mkdir невозможны (read-only) — возвращают false.

use crate::vfs::{StatInfo, DirEntry};
use crate::driver::uart;

fn up(s: &str) { uart::write_str(s); }

static mut READY: bool = false;
static mut ROOT_LBA: u32 = 0; // в 2048-единицах

fn read_sector(lba: u32, buf: &mut [u8; 2048]) -> bool {
    crate::driver::ide::cdrom_read(lba, 1, buf.as_mut_ptr(), 2048)
}

/// Инициализация: PVD в секторе 16, корень из него. Возвращает успех.
pub fn init() -> bool {
    let mut pvd = [0u8; 2048];
    if !read_sector(16, &mut pvd) {
        return false;
    }
    if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
        up("[ISO] no PVD\r\n");
        return false;
    }
    // root record at 156: extent LE +2, size LE +10
    let ext = u32::from_le_bytes([pvd[158], pvd[159], pvd[160], pvd[161]]);
    unsafe {
        ROOT_LBA = ext;
        READY = true;
    }
    up("[ISO] ISO9660 mounted, root=");
    dec(ext as u64);
    up("\r\n");
    true
}

fn dec(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut b = [0u8; 20];
    let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putchar(b[i]); }
}

fn le32(b: &[u8], o: usize) -> u32 {
    b[o] as u32 | ((b[o + 1] as u32) << 8) | ((b[o + 2] as u32) << 16) | ((b[o + 3] as u32) << 24)
}

/// Имя записи без версии ";1" (для файлов).
fn strip_ver(name: &[u8]) -> &[u8] {
    if name.len() > 2 && name[name.len() - 2] == b';' {
        &name[..name.len() - 2]
    } else {
        name
    }
}

/// Найти запись `comp` в каталоге с экстентом dir_lba.
/// Возвращает (extent, size, flags).
/// Чистый парсинг одного 2048-сектора каталога: найти `comp`.
/// Возвращает (extent, size, flags). Используется и вживую, и в KAT.
fn scan_sector(buf: &[u8; 2048], comp: &[u8]) -> Option<(u32, u32, u8)> {
    let mut o = 0usize;
    while o + 33 <= 2048 {
        let rl = buf[o] as usize;
        if rl == 0 {
            break; // паддинг до конца сектора
        }
        if o + rl > 2048 {
            break;
        }
        let flags = buf[o + 25];
        let nl = buf[o + 32] as usize;
        if nl > 0 && o + 33 + nl <= 2048 {
            let nm = &buf[o + 33..o + 33 + nl];
            // '.' и '..' имеют коды 0/1 длиной 1
            let is_dot = nl == 1 && (nm[0] == 0 || nm[0] == 1);
            let cmp: &[u8] = if is_dot { nm } else { strip_ver(nm) };
            let want_dot = comp == b"." || comp == b"..";
            if is_dot == want_dot && cmp.len() == comp.len() {
                let mut eq = true;
                for k in 0..cmp.len() {
                    let mut a = cmp[k];
                    let mut b = comp[k];
                    if !is_dot {
                        if a >= b'a' && a <= b'z' {
                            a -= 32;
                        }
                        if b >= b'a' && b <= b'z' {
                            b -= 32;
                        }
                    }
                    if a != b {
                        eq = false;
                        break;
                    }
                }
                if eq {
                    if is_dot {
                        let ok = (nm[0] == 0 && comp == b".")
                            || (nm[0] == 1 && comp == b"..");
                        if !ok {
                            o += rl;
                            continue;
                        }
                    }
                    let ext = le32(buf, o + 2);
                    let sz = le32(buf, o + 10);
                    return Some((ext, sz, flags));
                }
            }
        }
        o += rl;
    }
    None
}

fn find_in_dir(dir_lba: u32, comp: &[u8]) -> Option<(u32, u32, u8)> {
    let mut lba = dir_lba;
    // каталог может занимать несколько секторов; идём пока не встретим
    // пустой сектор (эвристика достаточна для наших дисков: ≤8 секторов)
    for _ in 0..8 {
        let mut buf = [0u8; 2048];
        if !read_sector(lba, &mut buf) {
            return None;
        }
        if let Some(v) = scan_sector(&buf, comp) {
            return Some(v);
        }
        // пустой сектор = конец каталога
        if buf[0] == 0 {
            break;
        }
        lba += 1;
    }
    None
}

/// Резолв абсолютного пути -> (extent, size, is_dir).
fn resolve(path: &[u8]) -> Option<(u32, u32, bool)> {
    if !unsafe { READY } {
        return None;
    }
    // корень
    let mut p = path;
    while !p.is_empty() && p[0] == b'/' {
        p = &p[1..];
    }
    if p.is_empty() {
        return Some((unsafe { ROOT_LBA }, 2048, true));
    }
    let mut dir = unsafe { ROOT_LBA };
    loop {
        // следующий компонент
        let mut e = 0;
        while e < p.len() && p[e] != b'/' {
            e += 1;
        }
        let comp = &p[..e];
        let (ext, sz, flags) = find_in_dir(dir, comp)?;
        let mut rest = &p[e..];
        while !rest.is_empty() && rest[0] == b'/' {
            rest = &rest[1..];
        }
        if rest.is_empty() {
            return Some((ext, sz, flags & 2 != 0));
        }
        if flags & 2 == 0 {
            return None; // не каталог, а путь идёт дальше
        }
        dir = ext;
        p = rest;
    }
}

// ── Таблица открытых файлов ────────────────────────────────────────

const MAX_OPEN: usize = 16;

#[derive(Clone, Copy)]
struct Handle {
    used: bool,
    extent: u32,
    size: u32,
}

impl Handle {
    const fn empty() -> Self {
        Handle { used: false, extent: 0, size: 0 }
    }
}

static mut HANDLES: [Handle; MAX_OPEN] = [Handle::empty(); MAX_OPEN];

fn iso_open(path: &[u8], _flags: u64) -> Option<usize> {
    let (ext, sz, is_dir) = resolve(path)?;
    if is_dir {
        return Some(!0usize); // корень/каталог для readdir
    }
    unsafe {
        for i in 0..MAX_OPEN {
            if !HANDLES[i].used {
                HANDLES[i] = Handle { used: true, extent: ext, size: sz };
                return Some(i);
            }
        }
        None
    }
}

fn iso_close(handle: usize) {
    if handle == !0usize {
        return;
    }
    unsafe {
        if handle < MAX_OPEN {
            HANDLES[handle] = Handle::empty();
        }
    }
}

fn iso_read(handle: usize, buf: &mut [u8], offset: u64) -> Option<usize> {
    if handle == !0usize {
        return Some(0);
    }
    let (extent, size) = unsafe {
        if handle >= MAX_OPEN || !HANDLES[handle].used {
            return None;
        }
        (HANDLES[handle].extent, HANDLES[handle].size)
    };
    if offset >= size as u64 || buf.is_empty() {
        return Some(0);
    }
    let mut total = 0usize;
    let mut off = offset;
    let end = (offset + buf.len() as u64).min(size as u64);
    while off < end {
        let sec = extent + (off / 2048) as u32;
        let so = (off % 2048) as usize;
        let mut bounce = [0u8; 2048];
        if !read_sector(sec, &mut bounce) {
            break;
        }
        let n = (2048 - so).min((end - off) as usize);
        buf[total..total + n].copy_from_slice(&bounce[so..so + n]);
        total += n;
        off += n as u64;
    }
    Some(total)
}

fn iso_stat(path: &[u8]) -> Option<StatInfo> {
    let (_ext, sz, is_dir) = resolve(path)?;
    Some(StatInfo { size: sz, is_dir, cluster: 0 })
}

fn iso_readdir(path: &[u8], entries: &mut [DirEntry]) -> Option<usize> {
    let (ext, _sz, is_dir) = resolve(path)?;
    if !is_dir {
        return None;
    }
    let mut n = 0usize;
    let mut lba = ext;
    'outer: for _ in 0..8 {
        let mut buf = [0u8; 2048];
        if !read_sector(lba, &mut buf) {
            break;
        }
        let mut o = 0usize;
        let mut any = false;
        while o + 33 <= 2048 {
            let rl = buf[o] as usize;
            if rl == 0 {
                break;
            }
            if o + rl > 2048 {
                break;
            }
            any = true;
            let flags = buf[o + 25];
            let nl = buf[o + 32] as usize;
            if nl > 0 && o + 33 + nl <= 2048 && n < entries.len() {
                let nm = &buf[o + 33..o + 33 + nl];
                // '.'/'..' показываем как есть
                let (show, show_len) = if nl == 1 && (nm[0] == 0 || nm[0] == 1) {
                    (&b"."[..], 1)
                } else {
                    let s = strip_ver(nm);
                    (s, s.len().min(32))
                };
                // '..' как две точки
                if nl == 1 && nm[0] == 1 {
                    entries[n].name[0] = b'.';
                    entries[n].name[1] = b'.';
                    for k in 2..32 {
                        entries[n].name[k] = 0;
                    }
                } else {
                    for k in 0..32 {
                        entries[n].name[k] = if k < show_len { show[k] } else { 0 };
                    }
                }
                entries[n].is_dir = flags & 2 != 0;
                entries[n].size = le32(&buf, o + 10);
                n += 1;
            }
            o += rl;
        }
        if !any {
            break 'outer;
        }
        lba += 1;
    }
    Some(n)
}

fn iso_no_mkdir(_p: &[u8]) -> bool {
    false
}
fn iso_no_rmdir(_p: &[u8]) -> bool {
    false
}
fn iso_no_unlink(_p: &[u8]) -> bool {
    false
}
fn iso_no_write(_h: usize, _b: &[u8], _o: u64) -> Option<usize> {
    None
}

static ISO_OPS: crate::vfs::FsOps = crate::vfs::FsOps {
    open: iso_open,
    close: iso_close,
    read: iso_read,
    write: iso_no_write,
    stat: iso_stat,
    readdir: iso_readdir,
    mkdir: iso_no_mkdir,
    rmdir: iso_no_rmdir,
    unlink: iso_no_unlink,
};

/// Зарегистрировать драйвер (монтирование — в lib.rs fallback).
pub fn register() -> usize {
    unsafe { crate::vfs::register_driver(&ISO_OPS) }
}

/// KAT: синтетический root-сектор гоняется через настоящий scan_sector.
/// Биты: 1=strip_ver, 2=le32, 4=file miss, 8=file extent/size,
/// 16=case-insensitive, 32=dot/dotdot, 64=missing->None. 0 = ok.
pub fn self_test() -> u32 {
    if strip_ver(b"HI.TXT;1") != b"HI.TXT" {
        return 1;
    }
    if le32(&[0x78, 0x56, 0x34, 0x12], 0) != 0x12345678 {
        return 2;
    }
    // root-сектор: '.', '..', 'HI.TXT;1' (extent 18, size 5, файл)
    let mut sec = [0u8; 2048];
    let mut o = 0usize;
    let mut emit = |o: &mut usize, extent: u32, size: u32, flags: u8, name: &[u8]| {
        let rl = 33 + name.len() + (if name.len() % 2 == 0 { 1 } else { 0 });
        sec[*o] = rl as u8;
        sec[*o + 25] = flags;
        sec[*o + 2..*o + 6].copy_from_slice(&extent.to_le_bytes());
        sec[*o + 10..*o + 14].copy_from_slice(&size.to_le_bytes());
        sec[*o + 32] = name.len() as u8;
        sec[*o + 33..*o + 33 + name.len()].copy_from_slice(name);
        *o += rl;
    };
    emit(&mut o, 17, 2048, 2, &[0]);
    emit(&mut o, 17, 2048, 2, &[1]);
    emit(&mut o, 18, 5, 0, b"HI.TXT;1");
    match scan_sector(&sec, b"HI.TXT") {
        Some((18, 5, 0)) => {}
        _ => return 4,
    }
    match scan_sector(&sec, b"hi.txt") {
        Some((18, 5, _)) => {}
        _ => return 16,
    }
    match scan_sector(&sec, b".") {
        Some((17, _, f)) if f & 2 != 0 => {}
        _ => return 32,
    }
    match scan_sector(&sec, b"..") {
        Some((17, _, _)) => {}
        _ => return 32,
    }
    if scan_sector(&sec, b"NOPE").is_some() {
        return 64;
    }
    0
}
