// ext4 filesystem driver — read/write via block layer.
//
// Reference: ext4 disk layout (kernel.org/doc/html/latest/filesystems/ext4/)
// Superblock at byte 1024, Block Group Descriptor Table after it.
// Supports: extent-based inodes, basic read/write/create/delete, directory ops.

#![allow(dead_code, unused_variables)]

use crate::driver::uart;
use crate::block;
use crate::vfs::MAX_NAME;

// ── On-disk structures (little-endian) ─────────────────────────────

pub const EXT4_MAGIC: u16 = 0xEF53;
const S_IFDIR: u16 = 0x4000;
const S_IFREG: u16 = 0x8000;
const EXT4_ET_EXTENT: u16 = 0x8000;  // extent tree magic
const EXT4_FEATURE_INCOMPAT_EXTENTS: u32 = 0x0040;
const EXT4_DIR_ENTRY_TYPE_FILE: u8 = 1;
const EXT4_DIR_ENTRY_TYPE_DIR: u8 = 2;

// ── Superblock (1024 bytes at partition offset 1024) ───────────────

pub struct Ext4Super {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub r_blocks_count: u32,
    pub free_blocks: u32,
    pub free_inodes: u32,
    pub first_data_block: u32,
    pub block_size: u32,
    pub blocks_per_group: u32,
    pub inodes_per_group: u32,
    pub inode_size: u32,
    pub inode_reserved_gdt_blocks: u32,
    pub meta_bg: u32,
    pub feature_compat: u32,
    pub feature_incompat: u32,
    pub feature_ro_compat: u32,
    pub blocks_per_group_bits: u32,
    pub frags_per_group: u32,
    pub num_block_groups: u32,
    pub mtime: u32,
    pub wtime: u32,
    pub mount_count: u16,
    pub max_mount_count: u16,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub minor_rev_level: u16,
    pub last_check: u32,
    pub check_interval: u32,
    pub creator_os: u32,
    pub rev_level: u32,
    pub def_resuid: u16,
    pub def_resgid: u16,
    pub first_ino: u32,
    pub block_group_nr: u32,
    pub feature_compat2: u32,
    pub uuid: [u8; 16],
    pub label: [u8; 16],
    pub last_mounted: [u8; 64],
    pub algo_bitmap: u32,
    pub prealloc_blocks: u8,
    pub prealloc_dir_blocks: u8,
    pub reserved_gdt_blocks: u16,
    pub journal_uuid: [u8; 16],
    pub journal_inum: u32,
    pub journal_dev: u32,
    pub last_orphan: u32,
    pub hash_seed: [u32; 4],
    pub def_hash_version: u8,
    pub jnl_backup_type: u8,
    pub desc_size: u32,
    pub default_mount_opts: u32,
    pub first_meta_bg: u32,
    pub mkfs_time: u32,
    pub jnl_blocks: [u32; 17],
}

// ── Block Group Descriptor (32 or 64 bytes) ───────────────────────

#[derive(Clone, Copy, Default)]
struct Bgd {
    block_bitmap: u32,
    inode_bitmap: u32,
    inode_table: u32,
    free_blocks_count: u16,
    free_inodes_count: u16,
    used_dirs_count: u16,
    pad: u16,
    reserved: [u8; 12],
}

// ── Inode ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Default)]
struct Inode {
    mode: u16,
    uid: u16,
    size_lo: u32,
    atime: u32,
    ctime: u32,
    mtime: u32,
    dtime: u32,
    gid: u16,
    links_count: u16,
    blocks: u32,
    flags: u32,
    osd1: u32,
    block: [u32; 15], // 12 direct + 1 indirect + 1 double + 1 triple
    generation: u32,
    file_acl: u32,
    size_hi: u32,
    extra_isize: u16,
}

impl Inode {
    fn size(&self) -> u64 {
        (self.size_hi as u64) << 32 | (self.size_lo as u64)
    }
    fn is_dir(&self) -> bool { self.mode & S_IFDIR != 0 }
    fn is_reg(&self) -> bool { self.mode & S_IFREG != 0 }
}

// ── Extent tree ────────────────────────────────────────────────────

#[derive(Clone, Copy, Default)]
struct ExtentHeader {
    magic: u16,
    entries: u16,
    max_entries: u16,
    depth: u16,
    generation: u32,
}

#[derive(Clone, Copy, Default)]
struct Extent {
    block: u32,        // first file block
    start_lo: u32,     // start physical block (low)
    start_hi: u16,     // start physical block (high)
    len: u16,          // length in blocks
}

#[derive(Clone, Copy, Default)]
struct ExtIndex {
    block: u32,
    start_lo: u32,
    start_hi: u16,
    uid: u16,
}

// ── Directory entry ────────────────────────────────────────────────

const EXT4_DIR_ENTRY_BASE: usize = 8; // inode(4) + rec_len(2) + name_len(1) + file_type(1)

// ── State ──────────────────────────────────────────────────────────

static mut SUPER: Ext4Super = Ext4Super {
    inodes_count: 0, blocks_count: 0, r_blocks_count: 0,
    free_blocks: 0, free_inodes: 0, first_data_block: 0,
    block_size: 0, blocks_per_group: 0, inodes_per_group: 0,
    inode_size: 0, inode_reserved_gdt_blocks: 0, meta_bg: 0,
    feature_compat: 0, feature_incompat: 0, feature_ro_compat: 0,
    blocks_per_group_bits: 0, frags_per_group: 0, num_block_groups: 0,
    mtime: 0, wtime: 0, mount_count: 0, max_mount_count: 0,
    magic: 0, state: 0, errors: 0, minor_rev_level: 0,
    last_check: 0, check_interval: 0, creator_os: 0, rev_level: 0,
    def_resuid: 0, def_resgid: 0, first_ino: 0, block_group_nr: 0,
    feature_compat2: 0, uuid: [0; 16], label: [0; 16], last_mounted: [0; 64],
    algo_bitmap: 0, prealloc_blocks: 0, prealloc_dir_blocks: 0,
    reserved_gdt_blocks: 0, journal_uuid: [0; 16], journal_inum: 0,
    journal_dev: 0, last_orphan: 0, hash_seed: [0; 4],
    def_hash_version: 0, jnl_backup_type: 0, desc_size: 0,
    default_mount_opts: 0, first_meta_bg: 0, mkfs_time: 0,
    jnl_blocks: [0; 17],
};
static mut SUPER_VALID: bool = false;
static mut PART_BASE: u64 = 0;
static mut BGDT_LBA: u64 = 0;
static mut INODE_SIZE: u32 = 128;
static mut BLOCK_SIZE: u32 = 4096;

// ── File handles ───────────────────────────────────────────────────

