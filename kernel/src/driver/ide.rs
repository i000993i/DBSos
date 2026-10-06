//! IDE PATA (PIIX3/ICH) PIO-драйвер + ATAPI для CD-ROM.
//!
//! Зачем: VirtualBox отдаёт CD-ROM и диски через IDE-legacy (PIIX3) —
//! без этого драйвера на VBox нет ни одного блочного устройства
//! (NVMe-диска там нет, AHCI может отсутствовать).
//! Только PIO (без BusMaster DMA): медленно, но работает везде.
//! Канал 0: 0x1F0/0x3F6, канал 1: 0x170/0x376. LBA28 + ATAPI READ(12).

use crate::driver::uart;
use crate::io;

const DATA_OFF: u16 = 0;
const COUNT_OFF: u16 = 2;
const LBA_LO_OFF: u16 = 3;
const LBA_MID_OFF: u16 = 4;
const LBA_HI_OFF: u16 = 5;
const DRIVE_OFF: u16 = 6;
const STATUS_OFF: u16 = 7; // чтение = STATUS, запись = COMMAND

const ST_ERR: u8 = 0x01;
const ST_DRQ: u8 = 0x08;
const ST_BSY: u8 = 0x80;
// ST_RDY (0x40) не проверяем: часть приводов его не поднимает в PIO.

const CMD_IDENTIFY: u8 = 0xEC;
const CMD_IDENTIFY_PACKET: u8 = 0xA1;
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_PACKET: u8 = 0xA0;

fn cmd_base(ch: u8) -> u16 {
    if ch == 0 { 0x1F0 } else { 0x170 }
}
fn ctl_base(ch: u8) -> u16 {
    if ch == 0 { 0x3F6 } else { 0x376 }
}

/// 16-битное чтение data-порта (io.rs даёт только 8/32-битные).
fn inword(port: u16) -> u16 {
    let v: u16;
    unsafe { core::arch::asm!("in ax, dx", out("ax") v, in("dx") port) };
    v
}

fn status(ch: u8) -> u8 {
    unsafe { io::inb(cmd_base(ch) + STATUS_OFF) }
}

fn delay400(ch: u8) {
    // ~400ns: 4 холостых чтения control-порта
    unsafe {
        for _ in 0..4 {
            let _ = io::inb(ctl_base(ch));
        }
    }
}

fn select(ch: u8, drive: u8) {
    unsafe { io::outb(cmd_base(ch) + DRIVE_OFF, 0xA0 | (drive << 4)) };
    delay400(ch);
}

fn wait_bsy_clear(ch: u8, iters: u32) -> bool {
    let mut i = 0u32;
    while status(ch) & ST_BSY != 0 {
        i += 1;
        if i >= iters {
            return false;
        }
        core::hint::spin_loop();
    }
    true
}

fn wait_drq(ch: u8, iters: u32) -> bool {
    let mut i = 0u32;
    loop {
        let s = status(ch);
        if s & ST_BSY == 0 {
            if s & ST_ERR != 0 {
                return false;
            }
            if s & ST_DRQ != 0 {
                return true;
            }
        }
        i += 1;
        if i >= iters {
            return false;
        }
        core::hint::spin_loop();
    }
}

#[derive(Clone, Copy)]
pub struct IdeDevice {
    pub present: bool,
    pub atapi: bool,
    pub channel: u8,
    pub drive: u8,
    pub lba28: u32,
    pub model: [u8; 40],
}

impl IdeDevice {
    const fn empty() -> Self {
        IdeDevice { present: false, atapi: false, channel: 0, drive: 0, lba28: 0, model: [0; 40] }
    }
}

static mut DEVS: [IdeDevice; 4] = [IdeDevice::empty(); 4];

fn read_id_block(ch: u8, out_words: &mut [u16; 256]) -> bool {
    if !wait_drq(ch, 4_000_000) {
        return false;
    }
    for i in 0..256 {
        out_words[i] = inword(cmd_base(ch) + DATA_OFF);
    }
    true
}

