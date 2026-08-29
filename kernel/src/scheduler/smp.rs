// SMP: bring-up of application processors via INIT-SIPI-SIPI.
//
// The kernel is a UEFI PE application linked by rust-lld (COFF flavor) with a
// loader-assigned image base, so link-time "0x100000" assumptions do NOT hold.
// The trampoline (.smp section) is raw machine code with its own low-memory
// workspace. All pointers are filled by the BSP at runtime; the code itself
// only references fixed low addresses (TRAMP_PHYS + constants).

use core::arch::asm;
use crate::io;

const TRAMP_PHYS: u64 = 0x8000;

// Workspace for the trampoline. IMPORTANT: must NOT overlap the .smp code
// that is copied to TRAMP_PHYS (the .smp section spans 0x8000..len). The
// offsets below are relative to W_BASE, which sits at 0x7800: below the code
// (>= 0x8000) and above the AP GDTs (<= 0x7200).
const W_BASE: u64 = 0x7800;

// Workspace offsets (relative to W_BASE), stable.
const W_GDTD: usize = 0x160;  // u16 limit + u64 base (real-mode GDT desc)
const W_IDTD: usize = 0x16A;  // u16 limit + u64 base (real-mode IDT desc, 0)
const W_LGDT: usize = 0x174;  // u16 limit + u64 base (long-mode GDT desc)
const W_LIDT: usize = 0x17E;  // u16 limit + u64 base (long-mode IDT desc, 0)
const W_PML4: usize = 0x188;  // u64
const W_STACK: usize = 0x190; // u64
const W_APIC: usize = 0x198;  // u64
const W_ENTRY: usize = 0x1A0; // u64
const W_CONSUMED: usize = 0x1A8; // u64: 1 once the AP has read the workspace

// Reserved low-memory regions (below 1 MB, identity-mapped).
const PG_PML4: u64 = 0x5000;
const PG_PDPT: u64 = 0x5100;
const PG_PD: u64 = 0x5200;
const PG_PT: u64 = 0x5300;
const AP0_GDT: u64 = 0x7000;
const AP1_GDT: u64 = 0x7200;
const AP0_STACK: u64 = 0x9000;
const AP1_STACK: u64 = 0x20000;

const LAPIC_HH_BASE: u64 = 0xFFFF_FFFF_FEE0_0000;
const ICR: u32 = 0x300;