const MAX_OPEN_EXT4: usize = 32;
static mut OPEN_INODES: [u32; MAX_OPEN_EXT4] = [0; MAX_OPEN_EXT4];
static mut OPEN_SIZES: [u64; MAX_OPEN_EXT4] = [0; MAX_OPEN_EXT4];
static mut OPEN_COUNT: usize = 0;

fn block_size() -> u32 { unsafe { BLOCK_SIZE } }

// ── little-endian readers ──────────────────────────────────────────

fn le16(b: &[u8], o: usize) -> u16 { b[o] as u16 | ((b[o + 1] as u16) << 8) }
fn le32(b: &[u8], o: usize) -> u32 {
    b[o] as u32 | ((b[o + 1] as u32) << 8) | ((b[o + 2] as u32) << 16) | ((b[o + 3] as u32) << 24)
}

fn le32_fix(b: &[u8], o: usize) -> u32 {
    b[o] as u32 | ((b[o + 1] as u32) << 8) | ((b[o + 2] as u32) << 16) | ((b[o + 3] as u32) << 24)
}

fn rtc_timestamp() -> u32 {
    let t = crate::driver::rtc::read();
    // Simple epoch timestamp approximation
    ((t.year as u32 - 1970) * 365 + (t.month as u32) * 30 + t.day as u32) * 86400
        + (t.hour as u32) * 3600 + (t.minute as u32) * 60 + t.second as u32
}

// ── Block I/O ──────────────────────────────────────────────────────

fn read_block(blk: u32, buf: &mut [u8]) -> bool {
    unsafe {
        let lba = PART_BASE + (blk as u64) * (BLOCK_SIZE as u64 / 512);
        let count = (BLOCK_SIZE / 512) as u16;
        let phys = crate::memory::palloc();
        if phys == 0 { return false; }
        let ok = block::read_sectors(lba, count, phys as *mut u8);
        if ok {
            let copy_len = buf.len().min(BLOCK_SIZE as usize);
            core::ptr::copy_nonoverlapping(phys as *const u8, buf.as_mut_ptr(), copy_len);
        }
        crate::memory::pfree(phys);
        ok
    }
}

fn write_block(blk: u32, buf: &[u8]) -> bool {
    unsafe {
        let lba = PART_BASE + (blk as u64) * (BLOCK_SIZE as u64 / 512);
        let count = (BLOCK_SIZE / 512) as u16;
        let phys = crate::memory::palloc();
        if phys == 0 { return false; }
        let copy_len = buf.len().min(BLOCK_SIZE as usize);
        core::ptr::copy_nonoverlapping(buf.as_ptr(), phys as *mut u8, copy_len);
        let ok = block::write_sectors(lba, count, phys as *mut u8);
        crate::memory::pfree(phys);
        ok
    }
}

fn read_phys(buf: &mut [u8], lba: u64, count: u16) -> bool {
    block::read_sectors(lba, count, buf.as_mut_ptr() as *mut u8)
}

// ── Superblock ─────────────────────────────────────────────────────

fn parse_superblock(buf: &[u8]) -> Option<Ext4Super> {
    if le16(buf, 56) != EXT4_MAGIC { return None; }
    let log_block = le32_fix(buf, 24);
    if log_block > 6 { return None; }
    let block_size = 1024u32 << log_block;
    let inode_size = le16(buf, 88) as u32;
    let inode_size = if inode_size == 0 { 128 } else { inode_size };
    let blocks_per_group = le32_fix(buf, 32);
    let inodes_per_group = le32_fix(buf, 40);
    let num_block_groups = if blocks_per_group > 0 {
        let blocks = le32_fix(buf, 4);
        let fdb = le32_fix(buf, 20);
        ((blocks - fdb + blocks_per_group - 1) / blocks_per_group) as u32
    } else { 0 };
    let mut label = [0u8; 16];
    label.copy_from_slice(&buf[120..136]);
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(&buf[104..120]);
    let feature_incompat = le32_fix(buf, 96);

    Some(Ext4Super {
        inodes_count: le32_fix(buf, 0),
        blocks_count: le32_fix(buf, 4),
        r_blocks_count: le32_fix(buf, 8),
        free_blocks: le32_fix(buf, 12),
        free_inodes: le32_fix(buf, 16),
        first_data_block: le32_fix(buf, 20),
        block_size,
        blocks_per_group,
        inodes_per_group,
        inode_size,
        inode_reserved_gdt_blocks: le16(buf, 214) as u32,
        meta_bg: 0,
        feature_compat: le32_fix(buf, 92),
        feature_incompat,
        feature_ro_compat: le32_fix(buf, 100),
        blocks_per_group_bits: log_block,
        frags_per_group: le32_fix(buf, 36),
        num_block_groups,
        mtime: le32_fix(buf, 44),
        wtime: le32_fix(buf, 48),
        mount_count: le16(buf, 52),
        max_mount_count: le16(buf, 54),
        magic: le16(buf, 56),
        state: le16(buf, 58),
        errors: le16(buf, 60),
        minor_rev_level: le16(buf, 62),
        last_check: le32_fix(buf, 64),
        check_interval: le32_fix(buf, 68),
        creator_os: le32_fix(buf, 72),
        rev_level: le32_fix(buf, 76),
        def_resuid: le16(buf, 80),
        def_resgid: le16(buf, 82),
        first_ino: le32_fix(buf, 84),
        block_group_nr: le32_fix(buf, 0),
        feature_compat2: le32_fix(buf, 92),
        uuid,
        label,
        last_mounted: {
            let mut v = [0u8; 64];
            let end = 1088.min(buf.len());
            let copy_len = (end - 1024).min(64);
            v[..copy_len].copy_from_slice(&buf[1024..1024 + copy_len]);
            v
        },
        algo_bitmap: le32_fix(buf, 168),
        prealloc_blocks: buf[204],
        prealloc_dir_blocks: buf[205],
        reserved_gdt_blocks: le16(buf, 208),
        journal_uuid: {
            let mut v = [0u8; 16];
            v.copy_from_slice(&buf[136..152]);
            v
        },
        journal_inum: le32_fix(buf, 152),
        journal_dev: le32_fix(buf, 156),
        last_orphan: le32_fix(buf, 160),
        hash_seed: [
            le32_fix(buf, 200), le32_fix(buf, 196),
            le32_fix(buf, 192), le32_fix(buf, 188),
        ],
        def_hash_version: buf[206],
        jnl_backup_type: buf[207],
        desc_size: le16(buf, 260) as u32,        default_mount_opts: le32_fix(buf, 264),
        first_meta_bg: le32_fix(buf, 268),
        mkfs_time: le32_fix(buf, 272),
        jnl_blocks: {
            let mut v = [0u32; 17];
            for i in 0..17 { v[i] = le32_fix(buf, 280 + i * 4); }
            v
        },
    })
}

