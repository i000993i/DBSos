use crate::driver::uart;

fn use_nvme() -> bool { unsafe { crate::driver::nvme::FS_INIT } }

fn bps() -> u16 {
    if use_nvme() { unsafe { crate::driver::nvme::BPS } }
    else { unsafe { crate::driver::ahci::BPS } }
}
fn spc() -> u8 {
    if use_nvme() { unsafe { crate::driver::nvme::SPC } }
    else { unsafe { crate::driver::ahci::SPC } }
}
fn fat_sz() -> u64 {
    if use_nvme() { unsafe { crate::driver::nvme::FAT_SZ } }
    else { unsafe { crate::driver::ahci::FAT_SZ } }
}
fn root_ent() -> u16 {
    if use_nvme() { unsafe { crate::driver::nvme::ROOT_ENT } }
    else { unsafe { crate::driver::ahci::ROOT_ENT } }
}
fn reserved() -> u16 {
    if use_nvme() { unsafe { crate::driver::nvme::RESERVED } }
    else { unsafe { crate::driver::ahci::RESERVED } }
}
fn fats() -> u8 {
    if use_nvme() { unsafe { crate::driver::nvme::FATS } }
    else { unsafe { crate::driver::ahci::FATS } }
}
fn read_fat_sector(lba: u64, buf: &mut [u8; 512]) -> bool {
    if use_nvme() {
        crate::driver::nvme::read_fat_sector(lba, buf)
    } else {
        crate::driver::ahci::read_fat_sector(lba, buf)
    }
}

fn write_fat_sector(lba: u64, buf: &[u8; 512]) -> bool {
    if use_nvme() {
        crate::driver::nvme::write_fat_sector(lba, buf)
    } else {
        crate::driver::ahci::write_fat_sector(lba, buf)
    }
}

fn le_u16(buf: &[u8], off: usize) -> u16 {
    buf[off] as u16 | ((buf[off + 1] as u16) << 8)
}

fn le_u32(buf: &[u8], off: usize) -> u32 {
    buf[off] as u32 | ((buf[off + 1] as u32) << 8) | ((buf[off + 2] as u32) << 16) | ((buf[off + 3] as u32) << 24)
}

fn le_u16_write(buf: &mut [u8], off: usize, v: u16) {
    buf[off] = v as u8;
    buf[off + 1] = (v >> 8) as u8;
}

fn set_fat_entry(cluster: u16, value: u16) -> bool {
    let off = (cluster as u32) * 2;
    let sector = reserved() as u64 + (off / bps() as u32) as u64;
    let boff = (off % bps() as u32) as usize;
    let mut buf = [0u8; 512];
    if !read_fat_sector(sector, &mut buf) { return false; }
    le_u16_write(&mut buf, boff, value);
    for i in 1..fats() as u64 {
        if !write_fat_sector(sector + i * fat_sz(), &buf) { return false; }
    }
    write_fat_sector(sector, &buf)
}

fn alloc_cluster() -> Option<u16> {
    let sectors = fat_sz();
    let bps_val = bps() as u64;
    let entries_per_sector = bps_val / 2;
    for sec in 0..sectors {
        let mut buf = [0u8; 512];
        let lba = reserved() as u64 + sec;
        if !read_fat_sector(lba, &mut buf) { return None; }
        for i in 0..entries_per_sector as usize {
            let entry = le_u16(&buf, i * 2);
            if entry == 0 {
                let cluster = (sec * entries_per_sector) as u16 + i as u16;
                if cluster < 2 { continue; }
                if !set_fat_entry(cluster, 0xFFF7) { return None; }
                return Some(cluster);
            }
        }
    }
    None
}

fn root_dir_lba() -> u64 {
    reserved() as u64 + fats() as u64 * fat_sz()
}

fn root_dir_sectors() -> u64 {
    (root_ent() as u64 * 32 + 511) / 512
}

fn data_start_lba() -> u64 {
    reserved() as u64 + fats() as u64 * fat_sz() + root_dir_sectors()
}

fn cluster_to_lba(cluster: u16) -> u64 {
    data_start_lba() + ((cluster as u64).wrapping_sub(2)) * spc() as u64
}

fn read_fat_entry(cluster: u16) -> u16 {
    let fat_off = (cluster as u32) * 2;
    let sector = reserved() as u64 + (fat_off / bps() as u32) as u64;
    let off = (fat_off % bps() as u32) as usize;
    let mut buf = [0u8; 512];
    if !read_fat_sector(sector, &mut buf) {
        return 0xFFF7;
    }
    le_u16(&buf, off)
}

