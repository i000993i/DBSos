/// e1000e (82574L) — драйвер с DMA TX/RX, ARP + ICMP echo

use super::traits::*;
use super::uart;
use crate::memory;
use core::ptr::{read_volatile, write_volatile};

const E1000_VENDOR: u16 = 0x8086;

// --- MMIO регистры ---
const REG_CTRL: u32 = 0x0000;
const REG_STATUS: u32 = 0x0008;
const REG_MAC_LO: u32 = 0x5400;
const REG_MAC_HI: u32 = 0x5404;
const REG_RCTL: u32 = 0x0100;
const REG_TCTL: u32 = 0x0400;
const REG_TIPG: u32 = 0x0410;
const REG_RDBAL: u32 = 0x2800;
const REG_RDBAH: u32 = 0x2804;
const REG_RDLEN: u32 = 0x2808;
const REG_RDH: u32 = 0x2810;
const REG_RDT: u32 = 0x2818;
const REG_RXDCTL: u32 = 0x2828;
const REG_TDBAL: u32 = 0x3800;
const REG_TDBAH: u32 = 0x3804;
const REG_TDLEN: u32 = 0x3808;
const REG_TDH: u32 = 0x3810;
const REG_TDT: u32 = 0x3818;

const CTRL_SLU: u32 = 0x0040;
const CTRL_RST: u32 = 0x04000000;

const RCTL_EN: u32 = 0x00000002;
const RCTL_BSIZE_2048: u32 = 0x00000000; // BSIZE=00 → 2048 bytes
const RCTL_SECRC: u32 = 0x04000000;
const RCTL_BAM: u32 = 0x00008000;
const RCTL_UPE: u32 = 0x00000008;
const RCTL_MPE: u32 = 0x00000010;

const TCTL_EN: u32 = 0x00000002;
const TCTL_PSP: u32 = 0x00000008;
const TCTL_CT_SHIFT: u32 = 4;
const TCTL_CT: u32 = 15 << TCTL_CT_SHIFT;
const TCTL_COLD_SHIFT: u32 = 12;
const TCTL_COLD: u32 = 64 << TCTL_COLD_SHIFT;

// --- DMA ---
const NUM_DESC: usize = 16;

/// Legacy TX descriptor (16 bytes)
#[repr(C, packed(4))]
struct TxDesc {
    addr: u64,
    cmd_len: u32,
    oinfo_sta: u16,
    css_special: u16,
}

