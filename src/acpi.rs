use crate::io;

fn uart_print(s: &str) { crate::driver::uart::write_str(s); }

fn uart_hex(mut val: u64) {
    if val == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut b = [0u8; 16]; let mut i = 0;
    while val > 0 { let n = (val & 0xF) as u8; b[i] = if n < 10 { b'0' + n } else { b'A' + n - 10 }; val >>= 4; i += 1; }
    while i > 0 { i -= 1; crate::driver::uart::putchar(b[i]); }
}

fn uart_dec(mut val: u64) {
    if val == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut b = [0u8; 20]; let mut i = 0;
    while val > 0 { b[i] = b'0' + (val % 10) as u8; val /= 10; i += 1; }
    while i > 0 { i -= 1; crate::driver::uart::putchar(b[i]); }
}

#[repr(C, packed)]
struct AcpiHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}



/// Saved ACPI data (filled before EBS, parsed after EBS).
const RSDP_COPY_SIZE: usize = 36;
static mut RSDP_PHYS: u64 = 0;
static mut RSDP_COPY: [u8; RSDP_COPY_SIZE] = [0u8; RSDP_COPY_SIZE];
static mut XSDT_PHYS: u64 = 0;
static mut XSDT_LEN: u32 = 0;
static mut XSDT_COPY: [u8; 8192] = [0u8; 8192];  // max XSDT size
static mut FADT_PHYS: u64 = 0;
static mut FADT_COPY: [u8; 256] = [0u8; 256];    // max FADT size
static mut RESET_REG_ADDR: u64 = 0;
static mut RESET_REG_SPACE: u8 = 0;
static mut RESET_VALUE: u8 = 0;
static mut PM1A_CNT: u32 = 0;

pub unsafe fn set_rsdp(addr: u64) { RSDP_PHYS = addr; }
pub fn get_rsdp_addr() -> u64 { unsafe { RSDP_PHYS } }

/// Copy ACPI table data BEFORE ExitBootServices using only byte reads.
unsafe fn copy_from_phys(buf: &mut [u8], phys: u64, len: usize) -> bool {
    for i in 0..len {
        buf[i] = core::ptr::read_volatile(phys.wrapping_add(i as u64) as *const u8);
    }
    true
}

fn u32_at(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]])
}

fn u64_at(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        buf[off], buf[off+1], buf[off+2], buf[off+3],
        buf[off+4], buf[off+5], buf[off+6], buf[off+7],
    ])
}

