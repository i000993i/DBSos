/// PCI Configuration Space Access + Enumeration + MMIO validation

use crate::io;
use super::traits::*;
use super::uart;

const CONFIG_ADDR: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

/// Describes a valid MMIO region from a PCI BAR
#[derive(Clone, Copy)]
pub struct MmioRegion {
    pub base: u64,
    pub size: u64,
}

pub const MAX_MMIO_REGIONS: usize = 64;
pub static mut MMIO_REGIONS: [MmioRegion; MAX_MMIO_REGIONS] = [MmioRegion { base: 0, size: 0 }; MAX_MMIO_REGIONS];
pub static mut MMIO_REGION_COUNT: usize = 0;

fn add_mmio_region(base: u64, size: u64) {
    if size == 0 { return; }
    unsafe {
        let idx = MMIO_REGION_COUNT;
        if idx < MAX_MMIO_REGIONS {
            MMIO_REGIONS[idx] = MmioRegion { base, size };
            MMIO_REGION_COUNT = idx + 1;
        }
    }
}

/// Check if a physical address range falls within a known PCI MMIO region
pub fn validate_mmio(phys: u64, size: u64) -> bool {
    let end = phys.saturating_add(size);
    // Use a single unsafe block for static mut access
    let safe = unsafe {
        (0..MMIO_REGION_COUNT).any(|i| {
            let r = &MMIO_REGIONS[i];
            phys >= r.base && end <= r.base.saturating_add(r.size)
        })
    };
    if !safe {
        uart::write_str("[PCI] MMIO deny phys=");
        let hex = b"0123456789ABCDEF";
        uart::putchar(hex[((phys >> 28) & 0xF) as usize]);
        uart::putchar(hex[((phys >> 24) & 0xF) as usize]);
        uart::putchar(hex[((phys >> 20) & 0xF) as usize]);
        uart::putchar(hex[((phys >> 16) & 0xF) as usize]);
        uart::putchar(hex[((phys >> 12) & 0xF) as usize]);
        uart::putchar(hex[((phys >> 8) & 0xF) as usize]);
        uart::putchar(hex[((phys >> 4) & 0xF) as usize]);
        uart::putchar(hex[(phys & 0xF) as usize]);
        uart::write_str("\r\n");
    }
    safe
}

pub struct PciBusDriver;

impl Driver for PciBusDriver {
    fn name(&self) -> &'static str {
        "PCI Bus (enum)"
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Legacy
    }

    fn init(&self) -> DriverStatus {
        enumerate();
        DriverStatus::Ok
    }
}

pub fn read32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    let addr = 0x8000_0000u32
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xFC);
    unsafe {
        io::outl(CONFIG_ADDR, addr);
        io::inl(CONFIG_DATA)
    }
}

pub fn write32(bus: u8, dev: u8, func: u8, offset: u8, val: u32) {
    let addr = 0x8000_0000u32
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xFC);
    unsafe {
        io::outl(CONFIG_ADDR, addr);
        io::outl(CONFIG_DATA, val);
    }
}

pub fn read16(bus: u8, dev: u8, func: u8, offset: u8) -> u16 {
    let val = read32(bus, dev, func, offset);
    (val >> ((offset as u32 & 2) * 8)) as u16
}

fn hex_byte(v: u8) -> (u8, u8) {
    let hi = if v >> 4 < 10 { b'0' + (v >> 4) } else { b'A' + (v >> 4) - 10 };
    let lo = if v & 0xF < 10 { b'0' + (v & 0xF) } else { b'A' + (v & 0xF) - 10 };
    (hi, lo)
}

fn class_name(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (0x01, 0x00) => "SCSI",
        (0x01, 0x01) => "IDE",
        (0x01, 0x06) => "SATA",
        (0x01, 0x08) => "NVMe",
        (0x02, 0x00) => "Ethernet",
        (0x03, 0x00) => "VGA/GPU",
        (0x04, 0x01) => "Audio",
        (0x06, 0x00) => "Host Bridge",
        (0x06, 0x04) => "PCI-PCI Bridge",
        (0x0C, 0x03) => "USB",
        (0x0C, 0x05) => "SMBus",
        (0x08, 0x00) => "PIC",
        (0x08, 0x01) => "PIT",
        _ => "Other",
    }
}

