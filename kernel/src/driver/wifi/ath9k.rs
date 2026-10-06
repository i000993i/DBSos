#![allow(dead_code, non_snake_case, unused_variables)]
// Atheros ath9k WiFi driver — PCI detection, MMIO registers, DMA TX/RX.

use crate::driver::pci;
use crate::driver::uart;
use super::ieee80211;

// ── ath9k PCI IDs ──────────────────────────────────────────────────

const ATH9K_VENDOR: u16 = 0x168C; // Qualcomm Atheros

// Common ath9k device IDs
const AR9285_ID: u16 = 0x002B;
const AR9287_ID: u16 = 0x002E;
const AR9300_ID: u16 = 0x0030;
const AR9485_ID: u16 = 0x0032;
const AR9462_ID: u16 = 0x0036;
const AR9580_ID: u16 = 0x003C;
const AR9565_ID: u16 = 0x003E;
const AR9880_ID: u16 = 0x003D;
const AR9888_ID: u16 = 0x003F;
const AR99XX_ID: u16 = 0x0040;

const KNOWN_IDS: &[u16] = &[
    AR9285_ID, AR9287_ID, AR9300_ID, AR9485_ID, AR9462_ID,
    AR9580_ID, AR9565_ID, AR9880_ID, AR9888_ID, AR99XX_ID,
];

// ── MMIO Register Map ──────────────────────────────────────────────

const MAC_PCU_VERSION: u32 = 0x0000;

// AR (Baseband) registers
const fn AR_PHY(n: u32) -> u32 { 0x9800 + n * 4 }
const fn AR_PHY_CHAN(ch: u32, off: u32) -> u32 { AR_PHY(ch * 0x1000 + off) }
const fn AR_PHY_RFCHAN(n: u32) -> u32 { AR_PHY_CHAN(n, 0) }
const AR_PHY_MODE: u32 = AR_PHY_CHAN(0, 0x10);
const AR_PHY_TXPOWER: u32 = AR_PHY_CHAN(0, 0x20C4);

// DMA registers
const DMA_CR: u32 = 0x0000;
const DMA_RXCR: u32 = 0x0010;
const DMA_TXCR: u32 = 0x0018;
const DMA_RXDP: u32 = 0x001C;
const fn DMA_TXDP(n: u32) -> u32 { 0x0020 + n * 4 }
const fn DMA_TXDPU(n: u32) -> u32 { 0x0030 + n * 4 }
const fn DMA_TXSTATUS(n: u32) -> u32 { 0x0040 + n * 4 }

// Interrupt registers
const RTC_STATUS: u32 = 0x0044;
const RTC_IRQSTATUS: u32 = 0x0048;
const RTC_HOSTIRQSTATUS: u32 = 0x004C;
const RTC_IER: u32 = 0x0060;

// ── DMA Descriptor structures ──────────────────────────────────────

#[derive(Clone, Copy)]
#[repr(C)]
struct TxDesc {
    buf_addr: u32,
    control: u32,
    status: u32,
    pad: u32,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct RxDesc {
    buf_addr: u32,
    control: u32,
    status: u32,
    len: u32,
}

const TX_DESC_CTRL_LEN_M: u32 = 0x0FFF;
const RX_DESC_CTRL_REQ_INT: u32 = 1 << 0;

const NUM_TX_DESC: usize = 32;
const NUM_RX_DESC: usize = 64;
const TX_BUF_SIZE: usize = 2346;
const RX_BUF_SIZE: usize = 2346;

// ── Driver state ───────────────────────────────────────────────────

pub struct Ath9kState {
    pub mmio_base: u64,
    pub mmio_len: usize,
    pub device_id: u16,
    pub mac: [u8; 6],

    pub is_2ghz: bool,
    pub is_5ghz: bool,
    pub num_tx_queues: u32,
    pub current_channel: u16,