core::arch::global_asm!(
    ".section .smp, \"ax\"",

    // UART trace: print one character via COM1. Poll is BOUNDED so the AP can
    // never stall the boot forever on a stuck transmitter. Clobbers ax, cx, dx.
    ".macro ap_trace chr",
    "  mov cx, 0xFFFF",
    "1: mov dx, 0x3FD",
    "  in al, dx",
    "  test al, 0x20",
    "  jnz 2f",
    "  dec cx",
    "  jnz 1b",
    "2: mov dx, 0x3F8",
    "  mov al, \\chr",
    "  out dx, al",
    ".endm",

    // ---- 16-bit real mode ----
    ".code16",
    "_smp_start:",
    "  cli",
    "  ap_trace 'A'",
    "  xor ax, ax",
    "  mov ds, ax",
    "  mov es, ax",
    "  mov ss, ax",
    "  mov sp, 0x7C00",
    "  mov al, 0x02",
    "  out 0x92, al",
    "  ap_trace 'B'",
    "  lgdt [0x7960]",              // real-mode GDT descriptor (W_BASE+0x160)
    "  ap_trace '1'",
    "  lidt [0x796A]",              // zero IDT
    "  ap_trace '2'",
    "  mov eax, cr0",
    "  or al, 1",
    "  mov cr0, eax",
    "  ap_trace '3'",
    // In 16-bit protected mode (PE=1) now. Load all data segments from the GDT
    // including SS (selector 0x0000 would be a NULL selector -> #GP on the
    // first push). The 'P' marker is emitted only if 0x10 resolves cleanly.
    "  mov ax, 0x10",
    "  mov ds, ax",
    "  mov es, ax",
    "  mov ss, ax",
    "  mov esp, 0x7C00",
    "  ap_trace 'P'",
    // Stack-based far transfer to 32-bit protected mode: avoids the immediate
    // far-jump (EA) decode path of the TCG emulator. push imm8 selector, then
    // operand-size-overridden push imm32 offset, then far-return.
    ".byte 0x6A, 0x18",             // push 0x18 (32-bit code selector)
    ".byte 0x66, 0x68",             // pushl imm32
    ".long (_smp_32 - _smp_start) + 0x8000", // absolute 32-bit target
    ".byte 0x66, 0xCB",             // retfl: pop EIP32, pop CS16 -> switch mode

    "    // ---- 32-bit protected mode ----",
    ".code32",
    "_smp_32:",
    "  ap_trace 'X'",
    "  ap_trace 'C'",
    "  mov ax, 0x10",
    "  mov ds, ax",
    "  mov es, ax",
    "  mov ss, ax",
    "  ap_trace 'E'",
    "  mov edx, 0x5000",             // mini transition page tables
    "  mov cr3, edx",
    "  ap_trace 'F'",
    "  mov eax, 0xA0",               // CR4.PAE | CR4.PGE
    "  mov cr4, eax",
    "  mov ecx, 0xC0000080",         // EFER
    "  rdmsr",
    "  or eax, 0x100",               // LME
    "  wrmsr",
    "  ap_trace 'H'",               // LME set, paging about to be enabled
    "  mov eax, cr0",
    "  or eax, 0x80000000",          // PG
    "  mov cr0, eax",
    // Enter long mode with an IMMEDIATE far jump, exactly like Linux/xv6:
    // the first instruction after enabling paging must be the mode switch so
    // the TCG translator (and real silicon) refetch in 64-bit mode cleanly.
    ".byte 0xEA",                   // ljmpl: imm32 offset, then imm16 selector
    ".long (_smp_64 - _smp_start) + 0x8000", // absolute 64-bit target in low RAM
    ".word 0x0008",                 // CS = 0x08 (64-bit code, L=1)

    // ---- 64-bit long mode ----
    ".code64",
    "_smp_64:",
    "  ap_trace 'J'",               // paging on + long mode reached via the far jump
    "  ap_trace 'D'",
    "  mov ax, 0x10",
    "  mov ds, ax",
    "  mov es, ax",
    "  mov ss, ax",
    "  mov fs, ax",
    "  mov gs, ax",
    "  lgdt [0x7974]",               // W_LGDT
    "  lidt [0x797E]",               // W_LIDT
    "  mov rsp, qword ptr [0x7990]", // W_STACK
    "  mov rdi, qword ptr [0x7988]", // W_PML4 (BSP page tables)
    "  mov rsi, qword ptr [0x7998]", // W_APIC
    "  mov rdx, rsp",
    "  mov rax, qword ptr [0x79A0]", // W_ENTRY
    "  mov rbx, 1",
    "  mov qword ptr [0x79A8], rbx", // W_CONSUMED=1: AP finished reading workspace
    "  mov cr3, rdi",               // adopt BSP address space
    "  call rax",
    "loop_here:",
    "  cli",
    "  hlt",
    "  jmp loop_here",
    "_smp_end:",
);

fn pstore_u16(a: u64, off: usize, v: u16) {
    unsafe { ((a + off as u64) as *mut u16).write_volatile(v); }
}
fn pstore_parts(a: u64, off: usize, val: u64) {
    unsafe {
        let p = (a + off as u64) as *mut u8;
        for i in 0..8 {
            p.add(i).write_volatile((val >> (8 * i)) as u8);
        }
    }
}
fn pstore_u64(a: u64, off: usize, v: u64) {
    unsafe { ((a + off as u64) as *mut u64).write_volatile(v); }
}

fn build_gdt(gdt_base: u64) {
    pstore_u64(gdt_base, 0x00, 0);
    pstore_u64(gdt_base, 0x08, 0x00209A0000000000); // kernel code (L=1)
    pstore_u64(gdt_base, 0x10, 0x00CF92000000FFFF); // kernel data (G=1, 32-bit)
    pstore_u64(gdt_base, 0x18, 0x00CF9A000000FFFF); // 32-bit code (D=1, L=0)
}

unsafe fn build_ap_page_tables() {
    for a in [PG_PML4, PG_PDPT, PG_PD, PG_PT].iter() {
        core::ptr::write_bytes(*a as *mut u64, 0, 512 * 8);
    }
    pstore_u64(PG_PML4, 0x000, PG_PDPT | 0x3);           // PML4[0]
    pstore_u64(PG_PDPT, 0x000, PG_PD | 0x3);             // PDPT[0]
    pstore_u64(PG_PD, 0x000, 0x0 | 0x183);               // PD[0]: 2MB identity
    // LAPIC high-half.
    pstore_u64(PG_PML4, 0x1FF * 8, PG_PDPT | 0x3);       // PML4[511]
    pstore_u64(PG_PDPT, 0x1FF * 8, PG_PD | 0x3);         // PDPT[511]
    pstore_u64(PG_PD, 0x1F7 * 8, PG_PT | 0x3);           // PD[0x1F7]: FEE00000 >> 21 = 0x1F7
    pstore_u64(PG_PT, 0x000, 0xFEE00000 | 0x13);         // PT[0]: LAPIC
}

