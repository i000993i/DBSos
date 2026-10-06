// FAT driver — обёртка над fs.rs для VFS-интерфейса.
//
// Поддерживает FAT12/16/32. Определяет тип по BPB (BIOS Parameter Block).
// Все операции: open, read, write, stat, readdir, mkdir, rmdir, unlink.

use crate::vfs::{StatInfo, DirEntry, MAX_NAME};
use crate::driver::uart;

fn uart_print(s: &str) { uart::write_str(s); }

// ── BPB (BIOS Parameter Block) — общая структура ─────────────────

#[derive(Clone, Copy)]
struct Bpb {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    root_entry_count: u16,   // FAT12/16 only
    _total_sectors_16: u16,   // FAT12/16 only
    fat_size_16: u16,        // FAT12/16: sectors per FAT
    fat_size_32: u32,        // FAT32: sectors per FAT
    root_cluster: u32,       // FAT32: root dir cluster
    fat_type: FatType,       // FAT12, FAT16, or FAT32
    data_start: u64,         // first data sector
    total_clusters: u64,
    _fs_info_sector: u16,     // FAT32 only
}

#[derive(Clone, Copy, PartialEq)]
enum FatType { Fat12, Fat16, Fat32 }

// ── BPB reading helpers ─────────────────────────────────────────

fn part_base() -> u64 {
    match crate::block::active() {
        crate::block::Backend::Nvme => crate::driver::nvme::part_lba(),
        crate::block::Backend::Ahci => crate::driver::ahci::part_lba(),
        crate::block::Backend::Ide => {
            // IDE: MBR в LBA0, партиция — разбираем один раз и кэшируем
            ide_part_lba()
        }
        crate::block::Backend::None => {
            if unsafe { crate::driver::nvme::FS_INIT } {
                crate::driver::nvme::part_lba()
            } else if crate::driver::ahci::is_ready() {
                crate::driver::ahci::part_lba()
            } else {
                ide_part_lba()
            }
        }
    }
}

static mut IDE_PART_CACHED: bool = false;
static mut IDE_PART_LBA: u64 = 0;

fn ide_part_lba() -> u64 {
    unsafe {
        if IDE_PART_CACHED { return IDE_PART_LBA; }
    }
    // Прочитать MBR через block-слой напрямую (IDE ATA LBA0)
    let devs = crate::driver::ide::devices();
    let mut found: Option<crate::driver::ide::IdeDevice> = None;
    for d in devs { if d.present && !d.atapi { found = Some(d); break; } }
    let dev = match found { Some(d) => d, None => return 0 };
    let phys = crate::memory::palloc();
    if phys == 0 { return 0; }
    let ok = crate::driver::ide::ata_read(&dev, 0, 1, phys as *mut u8);
    let lba = if ok {
        let mbr = unsafe { core::slice::from_raw_parts(phys as *const u8, 512) };
        let sig = (mbr[0x1FE] as u16) | ((mbr[0x1FF] as u16) << 8);
        if sig != 0xAA55 { 0 }
        else if mbr[0x1C2] == 0xEE {
            // GPT на IDE — ищем через общий gpt-парсер
            ide_gpt_lba(&dev).unwrap_or(0)
        } else {
            (mbr[0x1C6] as u64) | ((mbr[0x1C7] as u64) << 8)
                | ((mbr[0x1C8] as u64) << 16) | ((mbr[0x1C9] as u64) << 24)
        }
    } else { 0 };
    crate::memory::pfree(phys);
    unsafe { IDE_PART_LBA = lba; IDE_PART_CACHED = true; }
    lba
}

fn ide_gpt_lba(dev: &crate::driver::ide::IdeDevice) -> Option<u64> {
    let phys = crate::memory::palloc();
    if phys == 0 { return None; }
    if !crate::driver::ide::ata_read(dev, 1, 1, phys as *mut u8) {
        crate::memory::pfree(phys);
        return None;
    }
    let res = {
        let hdr = unsafe { &*(phys as *const [u8; 512]) };
        crate::gpt::parse_header(hdr)
    };
    crate::memory::pfree(phys);
    let (entry_lba, entry_num, entry_size) = res?;
    // Читаем первую запись партиций
    let buf = crate::memory::palloc();
    if buf == 0 { return None; }
    if !crate::driver::ide::ata_read(dev, entry_lba as u32, 1, buf as *mut u8) {
        crate::memory::pfree(buf);
        return None;
    }
    let out = {
        let flat = unsafe { core::slice::from_raw_parts(buf as *const u8, 512) };
        crate::gpt::first_partition_lba(flat, entry_num, entry_size)
    };
    crate::memory::pfree(buf);
    out
}

fn read_bpb_sector(lba: u64, buf: &mut [u8; 512]) -> bool {
    // Единый путь: активный backend + part_base. Fallback — прямой перебор.
    let base = part_base();
    if crate::block::read_fat_sector(base, lba, buf) { return true; }
    // Fallback: попробовать каждый backend напрямую (когда probe ещё не выбран)
    if unsafe { crate::driver::nvme::FS_INIT }
        && crate::driver::nvme::read_fat_sector(lba, buf) { return true; }
    if crate::driver::ahci::is_ready()
        && crate::driver::ahci::read_fat_sector(lba, buf) { return true; }
    false
}

fn write_bpb_sector(lba: u64, buf: &[u8; 512]) -> bool {
    let base = part_base();
    if crate::block::write_fat_sector(base, lba, buf) { return true; }
    if unsafe { crate::driver::nvme::FS_INIT }
        && crate::driver::nvme::write_fat_sector(lba, buf) { return true; }
    if crate::driver::ahci::is_ready()
        && crate::driver::ahci::write_fat_sector(lba, buf) { return true; }
    false
}

fn le16(buf: &[u8], off: usize) -> u16 {
    buf[off] as u16 | (buf[off + 1] as u16) << 8
}

fn le32(buf: &[u8], off: usize) -> u32 {
    buf[off] as u32 | (buf[off+1] as u32) << 8 | (buf[off+2] as u32) << 16 | (buf[off+3] as u32) << 24
}

fn write_le16(buf: &mut [u8], off: usize, v: u16) {
    buf[off] = v as u8;
    buf[off+1] = (v >> 8) as u8;
}

fn write_le32(buf: &mut [u8], off: usize, v: u32) {
    buf[off] = v as u8;
    buf[off+1] = (v >> 8) as u8;
    buf[off+2] = (v >> 16) as u8;
    buf[off+3] = (v >> 24) as u8;
}