// ── Block Group Descriptor ─────────────────────────────────────────

fn read_bgd(bg_num: u32) -> Option<Bgd> {
    unsafe {
        let desc_size = if SUPER.desc_size >= 32 { SUPER.desc_size } else { 32 };
        // BGDT starts right after superblock (at block 1 for 4K blocks)
        let bgdt_start_block = if SUPER.block_size == 1024 { 2 } else { 1 };
        let offset_in_bgdt = (bg_num as u64) * (desc_size as u64);
        let bgdt_byte_offset = bgdt_start_block * SUPER.block_size as u64 + offset_in_bgdt;

        let mut buf = [0u8; 64]; // max desc_size = 64
        let read_len = desc_size as usize;
        // Read from the partition
        let lba = PART_BASE + bgdt_byte_offset / 512;
        let byte_off = (bgdt_byte_offset % 512) as usize;
        let mut sector = [0u8; 512];
        if !block::read_sectors(lba, 1, sector.as_mut_ptr()) { return None; }
        if byte_off + read_len <= 512 {
            buf[..read_len].copy_from_slice(&sector[byte_off..byte_off + read_len]);
        } else {
            let first = 512 - byte_off;
            buf[..first].copy_from_slice(&sector[byte_off..512]);
            let mut sector2 = [0u8; 512];
            if !block::read_sectors(lba + 1, 1, sector2.as_mut_ptr()) { return None; }
            buf[first..read_len].copy_from_slice(&sector2[..read_len - first]);
        }

        Some(Bgd {
            block_bitmap: le32_fix(&buf, 0),
            inode_bitmap: le32_fix(&buf, 4),
            inode_table: le32_fix(&buf, 8),
            free_blocks_count: le16(&buf, 12),
            free_inodes_count: le16(&buf, 14),
            used_dirs_count: le16(&buf, 16),
            pad: le16(&buf, 18),
            reserved: {
                let mut r = [0u8; 12];
                let end = read_len.min(32);
                if end > 20 { r[..end - 20].copy_from_slice(&buf[20..end]); }
                r
            },
        })
    }
}

// ── Inode read/write ───────────────────────────────────────────────

fn inode_group(inode_num: u32) -> u32 {
    unsafe { (inode_num - 1) / SUPER.inodes_per_group }
}

fn inode_index(inode_num: u32) -> u32 {
    unsafe { (inode_num - 1) % SUPER.inodes_per_group }
}

fn read_inode(inode_num: u32, inode_buf: &mut [u8]) -> bool {
    if inode_num < 1 { return false; }
    let bg = inode_group(inode_num);
    let idx = inode_index(inode_num);
    unsafe {
        let bgd = match read_bgd(bg) {
            Some(b) => b,
            None => return false,
        };
        let inode_table_block = bgd.inode_table;
        let inode_byte_offset = (inode_table_block as u64) * (BLOCK_SIZE as u64)
            + (idx as u64) * (SUPER.inode_size as u64);
        let lba = PART_BASE + inode_byte_offset / 512;
        let byte_off = (inode_byte_offset % 512) as usize;
        let inode_sz = SUPER.inode_size as usize;
        let mut sector = [0u8; 512];
        if !block::read_sectors(lba, 1, sector.as_mut_ptr()) { return false; }
        if byte_off + inode_sz <= 512 {
            inode_buf[..inode_sz].copy_from_slice(&sector[byte_off..byte_off + inode_sz]);
        } else {
            let first = 512 - byte_off;
            inode_buf[..first].copy_from_slice(&sector[byte_off..512]);
            let mut sector2 = [0u8; 512];
            if !block::read_sectors(lba + 1, 1, sector2.as_mut_ptr()) { return false; }
            inode_buf[first..inode_sz].copy_from_slice(&sector2[..inode_sz - first]);
        }
        true
    }
}

fn parse_inode(buf: &[u8]) -> Inode {
    Inode {
        mode: le16(buf, 0),
        uid: le16(buf, 2),
        size_lo: le32_fix(buf, 4),
        atime: le32_fix(buf, 8),
        ctime: le32_fix(buf, 12),
        mtime: le32_fix(buf, 16),
        dtime: le32_fix(buf, 20),
        gid: le16(buf, 24),
        links_count: le16(buf, 26),
        blocks: le32_fix(buf, 28),
        flags: le32_fix(buf, 32),
        osd1: le32_fix(buf, 36),
        block: {
            let mut b = [0u32; 15];
            for i in 0..15 { b[i] = le32_fix(buf, 40 + i * 4); }
            b
        },
        generation: le32_fix(buf, 100),
        file_acl: le32_fix(buf, 104),
        size_hi: le32_fix(buf, 108),
        extra_isize: 0,
    }
}

fn write_inode(inode_num: u32, inode: &Inode) -> bool {
    let mut buf = [0u8; 256];
    // mode
    buf[0] = (inode.mode & 0xFF) as u8;
    buf[1] = ((inode.mode >> 8) & 0xFF) as u8;
    buf[2] = (inode.uid & 0xFF) as u8;
    buf[3] = ((inode.uid >> 8) & 0xFF) as u8;
    // size_lo
    buf[4..8].copy_from_slice(&inode.size_lo.to_le_bytes());
    // timestamps
    buf[8..12].copy_from_slice(&inode.atime.to_le_bytes());
    buf[12..16].copy_from_slice(&inode.ctime.to_le_bytes());
    buf[16..20].copy_from_slice(&inode.mtime.to_le_bytes());
    buf[20..24].copy_from_slice(&inode.dtime.to_le_bytes());
    // gid, links_count
    buf[24] = (inode.gid & 0xFF) as u8;
    buf[25] = ((inode.gid >> 8) & 0xFF) as u8;
    buf[26] = (inode.links_count & 0xFF) as u8;
    buf[27] = ((inode.links_count >> 8) & 0xFF) as u8;
    // blocks
    buf[28..32].copy_from_slice(&inode.blocks.to_le_bytes());
    // flags
    buf[32..36].copy_from_slice(&inode.flags.to_le_bytes());
    // osd1
    buf[36..40].copy_from_slice(&inode.osd1.to_le_bytes());
    // block pointers
    for i in 0..15 {
        buf[40 + i * 4..44 + i * 4].copy_from_slice(&inode.block[i].to_le_bytes());
    }
    // generation, file_acl, size_hi
    buf[100..104].copy_from_slice(&inode.generation.to_le_bytes());
    buf[104..108].copy_from_slice(&inode.file_acl.to_le_bytes());
    buf[108..112].copy_from_slice(&inode.size_hi.to_le_bytes());

    // Write inode back
    let bg = inode_group(inode_num);
    let idx = inode_index(inode_num);
    unsafe {
        let bgd = match read_bgd(bg) {
            Some(b) => b,
            None => return false,
        };
        let inode_table_block = bgd.inode_table;
        let inode_byte_offset = (inode_table_block as u64) * (BLOCK_SIZE as u64)
            + (idx as u64) * (SUPER.inode_size as u64);
        let lba = PART_BASE + inode_byte_offset / 512;
        let byte_off = (inode_byte_offset % 512) as usize;
        let inode_sz = SUPER.inode_size as usize;
        // Read existing inode sector, patch it, write back
        let mut sector = [0u8; 512];
        if !block::read_sectors(lba, 1, sector.as_mut_ptr()) { return false; }
        if byte_off + inode_sz <= 512 {
            sector[byte_off..byte_off + inode_sz].copy_from_slice(&buf[..inode_sz]);
            if !block::write_sectors(lba, 1, sector.as_mut_ptr()) { return false; }
        } else {
            let first = 512 - byte_off;
            sector[byte_off..512].copy_from_slice(&buf[..first]);
            if !block::write_sectors(lba, 1, sector.as_mut_ptr()) { return false; }
            let mut sector2 = [0u8; 512];
            if !block::read_sectors(lba + 1, 1, sector2.as_mut_ptr()) { return false; }
            sector2[..inode_sz - first].copy_from_slice(&buf[first..inode_sz]);
            if !block::write_sectors(lba + 1, 1, sector2.as_mut_ptr()) { return false; }
        }
    }
    true
}

