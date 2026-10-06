//! Multi-user support — LEVEL_0 kernel primitive for LEVEL_1/2
//!
//! Simple in-memory user DB, max 16 users. Stored in `/etc/passwd` on disk if present.

use crate::driver::uart;

#[derive(Clone, Copy)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
    pub name: [u8; 32],
    pub name_len: u8,
    pub home: [u8; 64],
    pub home_len: u8,
    pub shell: [u8; 32],
    pub shell_len: u8,
    pub password_hash: u32, // salted hash, 0 = no password (guest)
    pub salt: u32,
    pub active: bool,
}

impl User {
    pub const fn empty() -> Self {
        Self { uid: 0, gid: 0, name: [0;32], name_len: 0, home: [0;64], home_len: 0, shell: [0;32], shell_len: 0, password_hash: 0, salt: 0, active: false }
    }
    pub fn name_str(&self) -> &str { core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("?") }
}

const MAX_USERS: usize = 16;
static mut USERS: [User; MAX_USERS] = [User::empty(); MAX_USERS];
static mut USER_COUNT: usize = 0;
static mut CURRENT_UID: u32 = 0; // 0 = root

fn hash_password(pw: &[u8]) -> u32 {
    // kept for migration: unsalted fallback
    let mut h: u32 = 5381;
    for &b in pw { h = h.wrapping_mul(33).wrapping_add(b as u32); }
    h
}
fn hash_password_salted(pw: &[u8], salt: u32) -> u32 {
    // salted djb2 with pepper iterations (simple KDF)
    let mut h: u32 = 5381 ^ salt;
    for &b in pw { h = h.wrapping_mul(33).wrapping_add(b as u32).wrapping_add(salt & 0xFF); }
    // 100 iterations to slow brute force a bit
    for _ in 0..100 {
        h = h.wrapping_mul(33) ^ salt;
    }
    h
}
fn generate_salt(uid: u32, name: &[u8]) -> u32 {
    // Deterministic per-user but unique: ticks + uid + name hash
    let ticks = crate::timer::ticks() as u32;
    let mut h: u32 = ticks.wrapping_mul(2654435761).wrapping_add(uid.wrapping_mul(1597334677));
    for &b in name { h = h.wrapping_mul(33).wrapping_add(b as u32); }
    if h == 0 { h = 0x9E3779B9u32 ^ uid; }
    h
}

#[allow(dead_code)]
fn init_user(uid: u32, gid: u32, name: &[u8], home: &[u8], pw_hash: u32) {
    unsafe {
        if USER_COUNT >= MAX_USERS { return; }
        let mut u = User::empty();
        u.uid = uid; u.gid = gid;
        let nl = name.len().min(31); u.name[..nl].copy_from_slice(&name[..nl]); u.name_len = nl as u8;
        let hl = home.len().min(63); u.home[..hl].copy_from_slice(&home[..hl]); u.home_len = hl as u8;
        let sh = b"/bin/sh"; u.shell[..sh.len()].copy_from_slice(sh); u.shell_len = sh.len() as u8;
        u.password_hash = pw_hash; u.active = true;
        // salt: if hash is 0 (guest) keep 0, else generate
        u.salt = if pw_hash == 0 { 0 } else { generate_salt(uid, name) };
        // Re-hash with salt if not guest
        if pw_hash != 0 {
            // We assume caller already passed unsalted hash; recompute salted from name-based lookup?
            // Instead if caller is init() with hash_password(name), we replace with salted of same plaintext name-as-password shortcut: not ideal.
            // For backwards compat: keep as-is but store salt for future password changes.
            // Real salted hash will be computed in add_user/authenticate path.
        }
        USERS[USER_COUNT] = u; USER_COUNT += 1;
    }
}
fn init_user_salted(uid: u32, gid: u32, name: &[u8], home: &[u8], pw: &[u8]) {
    unsafe {
        if USER_COUNT >= MAX_USERS { return; }
        let mut u = User::empty();
        u.uid = uid; u.gid = gid;
        let nl = name.len().min(31); u.name[..nl].copy_from_slice(&name[..nl]); u.name_len = nl as u8;
        let hl = home.len().min(63); u.home[..hl].copy_from_slice(&home[..hl]); u.home_len = hl as u8;
        let sh = b"/bin/sh"; u.shell[..sh.len()].copy_from_slice(sh); u.shell_len = sh.len() as u8;
        if pw.is_empty() {
            u.password_hash = 0; u.salt = 0;
        } else {
            u.salt = generate_salt(uid, name);
            u.password_hash = hash_password_salted(pw, u.salt);
        }
        u.active = true;
        USERS[USER_COUNT] = u; USER_COUNT += 1;
    }
}

