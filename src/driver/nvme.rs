use super::uart;
use core::ptr::{read_volatile, write_volatile};

const NVME_CLASS: u8 = 0x01;
const NVME_SUBCLASS: u8 = 0x08;

const REG_CAP: u64 = 0x0000;
const REG_CC: u64 = 0x0014;
const REG_CSTS: u64 = 0x001C;
const REG_AQA: u64 = 0x0024;
const REG_ASQ: u64 = 0x0028;
const REG_ACQ: u64 = 0x0030;

const CC_EN: u32 = 0x0001;
const CC_IOSQES: u32 = 6 << 16;
const CC_IOCQES: u32 = 4 << 20;
const CSTS_RDY: u32 = 0x0001;

const ADMIN_CREATE_CQ: u8 = 0x05;
const ADMIN_CREATE_SQ: u8 = 0x01;
const ADMIN_IDENTIFY: u8 = 0x06;
const IO_READ: u8 = 0x02;
const IO_WRITE: u8 = 0x01;
const PSDT_PRP: u8 = 0x00;

const QD: u32 = 64;
const MAX_PAGES: usize = 256;

static mut MMIO: u64 = 0;
pub static mut INIT: bool = false;
pub static mut FS_INIT: bool = false;
static mut NSID: u32 = 0;
pub static mut NS_LBA_SIZE: u32 = 512;
pub static mut NS_LBA_COUNT: u64 = 0;
static mut DSTRD: u32 = 0;
static mut ADM_SQ: u64 = 0;
static mut ADM_CQ: u64 = 0;
static mut ADM_SQT: u32 = 0;
static mut ADM_CQH: u32 = 0;
static mut ADM_PHASE: u32 = 1;
static mut IO_SQ: u64 = 0;
static mut IO_CQ: u64 = 0;
static mut IO_SQT: u32 = 0;
static mut IO_CQH: u32 = 0;
static mut IO_PHASE: u32 = 1;
static mut IO_QID: u32 = 0;
static mut CID: u16 = 0;

// FAT partition info
pub static mut PART_LBA: u64 = 0;
pub static mut BPS: u16 = 512;
pub static mut SPC: u8 = 1;
pub static mut FAT_SZ: u64 = 0;
pub static mut ROOT_ENT: u16 = 0;
pub static mut RESERVED: u16 = 1;
pub static mut FATS: u8 = 2;
pub static mut IS_FAT32: bool = false;
pub static mut ROOT_CLUSTER: u64 = 0;

fn mmio32(off: u64) -> *mut u32 { (unsafe { MMIO } + off) as *mut u32 }
fn mmio64(off: u64) -> *mut u64 { (unsafe { MMIO } + off) as *mut u64 }
fn reg32(off: u64) -> u32 { unsafe { read_volatile(mmio32(off)) } }
fn wr32(off: u64, v: u32) { unsafe { write_volatile(mmio32(off), v) } }
fn reg64(off: u64) -> u64 { unsafe { read_volatile(mmio64(off)) } }
fn wr64(off: u64, v: u64) { unsafe { write_volatile(mmio64(off), v) } }

fn sq_db(qid: u32) -> u64 { 0x1000 + ((2 * qid) as u64) * (1u64 << (2 + unsafe { DSTRD })) }
fn cq_db(qid: u32) -> u64 { 0x1000 + ((2 * qid + 1) as u64) * (1u64 << (2 + unsafe { DSTRD })) }

fn spin_until(mut f: impl FnMut() -> bool, max_us: u64) -> bool {
    use crate::timer;
    let start = timer::ticks();
    while !f() {
        if timer::ticks().wrapping_sub(start) > max_us * 10 {
            return false;
        }
        core::hint::spin_loop();
    }
    true
}

fn hex32(v: u32) {
    let h = b"0123456789ABCDEF";
    for i in (0..8).rev() { uart::putchar(h[((v >> (i * 4)) & 0xF) as usize]); }
}