/// Copy the .smp block to TRAMP_PHYS and fill workspace descriptors.
unsafe fn prepare_trampoline(ap_index: usize, apic_id: u32) {
    extern "C" { static _smp_start: u8; static _smp_32: u8; static _smp_64: u8; static _smp_end: u8; }
    let start = &raw const _smp_start as u64;
    let off32 = &raw const _smp_32 as u64 - start;
    let off64 = &raw const _smp_64 as u64 - start;
    let len = (&raw const _smp_end as u64) - start;
    crate::driver::uart::write_str("[SMP] tramp start=0x");
    uart_hex(start);
    crate::driver::uart::write_str(" off32=0x");
    uart_hex(off32);
    crate::driver::uart::write_str(" off64=0x");
    uart_hex(off64);
    crate::driver::uart::write_str(" len=0x");
    uart_hex(len);
    crate::driver::uart::write_str("\r\n");
    // Dump the SOURCE section bytes (from _smp_start) to compare against the
    // copy and the on-disk ELF .smp section.
    {
        let p = start as usize;
        crate::driver::uart::write_str("[SMP] src[60..B0]:");
        for i in 0x60usize..0xB0 {
            if i % 16 == 0 {
                crate::driver::uart::write_str("\r\n  ");
                uart_hex(i as u64);
                crate::driver::uart::write_str(": ");
            }
            let b: u8 = ((p + i) as *const u8).read_volatile();
            uart_hex(b as u64);
            crate::driver::uart::write_str(" ");
        }
        crate::driver::uart::write_str("\r\n");
    }
    for i in 0..len as usize {
        ((TRAMP_PHYS + i as u64) as *mut u8).write_volatile(
            ((start + i as u64) as *const u8).read_volatile(),
        );
    }
    crate::driver::uart::write_str("[SMP] tramp copied\r\n");

    // Verify far-jump bytes at the copy site: dump from offset 0x60 to 0xB0
    // (covers the '2'/'3' traces, the 16->32 far jump and _smp_32 head).
    {
        let p = TRAMP_PHYS as usize;
        crate::driver::uart::write_str("[SMP] copy[60..B0]:");
        for i in 0x60usize..0xB0 {
            if i % 16 == 0 {
                crate::driver::uart::write_str("\r\n  ");
                uart_hex(i as u64);
                crate::driver::uart::write_str(": ");
            }
            let b: u8 = ((p + i) as *const u8).read_volatile();
            uart_hex(b as u64);
            crate::driver::uart::write_str(" ");
        }
        crate::driver::uart::write_str("\r\n");
    }

    // Verify the far-jump target offset computed in asm matches the Rust symbol.
    {
        crate::driver::uart::write_str("[SMP] off32 target byte: ");
        let target: u8 = ((TRAMP_PHYS + off32 as u64) as *const u8).read_volatile();
        uart_hex(target as u64);
        crate::driver::uart::write_str(" <- first byte of _smp_32\r\n");
    }

    // Far-jump targets are now assembled as absolute low-RAM addresses
    // ((label - _smp_start) + 0x8000), no runtime patching needed.

    let (gdt_base, stack_base) = if ap_index == 0 {
        (AP0_GDT, AP0_STACK)
    } else {
        (AP1_GDT, AP1_STACK)
    };
    build_gdt(gdt_base);
    crate::driver::uart::write_str("[SMP] gdt built: gdt[0x18]=");
    for i in 0usize..8 {
        let b: u8 = ((gdt_base + 0x18 + i as u64) as *const u8).read_volatile();
        uart_hex(b as u64);
        crate::driver::uart::write_str(" ");
    }
    crate::driver::uart::write_str("\r\n");

    pstore_u16(W_BASE, W_GDTD, 31);
    pstore_parts(W_BASE, W_GDTD + 2, gdt_base);
    pstore_u16(W_BASE, W_IDTD, 0);
    pstore_parts(W_BASE, W_IDTD + 2, 0);
    pstore_u16(W_BASE, W_LGDT, 31);
    pstore_parts(W_BASE, W_LGDT + 2, gdt_base);
    pstore_u16(W_BASE, W_LIDT, 0);
    pstore_parts(W_BASE, W_LIDT + 2, 0);
    pstore_u64(W_BASE, W_CONSUMED, 0); // reset AP-consumed flag before launch
    crate::driver::uart::write_str("[SMP] descriptors ok\r\n");
    // Dump the GDTR pseudo-descriptor the AP loads via lgdt [0x7960]:
    // u16 limit + u64 base.
    {
        let p = (W_BASE + W_GDTD as u64) as usize;
        crate::driver::uart::write_str("[SMP] w_gdtd[0x7960]:");
        for i in 0usize..10 {
            let b: u8 = ((p + i) as *const u8).read_volatile();
            uart_hex(b as u64);
            crate::driver::uart::write_str(" ");
        }
        crate::driver::uart::write_str("\r\n");
    }

    let bsp_pml4: u64;
    asm!("mov {}, cr3", out(reg) bsp_pml4);
    crate::driver::uart::write_str("[SMP] cr3 read\r\n");
    pstore_u64(W_BASE, W_PML4, bsp_pml4);
    pstore_u64(W_BASE, W_STACK, stack_base + 0x10000);
    pstore_u64(W_BASE, W_APIC, apic_id as u64);
    pstore_u64(W_BASE, W_ENTRY, ap_entry as *const () as usize as u64);
    crate::driver::uart::write_str("[SMP] constants ok\r\n");
}