/// Legacy RX descriptor (16 bytes)
#[repr(C, packed(4))]
struct RxDesc {
    addr: u64,
    length: u16,
    csum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

static mut MMIO: u64 = 0;

// DMA-память: кольца + буферы
static mut TX_RING_PHYS: u64 = 0;
static mut TX_RING: *mut TxDesc = 0 as *mut TxDesc;
static mut TX_BUFS: [u64; NUM_DESC] = [0; NUM_DESC];
static mut TX_HEAD: usize = 0;
static mut TX_TAIL: usize = 0;

static mut RX_RING_PHYS: u64 = 0;
static mut RX_RING: *mut RxDesc = 0 as *mut RxDesc;
static mut RX_BUFS: [u64; NUM_DESC] = [0; NUM_DESC];
static mut RX_HEAD: usize = 0;

// MAC + IP
static mut OUR_MAC: [u8; 6] = [0; 6];
static OUR_IP: [u8; 4] = [10, 0, 2, 15];

fn mmio_read32(reg: u32) -> u32 {
    let base = unsafe { MMIO };
    if base == 0 { return 0; }
    unsafe { read_volatile((base as usize + reg as usize) as *const u32) }
}

fn mmio_write32(reg: u32, val: u32) {
    let base = unsafe { MMIO };
    if base == 0 { return; }
    unsafe { write_volatile((base as usize + reg as usize) as *mut u32, val) }
}

static mut NIC_BUS: u8 = 0;
static mut NIC_DEV: u8 = 0;
static mut NIC_FUNC: u8 = 0;

fn find_nic() -> bool {
    for dev in 0..32 {
        for func in 0..8 {
            let vendor = super::pci::read16(0, dev as u8, func as u8, 0x00);
            if vendor != E1000_VENDOR {
                if func == 0 { break; }
                continue;
            }
            let reg08 = super::pci::read32(0, dev as u8, func as u8, 0x08);
            let class = ((reg08 >> 24) & 0xFF) as u8;
            if class != 0x02 { continue; }
            let bar0 = super::pci::read32(0, dev as u8, func as u8, 0x10);
            let mmio = (bar0 & 0xFFFFFFF0) as u64;
            unsafe { MMIO = mmio; NIC_BUS = 0; NIC_DEV = dev as u8; NIC_FUNC = func as u8; }
            return true;
        }
    }
    false
}

/// Включить Bus Master + Memory/IO space в PCI Command Register
fn pci_enable_bus_master() {
    unsafe {
        let cmd = super::pci::read16(NIC_BUS, NIC_DEV, NIC_FUNC, 0x04);
        // bit 0 = IO space, bit 1 = mem space, bit 2 = bus master
        let new_cmd = cmd | 0x0007;
        // Write command register (offset 0x04) as 16-bit
        // PCI config offset 0x04 maps to read32 bits [15:0]  
        let old32 = super::pci::read32(NIC_BUS, NIC_DEV, NIC_FUNC, 0x04);
        super::pci::write32(NIC_BUS, NIC_DEV, NIC_FUNC, 0x04,
            (old32 & 0xFFFF0000) | (new_cmd as u32));
    }
}

fn read_mac() -> [u8; 6] {
    let lo = mmio_read32(REG_MAC_LO);
    let hi = mmio_read32(REG_MAC_HI);
    [
        lo as u8, (lo >> 8) as u8, (lo >> 16) as u8, (lo >> 24) as u8,
        hi as u8, (hi >> 8) as u8,
    ]
}

fn hex_nib(v: u8) -> u8 {
    let n = v & 0xF;
    if n < 10 { b'0' + n } else { b'A' + n - 10 }
}

fn print_mac(mac: &[u8; 6]) {
    for (i, &b) in mac.iter().enumerate() {
        uart::putchar(hex_nib(b >> 4));
        uart::putchar(hex_nib(b));
        if i < 5 { uart::putchar(b':'); }
    }
}

pub fn print_ip(ip: &[u8; 4]) {
    for i in 0..4 {
        uart_dec(ip[i] as u64);
        if i < 3 { uart::putchar(b'.'); }
    }
}

// ====== DMA init ======

fn dma_init() -> bool {
    unsafe {
        // TX ring (выровнен на 16 байт)
        let tx = memory::palloc();
        if tx == 0 { return false; }
        TX_RING_PHYS = tx;
        TX_RING = tx as *mut TxDesc;
        // TX packet buffers
        for i in 0..NUM_DESC {
            let buf = memory::palloc();
            if buf == 0 { return false; }
            TX_BUFS[i] = buf;
            (*TX_RING.add(i)) = TxDesc {
                addr: buf,
                cmd_len: 0,
                oinfo_sta: 0,
                css_special: 0,
            };
        }

        // RX ring
        let rx = memory::palloc();
        if rx == 0 { return false; }
        RX_RING_PHYS = rx;
        RX_RING = rx as *mut RxDesc;
        for i in 0..NUM_DESC {
            let buf = memory::palloc();
            if buf == 0 { return false; }
            RX_BUFS[i] = buf;
            (*RX_RING.add(i)) = RxDesc {
                addr: buf,
                length: 0,
                csum: 0,
                status: 0,
                errors: 0,
                special: 0,
            };
        }

        TX_HEAD = 0;
        TX_TAIL = 0;
        RX_HEAD = 0;
    }
    true
}

fn dma_rings_configure() {
    unsafe {
        // TX ring
        mmio_write32(REG_TDBAL, TX_RING_PHYS as u32);
        mmio_write32(REG_TDBAH, (TX_RING_PHYS >> 32) as u32);
        mmio_write32(REG_TDLEN, (NUM_DESC * 16) as u32);
        mmio_write32(REG_TDH, 0);
        mmio_write32(REG_TDT, 0);

        // RX ring: set up base and size, but RDT=0 (no descriptors initially)
        mmio_write32(REG_RDBAL, RX_RING_PHYS as u32);
        mmio_write32(REG_RDBAH, (RX_RING_PHYS >> 32) as u32);
        mmio_write32(REG_RDLEN, (NUM_DESC * 16) as u32);
        mmio_write32(REG_RDH, 0);
        mmio_write32(REG_RDT, 0);
    }
}

fn enable_rx() {
    // Set MAC in receive filter with Address Valid bit
    let mac = unsafe { OUR_MAC };
    mmio_write32(REG_MAC_LO, mac[0] as u32 | (mac[1] as u32) << 8 | (mac[2] as u32) << 16 | (mac[3] as u32) << 24);
    mmio_write32(REG_MAC_HI, (mac[4] as u32 | (mac[5] as u32) << 8) | 0x80000000);

    // Enable RX (order: set RCTL first, then give descriptors via RDT)
    mmio_write32(REG_RXDCTL, 0);
    mmio_write32(REG_RCTL, RCTL_EN | RCTL_BSIZE_2048 | RCTL_BAM | RCTL_SECRC | RCTL_UPE | RCTL_MPE);

    // Give all descriptors to the NIC
    mmio_write32(REG_RDT, (NUM_DESC - 1) as u32);
}

/// Отправить пакет (без ожидания DD — async)
fn tx_send(data: &[u8]) -> bool {
    unsafe {
        let desc = &mut *TX_RING.add(TX_TAIL);
        let buf = TX_BUFS[TX_TAIL];
        let buf_slice = core::slice::from_raw_parts_mut(buf as *mut u8, data.len());
        buf_slice.copy_from_slice(data);

        // cmd byte: EOP(bit0) | IFCS(bit1) | RS(bit3)
        let cmd_byte: u8 = 0x01 | 0x02 | 0x08;
        desc.cmd_len = ((cmd_byte as u32) << 24) | (data.len() as u32);
        desc.oinfo_sta = 0;

        // Flush descriptor + data to physical memory
        core::arch::asm!("wbinvd");

        let next = (TX_TAIL + 1) % NUM_DESC;
        mmio_write32(REG_TDT, next as u32);
        TX_TAIL = next;
        true
    }
}

/// Получить пакет (должен быть вызван до recv)
fn rx_available() -> bool {
    unsafe {
        core::arch::asm!("wbinvd");
        let desc = &*RX_RING.add(RX_HEAD);
        desc.status & 0x01 != 0 // DD = descriptor done
    }
}

fn rx_recv() -> Option<(&'static [u8], usize)> {
    unsafe {
        if !rx_available() { return None; }
        let desc = &*RX_RING.add(RX_HEAD);
        let len = desc.length as usize;
        let buf = RX_BUFS[RX_HEAD];
        let data = core::slice::from_raw_parts(buf as *const u8, len);
        let idx = RX_HEAD;
        RX_HEAD = (RX_HEAD + 1) % NUM_DESC;
        // Вернуть descriptor обратно NIC
        let next_rdt = (RX_HEAD + NUM_DESC - 1) % NUM_DESC;
        mmio_write32(REG_RDT, next_rdt as u32);
        // Сбросить статус
        let d = &mut *RX_RING.add(idx);
        d.status = 0;
        Some((data, idx))
    }
}

// ====== Ethernet / ARP / IP / ICMP ======

#[repr(C, packed)]
struct EthHdr {
    dst: [u8; 6],
    src: [u8; 6],
    ether_type: u16,
}

#[repr(C, packed)]
struct ArpPkt {
    hw_type: u16,
    proto: u16,
    hw_len: u8,
    proto_len: u8,
    op: u16,
    sender_mac: [u8; 6],
    sender_ip: [u8; 4],
    target_mac: [u8; 6],
    target_ip: [u8; 4],
}

#[repr(C, packed)]
struct IpHdr {
    ver_ihl: u8,
    dscp: u8,
    total_len: u16,
    id: u16,
    flags_frag: u16,
    ttl: u8,
    protocol: u8,
    checksum: u16,
    src: [u8; 4],
    dst: [u8; 4],
}

#[repr(C, packed)]
struct IcmpHdr {
    typ: u8,
    code: u8,
    csum: u16,
    rest: u32,
}

const ETH_TYPE_ARP: u16 = 0x0608; // в little-endian: 0x0806 на проводе
const ETH_TYPE_IP: u16 = 0x0008;  // 0x0800
const ARP_REQUEST: u16 = 0x0100;  // 1 в LE
const IP_PROTO_ICMP: u8 = 1;

fn ip_checksum(buf: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for i in (0..buf.len()).step_by(2) {
        let word = if i + 1 < buf.len() {
            (buf[i] as u32) << 8 | (buf[i + 1] as u32)
        } else {
            (buf[i] as u32) << 8
        };
        sum += word;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn build_arp_reply(pkt: &mut [u8; 60], sender_mac: &[u8; 6], sender_ip: &[u8; 4]) -> usize {
    let mac = unsafe { OUR_MAC };
    // Ethernet
    pkt[0..6].copy_from_slice(sender_mac);
    pkt[6..12].copy_from_slice(&mac);
    pkt[12] = 0x08; pkt[13] = 0x06; // ARP
    // ARP reply
    pkt[14] = 0x00; pkt[15] = 0x01; // hw = ethernet
    pkt[16] = 0x08; pkt[17] = 0x00; // proto = IPv4
    pkt[18] = 6; pkt[19] = 4;
    pkt[20] = 0x00; pkt[21] = 0x02; // reply
    pkt[22..28].copy_from_slice(&mac);
    pkt[28..32].copy_from_slice(&OUR_IP);
    pkt[32..38].copy_from_slice(sender_mac);
    pkt[38..42].copy_from_slice(sender_ip);
    42
}

fn build_icmp_reply(pkt: &mut [u8; 1514], sender_mac: &[u8; 6], ip_hdr: &IpHdr, icmp: &IcmpHdr, payload: &[u8]) -> usize {
    let total = 14 + 20 + 8 + payload.len();
    let mac = unsafe { OUR_MAC };
    // Ethernet
    pkt[0..6].copy_from_slice(sender_mac);
    pkt[6..12].copy_from_slice(&mac);
    pkt[12] = 0x08; pkt[13] = 0x00; // IPv4
    // IP header
    let ip_len = 20 + 8 + payload.len() as u16;
    let ip_off = 14;
    pkt[ip_off] = 0x45; // ver_ihl
    pkt[ip_off + 1] = 0; // dscp
    pkt[ip_off + 2] = (ip_len >> 8) as u8;
    pkt[ip_off + 3] = (ip_len & 0xFF) as u8;
    pkt[ip_off + 4] = 0; pkt[ip_off + 5] = 0; // id
    pkt[ip_off + 6] = 0; pkt[ip_off + 7] = 0; // flags_frag
    pkt[ip_off + 8] = 64; // ttl
    pkt[ip_off + 9] = IP_PROTO_ICMP;
    pkt[ip_off + 10] = 0; pkt[ip_off + 11] = 0; // checksum = 0
    pkt[ip_off + 12..ip_off + 16].copy_from_slice(&OUR_IP);
    pkt[ip_off + 16..ip_off + 20].copy_from_slice(&ip_hdr.src);
    // IP checksum
    let csum = ip_checksum(&pkt[ip_off..ip_off + 20]);
    pkt[ip_off + 10] = (csum >> 8) as u8;
    pkt[ip_off + 11] = (csum & 0xFF) as u8;
    // ICMP
    let icmp_off = ip_off + 20;
    pkt[icmp_off] = 0; // type = echo reply
    pkt[icmp_off + 1] = 0; // code
    pkt[icmp_off + 2] = 0; pkt[icmp_off + 3] = 0; // csum = 0
    pkt[icmp_off + 4..icmp_off + 8].copy_from_slice(&icmp.rest.to_le_bytes());
    // Payload
    if !payload.is_empty() {
        pkt[icmp_off + 8..icmp_off + 8 + payload.len()].copy_from_slice(payload);
    }
    // ICMP checksum
    let icmp_len = 8 + payload.len();
    let icmp_csum = ip_checksum(&pkt[icmp_off..icmp_off + icmp_len]);
    pkt[icmp_off + 2] = (icmp_csum >> 8) as u8;
    pkt[icmp_off + 3] = (icmp_csum & 0xFF) as u8;
    total
}

pub fn send_arp_request(target_ip: [u8; 4]) -> bool {
    // Debug: print TX ring info
    unsafe {
        uart::write_str("[NET] TX_RING=0x");
        uart_hex(TX_RING_PHYS);
        uart::write_str(" buf0=0x");
        uart_hex(TX_BUFS[0]);
        uart::write_str("\r\n");
    }
    // Build packet in a local array (no aliasing with TX buffer)
    let mut pkt = [0u8; 42];
    let eth_dst: [u8; 6] = [0xFF; 6];
    let mac = unsafe { OUR_MAC };
    // Ethernet
    pkt[0..6].copy_from_slice(&eth_dst);
    pkt[6..12].copy_from_slice(&mac);
    pkt[12] = 0x08; pkt[13] = 0x06; // ARP
    // ARP request
    pkt[14] = 0x00; pkt[15] = 0x01; // hw = ethernet
    pkt[16] = 0x08; pkt[17] = 0x00; // proto = IPv4
    pkt[18] = 6; pkt[19] = 4;       // hw/proto len
    pkt[20] = 0x00; pkt[21] = 0x01; // request
    pkt[22..28].copy_from_slice(&mac);
    pkt[28..32].copy_from_slice(&OUR_IP);
    let zero_mac: [u8; 6] = [0; 6];
    pkt[32..38].copy_from_slice(&zero_mac);
    pkt[38..42].copy_from_slice(&target_ip);

    tx_send(&pkt)
}

// ====== Публичный API ======

pub fn mac() -> [u8; 6] {
    unsafe { OUR_MAC }
}

pub fn dump_rx_state() {
    unsafe {
        let rdh = mmio_read32(REG_RDH);
        let rdt = mmio_read32(REG_RDT);
        let rctl = mmio_read32(REG_RCTL);
        let status = mmio_read32(REG_STATUS);
        let icr = mmio_read32(0x00C0);
        let gprc = mmio_read32(0x4074);
        let tpr = mmio_read32(0x40D0);
        uart::write_str("[NET] RDH="); uart_dec(rdh as u64);
        uart::write_str(" RDT="); uart_dec(rdt as u64);
        uart::write_str(" RCTL=0x"); uart_hex(rctl as u64);
        uart::write_str(" STATUS=0x"); uart_hex(status as u64);
        uart::write_str(" ICR=0x"); uart_hex(icr as u64);
        uart::write_str(" GPRC="); uart_dec(gprc as u64);
        uart::write_str(" TPR="); uart_dec(tpr as u64);
        // Check first 4 RX descriptors for DD bit
        for i in 0..4 {
            let d = &*RX_RING.add(i);
            uart::write_str(" RX["); uart_dec(i as u64);
            uart::write_str("].status=0x"); uart_hex(d.status as u64);
            uart::write_str(" len="); uart_dec(d.length as u64);
        }
        uart::write_str("\r\n");
    }
}

/// Software RX test: inject a test ARP frame into the RX ring to test poll/recv
pub fn rx_software_test() {
    unsafe {
        let buf = RX_BUFS[0];
        let data = core::slice::from_raw_parts_mut(buf as *mut u8, 60);
        let mac = OUR_MAC;
        // Build a fake ARP request from 10.0.2.1 to us
        let src_mac: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x35, 0x01];
        data[0..6].copy_from_slice(&src_mac);
        data[6..12].copy_from_slice(&mac);
        data[12] = 0x08; data[13] = 0x06; // ARP
        data[14] = 0x00; data[15] = 0x01; // hw=ethernet
        data[16] = 0x08; data[17] = 0x00; // proto=IPv4
        data[18] = 6; data[19] = 4;
        data[20] = 0x00; data[21] = 0x01; // ARP request
        data[22..28].copy_from_slice(&src_mac);
        data[28..32].copy_from_slice(&[10, 0, 2, 1]); // sender IP = gateway
        let zero_mac: [u8; 6] = [0; 6];
        data[32..38].copy_from_slice(&zero_mac);
        data[38..42].copy_from_slice(&OUR_IP); // target IP = us

        // Manually set DD bit in the RX descriptor
        let desc = &mut *RX_RING.add(0);
        desc.length = 42;
        desc.status = 0x01; // DD
        core::arch::asm!("wbinvd");
    }

    // Now poll should find it
    uart::write_str("[NET] RX software test: calling poll...\r\n");
    poll();
    // Reset RX ring back to clean state
    unsafe {
        mmio_write32(REG_RDH, 0);
        mmio_write32(REG_RDT, (NUM_DESC - 1) as u32);
        RX_HEAD = 0;
    }
    uart::write_str("[NET] RX software test done\r\n");
}

pub fn poll() {
    while rx_available() {
        if let Some((data, _idx)) = rx_recv() {
            if data.len() < 14 { continue; }
            let eth = unsafe { &*(data.as_ptr() as *const EthHdr) };

            if eth.ether_type == ETH_TYPE_ARP && data.len() >= 42 {
                let arp = unsafe { &*(data.as_ptr().add(14) as *const ArpPkt) };
                if arp.op == ARP_REQUEST && arp.target_ip == OUR_IP {
                    let mut resp = [0u8; 60];
                    let len = build_arp_reply(&mut resp, &arp.sender_mac, &arp.sender_ip);
                    tx_send(&resp[..len]);
                    uart::write_str("[NET] ARP reply to ");
                    print_ip(&arp.sender_ip);
                    uart::write_str("\r\n");
                }
            }

            if eth.ether_type == ETH_TYPE_IP && data.len() >= 42 {
                let ip = unsafe { &*(data.as_ptr().add(14) as *const IpHdr) };
                if ip.protocol == IP_PROTO_ICMP && ip.dst == OUR_IP {
                    let icmp = unsafe { &*(data.as_ptr().add(14 + 20) as *const IcmpHdr) };
                    if icmp.typ == 8 { // echo request
                        let payload = &data[14 + 20 + 8..];
                        let mut resp = [0u8; 1514];
                        let len = build_icmp_reply(&mut resp, &eth.src, ip, icmp, payload);
                        tx_send(&resp[..len]);
                        uart::write_str("[NET] ICMP echo reply to ");
                        print_ip(&ip.src);
                        uart::write_str("\r\n");
                    }
                }
            }
        }
    }
}

pub struct E1000Driver;

impl Driver for E1000Driver {
    fn name(&self) -> &'static str {
        "Intel e1000 Gigabit Ethernet"
    }

    fn device_type(&self) -> DeviceType {
        DeviceType::Pci { vendor: 0x8086, device: 0x100E, class: 0x02, subclass: 0x00 }
    }

    fn init(&self) -> DriverStatus {
        if !find_nic() {
            return DriverStatus::Unsupported;
        }

        // Enable PCI master + memory space
        pci_enable_bus_master();

        // Debug: read PCI command register
        let cmd_reg = unsafe { super::pci::read16(0, NIC_DEV, NIC_FUNC, 0x04) };
        uart::write_str("[NET] PCI cmd=");
        uart_dec(cmd_reg as u64);
        uart::write_str("\r\n");

        // Reset + link up
        mmio_write32(REG_CTRL, CTRL_RST);
        for _ in 0..10000 { core::hint::spin_loop(); }
        let ctrl = mmio_read32(REG_CTRL);
        mmio_write32(REG_CTRL, ctrl | CTRL_SLU);

        // Debug: verify MMIO read/write works
        let test_val = mmio_read32(REG_STATUS);
        uart::write_str("[NET] STATUS=");
        uart_dec(test_val as u64);
        uart::write_str("\r\n");

        // Read MAC
        let mac = read_mac();
        unsafe { OUR_MAC = mac; }
        uart::write_str("[NET] MAC: ");
        print_mac(&mac);
        uart::write_str("\r\n");

        if mac[0] == 0 && mac[1] == 0 && mac[2] == 0 {
            return DriverStatus::Error("bad MAC");
        }

        // Allocate DMA memory
        if !dma_init() {
            return DriverStatus::Error("DMA alloc failed");
        }

        dma_rings_configure();
        enable_rx();
        // Enable TX
        mmio_write32(REG_TCTL, TCTL_EN | TCTL_PSP | TCTL_CT | TCTL_COLD);
        mmio_write32(REG_TIPG, 0x0060200A); // default IPG

        uart::write_str("[NET] DMA rings: TX/RX ");
        uart_dec(NUM_DESC as u64);
        uart::write_str(" desc, IP ");
        print_ip(&OUR_IP);
        uart::write_str("\r\n");

        // Check link
        let status = mmio_read32(REG_STATUS);
        let link = (status >> 1) & 1;
        let speed = (status >> 6) & 3; // bits 7:6
        let fd = status & 1;
        uart::write_str("[NET] link="); uart_dec(link as u64);
        uart::write_str(" speed="); uart_dec(speed as u64);
        uart::write_str(" FD="); uart_dec(fd as u64);
        uart::write_str("\r\n");

        // Check TCTL value
        let tctl = mmio_read32(REG_TCTL);
        uart::write_str("[NET] TCTL=0x"); uart_hex(tctl as u64);
        uart::write_str("\r\n");

        // Debug: verify RX state after init
        let rctl = mmio_read32(REG_RCTL);
        let mac_lo = mmio_read32(REG_MAC_LO);
        let mac_hi = mmio_read32(REG_MAC_HI);
        uart::write_str("[NET] RCTL=0x"); uart_hex(rctl as u64);
        uart::write_str(" MAC_LO=0x"); uart_hex(mac_lo as u64);
        uart::write_str(" MAC_HI=0x"); uart_hex(mac_hi as u64);
        uart::write_str("\r\n");

        DriverStatus::Ok
    }
}

fn uart_hex(mut val: u64) {
    if val == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut buf = [0u8; 16];
    let mut i = 0;
    while val > 0 {
        let nib = (val & 0xF) as u8;
        buf[i] = if nib < 10 { b'0' + nib } else { b'A' + nib - 10 };
        val >>= 4;
        i += 1;
    }
    while i > 0 { i -= 1; crate::driver::uart::putchar(buf[i]); }
}

/// Test TX: send an ARP request and check descriptor status
pub fn tx_test() {
    // Use a raw descriptor write, bypassing tx_send, for testing
    unsafe {
        let tdh = mmio_read32(REG_TDH);
        let tdt = mmio_read32(REG_TDT);
        uart::write_str("[NET] TX test: TDH="); uart_dec(tdh as u64);
        uart::write_str(" TDT="); uart_dec(tdt as u64);
        uart::write_str("\r\n");

        // Build packet directly in TX buffer 0
        let buf = TX_BUFS[0];
        let pkt = core::slice::from_raw_parts_mut(buf as *mut u8, 60);
        // Ethernet: broadcast
        let dst: [u8; 6] = [0xFF; 6];
        let eth: &mut [u8; 14] = &mut *(pkt.as_mut_ptr() as *mut [u8; 14]);
        eth[0..6].copy_from_slice(&dst);
        eth[6..12].copy_from_slice(&*core::ptr::addr_of!(OUR_MAC));
        eth[12] = 0x08; eth[13] = 0x06; // ARP
        // ARP request
        let arp: &mut [u8; 28] = &mut *(pkt.as_mut_ptr().add(14) as *mut [u8; 28]);
        arp[0] = 0x00; arp[1] = 0x01; // hw = ethernet
        arp[2] = 0x08; arp[3] = 0x00; // proto = IPv4
        arp[4] = 6; arp[5] = 4; // hw/proto len
        arp[6] = 0x00; arp[7] = 0x01; // request
        arp[8..14].copy_from_slice(&*core::ptr::addr_of!(OUR_MAC));
        arp[14..18].copy_from_slice(&OUR_IP);
        let zero_mac: [u8; 6] = [0; 6];
        arp[18..24].copy_from_slice(&zero_mac);
        let gw: [u8; 4] = [10, 0, 2, 1];
        arp[24..28].copy_from_slice(&gw);

        let len = 42usize;

        // Set up TX descriptor 0
        let desc = &mut *TX_RING.add(0);
        desc.addr = buf;
        let cmd_byte: u8 = 0x01 | 0x02 | 0x08; // EOP | IFCS | RS
        desc.cmd_len = ((cmd_byte as u32) << 24) | (len as u32);
        desc.oinfo_sta = 0;

        uart::write_str("[NET] desc.cmd_len=0x"); uart_hex(desc.cmd_len as u64);
        uart::write_str(" oinfo_sta=0x"); uart_hex(desc.oinfo_sta as u64);
        uart::write_str("\r\n");

        // Flush cache
        core::arch::asm!("wbinvd");

        // Ring doorbell
        mmio_write32(REG_TDT, 1);

        uart::write_str("[NET] TDT written, waiting for DD...\r\n");
    }

    // Wait up to 1 second using timer
    let deadline = crate::timer::millis() + 1000;
    loop {
        let desc = unsafe { &*TX_RING.add(0) };
        if desc.oinfo_sta & 0x01 != 0 {
            uart::write_str("[NET] TX DD=1! status=0x"); uart_hex(desc.oinfo_sta as u64);
            uart::write_str("\r\n");
            break;
        }
        if crate::timer::millis() >= deadline {
            // Timeout
            let tdh = mmio_read32(REG_TDH);
            let tdt = mmio_read32(REG_TDT);
            uart::write_str("[NET] TX timeout! cmd_len=0x"); uart_hex(desc.cmd_len as u64);
            uart::write_str(" oinfo_sta=0x"); uart_hex(desc.oinfo_sta as u64);
            uart::write_str(" TDH="); uart_dec(tdh as u64);
            uart::write_str(" TDT="); uart_dec(tdt as u64);
            uart::write_str("\r\n");
            break;
        }
        core::hint::spin_loop();
    }

    // Reset TX ring state so subsequent sends work
    mmio_write32(REG_TDH, 0);
    mmio_write32(REG_TDT, 0);
    unsafe { TX_TAIL = 0; }
    uart::write_str("[NET] TX ring reset: TDH=0 TDT=0\r\n");
}

/// Send ICMP echo request to target IP (ping bypassing ARP)
/// Uses broadcast MAC — slirp responds with IP-level ICMP reply
pub fn send_icmp_ping(target_ip: [u8; 4]) -> bool {
    let mac = unsafe { OUR_MAC };
    // Build Ethernet + IP + ICMP packet (60 bytes min)
    let mut pkt = [0u8; 60];
    // Ethernet: broadcast destination
    let bcast: [u8; 6] = [0xFF; 6];
    pkt[0..6].copy_from_slice(&bcast);
    pkt[6..12].copy_from_slice(&mac);
    pkt[12] = 0x08; pkt[13] = 0x00; // IPv4
    // IP header (20 bytes)
    let ip_len: u16 = 20 + 8; // IP hdr + ICMP hdr
    pkt[14] = 0x45; // ver_ihl
    pkt[15] = 0;    // dscp
    pkt[16] = (ip_len >> 8) as u8;
    pkt[17] = (ip_len & 0xFF) as u8;
    pkt[18] = 0; pkt[19] = 0; // id
    pkt[20] = 0; pkt[21] = 0; // flags_frag
    pkt[22] = 64; // ttl
    pkt[23] = 1;  // ICMP protocol
    pkt[24] = 0; pkt[25] = 0; // checksum = 0
    pkt[26..30].copy_from_slice(&OUR_IP); // src IP
    pkt[30..34].copy_from_slice(&target_ip); // dst IP
    // IP checksum
    let csum = ip_checksum(&pkt[14..34]);
    pkt[24] = (csum >> 8) as u8;
    pkt[25] = (csum & 0xFF) as u8;
    // ICMP echo request (8 bytes)
    pkt[34] = 8;  // type = echo request
    pkt[35] = 0;  // code
    pkt[36] = 0; pkt[37] = 0; // csum = 0
    pkt[38] = 0; pkt[39] = 0; // id
    pkt[40] = 0; pkt[41] = 0; // seq
    // ICMP checksum
    let icmp_csum = ip_checksum(&pkt[34..42]);
    pkt[36] = (icmp_csum >> 8) as u8;
    pkt[37] = (icmp_csum & 0xFF) as u8;

    tx_send(&pkt[..42])
}

fn uart_dec(val: u64) {
    if val == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut buf = [0u8; 20];
    let mut i = 0;
    let mut v = val;
    while v > 0 { buf[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; crate::driver::uart::putchar(buf[i]); }
}