fn is_eoc(val: u16) -> bool {
    val >= 0xFFF8
}

fn format_name(entry: &[u8]) -> [u8; 13] {
    let mut name = [0u8; 13];
    let mut i = 0;
    for j in 0..8 {
        if entry[j] == b' ' { break; }
        name[i] = entry[j]; i += 1;
    }
    if entry[8] != b' ' {
        name[i] = b'.'; i += 1;
        for j in 8..11 {
            if entry[j] == b' ' { break; }
            name[i] = entry[j]; i += 1;
        }
    }
    name[i] = 0;
    name
}

fn name_match(entry: &[u8; 11], user: &[u8]) -> bool {
    let dot = user.iter().position(|&c| c == b'.');
    match dot {
        None => {
            for i in 0..8 {
                let ec = if i < user.len() { user[i].to_ascii_uppercase() } else { b' ' };
                if entry[i] != ec { return false; }
            }
            entry[8] == b' ' || entry[8] == 0
        }
        Some(d) => {
            let un = &user[..d];
            let ue = &user[d + 1..];
            for i in 0..8 {
                let ec = if i < un.len() { un[i].to_ascii_uppercase() } else { b' ' };
                if entry[i] != ec { return false; }
            }
            for i in 0..3 {
                let ec = if i < ue.len() { ue[i].to_ascii_uppercase() } else { b' ' };
                if entry[8 + i] != ec { return false; }
            }
            true
        }
    }
}

fn find_entry(name: &[u8]) -> Option<(u16, u32)> {
    let rlba = root_dir_lba();
    let rsects = root_dir_sectors();
    let dir_attr_lfn: u8 = 0x0F;
    let dir_attr_volume: u8 = 0x08;

    for sec in 0..rsects {
        let mut buf = [0u8; 512];
        if !read_fat_sector(rlba + sec, &mut buf) {
            return None;
        }
        for i in 0..16 {
            let off = i * 32;
            if buf[off] == 0 { return None; }
            if buf[off] == 0xE5 { continue; }
            let attr = buf[off + 11];
            if attr & dir_attr_lfn == dir_attr_lfn { continue; }
            if attr & dir_attr_volume != 0 { continue; }
            let ename: &[u8; 11] = &buf[off..off + 11].try_into().ok()?;
            if name_match(ename, name) {
                let clus = le_u16(&buf, off + 26);
                let size = le_u32(&buf, off + 28);
                return Some((clus, size));
            }
        }
    }
    None
}

pub fn ls() {
    let rlba = root_dir_lba();
    let rsects = root_dir_sectors();
    let dir_attr_lfn: u8 = 0x0F;
    let dir_attr_volume: u8 = 0x08;
    let dir_attr_dir: u8 = 0x10;

    uart::write_str(if use_nvme() { "NVMe Directory listing:\r\n" } else { "Directory listing:\r\n" });
    for sec in 0..rsects {
        let mut buf = [0u8; 512];
        if !read_fat_sector(rlba + sec, &mut buf) {
            uart::write_str("[FAT] read error\r\n");
            return;
        }
        for i in 0..16 {
            let off = i * 32;
            if buf[off] == 0 { return; }
            if buf[off] == 0xE5 { continue; }
            let attr = buf[off + 11];
            if attr & dir_attr_lfn == dir_attr_lfn { continue; }
            if attr & dir_attr_volume != 0 { continue; }
            let name = format_name(&buf[off..]);
            let name_slice = &name[..name.iter().position(|&c| c == 0).unwrap_or(11)];
            uart::write_str(if attr & dir_attr_dir != 0 { "  [DIR] " } else { "        " });
            uart::write_str(unsafe { core::str::from_utf8_unchecked(name_slice) });
            if attr & dir_attr_dir == 0 {
                let sz = le_u32(&buf, off + 28);
                uart::write_str("  ");
                let mut v = sz as u64;
                let mut b = [0u8; 20]; let mut bi = 0;
                if v == 0 { uart::putchar(b'0'); } else {
                    while v > 0 { b[bi] = b'0' + (v % 10) as u8; v /= 10; bi += 1; }
                    while bi > 0 { bi -= 1; uart::putchar(b[bi]); }
                }
                uart::write_str(" bytes");
            }
            uart::write_str("\r\n");
        }
    }
}

