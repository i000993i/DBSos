//! Permissions — VFS permission checks for multi-user
//!
//! Simple Unix-like rwx for owner/group/other. Stored in a side table
//! since FAT has no perms. For ext4, uses real inode perms.

use crate::user;

#[derive(Clone, Copy)]
pub struct Perm { pub owner: u32, pub group: u32, pub mode: u16 } // mode 0o777

impl Perm {
    pub fn default_for(path: &[u8]) -> Self {
        let uid = user::current_uid();
        let gid = user::current_uid(); // gid == uid for simplicity
        let mode = if path.starts_with(b"/etc") || path.starts_with(b"/root") { 0o700 }
        else if path.starts_with(b"/home") { 0o755 }
        else { 0o644 };
        Perm { owner: uid, group: gid, mode }
    }
}

const MAX_PERMS: usize = 128;
static mut PERMS: [Option<PermEntry>; MAX_PERMS] = [None; MAX_PERMS];

#[derive(Clone, Copy)]
struct PermEntry { path: [u8; 64], len: u8, perm: Perm }

const fn empty_entry() -> Option<PermEntry> { None }

pub fn init() {
    unsafe { for i in 0..MAX_PERMS { PERMS[i] = empty_entry(); } }
}

fn find(path: &[u8]) -> Option<Perm> {
    unsafe {
        for e in PERMS.iter().flatten() {
            if e.len as usize == path.len() && &e.path[..e.len as usize] == path { return Some(e.perm); }
        }
        None
    }
}

pub fn get(path: &[u8]) -> Perm { find(path).unwrap_or(Perm::default_for(path)) }

pub fn set(path: &[u8], perm: Perm) {
    unsafe {
        for e in PERMS.iter_mut() {
            if let Some(entry) = e {
                if entry.len as usize == path.len() && &entry.path[..entry.len as usize] == path {
                    entry.perm = perm; return;
                }
            }
        }
        for e in PERMS.iter_mut() {
            if e.is_none() {
                let mut p = [0u8; 64];
                let l = path.len().min(64);
                p[..l].copy_from_slice(&path[..l]);
                *e = Some(PermEntry { path: p, len: l as u8, perm });
                return;
            }
        }
    }
}

fn gid_of(uid: u32) -> u32 {
    // In our model gid == uid, but try to resolve real gid if user exists
    if user::find_by_uid(uid).is_some() {
        // We don't expose gid directly, but convention gid == uid or stored gid equals uid
        // So just return uid as gid proxy
        return uid;
    }
    uid
}

pub fn can_read(path: &[u8], uid: u32) -> bool {
    let perm = get(path);
    if uid == 0 { return true; } // root
    if uid == perm.owner { return perm.mode & 0o400 != 0; }
    let gid = gid_of(uid);
    if gid == perm.group { return perm.mode & 0o040 != 0; }
    perm.mode & 0o004 != 0
}

pub fn can_write(path: &[u8], uid: u32) -> bool {
    let perm = get(path);
    if uid == 0 { return true; }
    if uid == perm.owner { return perm.mode & 0o200 != 0; }
    let gid = gid_of(uid);
    if gid == perm.group { return perm.mode & 0o020 != 0; }
    perm.mode & 0o002 != 0
}

pub fn can_exec(path: &[u8], uid: u32) -> bool {
    let perm = get(path);
    if uid == 0 { return true; }
    if uid == perm.owner { return perm.mode & 0o100 != 0; }
    let gid = gid_of(uid);
    if gid == perm.group { return perm.mode & 0o010 != 0; }
    perm.mode & 0o001 != 0
}
