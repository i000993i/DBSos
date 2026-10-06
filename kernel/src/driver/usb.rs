//! USB host probe: детект контроллеров + XHCI capability-dump (read-only).
//!
//! Полный xHCI-драйвер (кольца, слоты, endpoints) — следующий этап;
//! здесь безопасная фаза: PCI-скан, маппинг BAR, чтение CAP-регистров,
//! число портов. Ничего не конфигурируем — железо не трогаем.

use crate::driver::uart;

pub fn probe() {
    uart::write_str("[USB] controllers:\r\n");
    let mut found = 0u32;
    for dev in 0..32u8 {
        for func in 0..8u8 {
            let vendor = crate::driver::pci::read16(0, dev, func, 0x00);
            if vendor == 0xFFFF {
                if func == 0 { break; }
                continue;
            }
            let reg = crate::driver::pci::read32(0, dev, func, 0x08);
            if (reg >> 24) as u8 != 0x0C || ((reg >> 16) & 0xFF) as u8 != 0x03 {
                continue;
            }
            found += 1;
            let progif = ((reg >> 8) & 0xFF) as u8;
            let kind = match progif {
                0x00 => "UHCI",
                0x10 => "OHCI",
                0x20 => "EHCI",
                0x30 => "xHCI",
                _ => "USB-?",
            };
            uart::write_str("  00:");
            hex8(dev);
            uart::write_str(" ");
            uart::write_str(kind);
            uart::write_str("\r\n");
            if progif == 0x30 {
                xhci_caps(dev, func);
            }
            let htype = ((crate::driver::pci::read32(0, dev, func, 0x0C) >> 16) & 0xFF) as u8;
            if func == 0 && htype & 0x80 == 0 { break; }
        }
    }
    if found == 0 {
        uart::write_str("  none (no USB controller on PCI)\r\n");
    }
}

/// XHCI operational dump: CAPLENGTH, VERSION, SLOTS, PORTS. Только чтение.
fn xhci_caps(dev: u8, func: u8) {
    let bar0 = crate::driver::pci::read32(0, dev, func, 0x10);
    if bar0 & 1 != 0 {
        uart::write_str("    xHCI: I/O BAR, skip\r\n");
        return;
    }
    let bt = (bar0 >> 1) & 0x3;
    let mut base = (bar0 & 0xFFFFFFF0) as u64;
    if bt == 2 {
        base |= (crate::driver::pci::read32(0, dev, func, 0x14) as u64) << 32;
    }
    if base == 0 {
        uart::write_str("    xHCI: BAR0=0\r\n");
        return;
    }
    unsafe { let _ = crate::vm::map_mmio(base, 4096); }
    unsafe {
        let caplen = core::ptr::read_volatile(base as *const u8);
        let ver = core::ptr::read_volatile((base + 2) as *const u16);
        let hcs1 = core::ptr::read_volatile((base + 4) as *const u32);
        let slots = (hcs1 & 0xFF) as u32;
        let ports = ((hcs1 >> 24) & 0xFF) as u32;
        uart::write_str("    xHCI caplen=");
        dec(caplen as u64);
        uart::write_str(" ver=");
        hex16(ver);
        uart::write_str(" slots=");
        dec(slots as u64);
        uart::write_str(" ports=");
        dec(ports as u64);
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

pub struct UsbDriver;

impl super::traits::Driver for UsbDriver {
    fn name(&self) -> &'static str { "USB probe (xHCI caps)" }
    fn device_type(&self) -> super::traits::DeviceType { super::traits::DeviceType::Legacy }
    fn init(&self) -> super::traits::DriverStatus {
        probe();
        super::traits::DriverStatus::Ok
    }
}