/// Read BPB from sector 0 of the partition.
fn read_bpb() -> Option<Bpb> {
    let mut buf = [0u8; 512];
    if !read_bpb_sector(0, &mut buf) { return None; }

    // Validate jump instruction and OEM name
    if buf[0] == 0xE9 || buf[0] == 0xEB {
        // OK
    } else if buf[0] == 0xE5 {
        return None; // not a FAT boot sector
    } else {
        // Could be valid, try anyway
    }

    let bps = le16(&buf, 11);
    if bps == 0 || (bps & (bps - 1)) != 0 { return None; } // not power of 2
    let spc = buf[13];
    if spc == 0 || (spc & (spc - 1)) != 0 { return None; }

    let reserved = le16(&buf, 14);
    let num_fats = buf[16];
    let root_entries = le16(&buf, 17);
    let tot16 = le16(&buf, 19);
    let _media = buf[21];
    let fat_sz16 = le16(&buf, 22);

    // FAT32 fields
    let fat_sz32 = le32(&buf, 36);
    let root_cluster = le32(&buf, 44);
    let fs_info = le16(&buf, 48);

    let fat_size = if fat_sz16 != 0 { fat_sz16 as u32 } else { fat_sz32 };

    // Total sectors
    let total_sectors = if tot16 != 0 {
        tot16 as u64
    } else {
        le32(&buf, 32) as u64
    };

    // Root dir sectors (FAT12/16 only)
    let root_dir_sectors = ((root_entries as u64 * 32 + bps as u64 - 1) / bps as u64) as u64;

    // Data start
    let data_start = reserved as u64 + num_fats as u64 * fat_size as u64 + root_dir_sectors;

    // Total data sectors
    let data_sectors = total_sectors - data_start;

    // Total clusters
    let total_clusters = data_sectors / spc as u64;

    // Determine FAT type
    let fat_type = if total_clusters < 4085 {
        FatType::Fat12
    } else if total_clusters < 65525 {
        FatType::Fat16
    } else {
        FatType::Fat32
    };

    Some(Bpb {
        bytes_per_sector: bps,
        sectors_per_cluster: spc,
        reserved_sectors: reserved,
        num_fats,
        root_entry_count: root_entries,
        _total_sectors_16: tot16,
        fat_size_16: fat_sz16,
        fat_size_32: fat_sz32,
        root_cluster,
        fat_type,
        data_start,
        total_clusters,
        _fs_info_sector: fs_info,
    })
}

/// Sectors per FAT для текущего типа: у FAT12/16 размер лежит в BPB+22,
/// у FAT32 — в BPB+36. Поле fat_size_32 для FAT12/16 содержит мусор EBPB
/// (drive number/signature), его использовать нельзя (root улетал на LBA ~2 млрд).
fn fat_sectors(bpb: &Bpb) -> u64 {
    match bpb.fat_type {
        FatType::Fat32 => bpb.fat_size_32 as u64,
        _ => bpb.fat_size_16 as u64,
    }
}

/// Read a FAT entry for a given cluster.
fn fat_read_entry(bpb: &Bpb, cluster: u32) -> u32 {
    match bpb.fat_type {
        FatType::Fat12 => {
            let off = cluster as u64 + cluster as u64 / 2;
            let sector = bpb.reserved_sectors as u64 + off / bpb.bytes_per_sector as u64;
            let byte_off = (off % bpb.bytes_per_sector as u64) as usize;
            let mut buf = [0u8; 512];
            if !read_bpb_sector(sector, &mut buf) { return 0x0FFF; }
            let val = buf[byte_off] as u32 | (buf[byte_off + 1] as u32) << 8;
            if cluster & 1 == 0 { val & 0x0FFF } else { val >> 4 }
        }
        FatType::Fat16 => {
            let off = cluster as u64 * 2;
            let sector = bpb.reserved_sectors as u64 + off / bpb.bytes_per_sector as u64;
            let byte_off = (off % bpb.bytes_per_sector as u64) as usize;
            let mut buf = [0u8; 512];
            if !read_bpb_sector(sector, &mut buf) { return 0xFFFF; }
            le16(&buf, byte_off) as u32
        }
        FatType::Fat32 => {
            let off = cluster as u64 * 4;
            let sector = bpb.reserved_sectors as u64 + off / bpb.bytes_per_sector as u64;
            let byte_off = (off % bpb.bytes_per_sector as u64) as usize;
            let mut buf = [0u8; 512];
            if !read_bpb_sector(sector, &mut buf) { return 0x0FFFFFFF; }
            le32(&buf, byte_off) & 0x0FFFFFFF
        }
    }
}

/// Write a FAT entry.
fn fat_write_entry(bpb: &Bpb, cluster: u32, value: u32) -> bool {
    match bpb.fat_type {
        FatType::Fat12 => {
            let off = cluster as u64 + cluster as u64 / 2;
            let sector = bpb.reserved_sectors as u64 + off / bpb.bytes_per_sector as u64;
            let byte_off = (off % bpb.bytes_per_sector as u64) as usize;
            let mut buf = [0u8; 512];
            if !read_bpb_sector(sector, &mut buf) { return false; }
            let val16 = buf[byte_off] as u32 | (buf[byte_off + 1] as u32) << 8;
            let new_val = if cluster & 1 == 0 {
                (val16 & 0xF000) | (value & 0x0FFF)
            } else {
                (val16 & 0x000F) | ((value & 0x0FFF) << 4)
            };
            write_le16(&mut buf, byte_off, new_val as u16);
            if !write_bpb_sector(sector, &buf) { return false; }
            // Mirror to second FAT
            if bpb.num_fats > 1 {
                let s2 = sector + fat_sectors(bpb);
                let _ = write_bpb_sector(s2, &buf);
            }
            true
        }
        FatType::Fat16 => {
            let off = cluster as u64 * 2;
            let sector = bpb.reserved_sectors as u64 + off / bpb.bytes_per_sector as u64;
            let byte_off = (off % bpb.bytes_per_sector as u64) as usize;
            let mut buf = [0u8; 512];
            if !read_bpb_sector(sector, &mut buf) { return false; }
            write_le16(&mut buf, byte_off, value as u16);
            if !write_bpb_sector(sector, &buf) { return false; }
            if bpb.num_fats > 1 {
                let s2 = sector + fat_sectors(bpb);
                let _ = write_bpb_sector(s2, &buf);
            }
            true
        }
        FatType::Fat32 => {
            let off = cluster as u64 * 4;
            let sector = bpb.reserved_sectors as u64 + off / bpb.bytes_per_sector as u64;
            let byte_off = (off % bpb.bytes_per_sector as u64) as usize;
            let mut buf = [0u8; 512];
            if !read_bpb_sector(sector, &mut buf) { return false; }
            let old = le32(&buf, byte_off);
            write_le32(&mut buf, byte_off, (old & 0xF0000000) | (value & 0x0FFFFFFF));
            if !write_bpb_sector(sector, &buf) { return false; }
            if bpb.num_fats > 1 {
                let s2 = sector + fat_sectors(bpb);
                let _ = write_bpb_sector(s2, &buf);
            }
            true
        }
    }
}