/// IDENTIFY одного привода. Возвращает устройство или None.
fn identify(ch: u8, drive: u8) -> Option<IdeDevice> {
    select(ch, drive);
    unsafe { io::outb(cmd_base(ch) + STATUS_OFF, CMD_IDENTIFY) };
    let s = status(ch);
    if s == 0x00 || s == 0xFF {
        return None; // никого нет
    }
    if !wait_bsy_clear(ch, 4_000_000) {
        return None;
    }
    // Сигнатура ATAPI/SATA в цилиндрах
    let mid = unsafe { io::inb(cmd_base(ch) + LBA_MID_OFF) };
    let hi = unsafe { io::inb(cmd_base(ch) + LBA_HI_OFF) };
    let mut dev = IdeDevice::empty();
    dev.channel = ch;
    dev.drive = drive;
    if mid == 0x14 && hi == 0xEB {
        // ATAPI: нужен IDENTIFY PACKET
        select(ch, drive);
        unsafe { io::outb(cmd_base(ch) + STATUS_OFF, CMD_IDENTIFY_PACKET) };
        if status(ch) == 0x00 || status(ch) == 0xFF {
            return None;
        }
        let mut w = [0u16; 256];
        if !read_id_block(ch, &mut w) {
            return None;
        }
        dev.present = true;
        dev.atapi = true;
        dev.lba28 = 0;
        for i in 0..20 {
            dev.model[i * 2] = (w[27 + i] >> 8) as u8;
            dev.model[i * 2 + 1] = (w[27 + i] & 0xFF) as u8;
        }
        return Some(dev);
    }
    if mid == 0x3C && hi == 0xC3 {
        return None; // SATA на legacy-порту — не наше
    }
    // Пытаемся как ATA
    let st = status(ch);
    if st & ST_ERR != 0 {
        return None;
    }
    let mut w = [0u16; 256];
    if !read_id_block(ch, &mut w) {
        return None;
    }
    dev.present = true;
    dev.atapi = false;
    dev.lba28 = (w[60] as u32) | ((w[61] as u32) << 16);
    for i in 0..20 {
        dev.model[i * 2] = (w[27 + i] >> 8) as u8;
        dev.model[i * 2 + 1] = (w[27 + i] & 0xFF) as u8;
    }
    Some(dev)
}

pub fn devices() -> [IdeDevice; 4] {
    unsafe { DEVS }
}

/// ATA LBA28 PIO-чтение (512-байтные сектора).
pub fn ata_read(dev: &IdeDevice, lba: u32, count: u16, buf: *mut u8) -> bool {
    if dev.atapi || count == 0 {
        return false;
    }
    let ch = dev.channel;
    select(ch, dev.drive);
    if !wait_bsy_clear(ch, 4_000_000) {
        return false;
    }
    unsafe {
        io::outb(cmd_base(ch) + COUNT_OFF, count as u8);
        io::outb(cmd_base(ch) + LBA_LO_OFF, lba as u8);
        io::outb(cmd_base(ch) + LBA_MID_OFF, (lba >> 8) as u8);
        io::outb(cmd_base(ch) + LBA_HI_OFF, (lba >> 16) as u8);
        io::outb(
            cmd_base(ch) + DRIVE_OFF,
            0xE0 | (dev.drive << 4) | ((lba >> 24) as u8 & 0x0F),
        );
        io::outb(cmd_base(ch) + STATUS_OFF, CMD_READ_SECTORS);
    }
    let mut off = 0usize;
    for _ in 0..count {
        if !wait_drq(ch, 4_000_000) {
            return false;
        }
        for _ in 0..256 {
            let w = inword(cmd_base(ch) + DATA_OFF);
            unsafe {
                *(buf.add(off)) = (w & 0xFF) as u8;
                *(buf.add(off + 1)) = (w >> 8) as u8;
            }
            off += 2;
        }
    }
    // Дождаться готовности (снять BSY)
    wait_bsy_clear(ch, 1_000_000)
}

