// VFS (Virtual File System) — единый интерфейс для всех файловых систем.
//
// Архитектура:
//   Mount table: массив (prefix, driver). prefix = "/mnt/data", driver = &FatFs.
//   open("/mnt/data/file.txt") → strip prefix → delegate to driver.
//   Операции: open, close, read, write, stat, readdir, mkdir, rmdir, unlink.
//
// Каждая ФС реализует набор function pointers (FsOps). VFS просто маршрутизирует.

use crate::driver::uart;

pub const MAX_MOUNTS: usize = 8;
pub const MAX_OPEN_FILES: usize = 64;
pub const MAX_PATH: usize = 128;
pub const MAX_NAME: usize = 32;

fn uart_print(s: &str) { uart::write_str(s); }

// ── Stat info ───────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct StatInfo {
    pub size: u32,
    pub is_dir: bool,
    pub cluster: u32,
}

// ── Directory entry (for readdir) ────────────────────────────────

#[derive(Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; MAX_NAME],
    pub is_dir: bool,
    pub size: u32,
}

// ── Filesystem operations (function pointers) ────────────────────

pub type FnOpen = fn(path: &[u8], flags: u64) -> Option<usize>;
pub type FnClose = fn(handle: usize);
pub type FnRead = fn(handle: usize, buf: &mut [u8], offset: u64) -> Option<usize>;
pub type FnWrite = fn(handle: usize, buf: &[u8], offset: u64) -> Option<usize>;
pub type FnStat = fn(path: &[u8]) -> Option<StatInfo>;
pub type FnReaddir = fn(path: &[u8], entries: &mut [DirEntry]) -> Option<usize>;
pub type FnMkdir = fn(path: &[u8]) -> bool;
pub type FnRmdir = fn(path: &[u8]) -> bool;
pub type FnUnlink = fn(path: &[u8]) -> bool;

pub struct FsOps {
    pub open: FnOpen,
    pub close: FnClose,
    pub read: FnRead,
    pub write: FnWrite,
    pub stat: FnStat,
    pub readdir: FnReaddir,
    pub mkdir: FnMkdir,
    pub rmdir: FnRmdir,
    pub unlink: FnUnlink,
}

// ── Driver registry ──────────────────────────────────────────────

static mut DRIVERS: [*const FsOps; MAX_MOUNTS] = [core::ptr::null(); MAX_MOUNTS];
static mut DRIVER_COUNT: usize = 0;

/// Register a filesystem driver. Returns its index.
pub unsafe fn register_driver(ops: *const FsOps) -> usize {
    let idx = DRIVER_COUNT;
    if idx >= MAX_MOUNTS { return !0usize; }
    DRIVERS[idx] = ops;
    DRIVER_COUNT += 1;
    idx
}

/// Get a driver by index.
unsafe fn get_driver(idx: usize) -> Option<&'static FsOps> {
    if idx >= DRIVER_COUNT { return None; }
    let ptr = DRIVERS[idx];
    if ptr.is_null() { return None; }
    Some(&*ptr)
}

// ── Mount table ──────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct MountEntry {
    prefix: [u8; MAX_PATH],
    prefix_len: usize,
    driver_idx: usize,
    in_use: bool,
}

static mut MOUNTS: [MountEntry; MAX_MOUNTS] = [MountEntry {
    prefix: [0; MAX_PATH],
    prefix_len: 0,
    driver_idx: 0,
    in_use: false,
}; MAX_MOUNTS];

/// Mount a filesystem at a given prefix path.
pub unsafe fn mount(prefix: &[u8], driver_idx: usize) -> bool {
    if driver_idx >= DRIVER_COUNT { return false; }

    let slot = (0..MAX_MOUNTS).find(|&i| !MOUNTS[i].in_use);
    let slot = match slot {
        Some(s) => s,
        None => return false,
    };

    let m = &mut MOUNTS[slot];
    let len = prefix.len().min(MAX_PATH - 1);
    m.prefix[..len].copy_from_slice(&prefix[..len]);
    m.prefix_len = len;
    m.driver_idx = driver_idx;
    m.in_use = true;
    true
}

/// Find the mount entry for a path and return (driver_idx, prefix_len).
fn resolve_mount(path: &[u8]) -> Option<(usize, usize)> {
    unsafe {
        let mut best_match = None;
        let mut best_len = 0usize;
        for i in 0..MAX_MOUNTS {
            let m = &MOUNTS[i];
            if !m.in_use { continue; }
            let pfx = &m.prefix[..m.prefix_len];
            if path.len() >= m.prefix_len && &path[..m.prefix_len] == pfx {
                if m.prefix_len > best_len {
                    best_len = m.prefix_len;
                    best_match = Some((m.driver_idx, m.prefix_len));
                }
            }
        }
        // Fall through to root mount (prefix "/")
        if best_match.is_none() {
            for i in 0..MAX_MOUNTS {
                let m = &MOUNTS[i];
                if !m.in_use { continue; }
                if m.prefix_len == 1 && m.prefix[0] == b'/' {
                    return Some((m.driver_idx, 1));
                }
            }
            return None;
        }
        best_match
    }
}