// ── Physical block from logical block (extent tree) ────────────────

fn extent_lookup(inode_buf: &[u8], logical: u32) -> Option<u32> {
    let inode = parse_inode(inode_buf);
    let size = inode.size();
    if (logical as u64) * (block_size() as u64) >= size { return None; }

    // Check if extent-based (flag 0x8000 in inode.flags)
    let use_extents = inode.flags & (1 << 14) != 0;

    if use_extents {
        // extent tree starts at block[0]
        let phys = inode.block[0];
        if phys == 0 { return None; }
        return extent_tree_lookup(phys, logical);
    }

    // Traditional block mapping
    if logical < 12 {
        let blk = inode.block[logical as usize];
        if blk == 0 { return None; }
        return Some(blk);
    }
    let mut remaining = logical - 12;
    // Indirect
    if inode.block[12] != 0 {
        if remaining < block_size() / 4 {
            let mut buf = [0u8; 4096];
            if !read_block(inode.block[12], &mut buf[..block_size() as usize]) { return None; }
            let blk = le32_fix(&buf, (remaining as usize) * 4);
            if blk != 0 { return Some(blk); }
            return None;
        }
        remaining -= block_size() / 4;
    }
    // Double indirect
    if inode.block[13] != 0 {
        let per_block = block_size() / 4;
        let per_double = per_block * per_block;
        if remaining < per_double {
            let idx1 = remaining / per_block;
            let idx2 = remaining % per_block;
            let mut buf = [0u8; 4096];
            if !read_block(inode.block[13], &mut buf) { return None; }
            let blk1 = le32_fix(&buf, (idx1 as usize) * 4);
            if blk1 == 0 { return None; }
            if !read_block(blk1, &mut buf) { return None; }
            let blk = le32_fix(&buf, (idx2 as usize) * 4);
            if blk != 0 { return Some(blk); }
            return None;
        }
        remaining -= per_double;
    }
    // Triple indirect
    if inode.block[14] != 0 {
        let per_block = block_size() / 4;
        let per_double = per_block * per_block;
        let per_triple = per_double * per_block;
        if remaining < per_triple {
            let idx1 = remaining / per_double;
            let idx2 = (remaining % per_double) / per_block;
            let idx3 = remaining % per_block;
            let mut buf = [0u8; 4096];
            if !read_block(inode.block[14], &mut buf) { return None; }
            let blk1 = le32_fix(&buf, (idx1 as usize) * 4);
            if blk1 == 0 { return None; }
            if !read_block(blk1, &mut buf) { return None; }
            let blk2 = le32_fix(&buf, (idx2 as usize) * 4);
            if blk2 == 0 { return None; }
            if !read_block(blk2, &mut buf) { return None; }
            let blk = le32_fix(&buf, (idx3 as usize) * 4);
            if blk != 0 { return Some(blk); }
            return None;
        }
    }
    None
}

fn extent_tree_lookup(tree_phys: u32, logical: u32) -> Option<u32> {
    let mut depth = 0u32;
    let mut node_phys = tree_phys;
    let mut buf = [0u8; 4096];

    // Read header
    loop {
        if !read_block(node_phys, &mut buf) { return None; }
        let hdr = ExtentHeader {
            magic: le16(&buf, 0),
            entries: le16(&buf, 2),
            max_entries: le16(&buf, 4),
            depth: le16(&buf, 6),
            generation: le32_fix(&buf, 8),
        };
        if hdr.magic != 0xF30A { return None; } // EXT4_EXTENT_MAGIC

        if hdr.depth == 0 {
            // Leaf node — search extents
            for i in 0..hdr.entries {
                let off = 12 + (i as usize) * 12;
                let ext = Extent {
                    block: le32_fix(&buf, off),
                    start_lo: le32_fix(&buf, off + 4),
                    start_hi: le16(&buf, off + 8) as u16,
                    len: le16(&buf, off + 10),
                };
                if logical >= ext.block && logical < ext.block + ext.len as u32 {
                    let phys = (ext.start_hi as u32) << 16 | ext.start_lo;
                    return Some(phys + (logical - ext.block));
                }
            }
            return None;
        }

        // Index node — find correct branch
        let mut found = false;
        for i in 0..hdr.entries {
            let off = 12 + (i as usize) * 12;
            let idx = ExtIndex {
                block: le32_fix(&buf, off),
                start_lo: le32_fix(&buf, off + 4),
                start_hi: le16(&buf, off + 8),
                uid: le16(&buf, off + 10),
            };
            if logical <= idx.block {
                let phys = (idx.start_hi as u32) << 16 | idx.start_lo;
                node_phys = phys;
                found = true;
                break;
            }
        }
        if !found {
            // Use the last index
            if hdr.entries > 0 {
                let off = 12 + ((hdr.entries - 1) as usize) * 12;
                let phys = (le16(&buf, off + 8) as u32) << 16 | le32_fix(&buf, off + 4);
                node_phys = phys;
            } else {
                return None;
            }
        }
        depth += 1;
        if depth > 5 { return None; } // prevent infinite loop
    }
}

// ── Allocate a free block ──────────────────────────────────────────