pub fn cat(name: &[u8]) {
    let (first_cluster, file_size) = match find_entry(name) {
        Some(v) => v,
        None => { uart::write_str("[FAT] file not found\r\n"); return; }
    };
    if first_cluster == 0 {
        uart::write_str("[FAT] empty file\r\n");
        return;
    }
    let bps = bps() as u64;
    let spc = spc() as u64;
    let cluster_sz = bps * spc;
    let mut remaining = file_size as u64;
    let mut cluster = first_cluster;
    while remaining > 0 && !is_eoc(cluster) && cluster != 0 {
        let to_read = if remaining < cluster_sz { remaining } else { cluster_sz };
        let nsecs = (to_read + bps - 1) / bps;
        let lba = cluster_to_lba(cluster);
        for s in 0..nsecs {
            let mut buf = [0u8; 512];
            if !read_fat_sector(lba + s, &mut buf) {
                uart::write_str("\r\n[FAT] read error\r\n");
                return;
            }
            let chunk = if (remaining as usize) < 512 { remaining as usize } else { 512 };
            uart::write_bytes(&buf[..chunk]);
            remaining = remaining.saturating_sub(512);
            if remaining == 0 { break; }
        }
        if remaining > 0 {
            cluster = read_fat_entry(cluster);
        }
    }
    if remaining > 0 {
        uart::write_str("\r\n[FAT] truncated\r\n");
    }
}

pub fn read_file(name: &[u8], buf: &mut [u8]) -> Option<usize> {
    let (first_cluster, file_size) = find_entry(name)?;
    let file_size = file_size as usize;
    if file_size > buf.len() { return None; }
    if first_cluster == 0 { return Some(0); }
    let bps = bps() as u64;
    let spc = spc() as u64;
    let cluster_sz = bps * spc;
    let mut remaining = file_size;
    let mut cluster = first_cluster;
    let mut offset = 0;
    while remaining > 0 && !is_eoc(cluster) && cluster != 0 {
        let to_read = if remaining < cluster_sz as usize { remaining } else { cluster_sz as usize };
        let nsecs = (to_read + bps as usize - 1) / bps as usize;
        let lba = cluster_to_lba(cluster);
        for s in 0..nsecs {
            let mut sec = [0u8; 512];
            if !read_fat_sector(lba + s as u64, &mut sec) { return None; }
            let chunk = core::cmp::min(remaining, 512);
            buf[offset..offset + chunk].copy_from_slice(&sec[..chunk]);
            offset += chunk;
            remaining -= chunk;
            if remaining == 0 { break; }
        }
        if remaining > 0 { cluster = read_fat_entry(cluster); }
    }
    Some(file_size)
}

pub fn read_file_path(path: &[u8], buf: &mut [u8]) -> Option<usize> {
    let parent = resolve_dir(path)?;
    let name = resolve_name(path);
    if name.is_empty() { return None; }
    let (first_cluster, file_size, _, _) = find_in_dir(parent, name)?;
    let file_size = file_size as usize;
    if file_size > buf.len() { return None; }
    if first_cluster == 0 { return Some(0); }
    let bps = bps() as u64;
    let spc = spc() as u64;
    let cluster_sz = bps * spc;
    let mut remaining = file_size;
    let mut cluster = first_cluster;
    let mut offset = 0;
    while remaining > 0 && !is_eoc(cluster) && cluster != 0 {
        let to_read = if remaining < cluster_sz as usize { remaining } else { cluster_sz as usize };
        let nsecs = (to_read + bps as usize - 1) / bps as usize;
        let lba = cluster_to_lba(cluster);
        for s in 0..nsecs {
            let mut sec = [0u8; 512];
            if !read_fat_sector(lba + s as u64, &mut sec) { return None; }
            let chunk = core::cmp::min(remaining, 512);
            buf[offset..offset + chunk].copy_from_slice(&sec[..chunk]);
            offset += chunk;
            remaining -= chunk;
            if remaining == 0 { break; }
        }
        if remaining > 0 { cluster = read_fat_entry(cluster); }
    }
    Some(file_size)
}

fn name_to_83(name: &[u8]) -> Option<[u8; 11]> {
    if name.is_empty() || name.len() > 12 { return None; }
    let mut entry = [b' '; 11];
    let dot = name.iter().position(|&c| c == b'.');
    match dot {
        None => {
            if name.len() > 8 { return None; }
            for i in 0..name.len() { entry[i] = name[i].to_ascii_uppercase(); }
        }
        Some(d) => {
            if d == 0 || d > 8 { return None; }
            let ext = &name[d + 1..];
            if ext.is_empty() || ext.len() > 3 { return None; }
            for i in 0..d { entry[i] = name[i].to_ascii_uppercase(); }
            for i in 0..ext.len() { entry[8 + i] = ext[i].to_ascii_uppercase(); }
        }
    }
    Some(entry)
}