/// The AP trampoline switches CR3 to the BSP address space while still executing
/// from TRAMP_PHYS (0x8000) with its stacks in low RAM (AP0 0x9000-0x19000,
/// AP1 0x20000-0x30000). Those instructions and stack writes must resolve in
/// the BSP page tables, so make sure the kernel's own PML4 identity-maps the
/// low 2MB. Limine usually maps this already; otherwise create a writable 2MB
/// identity mapping here.
unsafe fn ensure_bsp_low_identity() {
    use crate::vm::{identity_map_2mb, KERNEL_PML4, PTE_WRITABLE, virt_to_phys};
    let pml4 = KERNEL_PML4 as *mut u64;
    if virt_to_phys(pml4, 0x18000) != 0 {
        crate::driver::uart::write_str("[SMP] low 2MB already mapped in BSP PML4\r\n");
        return;
    }
    if identity_map_2mb(pml4, 0, 0x200000, PTE_WRITABLE) {
        crate::driver::uart::write_str("[SMP] low 2MB identity-mapped (RW)\r\n");
    } else {
        crate::driver::uart::write_str("[SMP] WARN: could not map low 2MB\r\n");
    }
}

fn low_delay() {
    let mut i = 0;
    while i < 5_000_000 {
        i += 1;
        unsafe { asm!("pause"); }
    }
}

/// The trampoline workspace is shared by all APs. Before reusing it for the
/// next CPU we must wait until the just-launched AP has read every field (it
/// sets W_CONSUMED right before switching CR3). Bounded poll so a crash in the
/// trampoline cannot hang the BSP forever.
fn wait_consumed() -> bool {
    let flag = (W_BASE + W_CONSUMED as u64) as *mut u64;
    let mut spins: u64 = 0;
    while unsafe { flag.read_volatile() } != 1 {
        spins += 1;
        if spins >= 200_000_000 {
            return false;
        }
        unsafe { asm!("pause"); }
    }
    unsafe { flag.write_volatile(0); }
    true
}

unsafe fn lapic_read(offset: u32) -> u32 {
    io::mmio_read32((LAPIC_HH_BASE + offset as u64) as *const u32)
}
unsafe fn lapic_write(offset: u32, value: u32) {
    io::mmio_write32((LAPIC_HH_BASE + offset as u64) as *mut u32, value);
}

unsafe fn send_icr(apic_id: u32, command: u32) {
    crate::driver::uart::write_str("[SMP] ICR high=");
    uart_hex((apic_id as u64) << 24);
    crate::driver::uart::write_str(" cmd=");
    uart_hex(command as u64);
    crate::driver::uart::write_str("\r\n");
    // ICR high dword (0x310) carries the APIC ID destination; the low dword
    // (0x300) triggers the send.
    lapic_write(ICR + 0x10, apic_id << 24);
    crate::driver::uart::write_str("[SMP] ICR high written\r\n");
    let v = lapic_read(ICR);
    crate::driver::uart::write_str("[SMP] ICR read before cmnd=");
    uart_hex(v as u64);
    crate::driver::uart::write_str("\r\n");
    lapic_write(ICR, command);
    crate::driver::uart::write_str("[SMP] ICR committed\r\n");
    let mut spins = 0u32;
    loop {
        let s = lapic_read(ICR);
        if s & (1 << 12) == 0 {
            break;
        }
        spins += 1;
        if spins % 1_000_000 == 0 {
            crate::driver::uart::write_str("[SMP] ICR busy spins=");
            uart_dec(spins as u64);
            crate::driver::uart::write_str(" val=");
            uart_hex(s as u64);
            crate::driver::uart::write_str("\r\n");
        }
        unsafe { asm!("pause"); }
    }
    crate::driver::uart::write_str("[SMP] ICR delivered\r\n");
}

