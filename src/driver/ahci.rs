use super::uart;
use core::ptr::{read_volatile, write_volatile};

const SATA_VENDOR: u16 = 0x8086;
const SATA_DEVICE: u16 = 0x2922;

const HBA_GHC: u64 = 0x0004;
const GHC_AE: u32 = 0x8000_0000;
const GHC_HR: u32 = 0x0000_0001;

const CAP_NP: u32 = 0x1F;

static mut AHCI_BASE: u64 = 0;
static mut PORT_INIT: bool = false;
static mut CLB_PHYS: u64 = 0;
static mut CT_PHYS: u64 = 0;

// Stored FAT parameters for post-EBS access
pub static mut PART_LBA: u64 = 0;
pub static mut BPS: u16 = 512;
pub static mut SPC: u8 = 1;
pub static mut FAT_SZ: u64 = 0;
pub static mut ROOT_ENT: u16 = 0;
pub static mut RESERVED: u16 = 1;
pub static mut FATS: u8 = 2;
pub static mut IS_FAT32: bool = false;
pub static mut ROOT_CLUSTER: u64 = 0;
fn find_ahci() -> bool {
    for dev in 0..32 {
        for func in 0..8 {
            let v = super::pci::read16(0, dev as u8, func as u8, 0);
            if v != SATA_VENDOR { if func == 0 { break; } continue; }
            let d = super::pci::read16(0, dev as u8, func as u8, 2);
            if d != SATA_DEVICE { continue; }
            let r = super::pci::read32(0, dev as u8, func as u8, 8);
            if (r >> 24) as u8 == 1 && ((r >> 16) & 0xFF) as u8 == 6 {
                let bar5 = super::pci::read32(0, dev as u8, func as u8, 0x24);
                unsafe { AHCI_BASE = (bar5 & 0xFFFFFFF0) as u64; }
                return true;
            }
        }
    }
    false
}

fn mmio32(off: u64) -> *mut u32 { (unsafe { AHCI_BASE } + off) as *mut u32 }
fn reg32(off: u64) -> u32 { unsafe { read_volatile(mmio32(off)) } }
fn wr32(off: u64, v: u32) { unsafe { write_volatile(mmio32(off), v) } }

// Port register offsets (port 0: base = 0x100)
fn port_base(port: u32) -> u64 { 0x100 + port as u64 * 0x80 }
// AHCI port register offsets (from port base = HBA_BASE + 0x100 + port*0x80)
fn p_is(p: u64) -> u32 { reg32(p + 0x10) }
fn p_cmd(p: u64) -> u32 { reg32(p + 0x18) }
fn p_tfd(p: u64) -> u32 { reg32(p + 0x20) }
fn p_sig(p: u64) -> u32 { reg32(p + 0x24) }
fn p_ssts(p: u64) -> u32 { reg32(p + 0x28) }
fn p_ci(p: u64) -> u32 { reg32(p + 0x38) }

fn wr_p_cmd(p: u64, v: u32) { wr32(p + 0x18, v) }
fn wr_p_ci(p: u64, v: u32) { wr32(p + 0x38, v) }
fn wr_p_clb(p: u64, v: u32) { wr32(p, v) }
fn wr_p_clbu(p: u64, v: u32) { wr32(p + 0x04, v) }
fn wr_p_fb(p: u64, v: u32) { wr32(p + 0x08, v) }
fn wr_p_fbu(p: u64, v: u32) { wr32(p + 0x0C, v) }
fn wr_p_ie(p: u64, v: u32) { wr32(p + 0x14, v) }

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

fn port_init(port: u32) -> bool {
    let pb = port_base(port);

    let ssts = p_ssts(pb);
    let sig = p_sig(pb);
    if (ssts & 0x0F) != 0x03 { return false; }
    if sig != 0x0000_0101 && sig != 0xEB14_0101 && sig != 0x9669_0101 { return false; }

    let clb_phys = crate::memory::palloc_n(1);
    if clb_phys == 0 { return false; }
    let fb_phys = crate::memory::palloc_n(4);
    if fb_phys == 0 { return false; }
    let ct_phys = crate::memory::palloc_n(1);
    if ct_phys == 0 { return false; }

    unsafe {
        core::ptr::write_bytes(clb_phys as *mut u8, 0, 4096);
        core::ptr::write_bytes(fb_phys as *mut u8, 0, 16384);
        core::ptr::write_bytes(ct_phys as *mut u8, 0, 4096);
    }

    // Stop port: clear ST (bit 0) and FRE (bit 4)
    let cmd = p_cmd(pb);
    wr_p_cmd(pb, cmd & !(1 | (1 << 4)));
    spin_until(|| (p_cmd(pb) & 0xC000_0000) == 0, 100_000); // wait for FR+CR to clear

    // Set our DMA base addresses
    wr_p_clb(pb, clb_phys as u32);
    wr_p_clbu(pb, (clb_phys >> 32) as u32);
    wr_p_fb(pb, fb_phys as u32);
    wr_p_fbu(pb, (fb_phys >> 32) as u32);

    // Enable port: ST + FRE + POD + SUD
    wr_p_ie(pb, 0);
    wr_p_cmd(pb, 0x0017);

    spin_until(|| (p_cmd(pb) & 0xC000_0000) == 0xC000_0000, 100_000); // wait for FR+CR

    // Clear interrupts
    wr32(pb + 0x10, 0xFFFFFFFF);

    unsafe {
        CLB_PHYS = clb_phys;
        CT_PHYS = ct_phys;
        let hdr = clb_phys as *mut u32;
        write_volatile(hdr, (5 << 0) | (1 << 16));
        write_volatile(hdr.add(2), ct_phys as u32);
        write_volatile(hdr.add(3), (ct_phys >> 32) as u32);
    }

    unsafe { PORT_INIT = true; }
    true
}

