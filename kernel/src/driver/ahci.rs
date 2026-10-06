use super::uart;
use core::ptr::{read_volatile, write_volatile};

const _SATA_VENDOR: u16 = 0x8086;
const _SATA_DEVICE: u16 = 0x2922;

const HBA_GHC: u64 = 0x0004;
const GHC_AE: u32 = 0x8000_0000;
const _GHC_HR: u32 = 0x0000_0001;

const CAP_NP: u32 = 0x1F;

static mut AHCI_BASE: u64 = 0;
static mut PORT_INIT: bool = false;
// Рабочий порт (лог доказал: init мог выбрать p>0, а do_command всегда бил в 0)
static mut PORT_N: u32 = 0;
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
    // Generic: любой PCI class 01h subclass 06h (SATA/AHCI), не только 8086:2922.
    // QEMU q35 даёт 8086:2922, но на железе встречаются 8086:2829/2822/8c02,
    // 1022:7801, 1b4b:9172 и т.д. Проверяем BAR5 и class, а не VID/DID.
    for dev in 0..32 {
        for func in 0..8 {
            let v = super::pci::read16(0, dev as u8, func as u8, 0);
            if v == 0xFFFF { if func == 0 { break; } continue; }
            let r = super::pci::read32(0, dev as u8, func as u8, 8);
            let class = (r >> 24) as u8;
            let subclass = ((r >> 16) & 0xFF) as u8;
            if class != 0x01 || subclass != 0x06 { continue; }
            // Пропускаем IDE-legacy в том же классе (subclass 01) — уже отсеяно выше.
            let bar5 = super::pci::read32(0, dev as u8, func as u8, 0x24);
            let base = (bar5 & 0xFFFF_FFF0) as u64;
            if base == 0 { continue; }
            // BAR5 должен быть MEM (bit0=0), не I/O
            if bar5 & 1 != 0 { continue; }
            unsafe { AHCI_BASE = base; }
            // Включить MEM + BusMaster: без MASTER HBA не может делать DMA
            // (CI висел с TFD=0x50 — команда не забиралась). Как в NVMe-драйвере.
            let cmd = super::pci::read16(0, dev as u8, func as u8, 0x04);
            super::pci::write32(0, dev as u8, func as u8, 0x04, (cmd | 0x6) as u32);
            uart::write_str("[AHCI] found PCI ");
            uart_hex32(v as u32);
            uart::write_str(":");
            uart_hex32(super::pci::read16(0, dev as u8, func as u8, 2) as u32);
            uart::write_str(" BAR5=");
            uart_hex32(bar5);
            uart::write_str("\r\n");
            return true;
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
    // Гибрид: таймер (HPET) + жёсткий кап итераций, чтобы не висеть если таймер не готов.
    // 100k итераций ~ единицы мс на spin_loop; кап 20M итераций гарантирует выход.
    let mut iter: u64 = 0;
    const ITER_CAP: u64 = 20_000_000;
    while !f() {
        iter += 1;
        if iter >= ITER_CAP { return false; }
        // Проверяем таймер только каждые 1024 итерации (дешевле)
        if (iter & 1023) == 0 && max_us > 0 {
            if timer::ticks().wrapping_sub(start) > max_us * 10 {
                return false;
            }
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
    // Ждём FR(bit14)+CR(bit15) == 0. Маска 0xC000, НЕ 0xC000_0000 (был баг — вечное ожидание).
    if !spin_until(|| (p_cmd(pb) & 0xC000) == 0, 500_000) {
        uart::write_str("[AHCI] port stop timeout\r\n");
        // Продолжаем: QEMU иногда держит CR — переинициализация всё равно пробуется
    }

    // Чистим залипшие SATA-ошибки ДО старта (раньше чистили после — ERR мог
    // блокировать первую команду и давать вечный CI=1).
    wr32(pb + 0x10, 0xFFFFFFFF); // PxIS
    wr32(pb + 0x30, 0xFFFFFFFF); // PxSERR (W1C)

    // Set our DMA base addresses
    wr_p_clb(pb, clb_phys as u32);
    wr_p_clbu(pb, (clb_phys >> 32) as u32);
    wr_p_fb(pb, fb_phys as u32);
    wr_p_fbu(pb, (fb_phys >> 32) as u32);

    // Enable port: ST + FRE + POD + SUD
    wr_p_ie(pb, 0);
    wr_p_cmd(pb, 0x0017);

    // Ждём FR+CR == 1 (биты 14,15). Bounded 500ms — дальше не висим.
    if !spin_until(|| (p_cmd(pb) & 0xC000) == 0xC000, 500_000) {
        uart::write_str("[AHCI] port start timeout (FR/CR not ready)\r\n");
        return false;
    }

    // Clear interrupts (SERR уже чист сверху)
    wr32(pb + 0x10, 0xFFFFFFFF);

    unsafe {
        CLB_PHYS = clb_phys;
        CT_PHYS = ct_phys;
        PORT_N = port;
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
    // Порт, прошедший port_init (НЕ хардкод 0 — иначе бьём в мёртвый порт)
    let pb = port_base(unsafe { PORT_N });

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

    // Flush CPU cache so HBA sees our CLB/CT/PRDT writes.
    // Раньше флашился только CT — HBA мог читать stale Command Header
    // (CLB) и игнорировать команду: вечный CI=1 при готовом TFD.
    // On QEMU wbinvd can trigger #DB; use clflush per-line instead.
    {
        for base in [clb, ct_addr] {
            for i in (0..(1024u64)).step_by(64) {
                unsafe { core::arch::asm!("clflush [{}]", in(reg) base + i); }
            }
        }
    }

    // Issue command: set bit 0 in PxCI
    wr_p_ci(pb, 1);

    // Wait for completion
    if !spin_until(|| (p_ci(pb) & 1) == 0, 30_000_000) {
        let tfd = p_tfd(pb);
        let ci = p_ci(pb);
        let is = p_is(pb);
        let serr = reg32(pb + 0x30);
        let ssts = p_ssts(pb);
        let sig = p_sig(pb);
        uart::write_str("[AHCI] timeout! CI="); uart_hex32(ci);
        uart::write_str(" TFD=0x"); uart_hex32(tfd);
        uart::write_str(" IS=0x"); uart_hex32(is);
        uart::write_str(" SERR=0x"); uart_hex32(serr);
        uart::write_str(" SSTS=0x"); uart_hex32(ssts);
        uart::write_str(" SIG=0x"); uart_hex32(sig);
        uart::write_str("\r\n");
        return false;
    }

    // Check for errors
    let tfd = p_tfd(pb);
    if tfd & 0x01 != 0 { return false; }

    // Flush CPU cache to see DMA data (no-op on QEMU, needed on real HW)
    // On QEMU wbinvd can trigger #DB; skip it safely
    //unsafe { core::arch::asm!("wbinvd"); }

    true
}

pub fn init() {
    if !find_ahci() { uart::write_str("[AHCI] no HBA (class 01/06 not found)\r\n"); return; }
    if unsafe { AHCI_BASE } == 0 { uart::write_str("[AHCI] BAR5=0, skip\r\n"); return; }

    // BIOS/OS Handoff (BOHC @ 0x28): забрать владение у UEFI/BIOS.
    // Без этого порт может остаться под контролем firmware и команды висят.
    {
        let bohc = reg32(0x28);
        // BOHC: bit0 BOS, bit1 OOS, bit4 BB. Запрашиваем OOS.
        if bohc & 0x02 == 0 {
            wr32(0x28, bohc | 0x02);
            // Ждём bounded: максимум ~1с, иначе продолжаем (QEMU BOHC может отсутствовать)
            let _ = spin_until(|| (reg32(0x28) & 0x10) == 0, 1_000_000);
        }
    }

    // HBA reset только если AE уже поднят и контроллер в странном состоянии.
    // Полный HR сбрасывает PI и может уронить QEMU ICH9 — делаем мягко.
    let ghc = reg32(HBA_GHC);
    if ghc & GHC_AE == 0 {
        wr32(HBA_GHC, GHC_AE);
        // AE должен подняться сразу; bounded wait 100ms
        if !spin_until(|| (reg32(HBA_GHC) & GHC_AE) != 0, 100_000) {
            uart::write_str("[AHCI] AE enable timeout\r\n");
            return;
        }
    } else {
        // Staggered spin-up: включаем SUD на всех реализованных портах
        let pi = reg32(0x0C);
        for p in 0..8u32 {
            if (pi >> p) & 1 == 0 { continue; }
            let pb = port_base(p);
            let cmd = p_cmd(pb);
            // POD (bit2) + SUD (bit1): раскрутить устройство
            wr_p_cmd(pb, cmd | 0x06);
        }
    }

    // Detect number of ports (кап 8 — дальше только трата времени)
    let cap = reg32(0x00);
    let n_ports = ((cap & CAP_NP) + 1).min(8);
    let pi = reg32(0x0C);

    // Try to find a port with a device
    let mut found = false;
    if pi & 1 != 0 {
        found = port_init(0);
    }
    if !found { for p in 1..n_ports { if (pi >> p) & 1 != 0 { if port_init(p) { found = true; break; } } } }

    if !found { uart::write_str("[AHCI] no device\r\n"); return; }
    uart::write_str("[AHCI] using port ");
    uart_dec(unsafe { PORT_N } as u64);
    uart::write_str("\r\n");

    // Read MBR via AHCI DMA
    let mbr_phys = crate::memory::palloc();
    if mbr_phys == 0 { return; }
    unsafe { core::ptr::write_bytes(mbr_phys as *mut u8, 0, 4096); }
    if !read_sectors(0, 1, mbr_phys as *mut u8) {
        crate::memory::pfree(mbr_phys);
        uart::write_str("[AHCI] MBR read failed (DMA timeout, no usable disk)\r\n");
        return;
    }

    // Read MBR via AHCI DMA; весь разбор — в скоупе, чтобы pfree не конфликтовал с заимствованием.
    let (sig, b0, ptype, pstart) = {
        let mbr = unsafe { core::slice::from_raw_parts(mbr_phys as *const u8, 512) };
        let sig = (mbr[0x1FE] as u16) | ((mbr[0x1FF] as u16) << 8);
        let b0 = mbr[0];
        let ptype = mbr[0x1C2];
        let pstart = (mbr[0x1C6] as u32) | ((mbr[0x1C7] as u32) << 8) |
                     ((mbr[0x1C8] as u32) << 16) | ((mbr[0x1C9] as u32) << 24);
        (sig, b0, ptype, pstart)
    };

    if sig != 0xAA55 {
        if b0 == 0xEB || b0 == 0xE9 {
            unsafe { PART_LBA = 0; }
            parse_fat_bpb_from(mbr_phys);
        }
        crate::memory::pfree(mbr_phys);
        return;
    }

    // GPT protective MBR: ищем партицию через GPT header (LBA1)
    if ptype == 0xEE {
        if let Some(gpt_lba) = gpt_first_lba() {
            unsafe { PART_LBA = gpt_lba; }
            crate::memory::pfree(mbr_phys);
            let vbr_phys = crate::memory::palloc();
            if vbr_phys == 0 { return; }
            unsafe { core::ptr::write_bytes(vbr_phys as *mut u8, 0, 4096); }
            if !read_sectors(gpt_lba, 1, vbr_phys as *mut u8) { crate::memory::pfree(vbr_phys); return; }
            parse_fat_bpb_from(vbr_phys);
            crate::memory::pfree(vbr_phys);
            uart::write_str("[AHCI] GPT part LBA="); uart_dec(gpt_lba);
            uart::write_str("\r\n");
            return;
        }
        crate::memory::pfree(mbr_phys);
        uart::write_str("[AHCI] GPT parse failed\r\n");
        return;
    }

    if ptype == 0 || !is_fat_type(ptype) {
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

/// GPT: header LBA1 -> entries -> первая usable партиция. Bounded, без зависаний.
fn gpt_first_lba() -> Option<u64> {
    let hdr_phys = crate::memory::palloc();
    if hdr_phys == 0 { return None; }
    unsafe { core::ptr::write_bytes(hdr_phys as *mut u8, 0, 4096); }
    if !read_sectors(1, 1, hdr_phys as *mut u8) { crate::memory::pfree(hdr_phys); return None; }
    let parsed = {
        let hdr = unsafe { &*(hdr_phys as *const [u8; 512]) };
        crate::gpt::parse_header(hdr)
    };
    crate::memory::pfree(hdr_phys);
    let (entry_lba, entry_num, entry_size) = parsed?;
    let total_bytes = (entry_num as u64) * (entry_size as u64);
    let sectors = ((total_bytes + 511) / 512).min(32) as u16;
    if sectors == 0 { return None; }
    // Читаем первые entries (хватает для первой партиции) в один 4K буфер
    let buf_phys = crate::memory::palloc();
    if buf_phys == 0 { return None; }
    unsafe { core::ptr::write_bytes(buf_phys as *mut u8, 0, 4096); }
    let want = sectors.min(8) as u64; // 8 секторов = 4KB
    for s in 0..want {
        let t = crate::memory::palloc();
        if t == 0 { crate::memory::pfree(buf_phys); return None; }
        if !read_sectors(entry_lba + s, 1, t as *mut u8) { crate::memory::pfree(t); crate::memory::pfree(buf_phys); return None; }
        unsafe { core::ptr::copy_nonoverlapping(t as *const u8, (buf_phys + s * 512) as *mut u8, 512); }
        crate::memory::pfree(t);
    }
    let copy_len = (want as usize * 512).min(4096);
    let result = {
        let flat = unsafe { core::slice::from_raw_parts(buf_phys as *const u8, copy_len) };
        crate::gpt::first_partition_lba(flat, entry_num, entry_size)
    };
    crate::memory::pfree(buf_phys);
    result
}

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

/// Готов ли порт (для block-слоя и probe).
pub fn is_ready() -> bool { unsafe { PORT_INIT } }

pub fn part_lba() -> u64 { unsafe { PART_LBA } }

fn build_write_fis(lba: u64, count: u16) -> [u32; 5] {
    [
        0x27 | (0x80 << 8) | (0x35 << 16), // H2D FIS, command, WRITE DMA EXT
        lba as u32 & 0xFFFFFF | (0x40 << 24), // LBA low 24 bits + LBA mode
        ((lba >> 24) as u32 & 0xFF) | ((lba >> 32) as u32 & 0xFF) << 8 | ((lba >> 40) as u32 & 0xFF) << 16,
        (count as u32 & 0xFF) | ((count as u32 >> 8) & 0xFF) << 8,
        0,
    ]
}

pub fn write_sectors(lba: u64, count: u16, buf: *mut u8) -> bool {
    // On QEMU wbinvd can trigger #DB; skip it safely
    //unsafe { core::arch::asm!("wbinvd"); }
    do_command(0x35, build_write_fis(lba, count), buf, count)
}

pub fn read_fat_sector(lba: u64, buf: &mut [u8; 512]) -> bool {
    let part_lba = unsafe { PART_LBA };
    read_sectors(part_lba + lba, 1, buf.as_mut_ptr())
}

pub fn write_fat_sector(lba: u64, buf: &[u8; 512]) -> bool {
    let part_lba = unsafe { PART_LBA };
    // Need a physical buffer for DMA — copy to a palloc'd page
    let phys = crate::memory::palloc();
    if phys == 0 { return false; }
    // Identity-map the page so CPU can write to it
    unsafe {
        let pml4 = crate::vm::KERNEL_PML4 as *mut u64;
        crate::vm::map_page(pml4, phys, phys, crate::vm::PTE_WRITABLE);
        core::ptr::copy_nonoverlapping(buf.as_ptr(), phys as *mut u8, 512);
    }
    let ok = write_sectors(part_lba + lba, 1, phys as *mut u8);
    // Unmap
    unsafe {
        let pml4 = crate::vm::KERNEL_PML4 as *mut u64;
        crate::vm::unmap_page(pml4, phys);
    }
    crate::memory::pfree(phys);
    ok
}

pub struct AhciDriver;

impl super::traits::Driver for AhciDriver {
    fn name(&self) -> &'static str { "AHCI SATA" }
    fn device_type(&self) -> super::traits::DeviceType {
        super::traits::DeviceType::Pci { vendor: 0x8086, device: 0x2922, class: 0x01, subclass: 0x06 }
    }
    fn init(&self) -> super::traits::DriverStatus {
        init();
        if unsafe { PORT_INIT } {
            super::traits::DriverStatus::Ok
        } else {
            super::traits::DriverStatus::Unsupported
        }
    }
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