fn alloc_block() -> Option<u32> {
    unsafe {
        let num_groups = SUPER.num_block_groups;
        for bg in 0..num_groups {
            let bgd = match read_bgd(bg) {
                Some(b) => b,
                None => continue,
            };
            if bgd.free_blocks_count == 0 { continue; }

            // Read block bitmap
            let mut bmp = [0u8; 4096];
            if !read_block(bgd.block_bitmap, &mut bmp) { continue; }

            for byte_idx in 0..bmp.len() {
                if bmp[byte_idx] == 0xFF { continue; }
                for bit in 0..8 {
                    if bmp[byte_idx] & (1 << bit) == 0 {
                        let global_idx = bg * SUPER.blocks_per_group + (byte_idx as u32) * 8 + bit + SUPER.first_data_block;
                        if global_idx >= SUPER.blocks_count { continue; }
                        // Mark block as used
                        bmp[byte_idx] |= 1 << bit;
                        if write_block(bgd.block_bitmap, &bmp) {
                            // Update free count in BGDT (we'd need to write it back)
                            return Some(global_idx);
                        }
                    }
                }
            }
        }
        None
    }
}

// ── Allocate a free inode ──────────────────────────────────────────

fn alloc_inode() -> Option<u32> {
    unsafe {
        let num_groups = SUPER.num_block_groups;
        for bg in 0..num_groups {
            let bgd = match read_bgd(bg) {
                Some(b) => b,
                None => continue,
            };
            if bgd.free_inodes_count == 0 { continue; }

            // Read inode bitmap
            let mut bmp = [0u8; 4096];
            if !read_block(bgd.inode_bitmap, &mut bmp) { continue; }

            for byte_idx in 0..bmp.len() {
                if bmp[byte_idx] == 0xFF { continue; }
                for bit in 0..8 {
                    if bmp[byte_idx] & (1 << bit) == 0 {
                        let global_idx = bg * SUPER.inodes_per_group + (byte_idx as u32) * 8 + bit + 1;
                        if global_idx > SUPER.inodes_count { continue; }
                        bmp[byte_idx] |= 1 << bit;
                        if write_block(bgd.inode_bitmap, &bmp) {
                            return Some(global_idx);
                        }
                    }
                }
            }
        }
        None
    }
}

// ── Directory operations ───────────────────────────────────────────

fn read_dir_entries(inode_num: u32, entries: &mut [crate::vfs::DirEntry]) -> Option<usize> {
    let mut inode_buf = [0u8; 256];
    if !read_inode(inode_num, &mut inode_buf) { return None; }
    let inode = parse_inode(&inode_buf);
    if !inode.is_dir() { return None; }

    let size = inode.size() as usize;
    let mut count = 0usize;
    let mut offset = 0usize;

    while offset < size && count < entries.len() {
        // Read logical block
        let log_block = (offset / block_size() as usize) as u32;
        let block_offset = offset % block_size() as usize;

        let phys = match extent_lookup(&inode_buf, log_block) {
            Some(p) => p,
            None => { offset += block_size() as usize; continue; }
        };

        let mut block_buf = [0u8; 4096];
        if !read_block(phys, &mut block_buf) { break; }

        let mut pos = block_offset;
        while pos + EXT4_DIR_ENTRY_BASE <= block_size() as usize {
            let rec_len = le16(&block_buf, pos + 4) as usize;
            if rec_len < EXT4_DIR_ENTRY_BASE || rec_len > block_size() as usize { break; }

            let inode_num = le32_fix(&block_buf, pos);
            if inode_num != 0 {
                let name_len = block_buf[pos + 6] as usize;
                let file_type = block_buf[pos + 7];
                let name = &block_buf[pos + EXT4_DIR_ENTRY_BASE..pos + EXT4_DIR_ENTRY_BASE + name_len.min(MAX_NAME - 1)];

                let mut entry = crate::vfs::DirEntry {
                    name: [0; MAX_NAME],
                    is_dir: file_type == EXT4_DIR_ENTRY_TYPE_DIR,
                    size: 0,
                };
                entry.name[..name.len()].copy_from_slice(name);

                // Get inode size for files
                if !entry.is_dir && inode_num > 0 {
                    let mut child_buf = [0u8; 256];
                    if read_inode(inode_num, &mut child_buf) {
                        let child = parse_inode(&child_buf);
                        entry.size = child.size_lo;
                    }
                }

                entries[count] = entry;
                count += 1;
            }

            pos += rec_len;
            offset += rec_len;
        }
        if pos + EXT4_DIR_ENTRY_BASE > block_size() as usize {
            offset = ((offset / block_size() as usize) + 1) * block_size() as usize;
        }
    }
    Some(count)
}

fn lookup_path(root_inode: u32, path: &[u8]) -> Option<u32> {
    if path.is_empty() || path == b"/" { return Some(root_inode); }

    let mut current_inode = root_inode;
    let mut pos = 0;
    if path[0] == b'/' { pos = 1; }

    while pos < path.len() {
        // Find next component
        let start = pos;
        while pos < path.len() && path[pos] != b'/' { pos += 1; }
        let component = &path[start..pos];
        if component.is_empty() { pos += 1; continue; }

        // Search directory entries of current_inode
        let mut dir_buf = [0u8; 256];
        if !read_inode(current_inode, &mut dir_buf) { return None; }
        let dir_inode = parse_inode(&dir_buf);
        if !dir_inode.is_dir() { return None; }

        let dir_size = dir_inode.size() as usize;
        let mut dir_offset = 0usize;
        let mut found = false;

        while dir_offset < dir_size {
            let log_block = (dir_offset / block_size() as usize) as u32;
            let block_off = dir_offset % block_size() as usize;

            let phys = match extent_lookup(&dir_buf, log_block) {
                Some(p) => p,
                None => { dir_offset += block_size() as usize; continue; }
            };

            let mut block = [0u8; 4096];
            if !read_block(phys, &mut block) { break; }

            let mut bpos = block_off;
            while bpos + EXT4_DIR_ENTRY_BASE <= block_size() as usize {
                let rec_len = le16(&block, bpos + 4) as usize;
                if rec_len < EXT4_DIR_ENTRY_BASE { break; }

                let ino = le32_fix(&block, bpos);
                let name_len = block[bpos + 6] as usize;
                if ino != 0 && name_len == component.len() {
                    let name = &block[bpos + EXT4_DIR_ENTRY_BASE..bpos + EXT4_DIR_ENTRY_BASE + name_len];
                    if name == component {
                        current_inode = ino;
                        found = true;
                        break;
                    }
                }
                bpos += rec_len;
            }
            if found { break; }
            dir_offset = ((dir_offset / block_size() as usize) + 1) * block_size() as usize;
        }
        if !found { return None; }
        pos += 1;
    }
    Some(current_inode)
}

