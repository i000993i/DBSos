//! tmpfs — RAM-ФС для /tmp (не было вообще: только FAT).
//!
//! Простая плоская таблица: 32 файла, до 8KB каждый через kmalloc.
//! Покрывает VFS FsOps: open/close/read/write/stat/readdir/mkdir/rmdir/unlink.
//! Потокобезопасность: вызывается под cli из VFS/syscall путей (как FAT).

use crate::vfs::{StatInfo, DirEntry, MAX_NAME};

const MAX_FILES: usize = 32;
const MAX_FILE_SIZE: usize = 8192;

#[derive(Clone, Copy)]
struct TmpFile {
    in_use: bool,
    is_dir: bool,
    name: [u8; MAX_NAME],
    name_len: usize,
    data: *mut u8,
    size: usize,
    cap: usize,
}

impl TmpFile {
    const fn empty() -> Self {
        TmpFile { in_use: false, is_dir: false, name: [0; MAX_NAME], name_len: 0, data: core::ptr::null_mut(), size: 0, cap: 0 }
    }
}

static mut FILES: [TmpFile; MAX_FILES] = [TmpFile::empty(); MAX_FILES];
static mut HANDLES: [Option<usize>; MAX_FILES] = [None; MAX_FILES];

fn norm(path: &[u8]) -> &[u8] {
    // "/a" -> "a", "/" -> "" (root)
    let mut p = path;
    while p.len() > 1 && p[0] == b'/' { p = &p[1..]; }
    if p == b"/" { return b""; }
    // отрезать trailing '/'
    while p.len() > 1 && p[p.len()-1] == b'/' { p = &p[..p.len()-1]; }
    // только плоский корень: взять последний компонент
    if let Some(pos) = p.iter().rposition(|&c| c == b'/') {
        &p[pos+1..]
    } else { p }
}

fn find(name: &[u8]) -> Option<usize> {
    if name.is_empty() { return None; }
    unsafe {
        for i in 0..MAX_FILES {
            if FILES[i].in_use && !FILES[i].is_dir
                && FILES[i].name_len == name.len()
                && FILES[i].name[..name.len()] == *name {
                return Some(i);
            }
        }
    }
    None
}

fn free_slot() -> Option<usize> {
    unsafe { (0..MAX_FILES).find(|&i| !FILES[i].in_use) }
}

fn tmp_open(path: &[u8], flags: u64) -> Option<usize> {
    let name = norm(path);
    // root open для readdir/stat — хендл !0
    if name.is_empty() { return Some(!0usize); }
    if name.len() > MAX_NAME { return None; }
    if let Some(idx) = find(name) {
        // O_TRUNC = 0x200
        if flags & 0x200 != 0 {
            unsafe { FILES[idx].size = 0; }
        }
        // занять хендл
        unsafe {
            if let Some(h) = (0..MAX_FILES).find(|&h| HANDLES[h].is_none()) {
                HANDLES[h] = Some(idx);
                return Some(h);
            }
        }
        return None;
    }
    // O_CREAT = 0x100
    if flags & 0x100 == 0 { return None; }
    let slot = free_slot()?;
    unsafe {
        let data = crate::heap::kmalloc(MAX_FILE_SIZE);
        if data.is_null() { return None; }
        FILES[slot].in_use = true;
        FILES[slot].is_dir = false;
        FILES[slot].name[..name.len()].copy_from_slice(name);
        FILES[slot].name_len = name.len();
        FILES[slot].data = data;
        FILES[slot].size = 0;
        FILES[slot].cap = MAX_FILE_SIZE;
        if let Some(h) = (0..MAX_FILES).find(|&h| HANDLES[h].is_none()) {
            HANDLES[h] = Some(slot);
            return Some(h);
        }
        crate::heap::kfree(data);
        FILES[slot] = TmpFile::empty();
    }
    None
}

fn tmp_close(handle: usize) {
    if handle == !0usize { return; }
    unsafe { if handle < MAX_FILES { HANDLES[handle] = None; } }
}

fn tmp_read(handle: usize, buf: &mut [u8], offset: u64) -> Option<usize> {
    if handle == !0usize { return Some(0); }
    unsafe {
        if handle >= MAX_FILES { return None; }
        let idx = HANDLES[handle]?;
        let f = &FILES[idx];
        let off = offset as usize;
        if off >= f.size { return Some(0); }
        let n = buf.len().min(f.size - off);
        core::ptr::copy_nonoverlapping(f.data.add(off), buf.as_mut_ptr(), n);
        Some(n)
    }
}

fn tmp_write(handle: usize, buf: &[u8], offset: u64) -> Option<usize> {
    if handle == !0usize { return None; }
    unsafe {
        if handle >= MAX_FILES { return None; }
        let idx = HANDLES[handle]?;
        let f = &mut FILES[idx];
        let off = offset as usize;
        if off > MAX_FILE_SIZE { return None; }
        let n = buf.len().min(MAX_FILE_SIZE.saturating_sub(off));
        core::ptr::copy_nonoverlapping(buf.as_ptr(), f.data.add(off), n);
        if off + n > f.size { f.size = off + n; }
        Some(n)
    }
}

fn tmp_stat(path: &[u8]) -> Option<StatInfo> {
    let name = norm(path);
    if name.is_empty() {
        return Some(StatInfo { size: 0, is_dir: true, cluster: 0 });
    }
    unsafe {
        let idx = find(name)?;
        Some(StatInfo { size: FILES[idx].size as u32, is_dir: false, cluster: idx as u32 })
    }
}

fn tmp_readdir(path: &[u8], entries: &mut [DirEntry]) -> Option<usize> {
    let name = norm(path);
    // только корень
    if !name.is_empty() { return None; }
    let mut n = 0;
    unsafe {
        for i in 0..MAX_FILES {
            if FILES[i].in_use && n < entries.len() {
                let mut nm = [0u8; 32];
                let l = FILES[i].name_len.min(32);
                nm[..l].copy_from_slice(&FILES[i].name[..l]);
                entries[n] = DirEntry { name: nm, is_dir: false, size: FILES[i].size as u32 };
                n += 1;
            }
        }
    }
    Some(n)
}

fn tmp_mkdir(_path: &[u8]) -> bool { true }
fn tmp_rmdir(_path: &[u8]) -> bool { true }

fn tmp_unlink(path: &[u8]) -> bool {
    let name = norm(path);
    unsafe {
        let idx = match find(name) { Some(i) => i, None => return false };
        // закрыть хендлы
        for h in 0..MAX_FILES { if HANDLES[h] == Some(idx) { HANDLES[h] = None; } }
        if !FILES[idx].data.is_null() { crate::heap::kfree(FILES[idx].data); }
        FILES[idx] = TmpFile::empty();
        true
    }
}

static TMP_OPS: crate::vfs::FsOps = crate::vfs::FsOps {
    open: tmp_open,
    close: tmp_close,
    read: tmp_read,
    write: tmp_write,
    stat: tmp_stat,
    readdir: tmp_readdir,
    mkdir: tmp_mkdir,
    rmdir: tmp_rmdir,
    unlink: tmp_unlink,
};

/// Mount tmpfs at /tmp. Возвращает true при успехе.
pub fn init() -> bool {
    let idx = unsafe { crate::vfs::register_driver(&TMP_OPS) };
    if idx == !0usize { return false; }
    let ok = unsafe { crate::vfs::mount(b"/tmp", idx) };
    if ok {
        crate::driver::uart::write_str("[tmpfs] mounted at /tmp (32x8K RAM)\r\n");
    }
    ok
}