fn vfat_checksum(short: &[u8; 11]) -> u8 {
    let mut sum: u8 = 0;
    for i in 0..11 {
        sum = ((sum & 1) << 7) | (sum >> 1);
        sum = sum.wrapping_add(short[i]);
    }
    sum
}

fn read_dir_sector(dir_cluster: u16, sec_idx: u64, buf: &mut [u8; 512]) -> bool {
    if dir_cluster == 0 {
        if sec_idx >= root_dir_sectors() { return false; }
        read_fat_sector(root_dir_lba() + sec_idx, buf)
    } else {
        let spc_val = spc() as u64;
        let head = sec_idx / spc_val;
        let tail = sec_idx % spc_val;
        let mut cluster = dir_cluster;
        for _ in 0..head {
            if is_eoc(cluster) { return false; }
            cluster = read_fat_entry(cluster);
            if cluster == 0 { return false; }
        }
        read_fat_sector(cluster_to_lba(cluster) + tail, buf)
    }
}

fn write_dir_sector(dir_cluster: u16, sec_idx: u64, buf: &[u8; 512]) -> bool {
    if dir_cluster == 0 {
        if sec_idx >= root_dir_sectors() { return false; }
        write_fat_sector(root_dir_lba() + sec_idx, buf)
    } else {
        let spc_val = spc() as u64;
        let head = sec_idx / spc_val;
        let tail = sec_idx % spc_val;
        let mut cluster = dir_cluster;
        for _ in 0..head {
            if is_eoc(cluster) { return false; }
            cluster = read_fat_entry(cluster);
            if cluster == 0 { return false; }
        }
        write_fat_sector(cluster_to_lba(cluster) + tail, buf)
    }
}

fn find_in_dir(dir_cluster: u16, name: &[u8]) -> Option<(u16, u32, u64, u8)> {
    let attr_lfn: u8 = 0x0F;
    let attr_vol: u8 = 0x08;
    let max_sec = if dir_cluster == 0 { root_dir_sectors() } else { u64::MAX };
    for sec in 0..max_sec {
        let mut buf = [0u8; 512];
        if !read_dir_sector(dir_cluster, sec, &mut buf) { return None; }
        for i in 0..16 {
            let off = i * 32;
            if buf[off] == 0 { return None; }
            if buf[off] == 0xE5 { continue; }
            let attr = buf[off + 11];
            if attr & attr_lfn == attr_lfn { continue; }
            if attr & attr_vol != 0 { continue; }
            let ename: &[u8; 11] = &buf[off..off + 11].try_into().ok()?;
            if name_match(ename, name) {
                return Some((le_u16(&buf, off + 26), le_u32(&buf, off + 28), sec * 16 + i as u64, attr));
            }
        }
    }
    None
}

fn split_path_last<'a>(path: &'a [u8]) -> (&'a [u8], &'a [u8]) {
    if let Some(pos) = path.iter().rposition(|&c| c == b'/') {
        (&path[..pos], &path[pos + 1..])
    } else {
        (&[] as &[u8], path)
    }
}

fn resolve_dir(path: &[u8]) -> Option<u16> {
    let (parent_path, _) = split_path_last(path);
    if parent_path.is_empty() { return Some(0); }
    let mut cluster = 0u16;
    for component in parent_path.split(|&c| c == b'/') {
        if component.is_empty() { continue; }
        let (sub_cluster, _, _, attr) = find_in_dir(cluster, component)?;
        if attr & 0x10 == 0 { return None; }
        cluster = sub_cluster;
    }
    Some(cluster)
}

fn resolve_name<'a>(path: &'a [u8]) -> &'a [u8] {
    split_path_last(path).1
}

fn free_cluster_chain(mut cluster: u16) -> bool {
    while !is_eoc(cluster) && cluster != 0 {
        let next = read_fat_entry(cluster);
        if !set_fat_entry(cluster, 0) { return false; }
        cluster = next;
    }
    true
}

