//! Adaptive driver manager — определяет железо и скачивает драйверы
//!
//! Сканирует PCI, сопоставляет с KNOWN_DRIVERS, для неизвестных предлагает `pkg install`.

use super::uart;
use super::pci;

#[derive(Clone, Copy, PartialEq)]
pub enum State { Builtin, Detected, Missing }

pub struct Known {
    pub vendor: u16,
    pub device: u16,
    pub name: &'static str,
    pub pkg: &'static str, // pkg name for `pkg install`
    pub desc: &'static str,
}

// Таблица известных устройств -> драйвер. Расширяется без перекомпиляции через /etc/drivers.db
pub const KNOWN: &[Known] = &[
    Known { vendor: 0x8086, device: 0x100E, name: "e1000", pkg: "driver-e1000", desc: "Intel 82540EM Gigabit Ethernet" },
    Known { vendor: 0x8086, device: 0x10D3, name: "e1000e", pkg: "driver-e1000e", desc: "Intel 82574L Gigabit" },
    Known { vendor: 0x10EC, device: 0x8139, name: "rtl8139", pkg: "driver-rtl8139", desc: "Realtek RTL8139 Fast Ethernet" },
    Known { vendor: 0x1B36, device: 0x0010, name: "nvme", pkg: "driver-nvme", desc: "QEMU NVMe Ctrl" },
    Known { vendor: 0x8086, device: 0x2922, name: "ahci", pkg: "driver-ahci", desc: "Intel ICH9 AHCI" },
    Known { vendor: 0x1234, device: 0x1111, name: "bochs-vga", pkg: "driver-bochs", desc: "Bochs VGA" },
    Known { vendor: 0x1AF4, device: 0x1000, name: "virtio-net", pkg: "driver-virtio", desc: "VirtIO Network" },
    Known { vendor: 0x1AF4, device: 0x1001, name: "virtio-blk", pkg: "driver-virtio-blk", desc: "VirtIO Block" },
    Known { vendor: 0x1AF4, device: 0x1041, name: "virtio-gpu", pkg: "driver-virtio-gpu", desc: "VirtIO GPU" },
    Known { vendor: 0x1AF4, device: 0x1042, name: "virtio-input", pkg: "driver-virtio-input", desc: "VirtIO Input" },
    Known { vendor: 0x8086, device: 0x2930, name: "smbus", pkg: "driver-smbus", desc: "Intel ICH9 SMBus (no driver yet)" },
    Known { vendor: 0x10DE, device: 0x1C82, name: "nouveau", pkg: "driver-nouveau", desc: "NVIDIA GP107" },
    Known { vendor: 0x1002, device: 0x67DF, name: "amdgpu", pkg: "driver-amdgpu", desc: "AMD Ellesmere" },
    Known { vendor: 0x8086, device: 0x5912, name: "i915", pkg: "driver-i915", desc: "Intel HD Graphics 630" },
    Known { vendor: 0x8086, device: 0x2668, name: "hda", pkg: "driver-hda", desc: "Intel HD Audio (no driver yet)" },
];

fn find_known(vendor: u16, device: u16) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.vendor == vendor && k.device == device)
}

pub fn status_for(vendor: u16, device: u16, class: u8, subclass: u8) -> (State, &'static str) {
    if let Some(k) = find_known(vendor, device) {
        // Builtin drivers already loaded: e1000, rtl8139, nvme, ahci, bochs
        let builtin = matches!(k.name, "e1000" | "e1000e" | "rtl8139" | "nvme" | "ahci" | "bochs-vga");
        if builtin { (State::Builtin, k.name) } else { (State::Missing, k.name) }
    } else {
        // Generic class fallback (с учётом subclass чтобы не путать USB и SMBus)
        match (class, subclass) {
            (0x01, _) => (State::Missing, "storage"),
            (0x02, _) => (State::Missing, "network"),
            (0x03, _) => (State::Missing, "gpu"),
            (0x04, _) => (State::Missing, "audio-hda"),
            (0x0C, 0x03) => (State::Missing, "usb-xhci"),
            (0x0C, 0x05) => (State::Missing, "smbus"),
            (0x0C, _) => (State::Missing, "serial-bus"),
            _ => (State::Missing, "unknown"),
        }
    }
}