pub fn init() {
    // Try to load from disk; VFS may not be ready yet, so we create defaults first
    // and then try to load — if load succeeds it will override defaults
    init_user_salted(0, 0, b"root", b"/root", b"root");
    init_user_salted(1000, 1000, b"guest", b"/home/guest", b"");
    init_user_salted(1001, 1001, b"user", b"/home/user", b"user");
    unsafe { CURRENT_UID = 0; }
    unsafe { crate::scheduler::TASKS[crate::scheduler::CURRENT].uid = 0; crate::scheduler::TASKS[crate::scheduler::CURRENT].gid = 0; }
    uart::write_str("[USER] multi-user ready: root, guest, user\r\n");
    // If /etc/passwd exists, reload from it (overrides defaults)
    // This is deferred: caller should invoke try_reload after VFS mount.
}
pub fn post_vfs_init() {
    if load_from_disk() {
        uart::write_str("[USER] reloaded from /etc/passwd\r\n");
        unsafe { CURRENT_UID = 0; crate::scheduler::TASKS[crate::scheduler::CURRENT].uid=0; crate::scheduler::TASKS[crate::scheduler::CURRENT].gid=0; }
    } else {
        save_to_disk();
    }
}

pub fn current_uid() -> u32 {
    // Per-task uid if scheduler is running, else global
    unsafe {
        let cur = crate::scheduler::CURRENT;
        if cur < crate::scheduler::MAX_TASKS && crate::scheduler::TASKS[cur].state != crate::scheduler::TaskState::Free {
            let uid = crate::scheduler::TASKS[cur].uid;
            // Keep global in sync for legacy callers (shell/plasma single-task mode)
            // But trust per-task value if non-zero or if we have a valid task
            // For TASKS[0] kernel task, both match.
            if crate::scheduler::TASKS[cur].id != 0 || uid != 0 || CURRENT_UID == 0 {
                return uid;
            }
        }
        CURRENT_UID
    }
}
pub fn current_gid() -> u32 {
    unsafe {
        let cur = crate::scheduler::CURRENT;
        if cur < crate::scheduler::MAX_TASKS && crate::scheduler::TASKS[cur].state != crate::scheduler::TaskState::Free {
            return crate::scheduler::TASKS[cur].gid;
        }
        CURRENT_UID
    }
}
pub fn set_current_uid(uid: u32) {
    unsafe {
        CURRENT_UID = uid;
        let cur = crate::scheduler::CURRENT;
        if cur < crate::scheduler::MAX_TASKS {
            crate::scheduler::TASKS[cur].uid = uid;
            // gid == uid for simplicity, unless we know user's gid
            if let Some(idx) = find_by_uid(uid) {
                crate::scheduler::TASKS[cur].gid = USERS[idx].gid;
            } else {
                crate::scheduler::TASKS[cur].gid = uid;
            }
        }
    }
}
pub fn current_user() -> Option<&'static User> {
    let uid = current_uid();
    unsafe { USERS[..USER_COUNT].iter().find(|u| u.uid == uid) }
}
pub fn current_name() -> &'static str { current_user().map(|u| u.name_str()).unwrap_or("unknown") }

pub fn find_by_name(name: &[u8]) -> Option<usize> {
    unsafe { USERS[..USER_COUNT].iter().position(|u| &u.name[..u.name_len as usize] == name) }
}
pub fn find_by_uid(uid: u32) -> Option<usize> {
    unsafe { USERS[..USER_COUNT].iter().position(|u| u.uid == uid) }
}
pub fn uid_by_name(name: &[u8]) -> Option<u32> {
    find_by_name(name).map(|idx| unsafe { USERS[idx].uid })
}
pub fn gid_by_name(name: &[u8]) -> Option<u32> {
    find_by_name(name).map(|idx| unsafe { USERS[idx].gid })
}

pub fn authenticate(name: &[u8], password: &[u8]) -> bool {
    if let Some(idx) = find_by_name(name) {
        unsafe {
            let u = &USERS[idx];
            if u.password_hash == 0 { return true; } // guest
            if u.salt == 0 {
                // legacy unsalted
                return u.password_hash == hash_password(password);
            }
            u.password_hash == hash_password_salted(password, u.salt)
        }
    } else { false }
}

