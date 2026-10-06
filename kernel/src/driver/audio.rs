//! HDAudio probe: детект Intel HD Audio + read-only dump (GCAP/STATESTS).
//!
//! Полный HDA-драйвер (CORB/RIRB, DMA streams) — следующий этап;
//! здесь безопасная фаза: PCI-скан multimedia-класса, маппинг BAR,
//! чтение GCAP (число streams) и STATESTS (битмаска кодеков).
//! S3/suspend: нужны _S3 SLP_TYPa из AML DSDT — вне скоупа чтения регистров.

use crate::driver::uart;

pub fn probe() {
    uart::write_str("[AUDIO] controllers:\r\n");
    let mut found = 0u32;
    for dev in 0..32u8 {
        for func in 0..8u8 {
            let vendor = crate::driver::pci::read16(0, dev, func, 0x00);
            if vendor == 0xFFFF {
                if func == 0 { break; }
                continue;
            }
            let reg = crate::driver::pci::read32(0, dev, func, 0x08);
            if (reg >> 24) as u8 != 0x04 {
                continue;
            }
            found += 1;
            let subclass = ((reg >> 16) & 0xFF) as u8;
            let kind = match subclass {
                0x01 => "AC97",
                0x03 => "HDA",
                _ => "audio-?",
            };
            uart::write_str("  00:");
            hex8(dev);
            uart::write_str(" ");
            uart::write_str(kind);
            uart::write_str("\r\n");
            if subclass == 0x03 {
                hda_dump(dev, func);
            }
            let htype = ((crate::driver::pci::read32(0, dev, func, 0x0C) >> 16) & 0xFF) as u8;
            if func == 0 && htype & 0x80 == 0 { break; }
        }
    }
    if found == 0 {
        uart::write_str("  none (no audio controller on PCI)\r\n");
    }
}

/// HDA GCAP + STATESTS, только чтение.
fn hda_dump(dev: u8, func: u8) {
    let bar0 = crate::driver::pci::read32(0, dev, func, 0x10);
    if bar0 & 1 != 0 {
        uart::write_str("    HDA: I/O BAR, skip\r\n");
        return;
    }
    let bt = (bar0 >> 1) & 0x3;
    let mut base = (bar0 & 0xFFFFFFF0) as u64;
    if bt == 2 {
        base |= (crate::driver::pci::read32(0, dev, func, 0x14) as u64) << 32;
    }
    if base == 0 {
        uart::write_str("    HDA: BAR0=0\r\n");
        return;
    }
    unsafe { let _ = crate::vm::map_mmio(base, 16384); }
    unsafe {
        let gcap = core::ptr::read_volatile(base as *const u16);
        let statests = core::ptr::read_volatile((base + 0x0E) as *const u16);
        let oss = ((gcap >> 12) & 0xF) as u32; // output streams
        let iss = ((gcap >> 8) & 0xF) as u32; // input streams
        uart::write_str("    HDA GCAP=0x");
        hex16(gcap);
        uart::write_str(" out-streams=");
        dec(oss as u64);
        uart::write_str(" in-streams=");
        dec(iss as u64);
        uart::write_str(" codecs=0x");
        hex16(statests);
        uart::write_str("\r\n");
    }
}

fn hex8(v: u8) {
    uart::putchar(b"0123456789ABCDEF"[(v >> 4) as usize]);
    uart::putchar(b"0123456789ABCDEF"[(v & 0xF) as usize]);
}
fn hex16(v: u16) {
    hex8((v >> 8) as u8);
    hex8(v as u8);
}
fn dec(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut b = [0u8; 20];
    let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putchar(b[i]); }
}

pub struct AudioDriver;

impl super::traits::Driver for AudioDriver {
    fn name(&self) -> &'static str { "Audio probe (HDA caps)" }
    fn device_type(&self) -> super::traits::DeviceType { super::traits::DeviceType::Legacy }
    fn init(&self) -> super::traits::DriverStatus {
        probe();
        super::traits::DriverStatus::Ok
    }
}