unsafe fn send_init_sipi(apic_id: u32) {
    crate::driver::uart::write_str("[SMP] INIT\r\n");
    send_icr(apic_id, 0x00000500); // INIT
    low_delay();
    crate::driver::uart::write_str("[SMP] SIPI1\r\n");
    send_icr(apic_id, 0x00000608); // SIPI (vector 0x08 -> 0x8000)
    low_delay();
    crate::driver::uart::write_str("[SMP] SIPI2\r\n");
    send_icr(apic_id, 0x00000608); // SIPI again
    low_delay();
    crate::driver::uart::write_str("[SMP] SIPI done\r\n");
}

fn uart_dec(mut v: u64) {
    use crate::driver::uart;
    if v == 0 {
        uart::putchar(b'0');
        return;
    }
    let mut b = [0u8; 20];
    let mut i = 0;
    while v > 0 {
        b[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        uart::putchar(b[i]);
    }
}

fn uart_hex(v: u64) {
    use crate::driver::uart;
    let hex = b"0123456789ABCDEF";
    uart::write_str("0x");
    for i in (0..16).rev() {
        uart::putchar(hex[((v >> (i * 4)) & 0xF) as usize]);
    }
}

/// Bring up all other CPUs listed in the MADT.
pub fn init() {
    unsafe {
        crate::driver::uart::write_str("[SMP] init enter\r\n");
        let ncpu = crate::acpi::ncpu();
        if ncpu <= 1 {
            crate::driver::uart::write_str("[SMP] single CPU, none to wake\r\n");
            return;
        }
        build_ap_page_tables();
        crate::driver::uart::write_str("[SMP] page tables ok\r\n");
        ensure_bsp_low_identity();
        {
            crate::driver::uart::write_str("[SMP] mini PT: PML4[0]=");
            uart_hex(((PG_PML4 + 0x000) as *const u64).read_volatile());
            crate::driver::uart::write_str(" PML4[511]=");
            uart_hex(((PG_PML4 + 0x1FF * 8) as *const u64).read_volatile());
            crate::driver::uart::write_str("\r\n[SMP] mini PT: PDPT[0]=");
            uart_hex(((PG_PDPT + 0x000) as *const u64).read_volatile());
            crate::driver::uart::write_str(" PDPT[511]=");
            uart_hex(((PG_PDPT + 0x1FF * 8) as *const u64).read_volatile());
            crate::driver::uart::write_str("\r\n[SMP] mini PT: PD[0]=");
            uart_hex(((PG_PD + 0x000) as *const u64).read_volatile());
            crate::driver::uart::write_str(" PD[0x1F7]=");
            uart_hex(((PG_PD + 0x1F7 * 8) as *const u64).read_volatile());
            crate::driver::uart::write_str(" PT[0]=");
            uart_hex(((PG_PT + 0x000) as *const u64).read_volatile());
            crate::driver::uart::write_str("\r\n");
        }

        let max = ncpu.min(3); // two AP slots reserved
        for cpu in 1..max {
            let apic_id = match crate::acpi::apic_id_of(cpu) {
                Some(id) => id as u32,
                None => continue,
            };
            prepare_trampoline(cpu - 1, apic_id);
            crate::driver::uart::write_str("[SMP] trampoline ok\r\n");
            crate::driver::uart::write_str("[SMP] waking CPU ");
            uart_dec(cpu as u64);
            crate::driver::uart::write_str(" apic=");
            uart_dec(apic_id as u64);
            crate::driver::uart::write_str(" ...\r\n");
            send_init_sipi(apic_id);
            if !wait_consumed() {
                crate::driver::uart::write_str(
                    "[SMP] WARN: AP did not consume workspace in time, stopping\r\n",
                );
                break;
            }
        }
        crate::driver::uart::write_str("[SMP] bring-up done\r\n");
    }
}

/// Called by the trampoline on an AP in long mode.
#[no_mangle]
unsafe extern "C" fn ap_entry(pml4: u64, apic_id: u64, stack: u64) -> ! {
    crate::driver::uart::write_str("[AP] alive, apic_id=");
    uart_dec(apic_id);
    crate::driver::uart::write_str(" stack=");
    uart_hex(stack);
    crate::driver::uart::write_str(" pml4=");
    uart_hex(pml4);
    crate::driver::uart::write_str("\r\n");
    loop {
        asm!("hlt", options(nostack, preserves_flags));
    }
}