/// Strip the mount prefix from a path.
fn strip_prefix(full_path: &[u8], prefix_len: usize) -> &[u8] {
    let mut i = prefix_len;
    while i < full_path.len() && full_path[i] == b'/' { i += 1; }
    &full_path[i..]
}

// ── Open file table ─────────────────────────────────────────────

pub struct VfsFile {
    pub driver_idx: usize,
    pub handle: usize,
    pub offset: u64,
    pub size: u32,
    pub flags: u64,
}

static mut OPEN_FILES: [Option<VfsFile>; MAX_OPEN_FILES] = [const { None }; MAX_OPEN_FILES];

/// Open a file through VFS. Returns fd index or -1.
pub fn open(path: &[u8], flags: u64) -> i32 {
    unsafe {
        let (driver_idx, prefix_len) = match resolve_mount(path) {
            Some(v) => v,
            None => return -1,
        };
        let rel = strip_prefix(path, prefix_len);

        let driver = match get_driver(driver_idx) {
            Some(d) => d,
            None => return -1,
        };
        let handle = match (driver.open)(rel, flags) {
            Some(h) => h,
            None => return -1,
        };

        // Get file size
        let size = (driver.stat)(rel).map(|s| s.size).unwrap_or(0);

        // Find free fd slot
        let fd_slot = (0..MAX_OPEN_FILES).find(|&i| OPEN_FILES[i].is_none());
        let fd_slot = match fd_slot {
            Some(s) => s,
            None => { (driver.close)(handle); return -1; }
        };

        OPEN_FILES[fd_slot] = Some(VfsFile {
            driver_idx,
            handle,
            offset: 0,
            size,
            flags,
        });
        fd_slot as i32
    }
}

/// Close a file descriptor.
pub fn close(fd: i32) -> i32 {
    if fd < 0 || fd as usize >= MAX_OPEN_FILES { return -1; }
    unsafe {
        if let Some(ref f) = OPEN_FILES[fd as usize] {
            if let Some(driver) = get_driver(f.driver_idx) {
                (driver.close)(f.handle);
            }
            OPEN_FILES[fd as usize] = None;
            0
        } else {
            -1
        }
    }
}

/// Read from a file descriptor.
pub fn read(fd: i32, buf: &mut [u8]) -> i64 {
    if fd < 0 || fd as usize >= MAX_OPEN_FILES { return -1; }
    unsafe {
        if let Some(ref mut f) = OPEN_FILES[fd as usize] {
            let driver = match get_driver(f.driver_idx) {
                Some(d) => d,
                None => return -1,
            };
            match (driver.read)(f.handle, buf, f.offset) {
                Some(n) => {
                    f.offset += n as u64;
                    n as i64
                }
                None => -1,
            }
        } else {
            -1
        }
    }
}

/// Write to a file descriptor.
pub fn write(fd: i32, buf: &[u8]) -> i64 {
    if fd < 0 || fd as usize >= MAX_OPEN_FILES { return -1; }
    unsafe {
        if let Some(ref mut f) = OPEN_FILES[fd as usize] {
            if f.flags & 3 == 0 { return -1; } // O_RDONLY
            let driver = match get_driver(f.driver_idx) {
                Some(d) => d,
                None => return -1,
            };
            match (driver.write)(f.handle, buf, f.offset) {
                Some(n) => {
                    f.offset += n as u64;
                    if f.offset > f.size as u64 { f.size = f.offset as u32; }
                    n as i64
                }
                None => -1,
            }
        } else {
            -1
        }
    }
}

/// Seek within a file.
pub fn lseek(fd: i32, offset: i64, whence: u32) -> i64 {
    if fd < 0 || fd as usize >= MAX_OPEN_FILES { return -1; }
    unsafe {
        if let Some(ref mut f) = OPEN_FILES[fd as usize] {
            let new_off = match whence {
                0 => offset as u64,                        // SEEK_SET
                1 => (f.offset as i64 + offset) as u64,    // SEEK_CUR
                2 => (f.size as i64 + offset) as u64,      // SEEK_END
                _ => return -1,
            };
            f.offset = new_off;
            new_off as i64
        } else {
            -1
        }
    }
}

/// Get file stat info.
pub fn stat(path: &[u8], out: &mut StatInfo) -> i32 {
    unsafe {
        let (driver_idx, prefix_len) = match resolve_mount(path) {
            Some(v) => v,
            None => return -1,
        };
        let rel = strip_prefix(path, prefix_len);
        let driver = match get_driver(driver_idx) {
            Some(d) => d,
            None => return -1,
        };
        match (driver.stat)(rel) {
            Some(info) => { *out = info; 0 }
            None => -1,
        }
    }
}