pub unsafe fn copy_tables() {
    if RSDP_PHYS == 0 { uart_print("[ACPI] no RSDP\r\n"); return; }

    // Copy RSDP
    copy_from_phys(&mut RSDP_COPY, RSDP_PHYS, RSDP_COPY_SIZE);
    uart_print("[ACPI] RSDP sig: ");
    for &b in &RSDP_COPY[..8] { crate::driver::uart::putchar(b); }
    uart_print(" rev=");
    uart_dec(RSDP_COPY[15] as u64);
    uart_print("\r\n");
    uart_print("[ACPI] RSDP rsdt_phys=0x");
    uart_hex(u32_at(&RSDP_COPY, 16) as u64);
    uart_print(" xsdt_phys=0x");
    uart_hex(u64_at(&RSDP_COPY, 24));
    uart_print("\r\n");

    let rev = RSDP_COPY[15];
    let xsdt_phys = if rev >= 2 { u64_at(&RSDP_COPY, 24) } else { 0 };

    if xsdt_phys != 0 {
        // Read XSDT length from header (bytes 4-7 at xsdt_phys)
        let mut len_buf = [0u8; 4];
        copy_from_phys(&mut len_buf, xsdt_phys.wrapping_add(4), 4);
        let xsdt_len = u32::from_le_bytes(len_buf) as usize;

        if xsdt_len > XSDT_COPY.len() {
            uart_print("[ACPI] XSDT too big: "); uart_dec(xsdt_len as u64); uart_print("\r\n");
            return;
        }
        copy_from_phys(&mut XSDT_COPY[..xsdt_len], xsdt_phys, xsdt_len);
        XSDT_PHYS = xsdt_phys;
        XSDT_LEN = xsdt_len as u32;
        uart_print("[ACPI] XSDT copied, len="); uart_dec(xsdt_len as u64); uart_print("\r\n");

        // Find FACP
        let hdr_sz = core::mem::size_of::<AcpiHeader>();
        let count = (xsdt_len - hdr_sz) / 8;
        let mut fadt_phys = 0u64;
        for i in 0..count {
            let entry = u64_at(&XSDT_COPY, hdr_sz + i * 8);
            if entry == 0 { continue; }
            let mut sig_buf = [0u8; 4];
            copy_from_phys(&mut sig_buf, entry, 4);
            if u32::from_le_bytes(sig_buf) == u32::from_le_bytes(*b"FACP") {
                fadt_phys = entry;
                uart_print("[ACPI] FACP at 0x"); uart_hex(entry); uart_print("\r\n");
                break;
            }
        }

        if fadt_phys != 0 {
            // Read FADT length
            let mut flen_buf = [0u8; 4];
            copy_from_phys(&mut flen_buf, fadt_phys.wrapping_add(4), 4);
            let fadt_len = u32::from_le_bytes(flen_buf) as usize;
            if fadt_len > FADT_COPY.len() {
                uart_print("[ACPI] FADT too big\r\n"); return;
            }
            copy_from_phys(&mut FADT_COPY[..fadt_len], fadt_phys, fadt_len);
            FADT_PHYS = fadt_phys;
            uart_print("[ACPI] FADT copied, len="); uart_dec(fadt_len as u64); uart_print("\r\n");

            // Extract fields from packed FADT:
            // reset_reg: GenericAddr at offset 115 (8-bit fields at 115-118, u64 at 119)
            // reset_value: u8 at offset 127
            // pm1a_cnt: u32 at offset 64
            RESET_REG_ADDR = u64_at(&FADT_COPY, 119);
            RESET_REG_SPACE = FADT_COPY[115];
            RESET_VALUE = FADT_COPY[127];
            PM1A_CNT = u32_at(&FADT_COPY, 64);

            uart_print("[ACPI] reset_reg space="); uart_dec(RESET_REG_SPACE as u64);
            uart_print(" addr=0x"); uart_hex(RESET_REG_ADDR);
            uart_print(" val=0x"); uart_hex(RESET_VALUE as u64);
            uart_print("\r\n[ACPI] pm1a_cnt=0x"); uart_hex(PM1A_CNT as u64);
            uart_print("\r\n[ACPI] OK\r\n");
        } else {
            uart_print("[ACPI] FACP not found\r\n");
        }
    } else {
        let rsdt_phys = u32_at(&RSDP_COPY, 16) as u64;
        if rsdt_phys != 0 {
            uart_print("[ACPI] RSDT fallback not impl\r\n");
        } else {
            uart_print("[ACPI] no RSDT/XSDT\r\n");
        }
    }
}

pub fn init() {
    if unsafe { RSDP_PHYS == 0 } {
        uart_print("[ACPI] no RSDP\r\n");
        return;
    }
    if unsafe { FADT_PHYS == 0 } {
        uart_print("[ACPI] FADT not copied\r\n");
        return;
    }
    uart_print("[ACPI] initialized (pre-EBS copy)\r\n");
}

pub fn reboot() {
    unsafe {
        if RESET_REG_ADDR != 0 {
            match RESET_REG_SPACE {
                1 => io::outb(RESET_REG_ADDR as u16, RESET_VALUE),
                _ => core::ptr::write_volatile(RESET_REG_ADDR as *mut u8, RESET_VALUE),
            }
            return;
        }
    }
    loop { unsafe { io::outb(0x64, 0xFE); } }
}

pub fn shutdown() {
    unsafe {
        if PM1A_CNT != 0 {
            io::outw(PM1A_CNT as u16, (5 << 10) | (1 << 13));
            return;
        }
    }
    unsafe { io::outw(0x604, 0x2000); }
}