fn fat_is_eoc(bpb: &Bpb, val: u32) -> bool {
    match bpb.fat_type {
        FatType::Fat12 => val >= 0x0FF8,
        FatType::Fat16 => val >= 0xFFF8,
        FatType::Fat32 => val >= 0x0FFFFFF8,
    }
}

fn fat_eoc(bpb: &Bpb) -> u32 {
    match bpb.fat_type {
        FatType::Fat12 => 0x0FFF,
        FatType::Fat16 => 0xFFFF,
        FatType::Fat32 => 0x0FFFFFFF,
    }
}

fn _fat_bad(bpb: &Bpb) -> u32 {
    match bpb.fat_type {
        FatType::Fat12 => 0x0FF7,
        FatType::Fat16 => 0xFFF7,
        FatType::Fat32 => 0x0FFFFFF7,
    }
}

/// Get the LBA of a cluster's first sector.
fn cluster_to_lba(bpb: &Bpb, cluster: u32) -> u64 {
    bpb.data_start + (cluster as u64 - 2) * bpb.sectors_per_cluster as u64
}

/// Get root directory cluster.
fn root_cluster(bpb: &Bpb) -> u32 {
    match bpb.fat_type {
        FatType::Fat12 | FatType::Fat16 => 0, // root is at a fixed location
        FatType::Fat32 => bpb.root_cluster,
    }
}

/// Allocate a free cluster. Returns cluster number or 0 on failure.
fn alloc_cluster(bpb: &Bpb) -> u32 {
    let max = match bpb.fat_type {
        FatType::Fat12 => 0x0FF0,
        FatType::Fat16 => 0xFFF0,
        FatType::Fat32 => 0x0FFFFFF0,
    };
    for cl in 2..max {
        if fat_read_entry(bpb, cl) == 0 {
            // Verify it's really free by reading the entry
            if fat_read_entry(bpb, cl) == 0 {
                return cl;
            }
        }
    }
    0
}

/// Free a cluster chain.
fn free_chain(bpb: &Bpb, start: u32) {
    let mut cl = start;
    let mut iter = 0u32;
    while !fat_is_eoc(bpb, cl) && cl >= 2 {
        iter += 1;
        if iter > 100_000 { break; } // cycle protection
        let next = fat_read_entry(bpb, cl);
        let _ = fat_write_entry(bpb, cl, 0);
        cl = next;
    }
}

/// Read data from a cluster chain into a buffer.
/// Корректная версия: intra-смещение только для первого кластера,
/// дальше — сплошной поток байт через LBA кластеров.
fn read_chain(bpb: &Bpb, first_cluster: u32, offset: u64, buf: &mut [u8]) -> usize {
    if buf.is_empty() || first_cluster < 2 { return 0; }
    let bps = bpb.bytes_per_sector as u64;
    let spc = bpb.sectors_per_cluster as u64;
    let cluster_sz = bps * spc;
    if cluster_sz == 0 { return 0; }

    // Найти стартовый кластер и смещение внутри него
    let mut cluster = first_cluster;
    let mut skip = offset / cluster_sz;
    let mut iter = 0u32;
    while skip > 0 {
        if fat_is_eoc(bpb, cluster) || cluster < 2 { return 0; }
        cluster = fat_read_entry(bpb, cluster);
        skip -= 1;
        iter += 1;
        if iter > 100_000 { return 0; }
    }
    let mut intra = (offset % cluster_sz) as usize;
    let mut out = 0usize;
    iter = 0;
    while out < buf.len() {
        iter += 1;
        if iter > 100_000 { break; }
        if fat_is_eoc(bpb, cluster) || cluster < 2 { break; }
        let lba = cluster_to_lba(bpb, cluster);
        // Сколько байт взять из этого кластера
        let avail = cluster_sz as usize - intra;
        let want = (buf.len() - out).min(avail);
        // Читаем нужные сектора кластера
        let first_sec = intra / bps as usize;
        let last_byte = intra + want; // exclusive
        let last_sec = (last_byte + bps as usize - 1) / bps as usize;
        for s in first_sec..last_sec {
            let mut sec = [0u8; 512];
            if !read_bpb_sector(lba + s as u64, &mut sec) { return out; }
            let sec_start = if s == first_sec { intra % bps as usize } else { 0 };
            let sec_end_cap = bps as usize;
            let remaining_in_range = last_byte - (s * bps as usize);
            let take = remaining_in_range.min(sec_end_cap - sec_start).min(buf.len() - out);
            if take == 0 { break; }
            buf[out..out + take].copy_from_slice(&sec[sec_start..sec_start + take]);
            out += take;
            if out >= buf.len() { break; }
        }
        intra = 0; // только первый кластер имел смещение
        if out >= buf.len() { break; }
        cluster = fat_read_entry(bpb, cluster);
    }
    out
}