fn add_dir_entry(dir_cluster: u16, name: &[u8], short: &[u8; 11], attr: u8, cluster: u16, size: u32) -> bool {
    let num_lfn = (name.len() + 12) / 13;
    let needed = if num_lfn > 0 { num_lfn + 1 } else { 1 };
    let checksum = vfat_checksum(short);
    let max_sec = if dir_cluster == 0 { root_dir_sectors() } else { u64::MAX };
    let mut run_start_sec = 0u64;
    let mut run_start_i = 0usize;
    let mut free_run: u64 = 0;
    for sec in 0..max_sec {
        let mut buf = [0u8; 512];
        if !read_dir_sector(dir_cluster, sec, &mut buf) {
            if dir_cluster == 0 { return false; }
            if let Some(new_cluster) = extend_subdir(dir_cluster) {
                let zero = [0u8; 512];
                let lba = cluster_to_lba(new_cluster);
                for s in 0..spc() as u64 { write_fat_sector(lba + s, &zero); }
                if !read_dir_sector(dir_cluster, sec, &mut buf) { return false; }
            } else { return false; }
        }
        for i in 0..16 {
            let off = i * 32;
            if buf[off] != 0 && buf[off] != 0xE5 {
                free_run = 0;
                continue;
            }
            if free_run == 0 {
                run_start_sec = sec;
                run_start_i = i;
            }
            free_run += 1;
            if free_run >= needed as u64 {
                let base_entry_idx = run_start_sec * 16 + run_start_i as u64;
                for k in 0..num_lfn {
                    let entry_idx = base_entry_idx + k as u64;
                    let s = entry_idx / 16;
                    let eo = ((entry_idx % 16) as usize) * 32;
                    let mut lbuf = [0u8; 512];
                    if !read_dir_sector(dir_cluster, s, &mut lbuf) { return false; }
                    for j in 0..32 { lbuf[eo + j] = 0; }
                    let chunk_idx = num_lfn - 1 - k;
                    lbuf[eo] = if k == 0 { (num_lfn as u8) | 0x40 } else { (chunk_idx + 1) as u8 };
                    lbuf[eo + 11] = 0x0F;
                    lbuf[eo + 13] = checksum;
                    let base_ci = chunk_idx * 13;
                    for j in 0..13 {
                        let ch_idx = base_ci + j;
                        let c: u16 = if ch_idx < name.len() {
                            name[ch_idx] as u16
                        } else if ch_idx == name.len() {
                            0x0000
                        } else {
                            0xFFFF
                        };
                        let (byte_off, _) = match j {
                            0..=4 => (1 + j * 2, 2),
                            5..=10 => (14 + (j - 5) * 2, 2),
                            _ => (28 + (j - 11) * 2, 2),
                        };
                        lbuf[eo + byte_off] = c as u8;
                        lbuf[eo + byte_off + 1] = (c >> 8) as u8;
                    }
                    if !write_dir_sector(dir_cluster, s, &lbuf) { return false; }
                }
                let entry_idx = base_entry_idx + num_lfn as u64;
                let s = entry_idx / 16;
                let eo = ((entry_idx % 16) as usize) * 32;
                let mut sbuf = [0u8; 512];
                if !read_dir_sector(dir_cluster, s, &mut sbuf) { return false; }
                sbuf[eo..eo + 11].copy_from_slice(short);
                sbuf[eo + 11] = attr;
                for j in 12..26 { sbuf[eo + j] = 0; }
                le_u16_write(&mut sbuf, eo + 26, cluster);
                let sz = size;
                sbuf[eo + 28] = sz as u8;
                sbuf[eo + 29] = (sz >> 8) as u8;
                sbuf[eo + 30] = (sz >> 16) as u8;
                sbuf[eo + 31] = (sz >> 24) as u8;
                return write_dir_sector(dir_cluster, s, &sbuf);
            }
        }
    }
    false
}

fn extend_subdir(dir_cluster: u16) -> Option<u16> {
    let new = alloc_cluster()?;
    set_fat_entry(new, 0xFFFF);
    let mut last = dir_cluster;
    while !is_eoc(read_fat_entry(last)) { last = read_fat_entry(last); }
    set_fat_entry(last, new);
    Some(new)
}

fn update_dir_entry(dir_cluster: u16, entry_idx: u64, cluster: u16, size: u32) -> bool {
    let sec = entry_idx / 16;
    let off = ((entry_idx % 16) as usize) * 32;
    let mut buf = [0u8; 512];
    if !read_dir_sector(dir_cluster, sec, &mut buf) { return false; }
    le_u16_write(&mut buf, off + 26, cluster);
    let sz = size;
    buf[off + 28] = sz as u8;
    buf[off + 29] = (sz >> 8) as u8;
    buf[off + 30] = (sz >> 16) as u8;
    buf[off + 31] = (sz >> 24) as u8;
    write_dir_sector(dir_cluster, sec, &buf)
}