fn find_nvme() -> bool {
    for dev in 0..32 {
        for func in 0..8 {
            let v = super::pci::read16(0, dev as u8, func as u8, 0);
            if v == 0xFFFF { if func == 0 { break; } continue; }
            let r = super::pci::read32(0, dev as u8, func as u8, 0x08);
            if (r >> 24) as u8 == NVME_CLASS && ((r >> 16) & 0xFF) as u8 == NVME_SUBCLASS {
                let bar0 = super::pci::read32(0, dev as u8, func as u8, 0x10);
                uart::write_str("[NVMe] raw bar0="); hex32(bar0);
                if bar0 & 1 != 0 { uart::write_str(" I/O\r\n"); continue; }
                let bt = (bar0 >> 1) & 0x3;
                let base = (bar0 & 0xFFFFFFF0) as u64;
                uart::write_str(" bt="); uart_dec(bt as u64);
                unsafe {
                    if bt == 2 {
                        let hi = super::pci::read32(0, dev as u8, func as u8, 0x14);
                        uart::write_str(" bar0_hi="); hex32(hi);
                        MMIO = base | ((hi as u64) << 32);
                    } else {
                        MMIO = base;
                    }
                }
                uart::write_str(" mmio="); hex32(unsafe { MMIO as u32 }); hex32((unsafe { MMIO >> 32 }) as u32);
                let cmd = super::pci::read16(0, dev as u8, func as u8, 0x04);
                uart::write_str(" cmd="); hex32(cmd as u32);
                super::pci::write32(0, dev as u8, func as u8, 0x04, (cmd | 0x6) as u32);
                let cmd2 = super::pci::read16(0, dev as u8, func as u8, 0x04);
                uart::write_str("->"); hex32(cmd2 as u32);
                uart::write_str("\r\n");
                return true;
            }
        }
    }
    false
}

fn admin_send(cmd: &[u32; 16]) -> bool {
    let sq = unsafe { ADM_SQ };
    let tail = unsafe { ADM_SQT };
    if sq == 0 { return false; }
    unsafe {
        let slot = (sq + tail as u64 * 64) as *mut u32;
        for i in 0..16 { write_volatile(slot.add(i), cmd[i]); }
    }
    unsafe { core::arch::asm!("wbinvd"); }
    let new_tail = (tail + 1) % QD;
    unsafe { ADM_SQT = new_tail; }

    wr32(sq_db(0), new_tail);
    true
}

fn admin_wait(cid: u16) -> bool {
    let cq = unsafe { ADM_CQ };
    let head = unsafe { ADM_CQH };
    let phase = unsafe { ADM_PHASE };
    if cq == 0 { return false; }

    for _ in 0..1_000_000 {
        let entry = unsafe { read_volatile((cq + head as u64 * 16 + 12) as *const u32) };
        if (entry >> 16) & 1 != 0 {
            let cid2 = (entry & 0xFFFF) as u16;
            let new_head = (head + 1) % QD;
            unsafe {
                ADM_CQH = new_head;
                if new_head == 0 { ADM_PHASE = if phase == 0 { 1 } else { 0 }; }
            }
            wr32(cq_db(0), new_head);
            // Status field: bits 31:16 of DW3, phase at bit 0, status at bits 15:1
            // For success: (status << 1) | phase = 0 | phase → status field = 0 or 1
            return (entry >> 17) == 0 && cid2 == cid;
        }
        core::hint::spin_loop();
    }
    false
}

fn submit_admin(opcode: u8, nsid: u32, prp1: u64, prp2: u64, cdw10: u32, cdw11: u32) -> bool {
    let cid = unsafe { let c = CID; CID = CID.wrapping_add(1); c };
    let mut cmd = [0u32; 16];
    // NvmeCmd layout: opcode/flags(4B) | nsid(4B) | res1(8B) | mptr(8B) | dptr.prp1(8B) | dptr.prp2(8B) | cdw10(4B) | cdw11(4B) | ...
    // DW indices:    0               1            2-3          4-5         6-7               8-9              10          11
    cmd[0] = opcode as u32 | ((PSDT_PRP as u32) << 8) | ((cid as u32) << 16);
    cmd[1] = nsid;
    cmd[6] = prp1 as u32;
    cmd[7] = (prp1 >> 32) as u32;
    cmd[8] = prp2 as u32;
    cmd[9] = (prp2 >> 32) as u32;
    cmd[10] = cdw10;
    cmd[11] = cdw11;
    uart::write_str("[NVMe] admin cmd op="); hex32(opcode as u32);
    uart::write_str(" cid="); hex32(cid as u32);
    uart::write_str(" nsid="); hex32(nsid);
    uart::write_str(" prp1="); hex32(prp1 as u32); hex32((prp1 >> 32) as u32);
    uart::write_str(" cdw10="); hex32(cdw10);
    uart::write_str("\r\n");
    if !admin_send(&cmd) {
        uart::write_str("[NVMe] admin_send failed\r\n");
        return false;
    }
    if !admin_wait(cid) {
        uart::write_str("[NVMe] admin_wait timeout/error\r\n");
        // Dump CQ
        let cq = unsafe { ADM_CQ };
        let head = unsafe { ADM_CQH };
        if cq != 0 {
            for i in 0..4 {
                let e = unsafe { read_volatile((cq + head as u64 * 16 + i as u64 * 4) as *const u32) };
                hex32(e); uart::write_str(" ");
            }
            uart::write_str("\r\n");
        }
        return false;
    }
    true
}