fn build_read_fis(lba: u64, count: u16) -> [u32; 5] {
    [
        0x27 | (0x80 << 8) | (0x25 << 16),
        lba as u32 & 0xFFFFFF | (0x40 << 24),
        ((lba >> 24) as u32 & 0xFF) | ((lba >> 32) as u32 & 0xFF) << 8 | ((lba >> 40) as u32 & 0xFF) << 16,
        (count as u32 & 0xFF) | ((count as u32 >> 8) & 0xFF) << 8,
        0,
    ]
}

fn do_command(_cmd: u8, fis4: [u32; 5], buf: *mut u8, count: u16) -> bool {
    if !unsafe { PORT_INIT } { return false; }
    let pb = port_base(0);

    // Wait for port idle
    if !spin_until(|| (p_ci(pb) & 1) == 0, 10_000) {
        uart::write_str("[AHCI] port busy\r\n");
        return false;
    }

    let ct_addr = unsafe { CT_PHYS };
    if ct_addr == 0 { return false; }
    let clb = unsafe { CLB_PHYS };

    unsafe {
        let ct = ct_addr as *mut u32;
        for i in 0..5 { write_volatile(ct.add(i), fis4[i]); }
        // PRDT entry at ct_addr + 0x80 (128 bytes after FIS start)
        let prdt = ct.add(0x80 / 4);
        let data_phys = buf as u64;
        write_volatile(prdt, data_phys as u32);
        write_volatile(prdt.add(1), (data_phys >> 32) as u32);
        let byte_count = (count as u32) * 512;
        let dbc = byte_count - 1; // DBC is 1-based (0 = 1 byte)
        write_volatile(prdt.add(2), 0);                     // reserved
        write_volatile(prdt.add(3), dbc | 0x8000_0000);    // flags_size = DBC + I
    }

    // Update command header: FIS length, PRDT length, CTBA
    unsafe {
        let hdr = clb as *mut u32;
        write_volatile(hdr, (5 << 0) | (1 << 16));
        write_volatile(hdr.add(2), ct_addr as u32);
        write_volatile(hdr.add(3), (ct_addr >> 32) as u32);
    }

    // Clear port interrupts
    wr32(pb + 0x10, 0xFFFFFFFF);

    // Flush CPU cache so HBA sees our CLB/CT/PRDT writes
    unsafe { core::arch::asm!("wbinvd"); }

    // Issue command: set bit 0 in PxCI
    wr_p_ci(pb, 1);

    // Wait for completion
    if !spin_until(|| (p_ci(pb) & 1) == 0, 30_000_000) {
        let tfd = p_tfd(pb);
        let ci = p_ci(pb);
        let is = p_is(pb);
        uart::write_str("[AHCI] timeout! CI="); uart_hex32(ci);
        uart::write_str(" TFD=0x"); uart_hex32(tfd);
        uart::write_str(" IS=0x"); uart_hex32(is);
        uart::write_str("\r\n");
        return false;
    }

    // Check for errors
    let tfd = p_tfd(pb);
    if tfd & 0x01 != 0 { return false; }

    // Flush CPU cache to see DMA data (no-op on QEMU, needed on real HW)
    unsafe { core::arch::asm!("wbinvd"); }

    true
}