fn find_parent_and_name(path: &[u8]) -> Option<(u32, [u8; MAX_NAME], usize)> {
    let last_slash = path.iter().rposition(|&b| b == b'/')?;
    let parent_path = if last_slash == 0 { b"/" } else { &path[..last_slash] };
    let name = &path[last_slash + 1..];

    let root = unsafe { SUPER.first_data_block + 1 }; // root inode = 2
    let parent_inode = lookup_path(root, parent_path)?;
    let mut name_buf = [0u8; MAX_NAME];
    let len = name.len().min(MAX_NAME - 1);
    name_buf[..len].copy_from_slice(&name[..len]);
    Some((parent_inode, name_buf, len))
}

fn add_dir_entry(parent_inode: u32, child_inode: u32, name: &[u8], file_type: u8) -> bool {
    let mut inode_buf = [0u8; 256];
    if !read_inode(parent_inode, &mut inode_buf) { return false; }
    let mut inode = parse_inode(&inode_buf);

    let dir_size = inode.size() as usize;
    let name_len = name.len().min(255);
    let entry_len = EXT4_DIR_ENTRY_BASE + name_len;
    let entry_len = (entry_len + 3) & !3; // align to 4 bytes

    // Try to find free space in existing blocks
    let mut offset = 0usize;
    while offset < dir_size {
        let log_block = (offset / block_size() as usize) as u32;
        let block_off = offset % block_size() as usize;

        let phys = match extent_lookup(&inode_buf, log_block) {
            Some(p) => p,
            None => { offset += block_size() as usize; continue; }
        };

        let mut block = [0u8; 4096];
        if !read_block(phys, &mut block) { break; }

        let mut bpos = block_off;
        while bpos + EXT4_DIR_ENTRY_BASE <= block_size() as usize {
            let rec_len = le16(&block, bpos + 4) as usize;
            if rec_len < EXT4_DIR_ENTRY_BASE { break; }

            let ino = le32_fix(&block, bpos);
            let old_name_len = block[bpos + 6] as usize;
            let old_entry_len = (EXT4_DIR_ENTRY_BASE + old_name_len + 3) & !3;

            if ino == 0 && rec_len >= entry_len {
                // Free entry — write here
                block[bpos..bpos + 4].copy_from_slice(&child_inode.to_le_bytes());
                block[bpos + 4..bpos + 6].copy_from_slice(&(entry_len as u16).to_le_bytes());
                block[bpos + 6] = name_len as u8;
                block[bpos + 7] = file_type;
                block[bpos + EXT4_DIR_ENTRY_BASE..bpos + EXT4_DIR_ENTRY_BASE + name_len]
                    .copy_from_slice(&name[..name_len]);
                return write_block(phys, &block);
            }

            // If entry has extra space, split it
            if ino != 0 && rec_len > old_entry_len + entry_len {
                let new_rec_len = rec_len - old_entry_len;
                // Shrink existing entry
                block[bpos + 4..bpos + 6].copy_from_slice(&(old_entry_len as u16).to_le_bytes());
                // Write new entry after
                let new_pos = bpos + old_entry_len;
                block[new_pos..new_pos + 4].copy_from_slice(&child_inode.to_le_bytes());
                block[new_pos + 4..new_pos + 6].copy_from_slice(&(new_rec_len as u16).to_le_bytes());
                block[new_pos + 6] = name_len as u8;
                block[new_pos + 7] = file_type;
                block[new_pos + EXT4_DIR_ENTRY_BASE..new_pos + EXT4_DIR_ENTRY_BASE + name_len]
                    .copy_from_slice(&name[..name_len]);
                return write_block(phys, &block);
            }

            bpos += rec_len;
        }
        offset = ((offset / block_size() as usize) + 1) * block_size() as usize;
    }

    // No free space found — append a new block
    let new_blk = match alloc_block() {
        Some(b) => b,
        None => return false,
    };

    let mut block = [0u8; 4096];
    block[0..4].copy_from_slice(&child_inode.to_le_bytes());
    block[4..6].copy_from_slice(&(block_size() as u16).to_le_bytes());
    block[6] = name_len as u8;
    block[7] = file_type;
    block[EXT4_DIR_ENTRY_BASE..EXT4_DIR_ENTRY_BASE + name_len]
        .copy_from_slice(&name[..name_len]);

    if !write_block(new_blk, &block) { return false; }

    // Add block to inode (simplified: direct block append)
    // Find first empty direct block
    for i in 0..12 {
        if inode.block[i] == 0 {
            inode.block[i] = new_blk;
            inode.blocks += block_size() / 512;
            inode.size_lo += block_size();
            return write_inode(parent_inode, &inode);
        }
    }
    false
}

fn remove_dir_entry(parent_inode: u32, name: &[u8]) -> bool {
    let mut inode_buf = [0u8; 256];
    if !read_inode(parent_inode, &mut inode_buf) { return false; }
    let inode = parse_inode(&inode_buf);
    let dir_size = inode.size() as usize;
    let mut offset = 0usize;

    while offset < dir_size {
        let log_block = (offset / block_size() as usize) as u32;
        let block_off = offset % block_size() as usize;

        let phys = match extent_lookup(&inode_buf, log_block) {
            Some(p) => p,
            None => { offset += block_size() as usize; continue; }
        };

        let mut block = [0u8; 4096];
        if !read_block(phys, &mut block) { break; }

        let mut prev_pos: Option<usize> = None;
        let mut bpos = block_off;
        while bpos + EXT4_DIR_ENTRY_BASE <= block_size() as usize {
            let rec_len = le16(&block, bpos + 4) as usize;
            if rec_len < EXT4_DIR_ENTRY_BASE { break; }

            let ino = le32_fix(&block, bpos);
            let name_len = block[bpos + 6] as usize;
            if ino != 0 && name_len == name.len() {
                let entry_name = &block[bpos + EXT4_DIR_ENTRY_BASE..bpos + EXT4_DIR_ENTRY_BASE + name_len];
                if entry_name == name {
                    // Zero out this entry
                    let clear_len = if let Some(pp) = prev_pos {
                        // Merge with previous free entry
                        let prev_rec_len = le16(&block, pp + 4) as usize;
                        block[pp + 4..pp + 6].copy_from_slice(&((prev_rec_len + rec_len) as u16).to_le_bytes());
                        rec_len
                    } else {
                        // Make this a free entry (inode=0)
                        block[bpos..bpos + 4].copy_from_slice(&0u32.to_le_bytes());
                        rec_len
                    };
                    for i in bpos..bpos + clear_len {
                        block[i] = 0;
                    }
                    return write_block(phys, &block);
                }
            }
            if ino == 0 {
                prev_pos = Some(bpos);
            } else {
                prev_pos = None;
            }
            bpos += rec_len;
        }
        offset = ((offset / block_size() as usize) + 1) * block_size() as usize;
    }
    false
}

// ── VFS interface ──────────────────────────────────────────────────