fn io_send(cmd: &[u32; 16]) -> bool {
    let sq = unsafe { IO_SQ };
    let tail = unsafe { IO_SQT };
    if sq == 0 { return false; }
    unsafe {
        let slot = (sq + tail as u64 * 64) as *mut u32;
        for i in 0..16 { write_volatile(slot.add(i), cmd[i]); }
    }
    unsafe { core::arch::asm!("wbinvd"); }
    let new_tail = (tail + 1) % QD;
    unsafe { IO_SQT = new_tail; }
    wr32(sq_db(unsafe { IO_QID }), new_tail);
    true
}

fn io_wait(cid: u16) -> bool {
    let cq = unsafe { IO_CQ };
    let head = unsafe { IO_CQH };
    let phase = unsafe { IO_PHASE };
    if cq == 0 { return false; }
    for _ in 0..1_000_000 {
        let entry = unsafe { read_volatile((cq + head as u64 * 16 + 12) as *const u32) };
        if (entry >> 16) & 1 == phase {
            let status = entry >> 17;
            let cid2 = (entry & 0xFFFF) as u16;
            let new_head = (head + 1) % QD;
            unsafe {
                IO_CQH = new_head;
                if new_head == 0 { IO_PHASE = if phase == 0 { 1 } else { 0 }; }
            }
            wr32(cq_db(unsafe { IO_QID }), new_head);
            return status == 0 && cid2 == cid;
        }
        core::hint::spin_loop();
    }
    false
}

fn next_cid() -> u16 {
    unsafe { let c = CID; CID = CID.wrapping_add(1); c }
}

fn nvme_rw(opcode: u8, lba: u64, count: u16, buf: *mut u8) -> bool {
    if !unsafe { INIT } || count == 0 { return false; }
    let nsid = unsafe { NSID };
    let total_bytes = (count as usize) * 512;
    let pages = (total_bytes + 4095) / 4096;
    if pages > MAX_PAGES { return false; }

    let mut addrs = [0u64; MAX_PAGES];
    let mut i = 0;
    while i < pages {
        let p = crate::memory::palloc();
        if p == 0 {
            for j in 0..i { crate::memory::pfree(addrs[j]); }
            return false;
        }
        addrs[i] = p;
        i += 1;
    }

    // For write: copy data from buf to DMA pages first
    if opcode == IO_WRITE {
        let mut src = buf as usize;
        let mut rem = total_bytes;
        for i in 0..pages {
            let chunk = if rem > 4096 { 4096 } else { rem };
            unsafe { core::ptr::copy_nonoverlapping(src as *const u8, addrs[i] as *mut u8, chunk); }
            src += chunk;
            rem -= chunk;
        }
    }

    // Build PRP
    let prp1 = addrs[0];
    let prp2: u64;
    let mut prp_list = 0u64;

    if pages == 1 {
        prp2 = 0;
    } else if pages == 2 {
        prp2 = addrs[1];
    } else {
        prp_list = crate::memory::palloc();
        if prp_list == 0 {
            for i in 0..pages { crate::memory::pfree(addrs[i]); }
            return false;
        }
        let list = prp_list as *mut u64;
        for j in 0..(pages - 1) {
            unsafe { write_volatile(list.add(j), addrs[j + 1]); }
        }
        prp2 = prp_list;
    }

    // Submit
    let slba_lo = lba as u32;
    let slba_hi = (lba >> 32) as u32;
    let cid = next_cid();
    let mut cmd = [0u32; 16];
    cmd[0] = opcode as u32 | ((PSDT_PRP as u32) << 8) | ((cid as u32) << 16);
    cmd[1] = nsid;
    cmd[6] = prp1 as u32;
    cmd[7] = (prp1 >> 32) as u32;
    cmd[8] = prp2 as u32;
    cmd[9] = (prp2 >> 32) as u32;
    cmd[10] = slba_lo;
    cmd[11] = slba_hi;
    cmd[12] = (count as u32 - 1) & 0xFFFF;
    let ok = io_send(&cmd) && io_wait(cid);

    // For read: copy from DMA pages to buf
    if ok && opcode == IO_READ {
        let mut dst = buf as usize;
        let mut rem = total_bytes;
        for i in 0..pages {
            let chunk = if rem > 4096 { 4096 } else { rem };
            unsafe { core::ptr::copy_nonoverlapping(addrs[i] as *const u8, dst as *mut u8, chunk); }
            dst += chunk;
            rem -= chunk;
        }
    }

    for i in 0..pages { crate::memory::pfree(addrs[i]); }
    if prp_list != 0 { crate::memory::pfree(prp_list); }
    ok
}