fn init_dir_cluster(self_cluster: u16, parent_cluster: u16) -> bool {
    let lba = cluster_to_lba(self_cluster);
    let spc_val = spc() as u64;
    let mut buf = [0u8; 512];
    buf[0..8].copy_from_slice(b".       ");
    buf[11] = 0x10;
    le_u16_write(&mut buf, 26, self_cluster);
    buf[32..40].copy_from_slice(b"..      ");
    buf[43] = 0x10;
    le_u16_write(&mut buf, 58, parent_cluster);
    if !write_fat_sector(lba, &buf) { return false; }
    let zero = [0u8; 512];
    for s in 1..spc_val { if !write_fat_sector(lba + s, &zero) { return false; } }
    true
}

fn write_data_to_clusters(first_cluster: u16, data: &[u8]) -> bool {
    let spc_val = spc() as u64;
    let bps_val = bps() as u64;
    let bytes_per_cluster = spc_val * bps_val;
    let mut remaining = data.len();
    let mut cluster = first_cluster;
    let mut offset = 0;
    while remaining > 0 && !is_eoc(cluster) {
        let lba = cluster_to_lba(cluster);
        let chunk = core::cmp::min(remaining, bytes_per_cluster as usize);
        let nsecs = (chunk + 511) / 512;
        for s in 0..nsecs {
            let mut buf = [0u8; 512];
            let copy_sz = core::cmp::min(remaining, 512);
            buf[..copy_sz].copy_from_slice(&data[offset..offset + copy_sz]);
            if !write_fat_sector(lba + s as u64, &buf) { return false; }
            offset += copy_sz;
            remaining -= copy_sz;
            if remaining == 0 { break; }
        }
        if remaining > 0 { cluster = read_fat_entry(cluster); }
    }
    true
}

pub fn mkdir(path: &[u8]) -> bool {
    let parent = match resolve_dir(path) { Some(p) => p, None => { uart::write_str("[FS] path not found\r\n"); return false; } };
    let name = resolve_name(path);
    if name.is_empty() { uart::write_str("[FS] empty name\r\n"); return false; }
    if find_in_dir(parent, name).is_some() { uart::write_str("[FS] already exists\r\n"); return false; }
    let short = match name_to_83(name) { Some(s) => s, None => { uart::write_str("[FS] bad name\r\n"); return false; } };
    let cluster = match alloc_cluster() { Some(c) => c, None => { uart::write_str("[FS] no free clusters\r\n"); return false; } };
    set_fat_entry(cluster, 0xFFFF);
    if !init_dir_cluster(cluster, parent) { uart::write_str("[FS] init dir failed\r\n"); return false; }
    if !add_dir_entry(parent, name, &short, 0x10, cluster, 0) { uart::write_str("[FS] add entry failed\r\n"); return false; }
    uart::write_str("[FS] mkdir OK\r\n");
    true
}

