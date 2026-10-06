//! VirtIO/XHCI probe — детект железа без риска для boot.
//!
//! Полных virtio-очередей тут нет (это отдельный большой драйвер),
//! но для "полного взаимодействия с железом" нужно хотя бы:
//!  - найти все virtio PCI (1AF4:1000-1042) и USB XHCI (class 0C03)
//!  - включить BusMaster+Mem, зачитать BAR0, зарезервировать MMIO через vm::map_mmio
//!  - отчитаться в UART чтобы adapt/pkg знали что ставить.

use super::uart;
use super::pci;

fn hex16(v: u16) {
    let h = b"0123456789ABCDEF";
    uart::putchar(h[((v >> 12) & 0xF) as usize]);
    uart::putchar(h[((v >> 8) & 0xF) as usize]);
    uart::putchar(h[((v >> 4) & 0xF) as usize]);
    uart::putchar(h[(v & 0xF) as usize]);
}

fn virtio_name(device: u16) -> &'static str {
    match device {
        0x1000 => "virtio-net",
        0x1001 => "virtio-blk",
        0x1002 => "virtio-balloon",
        0x1003 => "virtio-console",
        0x1004 => "virtio-scsi",
        0x1005 => "virtio-rng",
        0x1009 => "virtio-9p",
        0x1041 => "virtio-gpu",
        0x1042 => "virtio-input",
        _ => "virtio-unknown",
    }
}

pub fn probe() {
    let mut found = 0;
    for dev in 0..32u8 {
        for func in 0..8u8 {
            let vendor = pci::read16(0, dev, func, 0x00);
            if vendor == 0xFFFF { if func == 0 { break; } continue; }
            let device = pci::read16(0, dev, func, 0x02);
            let class = (pci::read32(0, dev, func, 0x08) >> 24) as u8;
            let subclass = ((pci::read32(0, dev, func, 0x08) >> 16) & 0xFF) as u8;
            let is_virtio = vendor == 0x1AF4 && (0x1000..=0x1042).contains(&device);
            let is_xhci = class == 0x0C && subclass == 0x03;
            if !is_virtio && !is_xhci { continue; }
            found += 1;
            uart::write_str("[virtio-probe] ");
            if is_virtio { uart::write_str(virtio_name(device)); } else { uart::write_str("xhci-usb"); }
            uart::write_str(" @00:");
            uart::putchar(b'0' + (dev / 10));
            uart::putchar(b'0' + (dev % 10));
            uart::write_str(" vid=0x"); hex16(vendor);
            uart::write_str(" did=0x"); hex16(device);
            uart::write_str("\r\n");
            // Включить MEM+BusMaster чтобы BAR читался корректно
            let cmd = pci::read16(0, dev, func, 0x04);
            pci::write32(0, dev, func, 0x04, (cmd | 0x6) as u32);
            // BAR0 probe + reserve (без записи 0xFFFFFFFF чтобы не трогать устройство)
            let bar0 = pci::read32(0, dev, func, 0x10);
            if bar0 & 1 == 0 && bar0 & 0xFFFFFFF0 != 0 {
                let base = (bar0 & 0xFFFFFFF0) as u64;
                // Резервируем 4K для будущего драйвера; ошибки игнорим (ранний boot)
                unsafe { let _ = crate::vm::map_mmio(base, 4096); }
            }
            let htype = ((pci::read32(0, dev, func, 0x0C) >> 16) & 0xFF) as u8;
            if func == 0 && htype & 0x80 == 0 { break; }
        }
    }
    if found == 0 {
        uart::write_str("[virtio-probe] none\r\n");
    }
}

pub struct VirtioProbeDriver;

impl super::traits::Driver for VirtioProbeDriver {
    fn name(&self) -> &'static str { "virtio-probe" }
    fn device_type(&self) -> super::traits::DeviceType { super::traits::DeviceType::Legacy }
    fn init(&self) -> super::traits::DriverStatus {
        probe();
        super::traits::DriverStatus::Ok
    }
}