    tx_descs: [TxDesc; NUM_TX_DESC],
    tx_bufs: [[u8; TX_BUF_SIZE]; NUM_TX_DESC],
    tx_head: usize,
    tx_tail: usize,
    tx_phys: u64,

    rx_descs: [RxDesc; NUM_RX_DESC],
    rx_bufs: [[u8; RX_BUF_SIZE]; NUM_RX_DESC],
    rx_head: usize,
    rx_tail: usize,
    rx_phys: u64,

    pending_tx: usize,
    irq_status: u32,
}

impl Ath9kState {
    pub const fn new() -> Self {
        Ath9kState {
            mmio_base: 0, mmio_len: 0, device_id: 0, mac: [0; 6],
            is_2ghz: true, is_5ghz: false, num_tx_queues: 4, current_channel: 0,
            tx_descs: [TxDesc { buf_addr: 0, control: 0, status: 0, pad: 0 }; NUM_TX_DESC],
            tx_bufs: [[0; TX_BUF_SIZE]; NUM_TX_DESC],
            tx_head: 0, tx_tail: 0, tx_phys: 0,
            rx_descs: [RxDesc { buf_addr: 0, control: 0, status: 0, len: 0 }; NUM_RX_DESC],
            rx_bufs: [[0; RX_BUF_SIZE]; NUM_RX_DESC],
            rx_head: 0, rx_tail: 0, rx_phys: 0,
            pending_tx: 0, irq_status: 0,
        }
    }
}

static mut ATH9K: Ath9kState = Ath9kState::new();
static mut INITIALIZED: bool = false;

fn mmio_read32(off: u32) -> u32 {
    unsafe {
        let base = ATH9K.mmio_base as *const u32;
        core::ptr::read_volatile(base.add((off >> 2) as usize))
    }
}

fn mmio_write32(off: u32, val: u32) {
    unsafe {
        let base = ATH9K.mmio_base as *mut u32;
        core::ptr::write_volatile(base.add((off >> 2) as usize), val);
    }
}

fn up(s: &str) { uart::write_str(s); }

fn hex8(v: u8) {
    let h = b"0123456789ABCDEF";
    uart::putchar(h[(v >> 4) as usize]);
    uart::putchar(h[(v & 0xF) as usize]);
}

fn hex32(v: u32) {
    hex8((v >> 24) as u8);
    hex8((v >> 16) as u8);
    hex8((v >> 8) as u8);
    hex8(v as u8);
}

// ── PCI detection and BAR mapping ──────────────────────────────────

fn find_ath9k_pci(bus: u8, dev: u8, _func: u8) -> Option<(u8, u8, u8)> {
    for d in dev..32u8 {
        for f in 0..8u8 {
            let vendor = pci::read16(bus, d, f, 0x00);
            if vendor == 0xFFFF {
                if f == 0 { break; }
                continue;
            }
            if vendor != ATH9K_VENDOR { continue; }
            let device = pci::read16(bus, d, f, 0x02);
            let reg08 = pci::read32(bus, d, f, 0x08);
            let class = (reg08 >> 24) as u8;
            let subclass = ((reg08 >> 16) & 0xFF) as u8;
            if class == 0x02 && subclass == 0x80 {
                return Some((bus, d, f));
            }
            if KNOWN_IDS.contains(&device) {
                return Some((bus, d, f));
            }
        }
    }
    None
}

fn read_bar(bus: u8, dev: u8, func: u8, bar_idx: u8) -> Option<(u64, usize)> {
    let offset = (0x10 + bar_idx * 4) as u8;
    let bar = pci::read32(bus, dev, func, offset);
    if bar == 0 { return None; }

    let is_io = bar & 1 != 0;
    let is_64bit = (bar >> 2) & 3 == 2;
    if is_io { return None; }

    let mut addr = (bar & !0xF) as u64;
    if is_64bit {
        let bar_hi = pci::read32(bus, dev, func, offset + 4);
        addr |= (bar_hi as u64) << 32;
    }

    pci::write32(bus, dev, func, offset, 0xFFFFFFFF);
    let size_mask = pci::read32(bus, dev, func, offset);
    pci::write32(bus, dev, func, offset, bar);

    let size = if size_mask == 0 { 0x1000 } else { (!size_mask + 1) as usize };
    Some((addr, size))
}

fn read_mac_from_eeprom() -> [u8; 6] {
    unsafe {
        let mut mac = [0u8; 6];
        let devid = ATH9K.device_id;
        mac[0] = 0x00;
        mac[1] = 0x13;
        mac[2] = 0x74;
        mac[3] = ((devid >> 8) & 0xFF) as u8;
        mac[4] = (devid & 0xFF) as u8;
        mac[5] = 0x01;
        mac
    }
}

// ── Hardware initialization ────────────────────────────────────────

fn hw_reset() -> bool {
    up("[ath9k] reset MAC\r\n");
    mmio_write32(RTC_IER, 0);
    mmio_write32(RTC_IRQSTATUS, 0xFFFFFFFF);

    let val = mmio_read32(DMA_CR);
    mmio_write32(DMA_CR, val | (1 << 30));

    for _ in 0..1000 {
        let st = mmio_read32(RTC_STATUS);
        if st & (1 << 4) != 0 { break; }
    }

    let val = mmio_read32(DMA_CR);
    mmio_write32(DMA_CR, val & !(1 << 30));

    for _ in 0..10000 {
        let ver = mmio_read32(MAC_PCU_VERSION);
        if ver != 0 && ver != 0xFFFFFFFF { break; }
    }

    true
}

fn hw_set_band(is_2ghz: bool) {
    let mode = mmio_read32(AR_PHY_MODE);
    if is_2ghz {
        mmio_write32(AR_PHY_MODE, mode & !(1 << 0));
    } else {
        mmio_write32(AR_PHY_MODE, mode | (1 << 0));
    }
}

pub fn hw_set_channel(channel: u16) {
    unsafe { ATH9K.current_channel = channel; }

    if channel >= 2412 && channel <= 2484 {
        let chan_num = if channel <= 2484 { ((channel - 2412) / 5 + 1) as u32 } else { 14 };
        mmio_write32(AR_PHY_RFCHAN(1), chan_num | (1 << 7));
        up("[ath9k] ch="); uart::putchar(b'0' + (chan_num % 10) as u8);
    } else if channel >= 5180 {
        let chan_num = ((channel - 5000) / 5) as u32;
        mmio_write32(AR_PHY_RFCHAN(1), chan_num | (1 << 7) | (1 << 2));
        up("[ath9k] ch5g=");
        let mut v = chan_num;
        let mut buf = [0u8; 4]; let mut i = 0;
        while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
        while i > 0 { i -= 1; uart::putchar(buf[i]); }
    }
    up("\r\n");
}

fn hw_set_txpower(power: u32) {
    let val = (power & 0x3F) << 2;
    mmio_write32(AR_PHY_TXPOWER, val);
}

fn hw_set_mac(mac: &[u8; 6]) {
    let low = (mac[0] as u32) | ((mac[1] as u32) << 8)
        | ((mac[2] as u32) << 16) | ((mac[3] as u32) << 24);
    let high = (mac[4] as u32) | ((mac[5] as u32) << 8);
    mmio_write32(0x1000, low);
    mmio_write32(0x1004, high);
}

fn hw_init_dma() {
    unsafe {
        let tx_phys = &ATH9K.tx_descs as *const _ as u64;
        let rx_phys = &ATH9K.rx_descs as *const _ as u64;
        ATH9K.tx_phys = tx_phys;
        ATH9K.rx_phys = rx_phys;

        for i in 0..NUM_TX_DESC {
            let buf_phys = &ATH9K.tx_bufs[i] as *const _ as u64;
            ATH9K.tx_descs[i].buf_addr = buf_phys as u32;
            ATH9K.tx_descs[i].control = TX_BUF_SIZE as u32;
        }
        for i in 0..NUM_RX_DESC {
            let buf_phys = &ATH9K.rx_bufs[i] as *const _ as u64;
            ATH9K.rx_descs[i].buf_addr = buf_phys as u32;
            ATH9K.rx_descs[i].control = RX_DESC_CTRL_REQ_INT;
        }

        mmio_write32(DMA_TXDP(0), tx_phys as u32);
        mmio_write32(DMA_RXDP, rx_phys as u32);

        let tx_cfg = mmio_read32(DMA_TXCR);
        mmio_write32(DMA_TXCR, tx_cfg | (1 << 0));
        let rx_cfg = mmio_read32(DMA_RXCR);
        mmio_write32(DMA_RXCR, rx_cfg | (1 << 0));
    }
    up("[ath9k] DMA rings init\r\n");
}

fn hw_enable_interrupts() {
    let mut ier = 0u32;
    ier |= 1 << 0;
    ier |= 1 << 1;
    ier |= 1 << 2;
    ier |= 1 << 4;
    ier |= 1 << 15;
    mmio_write32(RTC_IER, ier);
    up("[ath9k] interrupts enabled\r\n");
}

pub fn hw_init() -> bool {
    unsafe {
        if INITIALIZED { return true; }
    }

    up("[ath9k] scanning PCI for Atheros WiFi...\r\n");

    let (bus, dev, func) = match find_ath9k_pci(0, 0, 0) {
        Some(pos) => pos,
        None => {
            up("[ath9k] no Atheros WiFi card found\r\n");
            return false;
        }
    };

    let device_id = pci::read16(bus, dev, func, 0x02);
    up("[ath9k] found device=0x");
    hex8((device_id >> 8) as u8);
    hex8(device_id as u8);
    up(" bus="); uart::putchar(b'0' + bus);
    up(" dev="); uart::putchar(b'0' + dev);
    up("\r\n");

    let cmd = pci::read16(bus, dev, func, 0x04);
    pci::write32(bus, dev, func, 0x04, (cmd as u32) | 0x04 | 0x01);

    let (bar0_addr, bar0_size) = match read_bar(bus, dev, func, 0) {
        Some(bar) => bar,
        None => {
            up("[ath9k] BAR0 not available\r\n");
            return false;
        }
    };
    up("[ath9k] BAR0=0x");
    hex32(bar0_addr as u32);
    up(" size=");
    let mut v = bar0_size as u64;
    let mut buf = [0u8; 20]; let mut i = 0;
    while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putchar(buf[i]); }
    up("\r\n");