pub fn write_file(path: &[u8], data: &[u8]) -> bool {
    let parent = match resolve_dir(path) { Some(p) => p, None => { uart::write_str("[FS] path not found\r\n"); return false; } };
    let name = resolve_name(path);
    if name.is_empty() { uart::write_str("[FS] empty name\r\n"); return false; }
    let short = match name_to_83(name) { Some(s) => s, None => { uart::write_str("[FS] bad name\r\n"); return false; } };

    let existing = find_in_dir(parent, name);

    if let Some((old_cluster, _, entry_idx, _)) = existing {
        if old_cluster != 0 { free_cluster_chain(old_cluster); }
        if data.is_empty() {
            update_dir_entry(parent, entry_idx, 0, 0);
            return true;
        }
        let bytes_per_cluster = spc() as u64 * bps() as u64;
        let needed = ((data.len() as u64) + bytes_per_cluster - 1) / bytes_per_cluster;
        if needed > 0xFFFF { uart::write_str("[FS] too large\r\n"); return false; }
        let first = match alloc_cluster() { Some(c) => c, None => { uart::write_str("[FS] no clusters\r\n"); return false; } };
        set_fat_entry(first, 0xFFFF);
        let mut prev = first;
        for _ in 1..needed {
            let c = match alloc_cluster() { Some(c) => c, None => { free_cluster_chain(first); return false; } };
            set_fat_entry(prev, c);
            set_fat_entry(c, 0xFFFF);
            prev = c;
        }
        if !write_data_to_clusters(first, data) { free_cluster_chain(first); return false; }
        update_dir_entry(parent, entry_idx, first, data.len() as u32);
    } else {
        if data.is_empty() {
            return add_dir_entry(parent, name, &short, 0x20, 0, 0);
        }
        let bytes_per_cluster = spc() as u64 * bps() as u64;
        let needed = ((data.len() as u64) + bytes_per_cluster - 1) / bytes_per_cluster;
        if needed > 0xFFFF { uart::write_str("[FS] too large\r\n"); return false; }
        let first = match alloc_cluster() { Some(c) => c, None => { uart::write_str("[FS] no clusters\r\n"); return false; } };
        set_fat_entry(first, 0xFFFF);
        let mut prev = first;
        for _ in 1..needed {
            let c = match alloc_cluster() { Some(c) => c, None => { free_cluster_chain(first); return false; } };
            set_fat_entry(prev, c);
            set_fat_entry(c, 0xFFFF);
            prev = c;
        }
        if !write_data_to_clusters(first, data) { free_cluster_chain(first); return false; }
        add_dir_entry(parent, name, &short, 0x20, first, data.len() as u32);
    }
    uart::write_str("[FS] write OK\r\n");
    true
}

pub fn ls_path(path: &[u8]) {
    let dir_cluster = match resolve_dir(path) {
        Some(c) => c,
        None => { uart::write_str("[FS] path not found\r\n"); return; }
    };
    let name = resolve_name(path);
    if !name.is_empty() {
        if let Some((_, _, _, attr)) = find_in_dir(dir_cluster, name) {
            if attr & 0x10 != 0 {
                let sub_cluster = find_in_dir(dir_cluster, name).unwrap().0;
                if sub_cluster == 0 { uart::write_str("[FS] listing root not supported via path\r\n"); return; }
                list_directory(sub_cluster);
            } else {
                uart::write_str("[FS] not a directory\r\n");
            }
        } else {
            uart::write_str("[FS] not found\r\n");
        }
    } else {
        list_directory(dir_cluster);
    }
}

fn list_directory(dir_cluster: u16) {
    let attr_lfn: u8 = 0x0F;
    let attr_vol: u8 = 0x08;
    let attr_dir: u8 = 0x10;
    let max_sec = if dir_cluster == 0 { root_dir_sectors() } else { u64::MAX };
    for sec in 0..max_sec {
        let mut buf = [0u8; 512];
        if !read_dir_sector(dir_cluster, sec, &mut buf) { return; }
        for i in 0..16 {
            let off = i * 32;
            if buf[off] == 0 { return; }
            if buf[off] == 0xE5 { continue; }
            let attr = buf[off + 11];
            if attr & attr_lfn == attr_lfn { continue; }
            if attr & attr_vol != 0 { continue; }
            let name = format_name(&buf[off..]);
            let name_slice = &name[..name.iter().position(|&c| c == 0).unwrap_or(11)];
            uart::write_str(if attr & attr_dir != 0 { "  [DIR] " } else { "        " });
            uart::write_str(unsafe { core::str::from_utf8_unchecked(name_slice) });
            if attr & attr_dir == 0 {
                let sz = le_u32(&buf, off + 28);
                uart::write_str("  ");
                let mut v = sz as u64;
                let mut b = [0u8; 20]; let mut bi = 0;
                if v == 0 { uart::putchar(b'0'); } else {
                    while v > 0 { b[bi] = b'0' + (v % 10) as u8; v /= 10; bi += 1; }
                    while bi > 0 { bi -= 1; uart::putchar(b[bi]); }
                }
                uart::write_str(" bytes");
            }
            uart::write_str("\r\n");
        }
    }
}

fn delete_lfn_entries(dir_cluster: u16, entry_idx: u64) {
    let mut idx: i64 = entry_idx as i64 - 1;
    while idx >= 0 {
        let sec = idx as u64 / 16;
        let off = ((idx as u64 % 16) as usize) * 32;
        let mut buf = [0u8; 512];
        if !read_dir_sector(dir_cluster, sec, &mut buf) { return; }
        if buf[off + 11] != 0x0F { return; }
        buf[off] = 0xE5;
        if !write_dir_sector(dir_cluster, sec, &buf) { return; }
        idx -= 1;
    }
}