pub fn init() {
    if !find_ahci() { return; }

    let ghc = reg32(HBA_GHC);
    if ghc & GHC_AE == 0 {
        wr32(HBA_GHC, GHC_HR);
        spin_until(|| (reg32(HBA_GHC) & GHC_HR) == 0, 1_000);
        wr32(HBA_GHC, GHC_AE);
    }

    // Detect number of ports
    let cap = reg32(0x00);
    let n_ports = (cap & CAP_NP) + 1;
    let pi = reg32(0x0C);

    // Try to find a port with a device
    let mut found = false;
    if pi & 1 != 0 {
        found = port_init(0);
    }
    if !found { for p in 1..n_ports { if (pi >> p) & 1 != 0 { if port_init(p) { found = true; break; } } } }

    if !found { uart::write_str("[AHCI] no device\r\n"); return; }

    // Read MBR via AHCI DMA
    let mbr_phys = crate::memory::palloc();
    if mbr_phys == 0 { return; }
    unsafe { core::ptr::write_bytes(mbr_phys as *mut u8, 0, 4096); }
    if !read_sectors(0, 1, mbr_phys as *mut u8) { crate::memory::pfree(mbr_phys); return; }

    let mbr = unsafe { core::slice::from_raw_parts(mbr_phys as *const u8, 512) };
    let sig = (mbr[0x1FE] as u16) | ((mbr[0x1FF] as u16) << 8);

    if sig != 0xAA55 {
        if mbr[0] == 0xEB || mbr[0] == 0xE9 {
            unsafe { PART_LBA = 0; }
            parse_fat_bpb_from(mbr_phys);
        }
        crate::memory::pfree(mbr_phys);
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

    // Read FAT VBR
    let vbr_phys = crate::memory::palloc();
    if vbr_phys == 0 { return; }
    unsafe { core::ptr::write_bytes(vbr_phys as *mut u8, 0, 4096); }
    if !read_sectors(pstart as u64, 1, vbr_phys as *mut u8) { crate::memory::pfree(vbr_phys); return; }
    parse_fat_bpb_from(vbr_phys);
    crate::memory::pfree(vbr_phys);
}

fn is_fat_type(pt: u8) -> bool { matches!(pt, 0x01|0x04|0x06|0x07|0x0B|0x0C|0x0E|0x1B|0x1C) }

fn parse_fat_bpb_from(phys: u64) {
    let bpb = unsafe { &*(phys as *const [u8; 512]) };
    let bps = (bpb[0x0B] as u16) | ((bpb[0x0C] as u16) << 8);
    if bps < 128 || bps > 4096 { uart::write_str("[FAT] bad BpS\r\n"); return; }
    let spc = bpb[0x0D];
    if spc == 0 || !spc.is_power_of_two() { uart::write_str("[FAT] bad SpC\r\n"); return; }
    let reserved = (bpb[0x0E] as u16) | ((bpb[0x0F] as u16) << 8);
    let fats = bpb[0x10];
    if fats == 0 { uart::write_str("[FAT] no FATs\r\n"); return; }
    let root_entries = (bpb[0x11] as u16) | ((bpb[0x12] as u16) << 8);
    let _total = if bpb[0x13..0x15].iter().any(|&x| x != 0) {
        (bpb[0x13] as u32) | ((bpb[0x14] as u32) << 8) | ((bpb[0x15] as u32) << 16)
    } else {
        leu32(&bpb[0x20..0x24])
    } as u64;

    let fat16_sz = (bpb[0x16] as u16) | ((bpb[0x17] as u16) << 8);
    let is_fat32 = fat16_sz == 0;
    let fat_sz = if is_fat32 { leu32(&bpb[0x24..0x28]) as u64 } else { fat16_sz as u64 };
    let root_cluster = if is_fat32 { leu32(&bpb[0x2C..0x30]) as u64 } else { 0 };

    uart::write_str(" FAT"); uart_dec(fat_sz as u64);
    uart::write_str(if is_fat32 {"32"} else {"16"});
    uart::write_str("\r\n");

    unsafe {
        BPS = bps; SPC = spc; FAT_SZ = fat_sz; ROOT_ENT = root_entries;
        RESERVED = reserved; FATS = fats; IS_FAT32 = is_fat32;
        ROOT_CLUSTER = root_cluster;
    }
}

pub fn read_sectors(lba: u64, count: u16, buf: *mut u8) -> bool {
    do_command(0x25, build_read_fis(lba, count), buf, count)
}

pub fn read_fat_sector(lba: u64, buf: &mut [u8; 512]) -> bool {
    let part_lba = unsafe { PART_LBA };
    read_sectors(part_lba + lba, 1, buf.as_mut_ptr())
}

pub fn write_fat_sector(_lba: u64, _buf: &[u8; 512]) -> bool {
    false
}

fn leu32(b: &[u8]) -> u32 { let mut v=0; for i in 0..b.len().min(4) { v|=(b[i] as u32)<<(i*8); } v }
fn uart_dec(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut b = [0u8; 20]; let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putchar(b[i]); }
}
fn uart_hex32(v: u32) {
    for i in (0..8).rev() { let n = (v>>(i*4))&0xF; uart::putchar(if n<10{b'0'+n as u8}else{b'A'+n as u8-10}); }
}

