//! block — единый слой блочных устройств.
//!
//! Порядок приоритета: NVMe -> AHCI/SATA -> IDE ATA -> virtio-blk.
//! FAT/ext4/ISO вызывают только `read_sector/write_sector` отсюда,
//! а не напрямую `nvme::`/`ahci::`. Это чинит "FAT mount не находит FAT
//! на SATA/NVMe": раньше `fat_driver` проверял только NVMe, иначе AHCI,
//! игнорируя IDE и случай когда оба живы.
//!
//! Все буферы — 512-байтные сектора. IDE PIO читает напрямую,
//! NVMe/AHCI — через DMA (требуют phys-буфер, выделяется внутри).

use crate::driver::uart;

fn up(s: &str) { uart::write_str(s); }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    None,
    Nvme,
    Ahci,
    Ide,
}

static mut ACTIVE: Backend = Backend::None;
static mut SECTOR_SIZE: u16 = 512;

pub fn active() -> Backend { unsafe { ACTIVE } }

/// Выбрать лучший доступный backend. Вызывать после `driver::init()`.
pub fn probe() {
    unsafe {
        // NVMe готов и нашёл FAT?
        if crate::driver::nvme::FS_INIT {
            ACTIVE = Backend::Nvme;
            SECTOR_SIZE = 512;
            up("[BLOCK] backend=nvme\r\n");
            return;
        }
        // AHCI поднял порт?
        if crate::driver::ahci::is_ready() {
            ACTIVE = Backend::Ahci;
            SECTOR_SIZE = 512;
            up("[BLOCK] backend=ahci\r\n");
            return;
        }
        // IDE ATA диск?
        if crate::driver::ide::first_disk().is_some() {
            ACTIVE = Backend::Ide;
            SECTOR_SIZE = 512;
            up("[BLOCK] backend=ide\r\n");
            return;
        }
        ACTIVE = Backend::None;
        up("[BLOCK] no block device (cdrom-only?)\r\n");
    }
}

/// Принудительно переключиться (для `disk use nvme|ahci|ide`).
pub fn set_backend(b: Backend) -> bool {
    unsafe {
        match b {
            Backend::Nvme if crate::driver::nvme::FS_INIT => { ACTIVE = b; true }
            Backend::Ahci if crate::driver::ahci::is_ready() => { ACTIVE = b; true }
            Backend::Ide if crate::driver::ide::first_disk().is_some() => { ACTIVE = b; true }
            Backend::None => { ACTIVE = b; true }
            _ => false,
        }
    }
}

pub fn backend_name() -> &'static str {
    match unsafe { ACTIVE } {
        Backend::Nvme => "nvme",
        Backend::Ahci => "ahci/sata",
        Backend::Ide => "ide/pata",
        Backend::None => "none",
    }
}

/// Прочитать `count` 512-байтных секторов с абсолютного LBA диска.
pub fn read_sectors(lba: u64, count: u16, buf: *mut u8) -> bool {
    if buf.is_null() || count == 0 { return false; }
    match unsafe { ACTIVE } {
        Backend::Nvme => {
            // NVMe требует phys-буфер страницей; buf у нас обычно palloc — ок.
            // Читаем посекторно чтобы не упираться в PRP-лимиты.
            for i in 0..count as u64 {
                let dst = unsafe { buf.add((i * 512) as usize) };
                if !crate::driver::nvme::read_sectors(lba + i, 1, dst) { return false; }
            }
            true
        }
        Backend::Ahci => {
            for i in 0..count as u64 {
                let dst = unsafe { buf.add((i * 512) as usize) };
                if !crate::driver::ahci::read_sectors(lba + i, 1, dst) { return false; }
            }
            true
        }
        Backend::Ide => {
            match crate::driver::ide::first_disk().map(|i| crate::driver::ide::devices()[i]) {
                Some(dev) => {
                    // IDE LBA28: делим большие запросы на чанки по 255
                    let mut left = count as u32;
                    let mut cur = lba;
                    let mut off = 0usize;
                    while left > 0 {
                        let n = left.min(255) as u16;
                        if cur > 0x0FFF_FFFF { return false; }
                        if !crate::driver::ide::ata_read(&dev, cur as u32, n, unsafe { buf.add(off) }) {
                            return false;
                        }
                        left -= n as u32;
                        cur += n as u64;
                        off += n as usize * 512;
                    }
                    true
                }
                None => false,
            }
        }
        Backend::None => {
            // Последний шанс: если backend не выбран, пробуем по очереди
            // (нужно на раннем boot когда probe ещё не вызван).
            if unsafe { crate::driver::nvme::FS_INIT } {
                unsafe { ACTIVE = Backend::Nvme; }
                return read_sectors(lba, count, buf);
            }
            if crate::driver::ahci::is_ready() {
                unsafe { ACTIVE = Backend::Ahci; }
                return read_sectors(lba, count, buf);
            }
            if crate::driver::ide::first_disk().is_some() {
                unsafe { ACTIVE = Backend::Ide; }
                return read_sectors(lba, count, buf);
            }
            false
        }
    }
}

/// Записать `count` секторов. Возвращает false на read-only/отсутствии.
pub fn write_sectors(lba: u64, count: u16, buf: *const u8) -> bool {
    if buf.is_null() || count == 0 { return false; }
    match unsafe { ACTIVE } {
        Backend::Nvme => {
            for i in 0..count as u64 {
                let src = unsafe { buf.add((i * 512) as usize) } as *mut u8;
                if !crate::driver::nvme::write_sectors(lba + i, 1, src) { return false; }
            }
            true
        }
        Backend::Ahci => {
            // AHCI write требует phys-копию внутри write_fat_sector; для
            // произвольного буфера копируем посекторно через palloc.
            for i in 0..count as u64 {
                let phys = crate::memory::palloc();
                if phys == 0 { return false; }
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        buf.add((i * 512) as usize), phys as *mut u8, 512);
                }
                let ok = crate::driver::ahci::write_sectors(lba + i, 1, phys as *mut u8);
                crate::memory::pfree(phys);
                if !ok { return false; }
            }
            true
        }
        Backend::Ide => false, // IDE PIO-write не реализован — честно read-only
        Backend::None => false,
    }
}

/// Удобные обёртки для FAT: LBA относительно начала FAT-партиции.
pub fn read_fat_sector(part_lba: u64, lba: u64, buf: &mut [u8; 512]) -> bool {
    read_sectors(part_lba + lba, 1, buf.as_mut_ptr())
}

pub fn write_fat_sector(part_lba: u64, lba: u64, buf: &[u8; 512]) -> bool {
    write_sectors(part_lba + lba, 1, buf.as_ptr())
}

/// Список устройств для `disk list`.
pub fn list() {
    up("[BLOCK] devices:\r\n");
    up("  * active: ");
    up(backend_name());
    up("\r\n");
    unsafe {
        up("  nvme: ");
        up(if crate::driver::nvme::FS_INIT { "ready" } else { "absent" });
        up("\r\n  ahci: ");
        up(if crate::driver::ahci::is_ready() { "ready" } else { "absent" });
        up("\r\n  ide: ");
        let devs = crate::driver::ide::devices();
        let mut n = 0;
        for d in devs {
            if d.present && !d.atapi { n += 1; }
        }
        if n == 0 { up("absent"); } else {
            up("ata disks=");
            uart::putchar(b'0' + n.min(9) as u8);
        }
        up("\r\n  cdrom: ");
        up(if crate::driver::ide::first_cdrom().is_some() { "present" } else { "absent" });
        up("\r\n");
    }
}