fn ext4_open(path: &[u8], _flags: u64) -> Option<usize> {
    unsafe {
        if !SUPER_VALID { return None; }
        let root = SUPER.first_data_block + 1;
        let inode_num = lookup_path(root, path)?;
        let mut inode_buf = [0u8; 256];
        if !read_inode(inode_num, &mut inode_buf) { return None; }
        let inode = parse_inode(&inode_buf);
        if inode.is_dir() && _flags & 0x3 != 0 { return None; } // dirs not writable

        let h = OPEN_COUNT;
        if h >= MAX_OPEN_EXT4 { return None; }
        OPEN_INODES[h] = inode_num;
        OPEN_SIZES[h] = inode.size();
        OPEN_COUNT = h + 1;
        Some(h + 0x10000) // handle offset to distinguish from FAT handles
    }
}

fn ext4_close(handle: usize) {
    unsafe {
        let h = handle.wrapping_sub(0x10000);
        if h < OPEN_COUNT {
            // Shift remaining
            for i in h..OPEN_COUNT - 1 {
                OPEN_INODES[i] = OPEN_INODES[i + 1];
                OPEN_SIZES[i] = OPEN_SIZES[i + 1];
            }
            OPEN_COUNT -= 1;
        }
    }
}

fn ext4_read(handle: usize, buf: &mut [u8], offset: u64) -> Option<usize> {
    unsafe {
        if !SUPER_VALID { return None; }
        let h = handle.wrapping_sub(0x10000);
        if h >= OPEN_COUNT { return None; }
        let inode_num = OPEN_INODES[h];
        let file_size = OPEN_SIZES[h];

        let mut inode_buf = [0u8; 256];
        if !read_inode(inode_num, &mut inode_buf) { return None; }

        let mut total = 0usize;
        let mut file_off = offset;

        while total < buf.len() && file_off < file_size {
            let log_block = (file_off / block_size() as u64) as u32;
            let block_off = (file_off % block_size() as u64) as usize;
            let to_read = (buf.len() - total).min(block_size() as usize - block_off);

            let phys = match extent_lookup(&inode_buf, log_block) {
                Some(p) => p,
                None => break,
            };

            let mut block = [0u8; 4096];
            if !read_block(phys, &mut block) { break; }

            let avail = to_read.min((file_size - file_off) as usize);
            buf[total..total + avail].copy_from_slice(&block[block_off..block_off + avail]);
            total += avail;
            file_off += avail as u64;
        }
        Some(total)
    }
}

fn ext4_write(handle: usize, buf: &[u8], offset: u64) -> Option<usize> {
    unsafe {
        if !SUPER_VALID { return None; }
        let h = handle.wrapping_sub(0x10000);
        if h >= OPEN_COUNT { return None; }
        let inode_num = OPEN_INODES[h];

        let mut inode_buf = [0u8; 256];
        if !read_inode(inode_num, &mut inode_buf) { return None; }
        let mut inode = parse_inode(&inode_buf);

        let mut total = 0usize;
        let mut file_off = offset;

        while total < buf.len() {
            let log_block = (file_off / block_size() as u64) as u32;
            let block_off = (file_off % block_size() as u64) as usize;
            let to_write = (buf.len() - total).min(block_size() as usize - block_off);

            let phys = match extent_lookup(&inode_buf, log_block) {
                Some(p) => p,
                None => {
                    // Allocate new block
                    match alloc_block() {
                        Some(b) => {
                            // Map it into the inode (direct blocks only for simplicity)
                            if log_block < 12 {
                                inode.block[log_block as usize] = b;
                            }
                            inode.blocks += BLOCK_SIZE / 512;
                            b
                        }
                        None => break,
                    }
                }
            };

            let mut block = [0u8; 4096];
            if !read_block(phys, &mut block) { break; }

            block[block_off..block_off + to_write].copy_from_slice(&buf[total..total + to_write]);
            if !write_block(phys, &block) { break; }

            total += to_write;
            file_off += to_write as u64;
        }

        if file_off > inode.size() {
            inode.size_lo = file_off as u32;
            inode.size_hi = (file_off >> 32) as u32;
        }
        write_inode(inode_num, &inode);
        OPEN_SIZES[h] = inode.size();
        Some(total)
    }
}

fn ext4_stat(path: &[u8]) -> Option<crate::vfs::StatInfo> {
    unsafe {
        if !SUPER_VALID { return None; }
        let root = SUPER.first_data_block + 1;
        let inode_num = lookup_path(root, path)?;
        let mut inode_buf = [0u8; 256];
        if !read_inode(inode_num, &mut inode_buf) { return None; }
        let inode = parse_inode(&inode_buf);
        Some(crate::vfs::StatInfo {
            size: inode.size_lo,
            is_dir: inode.is_dir(),
            cluster: inode_num,
        })
    }
}

fn ext4_readdir(path: &[u8], entries: &mut [crate::vfs::DirEntry]) -> Option<usize> {
    unsafe {
        if !SUPER_VALID { return None; }
        let root = SUPER.first_data_block + 1;
        let inode_num = lookup_path(root, path)?;
        read_dir_entries(inode_num, entries)
    }
}

fn ext4_mkdir(path: &[u8]) -> bool {
    unsafe {
        if !SUPER_VALID { return false; }
        let (parent_ino, name, name_len) = match find_parent_and_name(path) {
            Some(v) => v,
            None => return false,
        };

        // Check if already exists
        let mut parent_buf = [0u8; 256];
        if !read_inode(parent_ino, &mut parent_buf) { return false; }
        if lookup_path(parent_ino, &name[..name_len]).is_some() { return false; }

        // Allocate inode
        let new_ino = match alloc_inode() {
            Some(i) => i,
            None => return false,
        };

        // Initialize inode as directory
        let now = rtc_timestamp();
        let mut inode = Inode {
            mode: S_IFDIR | 0o755,
            uid: 0,
            size_lo: block_size(),
            atime: now,
            ctime: now,
            mtime: now,
            dtime: 0,
            gid: 0,
            links_count: 2, // . and ..
            blocks: block_size() / 512,
            flags: 0,
            osd1: 0,
            block: [0; 15],
            generation: 0,
            file_acl: 0,
            size_hi: 0,
            extra_isize: 0,
        };

        // Allocate a block for the directory content
        if let Some(blk) = alloc_block() {
            inode.block[0] = blk;

            // Create . and .. entries
            let mut dir_block = [0u8; 4096];
            // . entry
            dir_block[0..4].copy_from_slice(&new_ino.to_le_bytes());
            let dot_rec_len = ((EXT4_DIR_ENTRY_BASE + 1 + 3) & !3) as u16;
            dir_block[4..6].copy_from_slice(&dot_rec_len.to_le_bytes());
            dir_block[6] = 1; // name_len
            dir_block[7] = EXT4_DIR_ENTRY_TYPE_DIR;
            dir_block[EXT4_DIR_ENTRY_BASE] = b'.';
            // .. entry
            let dd_off = dot_rec_len as usize;
            dir_block[dd_off..dd_off + 4].copy_from_slice(&parent_ino.to_le_bytes());
            let dd_rec_len = (block_size() as u16) - dot_rec_len;
            dir_block[dd_off + 4..dd_off + 6].copy_from_slice(&dd_rec_len.to_le_bytes());
            dir_block[dd_off + 6] = 2; // name_len
            dir_block[dd_off + 7] = EXT4_DIR_ENTRY_TYPE_DIR;
            dir_block[dd_off + EXT4_DIR_ENTRY_BASE..dd_off + EXT4_DIR_ENTRY_BASE + 2].copy_from_slice(b"..");

            write_block(blk, &dir_block);
        }

        write_inode(new_ino, &inode);
        add_dir_entry(parent_ino, new_ino, &name[..name_len], EXT4_DIR_ENTRY_TYPE_DIR)
    }
}