fn is_fat_type(pt: u8) -> bool { matches!(pt, 0x01|0x04|0x06|0x07|0x0B|0x0C|0x0E|0x1B|0x1C) }

fn parse_fat_bpb(phys: u64) {
    let bpb = unsafe { &*(phys as *const [u8; 512]) };
    let bps = (bpb[0x0B] as u16) | ((bpb[0x0C] as u16) << 8);
    if bps < 128 || bps > 4096 { uart::write_str("[NVMe/FAT] bad BpS\r\n"); return; }
    let spc = bpb[0x0D];
    if spc == 0 || !spc.is_power_of_two() { uart::write_str("[NVMe/FAT] bad SpC\r\n"); return; }
    let reserved = (bpb[0x0E] as u16) | ((bpb[0x0F] as u16) << 8);
    let fats = bpb[0x10];
    if fats == 0 { uart::write_str("[NVMe/FAT] no FATs\r\n"); return; }
    let root_entries = (bpb[0x11] as u16) | ((bpb[0x12] as u16) << 8);
    let fat16_sz = (bpb[0x16] as u16) | ((bpb[0x17] as u16) << 8);
    let is_fat32 = fat16_sz == 0;
    let fat_sz = if is_fat32 {
        let v = (bpb[0x24] as u32) | ((bpb[0x25] as u32) << 8) | ((bpb[0x26] as u32) << 16) | ((bpb[0x27] as u32) << 24);
        v as u64
    } else {
        fat16_sz as u64
    };
    let root_cluster = if is_fat32 {
        let v = (bpb[0x2C] as u32) | ((bpb[0x2D] as u32) << 8) | ((bpb[0x2E] as u32) << 16) | ((bpb[0x2F] as u32) << 24);
        v as u64
    } else {
        0
    };

    uart::write_str(" NVMe/FAT"); uart_dec(fat_sz as u64);
    uart::write_str(if is_fat32 {"32"} else {"16"});
    uart::write_str("\r\n");

    unsafe {
        BPS = bps; SPC = spc; FAT_SZ = fat_sz; ROOT_ENT = root_entries;
        RESERVED = reserved; FATS = fats; IS_FAT32 = is_fat32;
        ROOT_CLUSTER = root_cluster;
    }
}