    unsafe {
        ATH9K.mmio_base = bar0_addr;
        ATH9K.mmio_len = bar0_size;
        ATH9K.device_id = device_id;
    }

    let ver = mmio_read32(MAC_PCU_VERSION);
    up("[ath9k] MAC version=0x");
    hex32(ver);
    up("\r\n");
    if ver == 0 || ver == 0xFFFFFFFF {
        up("[ath9k] invalid MAC version\r\n");
        return false;
    }

    hw_reset();

    let (is_2ghz, is_5ghz) = match device_id {
        AR9285_ID | AR9287_ID => (true, false),
        AR9300_ID | AR9485_ID | AR9565_ID | AR9580_ID => (true, true),
        _ => (true, false),
    };
    unsafe {
        ATH9K.is_2ghz = is_2ghz;
        ATH9K.is_5ghz = is_5ghz;
    }

    hw_set_band(is_2ghz);

    let mac = read_mac_from_eeprom();
    unsafe { ATH9K.mac = mac; }
    hw_set_mac(&mac);
    up("[ath9k] MAC=");
    for i in 0..6 { hex8(mac[i]); if i < 5 { uart::putchar(b':'); } }
    up("\r\n");

    hw_set_txpower(60);
    hw_set_channel(2412);
    hw_init_dma();
    hw_enable_interrupts();