/// Get file stat by FD.
pub fn fstat(fd: i32) -> u64 {
    if fd < 0 || fd as usize >= MAX_OPEN_FILES { return 0; }
    unsafe {
        if let Some(ref f) = OPEN_FILES[fd as usize] {
            f.size as u64
        } else {
            0
        }
    }
}

/// List directory entries.
pub fn readdir(path: &[u8], entries: &mut [DirEntry]) -> i32 {
    unsafe {
        let (driver_idx, prefix_len) = match resolve_mount(path) {
            Some(v) => v,
            None => return -1,
        };
        let rel = strip_prefix(path, prefix_len);
        let driver = match get_driver(driver_idx) {
            Some(d) => d,
            None => return -1,
        };
        match (driver.readdir)(rel, entries) {
            Some(n) => n as i32,
            None => -1,
        }
    }
}

/// Read directory and format as text (for `ls > file`)
pub fn readdir_to_buf(path: &[u8], buf: &mut [u8]) -> usize {
    let mut entries = [DirEntry {
        name: [0; MAX_NAME],
        is_dir: false,
        size: 0,
    }; 64];
    let n = readdir(path, &mut entries);
    if n < 0 { return 0; }
    let n = n as usize;
    let mut oi = 0;
    for i in 0..n {
        let e = &entries[i];
        let name_len = e.name.iter().position(|&c| c == 0).unwrap_or(MAX_NAME);
        for j in 0..name_len {
            if oi < buf.len() - 1 { buf[oi] = e.name[j]; oi += 1; }
        }
        if e.is_dir {
            if oi < buf.len() - 1 { buf[oi] = b'/'; oi += 1; }
        }
        // Добавляем размер
        if oi < buf.len() - 1 { buf[oi] = b'\t'; oi += 1; }
        let sz = e.size;
        let mut tmp = [0u8; 12];
        let mut ti = 0;
        if sz == 0 { tmp[ti] = b'0'; ti += 1; }
        else {
            let mut v = sz;
            while v > 0 { tmp[ti] = b'0' + (v % 10) as u8; v /= 10; ti += 1; }
            let mut j = 0;
            while j < ti / 2 { let t = tmp[j]; tmp[j] = tmp[ti-1-j]; tmp[ti-1-j] = t; j += 1; }
        }
        for j in 0..ti {
            if oi < buf.len() - 1 { buf[oi] = tmp[j]; oi += 1; }
        }
        if oi < buf.len() - 2 { buf[oi] = b'\r'; buf[oi+1] = b'\n'; oi += 2; }
    }
    oi
}

/// Create a directory.
pub fn mkdir(path: &[u8]) -> bool {
    unsafe {
        let (driver_idx, prefix_len) = match resolve_mount(path) {
            Some(v) => v,
            None => return false,
        };
        let rel = strip_prefix(path, prefix_len);
        let driver = match get_driver(driver_idx) {
            Some(d) => d,
            None => return false,
        };
        (driver.mkdir)(rel)
    }
}

/// Remove a directory.
pub fn rmdir(path: &[u8]) -> bool {
    unsafe {
        let (driver_idx, prefix_len) = match resolve_mount(path) {
            Some(v) => v,
            None => return false,
        };
        let rel = strip_prefix(path, prefix_len);
        let driver = match get_driver(driver_idx) {
            Some(d) => d,
            None => return false,
        };
        (driver.rmdir)(rel)
    }
}

/// Delete a file.
pub fn unlink(path: &[u8]) -> bool {
    unsafe {
        let (driver_idx, prefix_len) = match resolve_mount(path) {
            Some(v) => v,
            None => return false,
        };
        let rel = strip_prefix(path, prefix_len);
        let driver = match get_driver(driver_idx) {
            Some(d) => d,
            None => return false,
        };
        (driver.unlink)(rel)
    }
}

/// Check if a path is a directory.
pub fn is_dir(path: &[u8]) -> bool {
    let mut info = StatInfo { size: 0, is_dir: false, cluster: 0 };
    if stat(path, &mut info) == 0 { info.is_dir } else { false }
}

/// Get file size.
pub fn file_size(path: &[u8]) -> Option<u32> {
    let mut info = StatInfo { size: 0, is_dir: false, cluster: 0 };
    if stat(path, &mut info) == 0 { Some(info.size) } else { None }
}

/// Check if file exists.
pub fn exists(path: &[u8]) -> bool {
    let mut info = StatInfo { size: 0, is_dir: false, cluster: 0 };
    stat(path, &mut info) == 0
}

/// Initialize VFS.
pub fn init() {
    uart_print("[VFS] init: mount table ");
    uart_print_dec(MAX_MOUNTS as u64);
    uart_print(" slots, ");
    uart_print_dec(MAX_OPEN_FILES as u64);
    uart_print(" fd slots\r\n");
}

fn uart_print_dec(mut v: u64) {
    if v == 0 { uart_print("0"); return; }
    let mut buf = [0u8; 20]; let mut i = 0;
    while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart_print(core::str::from_utf8(&buf[i..i+1]).unwrap_or("?")); }
}