/// Write data to a cluster chain (allocating new clusters as needed).
fn write_chain(bpb: &Bpb, first_cluster: u32, offset: u64, data: &[u8]) -> usize {
    if data.is_empty() || first_cluster < 2 { return 0; }
    let bps = bpb.bytes_per_sector as u64;
    let spc = bpb.sectors_per_cluster as u64;
    let cluster_sz = bps * spc;
    if cluster_sz == 0 { return 0; }

    // Дойти до стартового кластера, доаллоцируя по пути
    let mut cluster = first_cluster;
    let mut skip = offset / cluster_sz;
    let mut iter = 0u32;
    while skip > 0 {
        iter += 1;
        if iter > 100_000 { return 0; }
        let next = fat_read_entry(bpb, cluster);
        if fat_is_eoc(bpb, next) || next < 2 {
            let nc = alloc_cluster(bpb);
            if nc == 0 { return 0; }
            let _ = fat_write_entry(bpb, nc, fat_eoc(bpb));
            let _ = fat_write_entry(bpb, cluster, nc);
            cluster = nc;
        } else {
            cluster = next;
        }
        skip -= 1;
    }
    let mut intra = (offset % cluster_sz) as usize;
    let mut done = 0usize;
    iter = 0;
    while done < data.len() {
        iter += 1;
        if iter > 100_000 { break; }
        if cluster < 2 { break; }
        if fat_is_eoc(bpb, cluster) {
            // Текущий — EOC-заглушка пустого файла: превращаем в данные
            let nc = alloc_cluster(bpb);
            if nc == 0 { break; }
            let _ = fat_write_entry(bpb, nc, fat_eoc(bpb));
            let _ = fat_write_entry(bpb, cluster, nc);
            cluster = nc;
        }
        let lba = cluster_to_lba(bpb, cluster);
        let avail = cluster_sz as usize - intra;
        let want = (data.len() - done).min(avail);
        let first_sec = intra / bps as usize;
        let last_byte = intra + want;
        let last_sec = (last_byte + bps as usize - 1) / bps as usize;
        for s in first_sec..last_sec {
            let mut sec = [0u8; 512];
            if !read_bpb_sector(lba + s as u64, &mut sec) { return done; }
            let sec_start = if s == first_sec { intra % bps as usize } else { 0 };
            let sec_off_in_range = s * bps as usize;
            // Сколько байт этого сектора входит в [intra, intra+want)
            let range_start = intra.max(sec_off_in_range);
            let range_end = (intra + want).min(sec_off_in_range + bps as usize);
            if range_end <= range_start { continue; }
            let take = (range_end - range_start).min(data.len() - done);
            let dst_off = sec_start + (range_start - sec_off_in_range.max(intra).min(range_start));
            // Проще: позиция в секторе = range_start - s*512
            let sec_pos = range_start - sec_off_in_range;
            sec[sec_pos..sec_pos + take].copy_from_slice(&data[done..done + take]);
            let _ = write_bpb_sector(lba + s as u64, &sec);
            done += take;
            if done >= data.len() { break; }
            let _ = dst_off;
        }
        intra = 0;
        if done >= data.len() { break; }
        // Переход к следующему кластеру с аллокацией
        let next = fat_read_entry(bpb, cluster);
        if fat_is_eoc(bpb, next) || next < 2 {
            let nc = alloc_cluster(bpb);
            if nc == 0 { break; }
            let _ = fat_write_entry(bpb, nc, fat_eoc(bpb));
            let _ = fat_write_entry(bpb, cluster, nc);
            cluster = nc;
        } else {
            cluster = next;
        }
    }
    done
}

// ── 8.3 name helpers ────────────────────────────────────────────