fn delete_dir_entry(dir_cluster: u16, entry_idx: u64) -> bool {
    delete_lfn_entries(dir_cluster, entry_idx);
    let sec = entry_idx / 16;
    let off = ((entry_idx % 16) as usize) * 32;
    let mut buf = [0u8; 512];
    if !read_dir_sector(dir_cluster, sec, &mut buf) { return false; }
    buf[off] = 0xE5;
    write_dir_sector(dir_cluster, sec, &buf)
}

pub fn rm(path: &[u8]) -> bool {
    let parent = match resolve_dir(path) { Some(p) => p, None => { uart::write_str("[FS] path not found\r\n"); return false; } };
    let name = resolve_name(path);
    if name.is_empty() { uart::write_str("[FS] empty name\r\n"); return false; }
    let (cluster, _, entry_idx, attr) = match find_in_dir(parent, name) {
        Some(v) => v,
        None => { uart::write_str("[FS] not found\r\n"); return false; }
    };
    if attr & 0x10 != 0 { uart::write_str("[FS] is a directory\r\n"); return false; }
    if cluster != 0 { free_cluster_chain(cluster); }
    if !delete_dir_entry(parent, entry_idx) { uart::write_str("[FS] delete entry failed\r\n"); return false; }
    uart::write_str("[FS] rm OK\r\n");
    true
}

pub fn rmdir(path: &[u8]) -> bool {
    let parent = match resolve_dir(path) { Some(p) => p, None => { uart::write_str("[FS] path not found\r\n"); return false; } };
    let name = resolve_name(path);
    if name.is_empty() { uart::write_str("[FS] empty name\r\n"); return false; }
    let (cluster, _, entry_idx, attr) = match find_in_dir(parent, name) {
        Some(v) => v,
        None => { uart::write_str("[FS] not found\r\n"); return false; }
    };
    if attr & 0x10 == 0 { uart::write_str("[FS] not a directory\r\n"); return false; }

    if cluster != 0 {
        let spc_val = spc() as u64;
        let mut buf = [0u8; 512];
        let mut empty = true;
        'scan: for s in 0..spc_val {
            if !read_dir_sector(cluster, s, &mut buf) { break; }
            for i in 0..16 {
                let off = i * 32;
                if buf[off] == 0 { break 'scan; }
                if buf[off] == 0xE5 { continue; }
                if i == 0 || i == 1 { continue; }
                let a = buf[off + 11];
                if a & 0x0F == 0x0F { continue; }
                if a & 0x08 != 0 { continue; }
                empty = false;
                break 'scan;
            }
        }
        if !empty { uart::write_str("[FS] directory not empty\r\n"); return false; }
        free_cluster_chain(cluster);
    }
    if !delete_dir_entry(parent, entry_idx) { uart::write_str("[FS] delete entry failed\r\n"); return false; }
    uart::write_str("[FS] rmdir OK\r\n");
    true
}

pub fn cat_path(path: &[u8]) {
    let parent = match resolve_dir(path) { Some(p) => p, None => { uart::write_str("[FS] not found\r\n"); return; } };
    let name = resolve_name(path);
    if name.is_empty() { uart::write_str("[FS] empty name\r\n"); return; }
    let (first_cluster, file_size, _, _) = match find_in_dir(parent, name) {
        Some(v) => v,
        None => { uart::write_str("[FS] not found\r\n"); return; }
    };
    if first_cluster == 0 { uart::write_str("[FS] empty file\r\n"); return; }
    let bps_val = bps() as u64;
    let spc_val = spc() as u64;
    let cluster_sz = bps_val * spc_val;
    let mut remaining = file_size as u64;
    let mut cluster = first_cluster;
    while remaining > 0 && !is_eoc(cluster) && cluster != 0 {
        let to_read = if remaining < cluster_sz { remaining } else { cluster_sz };
        let nsecs = (to_read + bps_val - 1) / bps_val;
        let lba = cluster_to_lba(cluster);
        for s in 0..nsecs {
            let mut buf = [0u8; 512];
            if !read_fat_sector(lba + s, &mut buf) { uart::write_str("\r\n[FS] read error\r\n"); return; }
            let chunk = core::cmp::min(remaining as usize, 512);
            uart::write_bytes(&buf[..chunk]);
            remaining -= chunk as u64;
            if remaining == 0 { break; }
        }
        if remaining > 0 { cluster = read_fat_entry(cluster); }
    }
}