pub fn login(name: &[u8], password: &[u8]) -> bool {
    if authenticate(name, password) {
        if let Some(idx) = find_by_name(name) {
            let uid = unsafe { USERS[idx].uid };
            set_current_uid(uid);
            uart::write_str("[USER] login "); uart::write_str(core::str::from_utf8(name).unwrap_or("?")); uart::write_str("\r\n");
            return true;
        }
    }
    false
}

pub fn add_user(name: &[u8], password: &[u8]) -> bool {
    if name.is_empty() || find_by_name(name).is_some() { return false; }
    let uid = unsafe { 1000 + USER_COUNT as u32 };
    let mut home = [0u8; 64];
    let pre = b"/home/";
    home[..pre.len()].copy_from_slice(pre);
    let nl = name.len().min(64 - pre.len());
    home[pre.len()..pre.len()+nl].copy_from_slice(&name[..nl]);
    init_user_salted(uid, uid, name, &home[..pre.len()+nl], password);
    // Create home dir via VFS if possible
    let _ = crate::vfs::mkdir(&home[..pre.len()+nl]);
    save_to_disk();
    true
}
pub fn set_password(name: &[u8], new_pw: &[u8]) -> bool {
    if let Some(idx) = find_by_name(name) {
        unsafe {
            if new_pw.is_empty() {
                USERS[idx].password_hash = 0;
                USERS[idx].salt = 0;
            } else {
                let salt = generate_salt(USERS[idx].uid, name);
                USERS[idx].salt = salt;
                USERS[idx].password_hash = hash_password_salted(new_pw, salt);
            }
        }
        save_to_disk();
        return true;
    }
    false
}

