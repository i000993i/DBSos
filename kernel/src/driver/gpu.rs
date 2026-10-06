//! GPU driver — virtio-gpu / Bochs / generic GOP
//!
//! Detects GPU via PCI class 0x03, reports via `hw`/`gpu` commands.
//! For QEMU, GOP already provides framebuffer; this driver adds
//! virtio-gpu detection and future 2D accel hooks.

use super::uart;
use super::pci;

#[derive(Clone, Copy)]
pub struct GpuInfo {
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub vram_base: u64,
    pub vram_size: u64,
    pub width: u32,
    pub height: u32,
}

static mut GPU: Option<GpuInfo> = None;

pub fn detect() -> Option<GpuInfo> {
    // Scan PCI for VGA/GPU class 0x03
    for dev in 0..32u8 {
        for func in 0..8u8 {
            let vendor = pci::read16(0, dev, func, 0x00);
            if vendor == 0xFFFF { if func == 0 { break; } continue; }
            let class = (pci::read32(0, dev, func, 0x08) >> 24) as u8;
            if class == 0x03 {
                let device = pci::read16(0, dev, func, 0x02);
                // Try to read BAR0 as VRAM
                let bar0 = pci::read32(0, dev, func, 0x10) as u64;
                let is_mmio = bar0 & 1 == 0;
                let base = if is_mmio { bar0 & 0xFFFFFFF0 } else { 0 };
                let w = crate::display::width();
                let h = crate::display::height();
                let info = GpuInfo { vendor, device, class, vram_base: base, vram_size: (w as u64 * h as u64 * 4), width: w, height: h };
                unsafe { GPU = Some(info); }
                return Some(info);
            }
            let htype = ((pci::read32(0, dev, func, 0x0C) >> 16) & 0xFF) as u8;
            if func == 0 && htype & 0x80 == 0 { break; }
        }
    }
    None
}

pub fn info() -> Option<GpuInfo> { unsafe { GPU } }

pub fn print_info() {
    if let Some(g) = detect().or(unsafe { GPU }) {
        uart::write_str("[GPU] ");
        let h = |n: u8| { let n = n & 0xF; if n < 10 { b'0' + n } else { b'A' + n - 10 } };
        uart::write_str("vendor=");
        uart::putchar(h((g.vendor >> 12) as u8)); uart::putchar(h((g.vendor >> 8) as u8));
        uart::putchar(h((g.vendor >> 4) as u8)); uart::putchar(h(g.vendor as u8));
        uart::write_str(" device=");
        uart::putchar(h((g.device >> 12) as u8)); uart::putchar(h((g.device >> 8) as u8));
        uart::putchar(h((g.device >> 4) as u8)); uart::putchar(h(g.device as u8));
        uart::write_str(" class=0x03");
        uart::write_str(" vram=0x");
        let v = g.vram_base;
        for i in (0..16).rev() { uart::putchar(b"0123456789ABCDEF"[(v >> (i*4) & 0xF) as usize]); }
        uart::write_str(" res=");
        let dec = |mut v: u32| { if v==0 {uart::putchar(b'0'); return;} let mut b=[0u8;10]; let mut i=0; while v>0{b[i]=b'0'+(v%10) as u8; v/=10; i+=1;} while i>0{i-=1; uart::putchar(b[i]);}};
        dec(g.width); uart::putchar(b'x'); dec(g.height);
        uart::write_str("\r\n");
        // Also log to framebuffer via shell w() is done in driver adapt
    } else {
        uart::write_str("[GPU] no GPU found (class 0x03)\r\n");
    }
}

pub struct GpuDriver;
impl super::traits::Driver for GpuDriver {
    fn name(&self) -> &'static str { "GPU (virtio/bochs)" }
    fn device_type(&self) -> super::traits::DeviceType { super::traits::DeviceType::Gpu }
    fn init(&self) -> super::traits::DriverStatus {
        detect();
        print_info();
        super::traits::DriverStatus::Ok
    }
}