fn name_to_83(name: &[u8]) -> Option<[u8; 11]> {
    if name.is_empty() || name.len() > 12 { return None; }
    let mut entry = [b' '; 11];
    match name.iter().position(|&c| c == b'.') {
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

fn _vfat_checksum(short: &[u8; 11]) -> u8 {
    let mut sum: u8 = 0;
    for &b in short {
        sum = ((sum & 1) << 7) | (sum >> 1);
        sum = sum.wrapping_add(b);
    }
    sum
}

/// Read 13 UTF-16 characters from an LFN directory entry into `out`.
/// Each LFN entry stores 13 UTF-16LE chars across 3 regions:
///   bytes  1..11  (5 chars, 10 bytes)
///   bytes 14..26  (6 chars, 12 bytes)
///   bytes 28..32  (2 chars,  4 bytes)
/// Returns the number of valid characters written.
fn read_lfn_chars(entry: &[u8], out: &mut [u16; 13]) -> usize {
    // Region 1: 5 chars at offsets 1,3,5,7,9
    for j in 0..5 {
        let off = 1 + j * 2;
        out[j] = entry[off] as u16 | (entry[off + 1] as u16) << 8;
    }
    // Region 2: 6 chars at offsets 14,16,18,20,22,24
    for j in 0..6 {
        let off = 14 + j * 2;
        out[5 + j] = entry[off] as u16 | (entry[off + 1] as u16) << 8;
    }
    // Region 3: 2 chars at offsets 28,30
    for j in 0..2 {
        let off = 28 + j * 2;
        out[11 + j] = entry[off] as u16 | (entry[off + 1] as u16) << 8;
    }
    // Count trailing nulls
    let mut count = 13;
    while count > 0 && out[count - 1] == 0 { count -= 1; }
    count
}

// ── Directory reading ───────────────────────────────────────────

/// Read a directory sector for a given directory cluster (0 = root for FAT16).
fn read_dir_sector(bpb: &Bpb, dir_cluster: u32, sec_idx: u64, buf: &mut [u8; 512]) -> bool {
    match bpb.fat_type {
        FatType::Fat12 | FatType::Fat16 => {
            if dir_cluster == 0 {
                // Root directory (fixed location)
                let root_lba = bpb.reserved_sectors as u64 + bpb.num_fats as u64 * fat_sectors(bpb);
                let root_sectors = ((bpb.root_entry_count as u64 * 32 + 511) / 512) as u64;
                if sec_idx >= root_sectors { return false; }
                read_bpb_sector(root_lba + sec_idx, buf)
            } else {
                // Subdirectory (cluster chain)
                let spc = bpb.sectors_per_cluster as u64;
                let head = sec_idx / spc;
                let tail = sec_idx % spc;
                let mut cluster = dir_cluster;
                for _ in 0..head {
                    if fat_is_eoc(bpb, cluster) { return false; }
                    cluster = fat_read_entry(bpb, cluster);
                    if cluster < 2 { return false; }
                }
                read_bpb_sector(cluster_to_lba(bpb, cluster) + tail, buf)
            }
        }
        FatType::Fat32 => {
            if dir_cluster == 0 {
                // Shouldn't happen for FAT32 (root has cluster chain)
                return false;
            }
            let spc = bpb.sectors_per_cluster as u64;
            let head = sec_idx / spc;
            let tail = sec_idx % spc;
            let mut cluster = dir_cluster;
            for _ in 0..head {
                if fat_is_eoc(bpb, cluster) { return false; }
                cluster = fat_read_entry(bpb, cluster);
                if cluster < 2 { return false; }
            }
            read_bpb_sector(cluster_to_lba(bpb, cluster) + tail, buf)
        }
    }
}

fn write_dir_sector(bpb: &Bpb, dir_cluster: u32, sec_idx: u64, buf: &[u8; 512]) -> bool {
    match bpb.fat_type {
        FatType::Fat12 | FatType::Fat16 => {
            if dir_cluster == 0 {
                let root_lba = bpb.reserved_sectors as u64 + bpb.num_fats as u64 * fat_sectors(bpb);
                let root_sectors = ((bpb.root_entry_count as u64 * 32 + 511) / 512) as u64;
                if sec_idx >= root_sectors { return false; }
                write_bpb_sector(root_lba + sec_idx, buf)
            } else {
                let spc = bpb.sectors_per_cluster as u64;
                let head = sec_idx / spc;
                let tail = sec_idx % spc;
                let mut cluster = dir_cluster;
                for _ in 0..head {
                    if fat_is_eoc(bpb, cluster) { return false; }
                    cluster = fat_read_entry(bpb, cluster);
                    if cluster < 2 { return false; }
                }
                write_bpb_sector(cluster_to_lba(bpb, cluster) + tail, buf)
            }
        }
        FatType::Fat32 => {
            if dir_cluster == 0 { return false; }
            let spc = bpb.sectors_per_cluster as u64;
            let head = sec_idx / spc;
            let tail = sec_idx % spc;
            let mut cluster = dir_cluster;
            for _ in 0..head {
                if fat_is_eoc(bpb, cluster) { return false; }
                cluster = fat_read_entry(bpb, cluster);
                if cluster < 2 { return false; }
            }
            write_bpb_sector(cluster_to_lba(bpb, cluster) + tail, buf)
        }
    }
}

/// Total sectors for a directory.
fn dir_max_sectors(bpb: &Bpb, dir_cluster: u32) -> u64 {
    match bpb.fat_type {
        FatType::Fat12 | FatType::Fat16 => {
            if dir_cluster == 0 {
                (bpb.root_entry_count as u64 * 32 + 511) / 512
            } else {
                u64::MAX // follow cluster chain
            }
        }
        FatType::Fat32 => u64::MAX,
    }
}

/// Find an entry in a directory by name. Returns (cluster, size, entry_idx, attr).
/// Matches both 8.3 short names and LFN (Long File Name) entries.
fn find_in_dir(bpb: &Bpb, dir_cluster: u32, name: &[u8]) -> Option<(u32, u32, u64, u8)> {
    let attr_lfn: u8 = 0x0F;
    let attr_vol: u8 = 0x08;
    let max_sec = dir_max_sectors(bpb, dir_cluster);

    // LFN accumulation state
    let mut lfn_name = [0u8; 256];
    let mut lfn_filled = false;

    for sec in 0..max_sec {
        let mut buf = [0u8; 512];
        if !read_dir_sector(bpb, dir_cluster, sec, &mut buf) { return None; }
        for i in 0..16 {
            let off = i * 32;
            if buf[off] == 0 { return None; } // end of dir
            if buf[off] == 0xE5 {
                lfn_filled = false;
                lfn_name = [0u8; 256];
                continue;
            }
            let attr = buf[off + 11];

            // LFN entry
            if attr & attr_lfn == attr_lfn {
                let seq = buf[off];
                let is_last = seq & 0x40 != 0;
                let seq_num = (seq & 0x3F) as usize;
                if seq_num == 0 || seq_num > 20 {
                    lfn_filled = false;
                    lfn_name = [0u8; 256];
                    continue;
                }
                let idx = seq_num - 1;
                let mut utf16 = [0u16; 13];
                read_lfn_chars(&buf[off..], &mut utf16);
                for j in 0..13 {
                    let pos = idx * 13 + j;
                    if pos >= 256 { break; }
                    let ch = utf16[j];
                    if ch == 0 {
                        lfn_name[pos] = 0;
                    } else if ch < 0x80 {
                        let mut c = ch as u8;
                        if c >= b'a' && c <= b'z' { c -= 32; }
                        lfn_name[pos] = c;
                    } else {
                        lfn_name[pos] = 0xFF;
                    }
                }
                if is_last { lfn_filled = true; }
                continue;
            }

            if attr & attr_vol != 0 {
                lfn_filled = false;
                lfn_name = [0u8; 256];
                continue;
            }

            // 8.3 entry — try matching
            let ename: &[u8; 11] = &buf[off..off + 11].try_into().ok()?;

            // Match against short 8.3 name
            if name_match(ename, name) {
                let cl = match bpb.fat_type {
                    FatType::Fat32 => le16(&buf, off + 20) as u32 * 65536 + le16(&buf, off + 26) as u32,
                    _ => le16(&buf, off + 26) as u32,
                };
                let sz = le32(&buf, off + 28);
                return Some((cl, sz, sec * 16 + i as u64, attr));
            }

            // Match against LFN name (case-insensitive)
            if lfn_filled {
                let lfn_len = lfn_name.iter().position(|&c| c == 0).unwrap_or(256);
                if lfn_match(&lfn_name[..lfn_len], name) {
                    let cl = match bpb.fat_type {
                        FatType::Fat32 => le16(&buf, off + 20) as u32 * 65536 + le16(&buf, off + 26) as u32,
                        _ => le16(&buf, off + 26) as u32,
                    };
                    let sz = le32(&buf, off + 28);
                    return Some((cl, sz, sec * 16 + i as u64, attr));
                }
            }

            // Reset LFN state after the 8.3 entry
            lfn_filled = false;
            lfn_name = [0u8; 256];
        }
    }
    None
}

/// Case-insensitive match of an ASCII name against a user-supplied path component.
/// Both are ASCII; `lfn` is the uppercased long filename, `user` is the raw lookup.
fn lfn_match(lfn: &[u8], user: &[u8]) -> bool {
    if lfn.len() != user.len() { return false; }
    for i in 0..lfn.len() {
        let a = lfn[i];
        let b = if user[i] >= b'a' && user[i] <= b'z' { user[i] - 32 } else { user[i] };
        if a != b { return false; }
    }
    true
}

/// Find the parent cluster for a path.
fn resolve_dir(bpb: &Bpb, path: &[u8]) -> Option<u32> {
    if path.is_empty() { return Some(root_cluster(bpb)); }

    let mut cluster = root_cluster(bpb);
    let mut pos = 0;

    // Skip leading slashes
    while pos < path.len() && path[pos] == b'/' { pos += 1; }

    while pos < path.len() {
        // Extract component
        let start = pos;
        while pos < path.len() && path[pos] != b'/' { pos += 1; }
        let comp = &path[start..pos];
        if comp.is_empty() { pos += 1; continue; }

        // Look up component in current cluster
        match find_in_dir(bpb, cluster, comp) {
            Some((sub_cl, _, _, attr)) => {
                if attr & 0x10 == 0 { return None; } // not a directory
                cluster = sub_cl;
            }
            None => return None,
        }

        // Skip slash
        while pos < path.len() && path[pos] == b'/' { pos += 1; }
    }
    Some(cluster)
}
fn parent_path<'a>(path: &'a [u8]) -> &'a [u8] {
    let name = last_component(path);
    if name.is_empty() { return b""; }
    // find start of name in path
    let mut end = path.len();
    while end > 0 && path[end - 1] == b'/' { end -= 1; }
    let mut start = end - name.len();
    while start > 0 && path[start - 1] == b'/' { start -= 1; }
    // parent is up to start (trim trailing slashes)
    let mut p_end = start;
    while p_end > 0 && path[p_end - 1] == b'/' { p_end -= 1; }
    // keep leading slash if original had it
    if p_end == 0 {
        if !path.is_empty() && path[0]==b'/' { return b"/"; }
        return b"";
    }
    &path[..p_end]
}
fn resolve_parent(bpb: &Bpb, path: &[u8]) -> Option<u32> {
    let p = parent_path(path);
    if p.is_empty() || (p.len()==1 && p[0]==b'/') { return Some(root_cluster(bpb)); }
    resolve_dir(bpb, p)
}