fn ext4_rmdir(path: &[u8]) -> bool {
    unsafe {
        if !SUPER_VALID { return false; }
        let root = SUPER.first_data_block + 1;
        let inode_num = match lookup_path(root, path) {
            Some(i) => i,
            None => return false,
        };
        let mut inode_buf = [0u8; 256];
        if !read_inode(inode_num, &mut inode_buf) { return false; }
        let inode = parse_inode(&inode_buf);
        if !inode.is_dir() { return false; }
        if inode.size() > 0 { return false; } // not empty

        let (parent_ino, name, name_len) = match find_parent_and_name(path) {
            Some(v) => v,
            None => return false,
        };

        // Free block
        if inode.block[0] != 0 {
            // Should free the block in bitmap, simplified
        }
        // Free inode bitmap entry
        // (simplified: just mark inode as deleted)
        let mut del_inode = inode;
        del_inode.dtime = rtc_timestamp();
        del_inode.links_count = 0;
        write_inode(inode_num, &del_inode);

        remove_dir_entry(parent_ino, &name[..name_len])
    }
}

fn ext4_unlink(path: &[u8]) -> bool {
    unsafe {
        if !SUPER_VALID { return false; }
        let root = SUPER.first_data_block + 1;
        let inode_num = match lookup_path(root, path) {
            Some(i) => i,
            None => return false,
        };
        let mut inode_buf = [0u8; 256];
        if !read_inode(inode_num, &mut inode_buf) { return false; }
        let inode = parse_inode(&inode_buf);
        if inode.is_dir() { return false; }

        let (parent_ino, name, name_len) = match find_parent_and_name(path) {
            Some(v) => v,
            None => return false,
        };

        // Mark inode as deleted
        let mut del_inode = inode;
        del_inode.dtime = rtc_timestamp();
        del_inode.links_count = 0;
        write_inode(inode_num, &del_inode);

        remove_dir_entry(parent_ino, &name[..name_len])
    }
}

// ── Init ───────────────────────────────────────────────────────────

pub fn init() {
    match detect() {
        Some(s) => {
            uart::write_str("[FS] ext4 detected: blocks=");
            dec(s.blocks_count as u64);
            uart::write_str(" bsize=");
            dec(s.block_size as u64);
            uart::write_str(" groups=");
            dec(s.num_block_groups as u64);
            uart::write_str(" inodes=");
            dec(s.inodes_per_group as u64);
            uart::write_str(" label=");
            for &c in s.label.iter() {
                if c == 0 { break; }
                uart::putchar(c);
            }
            uart::write_str("\r\n");

            unsafe {
                BLOCK_SIZE = s.block_size;
                INODE_SIZE = s.inode_size;
                SUPER = s;
                SUPER_VALID = true;
            }

            // Register with VFS
            let idx = unsafe { crate::vfs::register_driver(&EXT4_OPS) };
            // Mount at /home if FAT is at /
            unsafe { crate::vfs::mount(b"/home", idx); }
            uart::write_str("[FS] ext4 mounted at /home\r\n");
        }
        None => {
            uart::write_str("[FS] ext4: no superblock found\r\n");
        }
    }
    if self_test() != 0 {
        uart::write_str("[FS] ext4 self-test FAILED\r\n");
    }
}

fn detect() -> Option<Ext4Super> {
    let base: u64 = match crate::block::active() {
        crate::block::Backend::Nvme => crate::driver::nvme::part_lba(),
        crate::block::Backend::Ahci => crate::driver::ahci::part_lba(),
        _ => return None,
    };
    unsafe { PART_BASE = base; }
    let mut sb = [0u8; 1024];
    let lba = base + 2; // superblock at byte 1024 = sector 2
    if !block::read_sectors(lba, 2, sb.as_mut_ptr()) { return None; }
    parse_superblock(&sb)
}

fn dec(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut b = [0u8; 20]; let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putchar(b[i]); }
}

static EXT4_OPS: crate::vfs::FsOps = crate::vfs::FsOps {
    open: ext4_open,
    close: ext4_close,
    read: ext4_read,
    write: ext4_write,
    stat: ext4_stat,
    readdir: ext4_readdir,
    mkdir: ext4_mkdir,
    rmdir: ext4_rmdir,
    unlink: ext4_unlink,
};

pub fn self_test() -> u32 {
    // KAT on synthetic superblock
    let mut sb = [0u8; 1024];
    sb[0..4].copy_from_slice(&[0x00, 0x04, 0x00, 0x00]); // inodes=1024
    sb[4..8].copy_from_slice(&[0x00, 0x80, 0x00, 0x00]); // blocks=32768
    sb[20..24].copy_from_slice(&[0x01, 0x00, 0x00, 0x00]); // first_data=1
    sb[24..28].copy_from_slice(&[0x02, 0x00, 0x00, 0x00]); // log_block=2 (4K)
    sb[32..36].copy_from_slice(&[0x00, 0x80, 0x00, 0x00]); // blocks_per_group=32768
    sb[40..44].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]); // inodes_per_group=256
    sb[56..58].copy_from_slice(&[0x53, 0xEF]); // magic
    sb[120..136].copy_from_slice(b"DBSOS-TEST      ");
    sb[88..90].copy_from_slice(&[0x80, 0x00]); // inode_size=128

    match parse_superblock(&sb) {
        Some(s) => {
            if s.inodes_count != 1024 { return 1; }
            if s.blocks_count != 32768 { return 2; }
            if s.block_size != 4096 { return 4; }
            if s.blocks_per_group != 32768 { return 8; }
            if s.inodes_per_group != 256 { return 16; }
            0
        }
        None => 32,
    }
}