fn init_fs() {
    let mbr_phys = crate::memory::palloc();
    if mbr_phys == 0 { return; }
    unsafe { core::ptr::write_bytes(mbr_phys as *mut u8, 0, 4096); }
    if !read_sectors(0, 1, mbr_phys as *mut u8) {
        crate::memory::pfree(mbr_phys);
        return;
    }

    let mbr = unsafe { core::slice::from_raw_parts(mbr_phys as *const u8, 512) };
    let sig = (mbr[0x1FE] as u16) | ((mbr[0x1FF] as u16) << 8);

    if sig != 0xAA55 {
        if mbr[0] == 0xEB || mbr[0] == 0xE9 {
            unsafe { PART_LBA = 0; }
            parse_fat_bpb(mbr_phys);
        }
        crate::memory::pfree(mbr_phys);
        unsafe { FS_INIT = true; }
        return;
    }

    let ptype = mbr[0x1C2];
    let pstart = (mbr[0x1C6] as u32) | ((mbr[0x1C7] as u32) << 8) |
                 ((mbr[0x1C8] as u32) << 16) | ((mbr[0x1C9] as u32) << 24);

    if ptype == 0 || ptype == 0xEE || !is_fat_type(ptype) {
        crate::memory::pfree(mbr_phys);
        return;
    }

    unsafe { PART_LBA = pstart as u64; }
    crate::memory::pfree(mbr_phys);

    let vbr_phys = crate::memory::palloc();
    if vbr_phys == 0 { return; }
    unsafe { core::ptr::write_bytes(vbr_phys as *mut u8, 0, 4096); }
    if !read_sectors(pstart as u64, 1, vbr_phys as *mut u8) {
        crate::memory::pfree(vbr_phys);
        return;
    }
    parse_fat_bpb(vbr_phys);
    crate::memory::pfree(vbr_phys);
    unsafe { FS_INIT = true; }
}