/// Get the last component of a path.
fn last_component(path: &[u8]) -> &[u8] {
    let mut end = path.len();
    while end > 0 && path[end - 1] == b'/' { end -= 1; }
    let mut start = end;
    while start > 0 && path[start - 1] != b'/' { start -= 1; }
    &path[start..end]
}

/// Extend a directory's cluster chain by one cluster.
fn extend_dir(bpb: &Bpb, dir_cluster: u32) -> Option<u32> {
    let new = alloc_cluster(bpb);
    if new == 0 { return None; }
    let _ = fat_write_entry(bpb, new, fat_eoc(bpb));

    let mut last = dir_cluster;
    let mut iter = 0u32;
    loop {
        let next = fat_read_entry(bpb, last);
        if fat_is_eoc(bpb, next) { break; }
        last = next;
        iter += 1;
        if iter > 100_000 { return None; }
    }
    let _ = fat_write_entry(bpb, last, new);
    Some(new)
}

/// Initialize a new directory cluster with . and .. entries.
fn init_dir_cluster(bpb: &Bpb, self_cluster: u32, parent_cluster: u32) -> bool {
    let lba = cluster_to_lba(bpb, self_cluster);
    let mut buf = [0u8; 512];
    buf[0..8].copy_from_slice(b".       ");
    buf[11] = 0x10;
    match bpb.fat_type {
        FatType::Fat32 => {
            write_le16(&mut buf, 20, (self_cluster >> 16) as u16);
            write_le16(&mut buf, 26, self_cluster as u16);
        }
        _ => {
            write_le16(&mut buf, 26, self_cluster as u16);
        }
    }
    buf[32..40].copy_from_slice(b"..      ");
    buf[43] = 0x10;
    match bpb.fat_type {
        FatType::Fat32 => {
            write_le16(&mut buf, 52, (parent_cluster >> 16) as u16);
            write_le16(&mut buf, 58, parent_cluster as u16);
        }
        _ => {
            write_le16(&mut buf, 58, parent_cluster as u16);
        }
    }
    let _ = write_bpb_sector(lba, &buf);
    let zero = [0u8; 512];
    for s in 1..bpb.sectors_per_cluster as u64 {
        let _ = write_bpb_sector(lba + s, &zero);
    }
    true
}

/// Add a directory entry. Returns entry index or None.
fn add_dir_entry(bpb: &Bpb, dir_cluster: u32, _name: &[u8], short: &[u8; 11], attr: u8, cluster: u32, size: u32) -> Option<u64> {
    // Find a free slot in the directory
    let max_sec = dir_max_sectors(bpb, dir_cluster);
    for sec in 0..max_sec {
        let mut buf = [0u8; 512];
        if !read_dir_sector(bpb, dir_cluster, sec, &mut buf) {
            if dir_cluster == 0 { return None; }
            if let Some(new_cl) = extend_dir(bpb, dir_cluster) {
                let zero = [0u8; 512];
                let lba = cluster_to_lba(bpb, new_cl);
                for s in 0..bpb.sectors_per_cluster as u64 {
                    let _ = write_bpb_sector(lba + s, &zero);
                }
                if !read_dir_sector(bpb, dir_cluster, sec, &mut buf) { return None; }
            } else { return None; }
        }
        for i in 0..16 {
            let off = i * 32;
            if buf[off] != 0 && buf[off] != 0xE5 { continue; }

            // Found free slot — write short name entry
            buf[off..off + 11].copy_from_slice(short);
            buf[off + 11] = attr;
            for j in 12..26 { buf[off + j] = 0; }
            match bpb.fat_type {
                FatType::Fat32 => {
                    write_le16(&mut buf, off + 20, (cluster >> 16) as u16);
                    write_le16(&mut buf, off + 26, cluster as u16);
                }
                _ => {
                    write_le16(&mut buf, off + 26, cluster as u16);
                }
            }
            write_le32(&mut buf, off + 28, size);
            let _ = write_dir_sector(bpb, dir_cluster, sec, &buf);
            return Some(sec * 16 + i as u64);
        }
    }
    None
}

/// Delete a directory entry (mark as deleted).
fn delete_entry(bpb: &Bpb, dir_cluster: u32, entry_idx: u64) -> bool {
    let sec = entry_idx / 16;
    let off = ((entry_idx % 16) as usize) * 32;
    let mut buf = [0u8; 512];
    if !read_dir_sector(bpb, dir_cluster, sec, &mut buf) { return false; }
    buf[off] = 0xE5;
    write_dir_sector(bpb, dir_cluster, sec, &buf)
}

/// Check if directory is empty (only . and .. entries).
fn is_dir_empty(bpb: &Bpb, dir_cluster: u32) -> bool {
    let max_sec = dir_max_sectors(bpb, dir_cluster);
    for sec in 0..max_sec {
        let mut buf = [0u8; 512];
        if !read_dir_sector(bpb, dir_cluster, sec, &mut buf) { break; }
        for i in 0..16 {
            let off = i * 32;
            if buf[off] == 0 { return true; }
            if buf[off] == 0xE5 { continue; }
            let a = buf[off + 11];
            if a & 0x0F == 0x0F { continue; }
            if a & 0x08 != 0 { continue; }
            // . and .. — skip
            if i <= 1 && (buf[off] == b'.') { continue; }
            return false;
        }
    }
    true
}

// ── Static BPB cache ────────────────────────────────────────────

static mut CACHED_BPB: Option<Bpb> = None;