fn print_u32(mut v: u32) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut buf = [0u8; 10]; let mut n = 0;
    while v > 0 { buf[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
    while n > 0 { n -= 1; uart::putchar(buf[n]); }
}
pub fn list_users() {
    unsafe {
        for i in 0..USER_COUNT {
            let u = &USERS[i];
            uart::write_str("  uid="); print_u32(u.uid);
            uart::write_str(" gid="); print_u32(u.gid);
            uart::write_str(" "); uart::write_str(u.name_str());
            uart::write_str(" home="); uart::write_str(core::str::from_utf8(&u.home[..u.home_len as usize]).unwrap_or("/"));
            uart::write_str("\r\n");
        }
    }
}

pub fn is_root() -> bool { current_uid() == 0 }
pub fn set_uid_for_current(uid: u32) -> bool {
    if find_by_uid(uid).is_none() && uid != 0 { return false; }
    // Only root can change uid
    if !is_root() { return false; }
    set_current_uid(uid);
    true
}

// ── Persistence: /etc/passwd as "uid:gid:hash:salt:name:home\n" ───────

fn write_u32(buf: &mut [u8], pos: usize, mut v: u32) -> usize {
    if v == 0 { if pos < buf.len() { buf[pos]=b'0'; return pos+1; } else { return pos; } }
    let mut tmp = [0u8; 10];
    let mut n=0;
    while v>0 { tmp[n]=b'0'+(v%10) as u8; v/=10; n+=1; }
    let mut p=pos;
    while n>0 { n-=1; if p<buf.len(){buf[p]=tmp[n]; p+=1;} }
    p
}
fn parse_u32(s: &[u8]) -> Option<u32> {
    if s.is_empty(){return None;}
    let mut v:u32=0;
    for &c in s { if c<b'0'||c>b'9'{return None;} v=v.checked_mul(10)?.checked_add((c-b'0')as u32)?; }
    Some(v)
}

pub fn save_to_disk() {
    // Try /etc first, fallback to /
    let mut buf = [0u8; 2048];
    let mut pos=0usize;
    unsafe {
        for i in 0..USER_COUNT {
            let u=&USERS[i];
            pos=write_u32(&mut buf,pos,u.uid);
            if pos<buf.len(){buf[pos]=b':';pos+=1;}
            pos=write_u32(&mut buf,pos,u.gid);
            if pos<buf.len(){buf[pos]=b':';pos+=1;}
            pos=write_u32(&mut buf,pos,u.password_hash);
            if pos<buf.len(){buf[pos]=b':';pos+=1;}
            pos=write_u32(&mut buf,pos,u.salt);
            if pos<buf.len(){buf[pos]=b':';pos+=1;}
            let nl=u.name_len as usize;
            if pos+nl < buf.len(){buf[pos..pos+nl].copy_from_slice(&u.name[..nl]); pos+=nl;}
            if pos<buf.len(){buf[pos]=b':';pos+=1;}
            let hl=u.home_len as usize;
            if pos+hl < buf.len(){buf[pos..pos+hl].copy_from_slice(&u.home[..hl]); pos+=hl;}
            if pos<buf.len(){buf[pos]=b'\n';pos+=1;}
            if pos>1900{break;}
        }
    }
    let flags = 0x100 | 0x200 | 1; // O_CREAT|O_TRUNC|O_WRONLY
    // Try /etc/passwd, if fails try /passwd (root) to avoid mkdir issues
    let mut fd = crate::vfs::open(b"/etc/passwd", flags as u64);
    let mut path_saved: &[u8] = b"/etc/passwd";
    if fd < 0 {
        // try mkdir /etc then retry
        let _ = crate::vfs::mkdir(b"/etc");
        fd = crate::vfs::open(b"/etc/passwd", flags as u64);
        if fd < 0 {
            fd = crate::vfs::open(b"/passwd", flags as u64);
            path_saved = b"/passwd";
        }
    }
    if fd < 0 {
        // fallback to in-memory only (disk may be full or FAT busy) — not fatal
        uart::write_str("[USER] save skipped (no space, in-memory only)\r\n");
        return;
    }
    let n = crate::vfs::write(fd, &buf[..pos]);
    crate::vfs::close(fd);
    if n < 0 { uart::write_str("[USER] save failed: write\r\n"); } else { uart::write_str("[USER] saved to "); uart::write_str(core::str::from_utf8(path_saved).unwrap_or("?")); uart::write_str("\r\n"); }
    crate::permissions::set(path_saved, crate::permissions::Perm{owner:0,group:0,mode:0o600});
    crate::permissions::set(b"/etc", crate::permissions::Perm{owner:0,group:0,mode:0o700});
}

fn load_from_disk() -> bool {
    let mut fd = crate::vfs::open(b"/etc/passwd", 0);
    if fd < 0 { fd = crate::vfs::open(b"/passwd", 0); }
    if fd < 0 { return false; }
    let mut buf = [0u8; 2048];
    let n = crate::vfs::read(fd, &mut buf);
    crate::vfs::close(fd);
    if n <= 0 { return false; }
    let n = n as usize;
    let mut loaded=0usize;
    unsafe { USER_COUNT=0; }
    let mut i=0usize;
    while i<n {
        let mut j=i;
        while j<n && buf[j]!=b'\n' {j+=1;}
        let line=&buf[i..j];
        if !line.is_empty(){
            // split by ':'
            let mut parts: [&[u8];6]=[&[];6];
            let mut p=0usize; let mut s=0usize;
            for k in 0..line.len(){
                if line[k]==b':'{
                    if p<6{parts[p]=&line[s..k]; p+=1;}
                    s=k+1;
                }
            }
            if p<6 && s<=line.len(){parts[p]=&line[s..]; p+=1;}
            if p==6{
                if let (Some(uid),Some(gid),Some(hash),Some(salt)) = (parse_u32(parts[0]),parse_u32(parts[1]),parse_u32(parts[2]),parse_u32(parts[3])){
                    let name=parts[4]; let home=parts[5];
                    unsafe{
                        if USER_COUNT<MAX_USERS{
                            let mut u=User::empty();
                            u.uid=uid; u.gid=gid; u.password_hash=hash; u.salt=salt;
                            let nl=name.len().min(31); u.name[..nl].copy_from_slice(&name[..nl]); u.name_len=nl as u8;
                            let hl=home.len().min(63); u.home[..hl].copy_from_slice(&home[..hl]); u.home_len=hl as u8;
                            let sh=b"/bin/sh"; u.shell[..sh.len()].copy_from_slice(sh); u.shell_len=sh.len() as u8;
                            u.active=true;
                            USERS[USER_COUNT]=u; USER_COUNT+=1; loaded+=1;
                        }
                    }
                }
            }
        }
        i=j+1;
    }
    loaded>0
}

pub fn try_load_or_init() {
    if load_from_disk() {
        uart::write_str("[USER] loaded from /etc/passwd\r\n");
        unsafe { CURRENT_UID = 0; crate::scheduler::TASKS[crate::scheduler::CURRENT].uid=0; crate::scheduler::TASKS[crate::scheduler::CURRENT].gid=0; }
    } else {
        // Keep defaults already in place, then save them
        save_to_disk();
    }
}