fn hex_nib(n: u8) -> u8 { let n = n & 0xF; if n < 10 { b'0' + n } else { b'A' + n - 10 } }
fn hex8(v: u8) { uart::putchar(hex_nib(v >> 4)); uart::putchar(hex_nib(v)); }
fn hex16(v: u16) {
    uart::putchar(hex_nib((v >> 12) as u8)); uart::putchar(hex_nib((v >> 8) as u8));
    uart::putchar(hex_nib((v >> 4) as u8)); uart::putchar(hex_nib(v as u8));
}

pub fn print_report() {
    uart::write_str("=== Adaptive Drivers ===\r\n");
    // Header for TUI (uart) — shell will also render to framebuffer via its own w()
    for dev in 0..32u8 {
        for func in 0..8u8 {
            let vendor = pci::read16(0, dev, func, 0x00);
            if vendor == 0xFFFF { if func == 0 { break; } continue; }
            let device = pci::read16(0, dev, func, 0x02);
            let reg08 = pci::read32(0, dev, func, 0x08);
            let class = (reg08 >> 24) as u8;
            let subclass = ((reg08 >> 16) & 0xFF) as u8;
            let (st, name) = status_for(vendor, device, class, subclass);
            let tag = match st { State::Builtin => "builtin", State::Detected => "loaded", State::Missing => "fetch" };
            uart::write_str("  00:");
            hex8(dev); uart::putchar(b'.'); hex8(func);
            uart::putchar(b' ');
            hex16(vendor); uart::putchar(b':'); hex16(device);
            uart::write_str(" ");
            uart::write_str(name);
            uart::write_str(" ["); uart::write_str(tag); uart::write_str("]\r\n");
            if st == State::Missing {
                if let Some(k) = find_known(vendor, device) {
                    uart::write_str("    -> pkg install "); uart::write_str(k.pkg); uart::write_str("\r\n");
                } else {
                    uart::write_str("    -> no driver yet, class 0x");
                    hex8(class); uart::putchar(b':'); hex8(subclass);
                    uart::write_str("\r\n");
                }
            }
            let htype = ((pci::read32(0, dev, func, 0x0C) >> 16) & 0xFF) as u8;
            if func == 0 && htype & 0x80 == 0 { break; }
        }
    }
}

pub fn fetch(pkg: &[u8]) -> bool {
    // Delegates to pkg manager (network). For builtin, no fetch needed.
    // pkg::pkg_main expects "install NAME"
    let mut cmd = [0u8; 64];
    let pre = b"install ";
    if pkg.len() + pre.len() >= cmd.len() { return false; }
    cmd[..pre.len()].copy_from_slice(pre);
    cmd[pre.len()..pre.len()+pkg.len()].copy_from_slice(pkg);
    let len = pre.len() + pkg.len();
    crate::pkg::pkg_main(&cmd[..len]);
    true
}

/// Автовыбор активной сетевой карты после инициализации всех драйверов.
/// Политика: e1000 primary (поведение по умолчанию сохранено); если e1000
/// отсутствует, а RTL8139 готов — primary становится RTL8139 (MAC стека
/// переключается на него, IP остаётся дефолтным до DHCP).
/// Вызывать из lib.rs после print_report().
pub fn auto_select() {
    let e1000_ok = crate::driver::net::mac() != [0; 6];
    let rtl_ok = crate::driver::rtl8139::is_ready();
    if e1000_ok {
        crate::driver::net::set_active_nic(crate::driver::net::NIC_E1000);
        uart::write_str("[ADAPT] primary NIC: e1000");
        if rtl_ok {
            uart::write_str(" (rtl8139 standby, `net use rtl8139`)");
        }
        uart::write_str("\r\n");
    } else if rtl_ok {
        crate::driver::net::set_active_nic(crate::driver::net::NIC_RTL8139);
        crate::driver::net::set_mac(crate::driver::rtl8139::mac());
        uart::write_str("[ADAPT] primary NIC: rtl8139 (e1000 absent, MAC switched)\r\n");
    } else {
        uart::write_str("[ADAPT] no wired NIC ready\r\n");
    }
    // Wi-Fi итог: честно фиксируем наличие/отсутствие радио.
    let wifi = crate::driver::wifi::scan();
    if wifi == 0 {
        uart::write_str("[ADAPT] wifi: no radio — wired profiles only\r\n");
    }
}