    unsafe { INITIALIZED = true; }
    up("[ath9k] initialized OK\r\n");
    true
}

// ── TX: Send 802.11 frames ─────────────────────────────────────────

pub fn tx_frame(frame: &[u8]) -> Option<usize> {
    unsafe {
        let head = ATH9K.tx_head;
        let next = (head + 1) % NUM_TX_DESC;
        if next == ATH9K.tx_tail { return None; }

        let len = frame.len().min(TX_BUF_SIZE);
        ATH9K.tx_bufs[head][..len].copy_from_slice(&frame[..len]);
        ATH9K.tx_descs[head].control = (len as u32) & TX_DESC_CTRL_LEN_M;
        ATH9K.tx_descs[head].buf_addr = &ATH9K.tx_bufs[head] as *const _ as u32;
        ATH9K.tx_descs[head].status = 0;

        mmio_write32(DMA_TXDPU(0), next as u32);

        let idx = head;
        ATH9K.tx_head = next;
        ATH9K.pending_tx += 1;
        Some(idx)
    }
}

pub fn tx_mgmt_frame(subtype: u8, dst: &[u8; 6], src: &[u8; 6], bssid: &[u8; 6],
                      body: &[u8]) -> Option<usize> {
    let mut frame = [0u8; ieee80211::MAX_80211_FRAME];
    let mut pos = 0;

    let fc = ieee80211::make_fc(false, false, false, false, false, subtype, ieee80211::FC_TYPE_MGMT);
    frame[pos] = (fc & 0xFF) as u8; pos += 1;
    frame[pos] = (fc >> 8) as u8; pos += 1;

    frame[pos] = 0; frame[pos + 1] = 0; pos += 2;

    frame[pos..pos + 6].copy_from_slice(dst); pos += 6;
    frame[pos..pos + 6].copy_from_slice(src); pos += 6;
    frame[pos..pos + 6].copy_from_slice(bssid); pos += 6;

    frame[pos] = 0; frame[pos + 1] = 0; pos += 2;

    let body_len = body.len().min(ieee80211::MAX_80211_FRAME - pos);
    frame[pos..pos + body_len].copy_from_slice(&body[..body_len]);
    pos += body_len;

    tx_frame(&frame[..pos])
}