fn get_bpb() -> Option<Bpb> {
    unsafe {
        if let Some(bpb) = CACHED_BPB { return Some(bpb); }
        let bpb = read_bpb()?;
        CACHED_BPB = Some(bpb);
        Some(bpb)
    }
}

// ── VFS Driver implementation (function pointers) ────────────────

fn fat_open(path: &[u8], flags: u64) -> Option<usize> {
    let bpb = get_bpb()?;
    let dir = resolve_parent(&bpb, path)?;
    let name = last_component(path);
    if name.is_empty() { return Some(0); }
    if let Some((cl, sz, _, _)) = find_in_dir(&bpb, dir, name) {
        // TRUNC flag: truncate file to 0
        if flags & 0x200 != 0 {
            free_chain(&bpb, cl);
            // update dir entry size to 0 and cluster 0
            // find entry and zero it
            if let Some((_, _, entry_idx, _)) = find_in_dir(&bpb, dir, name) {
                let sec = entry_idx / 16;
                let off = ((entry_idx % 16) as usize) * 32;
                let mut buf = [0u8; 512];
                if read_dir_sector(&bpb, dir, sec, &mut buf) {
                    // clear cluster and size
                    match bpb.fat_type {
                        FatType::Fat32 => {
                            write_le16(&mut buf, off + 20, 0);
                            write_le16(&mut buf, off + 26, 0);
                        }
                        _ => write_le16(&mut buf, off + 26, 0),
                    }
                    write_le32(&mut buf, off + 28, 0);
                    let _ = write_dir_sector(&bpb, dir, sec, &buf);
                }
            }
            return Some(0);
        }
        return Some((cl as usize) << 32 | sz as usize)
    }
    // Not found — create if O_CREAT
    if flags & 0x100 == 0 { return None; }
    let short = name_to_83(name)?;
    let cluster = alloc_cluster(&bpb);
    if cluster == 0 { return None; }
    let _ = fat_write_entry(&bpb, cluster, fat_eoc(&bpb));
    // zero first cluster
    let zero = [0u8; 512];
    let lba = cluster_to_lba(&bpb, cluster);
    for s in 0..bpb.sectors_per_cluster as u64 { let _ = write_bpb_sector(lba + s, &zero); }
    if add_dir_entry(&bpb, dir, name, &short, 0x20, cluster, 0).is_none() { free_chain(&bpb, cluster); return None; }
    Some((cluster as usize) << 32 | 0)
}

fn fat_close(_handle: usize) {}

fn fat_read(handle: usize, buf: &mut [u8], offset: u64) -> Option<usize> {
    let bpb = get_bpb()?;
    let cluster = (handle >> 32) as u32;
    let size = (handle & 0xFFFFFFFF) as u64;
    if offset >= size { return Some(0); }
    if cluster < 2 {
        // Пустой файл (size 0) — нечего читать; раньше уходил в read_chain с cl=0
        return Some(0);
    }
    let avail = (size - offset) as usize;
    let to_read = buf.len().min(avail);
    if to_read == 0 { return Some(0); }
    let n = read_chain(&bpb, cluster, offset, &mut buf[..to_read]);
    Some(n)
}

fn fat_write(handle: usize, buf: &[u8], offset: u64) -> Option<usize> {
    let bpb = get_bpb()?;
    let cluster = (handle >> 32) as u32;
    let n = write_chain(&bpb, cluster, offset, buf);
    Some(n)
}

fn fat_stat(path: &[u8]) -> Option<StatInfo> {
    let bpb = get_bpb()?;
    if path.is_empty() || (path.len() == 1 && path[0] == b'/') {
        return Some(StatInfo { size: 0, is_dir: true, cluster: root_cluster(&bpb) });
    }
    // if path is a directory itself, try to resolve it directly
    if let Some(dir_cl) = resolve_dir(&bpb, path) {
        // Check if it's a directory (found as dir)
        return Some(StatInfo { size: 0, is_dir: true, cluster: dir_cl });
    }
    let dir = resolve_parent(&bpb, path)?;
    let name = last_component(path);
    if name.is_empty() { return Some(StatInfo { size: 0, is_dir: true, cluster: root_cluster(&bpb) }); }
    let (cl, sz, _, attr) = find_in_dir(&bpb, dir, name)?;
    Some(StatInfo { size: sz, is_dir: attr & 0x10 != 0, cluster: cl })
}

fn fat_readdir(path: &[u8], entries: &mut [DirEntry]) -> Option<usize> {
    let bpb = get_bpb()?;
    let dir_cluster = if path.is_empty() || (path.len() == 1 && path[0] == b'/') {
        root_cluster(&bpb)
    } else {
        resolve_dir(&bpb, path)?
    };

    let attr_lfn: u8 = 0x0F;
    let attr_vol: u8 = 0x08;
    let max_sec = dir_max_sectors(&bpb, dir_cluster);
    let mut count = 0usize;

    // LFN accumulation state.
    // LFN entries appear BEFORE the 8.3 entry, in reverse sequence order
    // (highest seq first). We fill lfn_name from the back using seq as index.
    let mut lfn_name = [0u8; 256];
    let mut lfn_filled = false; // true once we've seen the last-LFN entry (bit 6)

    for sec in 0..max_sec {
        if count >= entries.len() { break; }
        let mut buf = [0u8; 512];
        if !read_dir_sector(&bpb, dir_cluster, sec, &mut buf) { break; }
        for i in 0..16 {
            if count >= entries.len() { break; }
            let off = i * 32;

            // Empty marker — end of directory
            if buf[off] == 0 {
                return Some(count);
            }
            if buf[off] == 0xE5 {
                // Deleted entry — discard any pending LFN state
                lfn_filled = false;
                lfn_name = [0u8; 256];
                continue;
            }

            let attr = buf[off + 11];

            // LFN entry
            if attr & attr_lfn == attr_lfn {
                let seq = buf[off]; // sequence number
                let is_last = seq & 0x40 != 0;
                let seq_num = (seq & 0x3F) as usize; // 1-based
                if seq_num == 0 || seq_num > 20 {
                    // Invalid LFN entry — reset
                    lfn_filled = false;
                    lfn_name = [0u8; 256];
                    continue;
                }
                let idx = seq_num - 1; // 0-based
                // Extract 13 UTF-16 chars and convert to ASCII
                let mut utf16 = [0u16; 13];
                read_lfn_chars(&buf[off..], &mut utf16);
                for j in 0..13 {
                    let pos = idx * 13 + j;
                    if pos >= 256 { break; }
                    let ch = utf16[j];
                    if ch == 0 {
                        lfn_name[pos] = 0;
                    } else if ch < 0x80 {
                        let mut c = ch as u8;
                        if c >= b'a' && c <= b'z' { c -= 32; } // ASCII uppercase
                        lfn_name[pos] = c;
                    } else {
                        // Non-ASCII — mark as 0xFF (placeholder)
                        lfn_name[pos] = 0xFF;
                    }
                }
                if is_last { lfn_filled = true; }
                continue;
            }

            if attr & attr_vol != 0 {
                // Volume label — discard LFN state
                lfn_filled = false;
                lfn_name = [0u8; 256];
                continue;
            }

            // Normal 8.3 entry — emit it
            let raw_name = format_name(&buf[off..]);
            let name_len = raw_name.iter().position(|&c| c == 0).unwrap_or(13);
            let mut e = DirEntry {
                name: [0; MAX_NAME],
                is_dir: attr & 0x10 != 0,
                size: le32(&buf, off + 28),
            };

            if lfn_filled {
                // Use the accumulated long filename
                let lfn_len = lfn_name.iter().position(|&c| c == 0).unwrap_or(256);
                let copy_len = lfn_len.min(MAX_NAME - 1);
                e.name[..copy_len].copy_from_slice(&lfn_name[..copy_len]);
            } else {
                // Fall back to 8.3 short name
                let copy_len = name_len.min(MAX_NAME - 1);
                e.name[..copy_len].copy_from_slice(&raw_name[..copy_len]);
            }

            entries[count] = e;
            count += 1;

            // Reset LFN state for the next entry
            lfn_filled = false;
            lfn_name = [0u8; 256];
        }
    }
    Some(count)
}

