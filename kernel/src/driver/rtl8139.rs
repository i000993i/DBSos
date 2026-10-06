//! Realtek RTL8139 Fast Ethernet (PCI 10EC:8139) — вторая проводная карта.
//!
//! PIO-доступ (BAR0 = I/O ports), poll-режим без IRQ (как e1000 poll()).
//! Верхний стек общий: TX идёт через net::tx_route(), RX — через
//! net::input_frame(). Активная карта выбирается адаптером
//! (e1000 primary, RTL8139 fallback): `net use rtl8139`.

use crate::driver::uart;
use crate::io;

const VENDOR: u16 = 0x10EC;
const DEVICE: u16 = 0x8139;

// Регистры (смещение от IO base)
const REG_IDR0: u16 = 0x00; // MAC 6 байт
const REG_TSD0: u16 = 0x10; // TX status 4 x u32 (шаг 4)
const REG_TSAD0: u16 = 0x20; // TX addr 4 x u32 (шаг 4)
const REG_RBSTART: u16 = 0x30; // RX ring phys u32
const REG_CR: u16 = 0x37; // Command u8: RST=0x10 RE=0x08 TE=0x04
const REG_CAPR: u16 = 0x38; // Current read addr u16
const REG_CBR: u16 = 0x3A; // Current buffer addr u16 (только чтение)
const REG_IMR: u16 = 0x3C; // Interrupt mask u16
const REG_ISR: u16 = 0x3E; // Interrupt status u16 (W1C)
// TCR (0x40) оставлен на reset-значениях — дефолт достаточен для QEMU.
const REG_RCR: u16 = 0x44; // RX config u32
const REG_9346CR: u16 = 0x50; // Config9346 u8: unlock=0xC0
const REG_CONFIG1: u16 = 0x52; // Config1 u8: power on=0x00

const CR_RST: u8 = 0x10;
const CR_RE: u8 = 0x08;
const CR_TE: u8 = 0x04;

const TSD_OWN: u32 = 1 << 13;
const TSD_TOK: u32 = 1 << 15;

const ISR_ROK: u16 = 0x01;

const RCR_ACCEPT_ALL: u32 = 0x0F; // AAP|APM|AM|AB: всё + broadcast + multicast

const RX_RING_LEN: usize = 8192;
const TX_DESC_COUNT: usize = 4;
const TX_BUF_LEN: usize = 1536;
const MAX_FRAME: usize = 1514;

static mut IO_BASE: u16 = 0;
static mut READY: bool = false;
static mut MAC: [u8; 6] = [0; 6];
static mut RX_PHYS: u64 = 0;
static mut RX_OFF: usize = 0;
static mut TX_PHYS: [u64; TX_DESC_COUNT] = [0; TX_DESC_COUNT];
static mut TX_NEXT: usize = 0;
static mut RX_BOUNCE: [u8; MAX_FRAME] = [0; MAX_FRAME];

fn inw(port: u16) -> u16 {
    unsafe { io::inb(port) as u16 | ((io::inb(port + 1) as u16) << 8) }
}

fn find_rtl() -> bool {
    for dev in 0..32 {
        for func in 0..8 {
            let v = super::pci::read16(0, dev as u8, func as u8, 0);
            if v == 0xFFFF {
                if func == 0 { break; }
                continue;
            }
            if v != VENDOR { continue; }
            let d = super::pci::read16(0, dev as u8, func as u8, 2);
            if d != DEVICE { continue; }
            let bar0 = super::pci::read32(0, dev as u8, func as u8, 0x10);
            if bar0 & 1 == 0 { continue; } // ждём I/O BAR
            unsafe { IO_BASE = ((bar0 & 0xFFFFFFFC) & 0xFFFF) as u16; }
            if unsafe { IO_BASE } == 0 { continue; }
            // IO + BusMaster
            let cmd = super::pci::read16(0, dev as u8, func as u8, 0x04);
            super::pci::write32(0, dev as u8, func as u8, 0x04, (cmd | 0x5) as u32);
            return true;
        }
    }
    false
}

fn spin_timeout(mut f: impl FnMut() -> bool, iters: u32) -> bool {
    let mut i = 0u32;
    while !f() {
        i += 1;
        if i >= iters { return false; }
        core::hint::spin_loop();
    }
    true
}

pub fn is_ready() -> bool { unsafe { READY } }
pub fn mac() -> [u8; 6] { unsafe { MAC } }