/// ATAPI READ(12) — count 2048-байтных блоков с lba (в 2048-единицах).
pub fn atapi_read(dev: &IdeDevice, lba: u32, count: u16, buf: *mut u8, max: usize) -> bool {
    if !dev.atapi || count == 0 {
        return false;
    }
    if (count as usize) * 2048 > max {
        return false;
    }
    let ch = dev.channel;
    select(ch, dev.drive);
    if !wait_bsy_clear(ch, 4_000_000) {
        return false;
    }
    // features=0, count: PIO mode — max byte count 0 (не лимитируем)
    unsafe {
        io::outb(cmd_base(ch) + 1, 0x00); // FEATURES
        io::outb(cmd_base(ch) + COUNT_OFF, 0x00);
        io::outb(cmd_base(ch) + LBA_MID_OFF, 0x00);
        io::outb(cmd_base(ch) + LBA_HI_OFF, 0x00);
        io::outb(cmd_base(ch) + STATUS_OFF, CMD_PACKET);
    }
    if !wait_drq(ch, 4_000_000) {
        return false;
    }
    // CDB READ(12): A8 [0] LBA-BE(4) LEN-BE(4) [0] [0]
    let cnt32 = count as u32;
    let cdb = [
        0xA8u8,
        0x00,
        (lba >> 24) as u8,
        (lba >> 16) as u8,
        (lba >> 8) as u8,
        lba as u8,
        (cnt32 >> 24) as u8,
        (cnt32 >> 16) as u8,
        (cnt32 >> 8) as u8,
        cnt32 as u8,
        0x00,
        0x00,
    ];
    unsafe {
        for i in 0..6 {
            let w = cdb[i * 2] as u16 | ((cdb[i * 2 + 1] as u16) << 8);
            io::outw(cmd_base(ch) + DATA_OFF, w);
        }
    }
    // Фаза данных: может быть несколько DRQ-порций
    let mut got = 0usize;
    let want = count as usize * 2048;
    loop {
        if !wait_bsy_clear(ch, 4_000_000) {
            return false;
        }
        let s = status(ch);
        if s & ST_ERR != 0 {
            return false;
        }
        if s & ST_DRQ == 0 {
            break; // данных больше нет
        }
        // Сколько байт предлагает устройство (цилиндры)
        let lo = unsafe { io::inb(cmd_base(ch) + LBA_MID_OFF) } as usize;
        let hi = unsafe { io::inb(cmd_base(ch) + LBA_HI_OFF) } as usize;
        let mut n = lo | (hi << 8);
        if n == 0 || n % 2 != 0 {
            n = 2048;
        }
        if got + n > want {
            n = want - got;
        }
        if got + n > max {
            return false;
        }
        for _ in (0..n).step_by(2) {
            let w = inword(cmd_base(ch) + DATA_OFF);
            unsafe {
                *(buf.add(got)) = (w & 0xFF) as u8;
                *(buf.add(got + 1)) = (w >> 8) as u8;
            }
            got += 2;
        }
        if got >= want {
            // Дождаться завершения без DRQ
            if !wait_bsy_clear(ch, 4_000_000) {
                return false;
            }
            break;
        }
    }
    got == want
}

pub fn init() {
    uart::write_str("[IDE] PATA scan (PIO)...\r\n");
    let mut n = 0;
    for ch in 0..2u8 {
        for dr in 0..2u8 {
            if let Some(d) = identify(ch, dr) {
                unsafe { DEVS[(ch * 2 + dr) as usize] = d; }
                n += 1;
                uart::write_str("  IDE");
                uart::putchar(b'0' + ch);
                uart::write_str(if dr == 0 { ".0 " } else { ".1 " });
                uart::write_str(if d.atapi { "ATAPI CD " } else { "ATA disk " });
                uart::write_str("\r\n");
            }
        }
    }
    if n == 0 {
        uart::write_str("[IDE] no drives\r\n");
    }
}

/// Первый ATAPI CD-ROM (для ISO9660). Возвращает индекс в DEVS.
pub fn first_cdrom() -> Option<usize> {
    unsafe {
        for i in 0..4 {
            if DEVS[i].present && DEVS[i].atapi {
                return Some(i);
            }
        }
        None
    }
}

/// Первый ATA-диск (для IDE VDI/HDD).
pub fn first_disk() -> Option<usize> {
    unsafe {
        for i in 0..4 {
            if DEVS[i].present && !DEVS[i].atapi {
                return Some(i);
            }
        }
        None
    }
}

/// Чтение CD: lba/count в 2048-единицах.
pub fn cdrom_read(lba: u32, count: u16, buf: *mut u8, max: usize) -> bool {
    match first_cdrom() {
        Some(i) => unsafe { atapi_read(&DEVS[i], lba, count, buf, max) },
        None => false,
    }
}

pub struct IdeDriver;

impl super::traits::Driver for IdeDriver {
    fn name(&self) -> &'static str { "IDE PATA/ATAPI (PIO)" }
    fn device_type(&self) -> super::traits::DeviceType {
        super::traits::DeviceType::Pci { vendor: 0x8086, device: 0x7010, class: 0x01, subclass: 0x01 }
    }
    fn init(&self) -> super::traits::DriverStatus {
        init();
        super::traits::DriverStatus::Ok
    }
}