// ── RX: Receive 802.11 frames ──────────────────────────────────────

pub fn rx_frame() -> Option<(&'static [u8], usize)> {
    unsafe {
        let tail = ATH9K.rx_tail;
        let head = ATH9K.rx_head;
        if tail == head { return None; }

        let desc = &ATH9K.rx_descs[tail];
        let len = (desc.status & 0xFFF) as usize;

        if len == 0 || len > RX_BUF_SIZE {
            ATH9K.rx_tail = (tail + 1) % NUM_RX_DESC;
            return None;
        }

        let buf = &ATH9K.rx_bufs[tail][..len];
        ATH9K.rx_tail = (tail + 1) % NUM_RX_DESC;

        mmio_write32(DMA_RXDP, ATH9K.rx_tail as u32);

        Some((buf, len))
    }
}

pub fn process_rx_frame(data: &[u8]) {
    if data.len() < 24 { return; }

    let fc = (data[0] as u16) | ((data[1] as u16) << 8);
    let ftype = ieee80211::fc_type(fc);
    let subtype = ieee80211::fc_subtype(fc);

    match ftype {
        ieee80211::FC_TYPE_MGMT => {
            if subtype == ieee80211::FC_SUBTYPE_BEACON
                || subtype == ieee80211::FC_SUBTYPE_PROBE_RESP
            {
                crate::driver::wifi::handle_scan_response(data);
            }
            if subtype == ieee80211::FC_SUBTYPE_AUTH {
                crate::driver::wifi::handle_auth_response(data);
            }
            if subtype == ieee80211::FC_SUBTYPE_ASSOC_RESP {
                crate::driver::wifi::handle_assoc_response(data);
            }
        }
        ieee80211::FC_TYPE_DATA => {
            if data.len() > 24 {
                let payload = &data[24..];
                if payload.len() >= 8 {
                    let ether_type = (payload[6] as u16) << 8 | payload[7] as u16;
                    if ether_type == ieee80211::EAPOL_ETHER_TYPE {
                        crate::driver::wifi::handle_eapol(payload);
                    }
                }
            }
        }
        _ => {}
    }
}

