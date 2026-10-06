//! GPT partition parser — shared helper for NVMe/AHCI.
//!
//! Проблема: ESP образ собирается как GPT+FAT16 (`scripts/mk_esp.py`),
//! поэтому MBR — protective (ptype 0xEE). Старый код отбрасывал 0xEE
//! и FAT на SATA/NVMe никогда не находился.
//! Этот модуль находит первую usable партицию через GPT header (LBA1)
//! и таблицу партиций (обычно LBA2, 128 x 128 байт).

/// Check for protective MBR signature + 0xEE entry.
pub fn is_protective_mbr(mbr: &[u8]) -> bool {
    if mbr.len() < 512 { return false; }
    let sig = (mbr[0x1FE] as u16) | ((mbr[0x1FF] as u16) << 8);
    if sig != 0xAA55 { return false; }
    // Any of the 4 entries with type 0xEE?
    mbr[0x1C2] == 0xEE || mbr[0x1D2] == 0xEE || mbr[0x1E2] == 0xEE || mbr[0x1F2] == 0xEE
}

fn le32(b: &[u8], off: usize) -> u32 {
    (b[off] as u32) | ((b[off+1] as u32) << 8) | ((b[off+2] as u32) << 16) | ((b[off+3] as u32) << 24)
}
fn le64(b: &[u8], off: usize) -> u64 {
    le32(b, off) as u64 | ((le32(b, off+4) as u64) << 32)
}

/// Parse GPT header at LBA1.
/// Returns (part_entry_lba, entry_count, entry_size).
pub fn parse_header(hdr: &[u8; 512]) -> Option<(u64, u32, u32)> {
    // Signature "EFI PART"
    if &hdr[0..8] != b"EFI PART" { return None; }
    // Revision 1.0
    if hdr[8] != 0x00 || hdr[9] != 0x00 || hdr[10] != 0x01 { return None; }
    let entry_lba = le64(hdr, 72);
    let entry_num = le32(hdr, 80);
    let entry_size = le32(hdr, 84);
    if entry_lba == 0 || entry_num == 0 || entry_num > 1024 { return None; }
    if entry_size < 128 || entry_size > 512 { return None; }
    Some((entry_lba, entry_num, entry_size))
}

/// Scan GPT entry array (raw bytes) for first entry with non-zero type GUID.
/// Entry layout: type GUID (16) | unique GUID (16) | StartingLBA (8) | EndingLBA (8) | ...
pub fn first_partition_lba(entries: &[u8], entry_num: u32, entry_size: u32) -> Option<u64> {
    let n = entry_num.min(256) as usize;
    let sz = entry_size as usize;
    for i in 0..n {
        let off = i * sz;
        if off + 48 > entries.len() { break; }
        let e = &entries[off..off+sz.min(entries.len()-off)];
        // type GUID all zero = unused
        let mut used = false;
        for k in 0..16 { if e[k] != 0 { used = true; break; } }
        if !used { continue; }
        let start = le64(e, 32);
        if start != 0 { return Some(start); }
    }
    None
}