fn fat_mkdir(path: &[u8]) -> bool {
    let bpb = match get_bpb() { Some(b) => b, None => return false };
    let parent = match resolve_parent(&bpb, path) {
        Some(p) => p,
        None => { uart_print("[FAT] path not found\r\n"); return false; }
    };
    let name = last_component(path);
    if name.is_empty() { uart_print("[FAT] empty name\r\n"); return false; }
    if find_in_dir(&bpb, parent, name).is_some() {
        uart_print("[FAT] already exists\r\n"); return false;
    }
    let short = match name_to_83(name) {
        Some(s) => s,
        None => { uart_print("[FAT] bad name\r\n"); return false; }
    };
    let cluster = alloc_cluster(&bpb);
    if cluster == 0 {
        uart_print("[FAT] no free clusters\r\n"); return false;
    }
    let _ = fat_write_entry(&bpb, cluster, fat_eoc(&bpb));
    if !init_dir_cluster(&bpb, cluster, parent) {
        uart_print("[FAT] init dir failed\r\n"); return false;
    }
    if add_dir_entry(&bpb, parent, name, &short, 0x10, cluster, 0).is_none() {
        uart_print("[FAT] add entry failed\r\n"); return false;
    }
    true
}

fn fat_rmdir(path: &[u8]) -> bool {
    let bpb = match get_bpb() { Some(b) => b, None => return false };
    let parent = match resolve_parent(&bpb, path) {
        Some(p) => p,
        None => { uart_print("[FAT] path not found\r\n"); return false; }
    };
    let name = last_component(path);
    if name.is_empty() { uart_print("[FAT] empty name\r\n"); return false; }
    let (cluster, _, entry_idx, attr) = match find_in_dir(&bpb, parent, name) {
        Some(v) => v,
        None => { uart_print("[FAT] not found\r\n"); return false; }
    };
    if attr & 0x10 == 0 { uart_print("[FAT] not a directory\r\n"); return false; }
    if !is_dir_empty(&bpb, cluster) {
        uart_print("[FAT] directory not empty\r\n"); return false;
    }
    free_chain(&bpb, cluster);
    if !delete_entry(&bpb, parent, entry_idx) {
        uart_print("[FAT] delete entry failed\r\n"); return false;
    }
    true
}

fn fat_unlink(path: &[u8]) -> bool {
    let bpb = match get_bpb() { Some(b) => b, None => return false };
    let parent = match resolve_parent(&bpb, path) {
        Some(p) => p,
        None => { uart_print("[FAT] path not found\r\n"); return false; }
    };
    let name = last_component(path);
    if name.is_empty() { uart_print("[FAT] empty name\r\n"); return false; }
    let (cluster, _, entry_idx, attr) = match find_in_dir(&bpb, parent, name) {
        Some(v) => v,
        None => { uart_print("[FAT] not found\r\n"); return false; }
    };
    if attr & 0x10 != 0 { uart_print("[FAT] is a directory\r\n"); return false; }
    free_chain(&bpb, cluster);
    if !delete_entry(&bpb, parent, entry_idx) {
        uart_print("[FAT] delete entry failed\r\n"); return false;
    }
    true
}

static FAT_OPS: crate::vfs::FsOps = crate::vfs::FsOps {
    open: fat_open,
    close: fat_close,
    read: fat_read,
    write: fat_write,
    stat: fat_stat,
    readdir: fat_readdir,
    mkdir: fat_mkdir,
    rmdir: fat_rmdir,
    unlink: fat_unlink,
};

/// Init the FAT driver and mount at "/".
pub fn init() -> bool {
    let bpb = match read_bpb() {
        Some(b) => b,
        None => {
            uart_print("[FAT] no valid FAT filesystem found\r\n");
            return false;
        }
    };

    unsafe { CACHED_BPB = Some(bpb); }

    uart_print("[FAT] type=");
    match bpb.fat_type {
        FatType::Fat12 => uart_print("FAT12"),
        FatType::Fat16 => uart_print("FAT16"),
        FatType::Fat32 => uart_print("FAT32"),
    }
    uart_print(" bps=");
    uart_print_dec(bpb.bytes_per_sector as u64);
    uart_print(" spc=");
    uart_print_dec(bpb.sectors_per_cluster as u64);
    uart_print(" clusters=");
    uart_print_dec(bpb.total_clusters);
    uart_print("\r\n");

    // Register driver and mount at "/"
    let idx = unsafe { crate::vfs::register_driver(&FAT_OPS) };
    unsafe { crate::vfs::mount(b"/", idx); }

    true
}

fn uart_print_dec(mut v: u64) {
    if v == 0 { uart_print("0"); return; }
    let mut buf = [0u8; 20]; let mut i = 0;
    while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart_print(core::str::from_utf8(&buf[i..i+1]).unwrap_or("?")); }
}

// Need Vec for readdir LFN — replace with stack-based approach
// Actually removed Vec usage above, so we're fine.