pub fn init() {
    if !find_nvme() {
        uart::write_str("[NVMe] not found\r\n");
        return;
    }
    uart::write_str("[NVMe] controller found\r\n");

    let cap = reg64(REG_CAP);
    let dstrd = ((cap >> 32) & 0xF) as u32;
    unsafe { DSTRD = dstrd; }
    uart::write_str("[NVMe] CAP.MQES="); uart_dec((cap & 0xFFFF) as u64);
    uart::write_str(" DSTRD="); uart_dec(dstrd as u64);
    uart::write_str("\r\n");

    // Disable controller
    let cc = reg32(REG_CC);
    if cc & CC_EN != 0 {
        wr32(REG_CC, cc & !CC_EN);
        if !spin_until(|| (reg32(REG_CSTS) & CSTS_RDY) == 0, 500_000) {
            uart::write_str("[NVMe] disable timeout\r\n"); return;
        }
    }

    let adm_sq = crate::memory::palloc();
    let adm_cq = crate::memory::palloc();
    if adm_sq == 0 || adm_cq == 0 {
        uart::write_str("[NVMe] alloc failed\r\n");
        if adm_sq != 0 { crate::memory::pfree(adm_sq); }
        return;
    }
    unsafe { core::ptr::write_bytes(adm_sq as *mut u8, 0, 4096); }
    unsafe { core::ptr::write_bytes(adm_cq as *mut u8, 0, 4096); }
    unsafe { ADM_SQ = adm_sq; ADM_CQ = adm_cq; }

    let aqa = ((QD - 1) << 16) | (QD - 1);
    wr32(REG_AQA, aqa);
    wr64(REG_ASQ, adm_sq);
    wr64(REG_ACQ, adm_cq);

    wr32(REG_CC, CC_EN | CC_IOSQES | CC_IOCQES);
    if !spin_until(|| (reg32(REG_CSTS) & CSTS_RDY) != 0, 500_000) {
        uart::write_str("[NVMe] enable timeout\r\n"); return;
    }
    uart::write_str("[NVMe] controller ready\r\n");

    // Verify ASQ/ACQ were actually set
    let asq_val = reg64(REG_ASQ);
    let acq_val = reg64(REG_ACQ);
    uart::write_str("[NVMe] ASQ="); hex32(asq_val as u32); hex32((asq_val >> 32) as u32);
    uart::write_str(" ACQ="); hex32(acq_val as u32); hex32((acq_val >> 32) as u32);
    uart::write_str(" AQA="); hex32(reg32(REG_AQA));
    uart::write_str(" CC="); hex32(reg32(REG_CC));
    uart::write_str(" CSTS="); hex32(reg32(REG_CSTS));
    uart::write_str("\r\n");
    // Try reading CAP again to verify MMIO still works after enable
    uart::write_str("[NVMe] CAP="); hex32(reg64(REG_CAP) as u32); hex32((reg64(REG_CAP) >> 32) as u32);
    uart::write_str("\r\n");

    // Small delay
    crate::timer::usleep(1000);

    // IDENTIFY controller (CNS=1 requires NSID=0xFFFFFFFF)
    let ident_phys = crate::memory::palloc();
    if ident_phys == 0 { uart::write_str("[NVMe] ident alloc failed\r\n"); return; }
    unsafe { core::ptr::write_bytes(ident_phys as *mut u8, 0, 4096); }
    if !submit_admin(ADMIN_IDENTIFY, 0xFFFFFFFF, ident_phys, 0, 0x01, 0) {
        uart::write_str("[NVMe] identify controller failed\r\n");
        crate::memory::pfree(ident_phys); return;
    }

    // IDENTIFY namespace
    unsafe { core::ptr::write_bytes(ident_phys as *mut u8, 0, 4096); }
    if !submit_admin(ADMIN_IDENTIFY, 1, ident_phys, 0, 0x00, 0) {
        uart::write_str("[NVMe] identify namespace failed\r\n");
        crate::memory::pfree(ident_phys); return;
    }

    let ident_buf = unsafe { core::slice::from_raw_parts(ident_phys as *const u8, 4096) };
    let nsze = (ident_buf[0] as u64) | ((ident_buf[1] as u64) << 8) |
               ((ident_buf[2] as u64) << 16) | ((ident_buf[3] as u64) << 24) |
               ((ident_buf[4] as u64) << 32) | ((ident_buf[5] as u64) << 40) |
               ((ident_buf[6] as u64) << 48) | ((ident_buf[7] as u64) << 56);
    let flbas = ident_buf[26];
    let lba_idx = flbas & 0x0F;
    let ds_off = 128 + lba_idx as usize * 4 + 2;
    let ds = ident_buf[ds_off];
    let lba_size = 1u32 << ds;

    unsafe { NSID = 1; NS_LBA_COUNT = nsze; NS_LBA_SIZE = lba_size; }
    uart::write_str("[NVMe] NSID=1 size="); uart_dec(nsze);
    uart::write_str(" LBAs of "); uart_dec(lba_size as u64);
    uart::write_str(" bytes\r\n");
    crate::memory::pfree(ident_phys);

    // Create I/O CQ (qid=1)
    let io_cq = crate::memory::palloc();
    if io_cq == 0 { uart::write_str("[NVMe] IO CQ alloc failed\r\n"); return; }
    unsafe { core::ptr::write_bytes(io_cq as *mut u8, 0, 4096); }
    unsafe { IO_CQ = io_cq; }
    if !submit_admin(ADMIN_CREATE_CQ, 0, io_cq, 0, 1 | ((QD - 1) << 16), 1) {
        uart::write_str("[NVMe] create IO CQ failed\r\n"); return;
    }

    // Create I/O SQ (qid=1, cqid=1)
    let io_sq = crate::memory::palloc();
    if io_sq == 0 { uart::write_str("[NVMe] IO SQ alloc failed\r\n"); return; }
    unsafe { core::ptr::write_bytes(io_sq as *mut u8, 0, 4096); }
    unsafe { IO_SQ = io_sq; }
    if !submit_admin(ADMIN_CREATE_SQ, 0, io_sq, 0, 1 | ((QD - 1) << 16), 1 | (1 << 16)) {
        uart::write_str("[NVMe] create IO SQ failed\r\n"); return;
    }
    unsafe { IO_QID = 1; INIT = true; }
    uart::write_str("[NVMe] I/O queues ready\r\n");

    // Parse FAT from NVMe
    init_fs();

    uart::write_str("[NVMe] ready\r\n");
}

pub fn read_sectors(lba: u64, count: u16, buf: *mut u8) -> bool {
    nvme_rw(IO_READ, lba, count, buf)
}

pub fn write_sectors(lba: u64, count: u16, buf: *mut u8) -> bool {
    nvme_rw(IO_WRITE, lba, count, buf)
}

pub fn read_fat_sector(lba: u64, buf: &mut [u8; 512]) -> bool {
    if !unsafe { FS_INIT } { return false; }
    let part_lba = unsafe { PART_LBA };
    read_sectors(part_lba + lba, 1, buf.as_mut_ptr())
}

pub fn write_fat_sector(lba: u64, buf: &[u8; 512]) -> bool {
    if !unsafe { FS_INIT } { return false; }
    let part_lba = unsafe { PART_LBA };
    write_sectors(part_lba + lba, 1, buf.as_ptr() as *mut u8)
}

fn uart_dec(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut b = [0u8; 20]; let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putchar(b[i]); }
}