fn handle_tx_completion() {
    unsafe {
        for q in 0..ATH9K.num_tx_queues {
            let status = mmio_read32(DMA_TXSTATUS(q));
            if status != 0 {
                mmio_write32(DMA_TXSTATUS(q), 0);
                ATH9K.pending_tx = ATH9K.pending_tx.saturating_sub(1);
            }
        }
    }
}

fn handle_rx() {
    loop {
        match rx_frame() {
            Some((data, len)) => { process_rx_frame(&data[..len]); }
            None => break,
        }
    }
}

pub fn handle_irq() {
    unsafe {
        let status = mmio_read32(RTC_HOSTIRQSTATUS);
        if status == 0 || status == 0xFFFFFFFF { return; }
        ATH9K.irq_status = status;
        mmio_write32(RTC_IRQSTATUS, 0xFFFFFFFF);

        if status & (1 << 0) != 0 { handle_rx(); }
        if status & (1 << 1) != 0 { handle_tx_completion(); }
    }
}

// ── Scanning ───────────────────────────────────────────────────────

pub const CHANNELS_24GHZ: &[u16] = &[2412, 2417, 2422, 2427, 2432, 2437, 2442, 2447, 2452, 2457, 2462, 2467, 2472];
pub const CHANNELS_5GHZ: &[u16] = &[
    5180, 5200, 5220, 5240, 5260, 5280, 5300, 5320,
    5745, 5765, 5785, 5805, 5825,
];

pub fn hw_scan() -> usize {
    up("[ath9k] starting scan...\r\n");

    let mut channels = [0u16; 32];
    let mut nch = 0usize;
    for &ch in CHANNELS_24GHZ {
        if nch < 32 { channels[nch] = ch; nch += 1; }
    }
    if unsafe { ATH9K.is_5ghz } {
        for &ch in CHANNELS_5GHZ {
            if nch < 32 { channels[nch] = ch; nch += 1; }
        }
    }

    for i in 0..nch {
        let channel = channels[i];
        hw_set_channel(channel);

        let mut probe_body = [0u8; 128];
        let probe_len = ieee80211::build_probe_request(b"", &mut probe_body);
        let mac = unsafe { ATH9K.mac };
        let broadcast = ieee80211::BROADCAST;
        tx_mgmt_frame(
            ieee80211::FC_SUBTYPE_PROBE_REQ,
            &broadcast, &mac, &broadcast,
            &probe_body[..probe_len],
        );

        for _ in 0..50_000 {
            handle_rx();
        }
    }

    let count = crate::driver::wifi::scan_result_count();
    up("[ath9k] scan complete: ");
    let mut v = count as u64;
    let mut buf = [0u8; 10]; let mut i = 0;
    while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    if i == 0 { uart::putchar(b'0'); } else { while i > 0 { i -= 1; uart::putchar(buf[i]); } }
    up(" networks found\r\n");
    count
}

// ── Public API ─────────────────────────────────────────────────────

pub fn is_initialized() -> bool { unsafe { INITIALIZED } }
pub fn mac() -> [u8; 6] { unsafe { ATH9K.mac } }
pub fn current_channel() -> u16 { unsafe { ATH9K.current_channel } }
pub fn is_2ghz() -> bool { unsafe { ATH9K.is_2ghz } }
pub fn is_5ghz() -> bool { unsafe { ATH9K.is_5ghz } }
pub fn pending_tx_count() -> usize { unsafe { ATH9K.pending_tx } }