fn scan_bars(bus: u8, dev: u8, func: u8) {
    let mut bar_num = 0;
    while bar_num < 6 {
        let offset = 0x10 + bar_num * 4;
        let bar = read32(bus, dev, func, offset);
        if bar != 0 && bar & 1 == 0 {
            let bar_type = (bar >> 1) & 0x3;
            if bar_type == 0 {
                // 32-bit memory BAR
                let base = (bar & 0xFFFFFFF0) as u64;
                write32(bus, dev, func, offset, 0xFFFFFFFF);
                let probe = read32(bus, dev, func, offset);
                write32(bus, dev, func, offset, bar);
                let size = (!(probe & 0xFFFFFFF0) as u64).wrapping_add(1);
                add_mmio_region(base, size);
            } else if bar_type == 2 {
                // 64-bit memory BAR (type == 2) — two consecutive registers
                let bar_hi = read32(bus, dev, func, offset + 4);
                let base = ((bar & 0xFFFFFFF0) as u64) | ((bar_hi as u64) << 32);
                write32(bus, dev, func, offset, 0xFFFFFFFF);
                write32(bus, dev, func, offset + 4, 0xFFFFFFFF);
                let probe_lo = read32(bus, dev, func, offset);
                let probe_hi = read32(bus, dev, func, offset + 4);
                write32(bus, dev, func, offset, bar);
                write32(bus, dev, func, offset + 4, bar_hi);
                let probe = ((probe_lo & 0xFFFFFFF0) as u64) | ((probe_hi as u64) << 32);
                let size = (!probe).wrapping_add(1);
                if size > 0 {
                    add_mmio_region(base, size);
                }
                bar_num += 1; // skip next slot — already consumed
            }
        }
        bar_num += 1;
    }
}

fn enumerate() {
    uart::write_str("[PCI] Scanning bus 0...\r\n");

    for dev in 0..32 {
        for func in 0..8 {
            let vendor = read16(0, dev as u8, func as u8, 0x00);
            if vendor == 0xFFFF {
                if func == 0 { break; }
                continue;
            }

            let device = read16(0, dev as u8, func as u8, 0x02);
            let reg08 = read32(0, dev as u8, func as u8, 0x08);
            let class = ((reg08 >> 24) & 0xFF) as u8;
            let subclass = ((reg08 >> 16) & 0xFF) as u8;
            let htype = ((read32(0, dev as u8, func as u8, 0x0C) >> 16) & 0xFF) as u8;

            let (hb1, hb2) = hex_byte(dev as u8);
            let hf = hex_byte(func as u8);

            uart::write_str("  ");
            uart::putchar(b'0'); uart::putchar(b'0'); uart::putchar(b':');
            uart::putchar(hb1); uart::putchar(hb2); uart::putchar(b'.'); uart::putchar(hf.1);
            uart::write_str(" ");
            uart::putchar(hex_nib(vendor >> 12)); uart::putchar(hex_nib(vendor >> 8));
            uart::putchar(hex_nib(vendor >> 4)); uart::putchar(hex_nib(vendor));
            uart::putchar(b':');
            uart::putchar(hex_nib(device >> 12)); uart::putchar(hex_nib(device >> 8));
            uart::putchar(hex_nib(device >> 4)); uart::putchar(hex_nib(device));
            uart::write_str(" ");
            uart::write_str(class_name(class, subclass));
            uart::write_str("\r\n");

            scan_bars(0, dev as u8, func as u8);

            if func == 0 && htype & 0x80 == 0 {
                break;
            }
        }
    }

    // Log MMIO regions discovered
    unsafe {
        uart::write_str("[PCI] MMIO regions: ");
        uart::putchar(b'0' + MMIO_REGION_COUNT as u8);
        uart::write_str("\r\n");
    }
}

fn hex_nib(v: u16) -> u8 {
    let n = (v & 0xF) as u8;
    if n < 10 { b'0' + n } else { b'A' + n - 10 }
}