fn init_hw() -> bool {
    unsafe {
        let io = IO_BASE;
        // Power on + unlock config
        io::outb(io + REG_CONFIG1, 0x00);
        io::outb(io + REG_9346CR, 0xC0);
        // Reset
        io::outb(io + REG_CR, CR_RST);
        if !spin_timeout(|| io::inb(io + REG_CR) & CR_RST == 0, 2_000_000) {
            uart::write_str("[RTL8139] reset timeout\r\n");
            return false;
        }
        // MAC
        for i in 0..6 {
            MAC[i] = io::inb(io + REG_IDR0 + i as u16);
        }
        if MAC[0] == 0 && MAC[1] == 0 && MAC[2] == 0
            && MAC[3] == 0 && MAC[4] == 0 && MAC[5] == 0 {
            uart::write_str("[RTL8139] bad MAC\r\n");
            return false;
        }
        // RX ring: 3 страницы, используем первые 8192 байт
        let rx = crate::memory::palloc_n(3);
        if rx == 0 {
            uart::write_str("[RTL8139] RX alloc failed\r\n");
            return false;
        }
        core::ptr::write_bytes(rx as *mut u8, 0, 3 * 4096);
        RX_PHYS = rx;
        RX_OFF = 0;
        // TX buffers: по странице на дескриптор
        for i in 0..TX_DESC_COUNT {
            let p = crate::memory::palloc();
            if p == 0 {
                uart::write_str("[RTL8139] TX alloc failed\r\n");
                return false;
            }
            core::ptr::write_bytes(p as *mut u8, 0, 4096);
            TX_PHYS[i] = p;
        }
        TX_NEXT = 0;
        // RX buffer addr, mask IRQ (poll), accept-all, RX+TX enable
        io::outl(io + REG_RBSTART, rx as u32);
        io::outw(io + REG_IMR, 0x0000);
        io::outl(io + REG_RCR, RCR_ACCEPT_ALL);
        io::outw(io + REG_CAPR, 0x0000);
        io::outb(io + REG_CR, CR_RE | CR_TE);
        READY = true;
        uart::write_str("[RTL8139] MAC ");
        for i in 0..6 {
            let h = b"0123456789ABCDEF";
            uart::putchar(h[(MAC[i] >> 4) as usize]);
            uart::putchar(h[(MAC[i] & 0xF) as usize]);
            if i < 5 { uart::putchar(b':'); }
        }
        uart::write_str(" IO=0x");
        let mut b = IO_BASE;
        let mut tmp = [0u8; 4];
        let mut n = 0;
        if b == 0 { uart::putchar(b'0'); } else {
            while b > 0 { tmp[n] = b"0123456789ABCDEF"[(b & 0xF) as usize]; b >>= 4; n += 1; }
            while n > 0 { n -= 1; uart::putchar(tmp[n]); }
        }
        uart::write_str("\r\n");
        true
    }
}

/// Отправить Ethernet-кадр (до 1536 байт). Вызывается из net::tx_route().
pub fn send_frame(data: &[u8]) -> bool {
    unsafe {
        if !READY || data.is_empty() || data.len() > TX_BUF_LEN {
            return false;
        }
        let io = IO_BASE;
        let n = TX_NEXT;
        TX_NEXT = (TX_NEXT + 1) % TX_DESC_COUNT;
        // Дождаться свободного дескриптора (OWN=0)
        if !spin_timeout(|| (io::inl(io + REG_TSD0 + (n as u16) * 4) & TSD_OWN) == 0, 1_000_000) {
            return false;
        }
        core::ptr::copy_nonoverlapping(data.as_ptr(), TX_PHYS[n] as *mut u8, data.len());
        io::outl(io + REG_TSAD0 + (n as u16) * 4, TX_PHYS[n] as u32);
        io::outl(io + REG_TSD0 + (n as u16) * 4, data.len() as u32);
        // Дождаться завершения (TOK)
        if !spin_timeout(|| (io::inl(io + REG_TSD0 + (n as u16) * 4) & TSD_TOK) != 0, 2_000_000) {
            uart::write_str("[RTL8139] TX timeout\r\n");
            return false;
        }
        true
    }
}

/// Прочитать байты из RX-кольца с wrap-around.
unsafe fn ring_copy(off: usize, dst: *mut u8, len: usize) {
    let base = RX_PHYS as *const u8;
    let first = (RX_RING_LEN - off).min(len);
    core::ptr::copy_nonoverlapping(base.add(off), dst, first);
    if first < len {
        core::ptr::copy_nonoverlapping(base, dst.add(first), len - first);
    }
}

/// Забрать кадры из RX-кольца в общий стек. Вызывается из net::poll().
pub fn poll_rx() {
    unsafe {
        if !READY { return; }
        let io = IO_BASE;
        for _ in 0..64 {
            if (inw(io + REG_ISR) & ISR_ROK) == 0 { break; }
            // Заголовок пакета по текущему смещению
            let mut hdr = [0u8; 4];
            ring_copy(RX_OFF, hdr.as_mut_ptr(), 4);
            let _status = hdr[0] as u16 | ((hdr[1] as u16) << 8);
            let plen = hdr[2] as usize | ((hdr[3] as usize) << 8);
            // plen включает 4 байта CRC
            if plen < 14 + 4 || plen > MAX_FRAME + 4 {
                // Десинхрон: прыгаем к аппаратному указателю
                let cbr = inw(io + REG_CBR) as usize % RX_RING_LEN;
                RX_OFF = cbr;
                io::outw(io + REG_CAPR, ((cbr + RX_RING_LEN - 16) % RX_RING_LEN) as u16);
                io::outw(io + REG_ISR, ISR_ROK);
                continue;
            }
            let flen = (plen - 4).min(MAX_FRAME);
            ring_copy((RX_OFF + 4) % RX_RING_LEN, RX_BOUNCE.as_mut_ptr(), flen);
            RX_OFF = (RX_OFF + 4 + plen + 3) & !3;
            if RX_OFF >= RX_RING_LEN { RX_OFF -= RX_RING_LEN; }
            io::outw(io + REG_CAPR, ((RX_OFF + RX_RING_LEN - 16) % RX_RING_LEN) as u16);
            io::outw(io + REG_ISR, ISR_ROK);
            crate::driver::net::input_frame(&RX_BOUNCE[..flen]);
        }
    }
}

pub struct Rtl8139Driver;

impl super::traits::Driver for Rtl8139Driver {
    fn name(&self) -> &'static str { "Realtek RTL8139 Fast Ethernet" }
    fn device_type(&self) -> super::traits::DeviceType {
        super::traits::DeviceType::Pci { vendor: 0x10EC, device: 0x8139, class: 0x02, subclass: 0x00 }
    }
    fn init(&self) -> super::traits::DriverStatus {
        if !find_rtl() {
            return super::traits::DriverStatus::Unsupported;
        }
        uart::write_str("[RTL8139] controller found\r\n");
        if init_hw() {
            super::traits::DriverStatus::Ok
        } else {
            super::traits::DriverStatus::Error("init failed")
        }
    }
}
