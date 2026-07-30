use core::arch::asm;

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct IdtEntry {
    off_lo: u16,
    sel: u16,
    ist: u8,
    flags: u8,
    off_mid: u16,
    off_hi: u32,
    _rsvd: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        IdtEntry { off_lo: 0, sel: 0, ist: 0, flags: 0, off_mid: 0, off_hi: 0, _rsvd: 0 }
    }
    pub fn set_handler(&mut self, handler: u64, selector: u16) {
        self.off_lo = handler as u16;
        self.off_mid = (handler >> 16) as u16;
        self.off_hi = (handler >> 32) as u32;
        self.sel = selector;
        self.flags = 0x8E;
    }
}

pub static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

// Exceptions with error code: #DF(8), #TS(10), #NP(11), #SS(12), #GP(13), #PF(14), #AC(17)
// For these, CPU pushes error_code after RIP/CS/RFLAGS.
// For all others, no error code.
//
// Per-vector stubs:
//   Non-EC: push 0 (dummy), push <vec>, jmp exception_common
//   EC:     push <vec>, jmp exception_common
// After GPR saves: [R15..RAX, vec, error_code_or_0, RIP, CS, RFLAGS]
//                  vec at RSP+15*8, error_code at RSP+16*8

core::arch::global_asm!(
    ".macro exc_stub n, has_ec",
    "  .globl exc_stub_\\n",
    "  .balign 4",
    "  exc_stub_\\n:",
    "    .if \\has_ec",
    "      push \\n",
    "    .else",
    "      push 0",
    "      push \\n",
    "    .endif",
    "    jmp exception_common",
    ".endm",

    "exc_stub 0, 0",
    "exc_stub 1, 0",
    "exc_stub 2, 0",
    "exc_stub 3, 0",
    "exc_stub 4, 0",
    "exc_stub 5, 0",
    "exc_stub 6, 0",
    "exc_stub 7, 0",
    "exc_stub 8, 1",
    "exc_stub 9, 0",
    "exc_stub 10, 1",
    "exc_stub 11, 1",
    "exc_stub 12, 1",
    "exc_stub 13, 1",
    "exc_stub 14, 1",
    "exc_stub 15, 0",
    "exc_stub 16, 0",
    "exc_stub 17, 1",
    "exc_stub 18, 0",
    "exc_stub 19, 0",
    "exc_stub 20, 0",
    "exc_stub 21, 0",
    "exc_stub 22, 0",
    "exc_stub 23, 0",
    "exc_stub 24, 0",
    "exc_stub 25, 0",
    "exc_stub 26, 0",
    "exc_stub 27, 0",
    "exc_stub 28, 0",
    "exc_stub 29, 0",
    "exc_stub 30, 0",
    "exc_stub 31, 0",

    ".balign 16",
    "exception_common:",
    "  push rax",
    "  push rcx",
    "  push rdx",
    "  push rbx",
    "  push rbp",
    "  push rsi",
    "  push rdi",
    "  push r8",
    "  push r9",
    "  push r10",
    "  push r11",
    "  push r12",
    "  push r13",
    "  push r14",
    "  push r15",
    "  mov rcx, [rsp + 15*8]",  // vector
    "  mov rdx, [rsp + 16*8]",  // error_code (CPU pushes first = lowest addr)
    "  mov r8,  [rsp + 17*8]",  // saved RIP
    "  mov r9,  [rsp + 18*8]",  // saved CS
    "  sub rsp, 40",            // shadow space (32) + 5th arg slot (8)
    "  mov rax, [rsp + 40 + 19*8]", // saved RFLAGS (top of CPU frame)
    "  mov [rsp + 32], rax",    // 5th arg at the top of shadow space
    "  call default_handler_rust",
    "  add rsp, 40",
    "  add rsp, 16",
    "  pop r15",
    "  pop r14",
    "  pop r13",
    "  pop r12",
    "  pop r11",
    "  pop r10",
    "  pop r9",
    "  pop r8",
    "  pop rdi",
    "  pop rsi",
    "  pop rbp",
    "  pop rbx",
    "  pop rdx",
    "  pop rcx",
    "  pop rax",
    "  iretq",
);

extern "C" {
    fn exc_stub_0();
    fn exc_stub_1();
    fn exc_stub_2();
    fn exc_stub_3();
    fn exc_stub_4();
    fn exc_stub_5();
    fn exc_stub_6();
    fn exc_stub_7();
    fn exc_stub_8();
    fn exc_stub_9();
    fn exc_stub_10();
    fn exc_stub_11();
    fn exc_stub_12();
    fn exc_stub_13();
    fn exc_stub_14();
    fn exc_stub_15();
    fn exc_stub_16();
    fn exc_stub_17();
    fn exc_stub_18();
    fn exc_stub_19();
    fn exc_stub_20();
    fn exc_stub_21();
    fn exc_stub_22();
    fn exc_stub_23();
    fn exc_stub_24();
    fn exc_stub_25();
    fn exc_stub_26();
    fn exc_stub_27();
    fn exc_stub_28();
    fn exc_stub_29();
    fn exc_stub_30();
    fn exc_stub_31();
}

fn hex64(v: u64) {
    let hex = b"0123456789ABCDEF";
    for i in (0..16).rev() { crate::driver::uart::putchar(hex[((v >> (i*4)) & 0xF) as usize]); }
}

fn hex32(v: u32) {
    let hex = b"0123456789ABCDEF";
    crate::driver::uart::putchar(hex[((v >> 28) & 0xF) as usize]);
    crate::driver::uart::putchar(hex[((v >> 24) & 0xF) as usize]);
    crate::driver::uart::putchar(hex[((v >> 20) & 0xF) as usize]);
    crate::driver::uart::putchar(hex[((v >> 16) & 0xF) as usize]);
    crate::driver::uart::putchar(hex[((v >> 12) & 0xF) as usize]);
    crate::driver::uart::putchar(hex[((v >> 8) & 0xF) as usize]);
    crate::driver::uart::putchar(hex[((v >> 4) & 0xF) as usize]);
    crate::driver::uart::putchar(hex[(v & 0xF) as usize]);
}

#[no_mangle]
fn default_handler_rust(vector: u64, error_code: u64, saved_rip: u64, saved_cs: u64, saved_rflags: u64) {
    let names = [
        "DE ", "DB ", "NMI", "BP ", "OF ", "BR ", "UD ", "NM ",
        "DF ", "CSO", "TS ", "NP ", "SS ", "GP ", "PF ", "RSV",
        "MF ", "AC ", "MC ", "XM ", "VE ", "CP ", "?  ", "?  ",
        "?  ", "?  ", "?  ", "?  ", "?  ", "?  ", "?  ", "?  ",
    ];
    let idx = if vector < 32 { vector as usize } else { 31 };
    let ss_val: u16;
    unsafe {
        asm!("mov {0:x}, ss", out(reg) ss_val);
    }
    let cr2: u64;
    unsafe { asm!("mov {}, cr2", out(reg) cr2); }
    let cr3: u64;
    unsafe { asm!("mov {}, cr3", out(reg) cr3); }
    let cur_rsp: u64;
    unsafe { asm!("mov {}, rsp", out(reg) cur_rsp); }
    crate::driver::uart::write_str("\r\n!!! ");
    crate::driver::uart::write_str(names[idx]);
    crate::driver::uart::write_str(" vec=");
    hex32(vector as u32);
    crate::driver::uart::write_str(" ec=");
    hex32(error_code as u32);
    crate::driver::uart::write_str(" CR2=");
    hex64(cr2);
    crate::driver::uart::write_str(" CR3=");
    hex64(cr3);
    crate::driver::uart::write_str(" RSP=");
    hex64(cur_rsp);
    crate::driver::uart::write_str(" RIP=");
    hex64(saved_rip);
    crate::driver::uart::write_str(" CS=");
    hex64(saved_cs);
    crate::driver::uart::write_str(" SS=");
    hex32(ss_val as u32);
    crate::driver::uart::write_str(" RFLAGS=");
    hex64(saved_rflags);
    crate::driver::uart::write_str(" HALTED\r\n");
    loop { core::hint::spin_loop(); }
}

const PIC1: u16 = 0x20;
const PIC2: u16 = 0xA0;
const PIC1_DATA: u16 = 0x21;
const PIC2_DATA: u16 = 0xA1;

fn pic_remap() {
    unsafe {
        let m1: u8; asm!("in al, dx", out("al") m1, in("dx") PIC1_DATA);
        let m2: u8; asm!("in al, dx", out("al") m2, in("dx") PIC2_DATA);
        asm!("out dx, al", in("dx") PIC1, in("al") 0x11u8);
        asm!("out dx, al", in("dx") PIC2, in("al") 0x11u8);
        asm!("out dx, al", in("dx") PIC1_DATA, in("al") 0x20u8);
        asm!("out dx, al", in("dx") PIC2_DATA, in("al") 0x28u8);
        asm!("out dx, al", in("dx") PIC1_DATA, in("al") 4u8);
        asm!("out dx, al", in("dx") PIC2_DATA, in("al") 2u8);
        asm!("out dx, al", in("dx") PIC1_DATA, in("al") 0x01u8);
        asm!("out dx, al", in("dx") PIC2_DATA, in("al") 0x01u8);
        asm!("out dx, al", in("dx") PIC1_DATA, in("al") m1);
        asm!("out dx, al", in("dx") PIC2_DATA, in("al") m2);
    }
}

/// Saved kernel GDT base — restored when exiting a ring-3 task.
pub static mut KERNEL_GDT_BASE: u64 = 0;
pub static mut KERNEL_GDT_LIMIT: u16 = 0;

pub unsafe fn init() {
    // Build GDT: null, kernel code(0x08), kernel data(0x10),
    // user data(0x18), user code(0x20), user data dup for SYSRET SS(0x28)
    static GDT_RAW: [u64; 6] = [
        0,
        0x00209A0000000000, // kernel code, ring 0, 64-bit
        0x0000920000000000, // kernel data, ring 0, writable
        0x0000F20000000000, // user  data, ring 3, writable
        0x0020FA0000000000, // user  code, ring 3, 64-bit
        0x0000F20000000000, // user  data, ring 3 (SYSRET SS = CS+8)
    ];
    let gdt_limit = (core::mem::size_of_val(&GDT_RAW) - 1) as u16;
    KERNEL_GDT_BASE = &raw const GDT_RAW as u64;
    KERNEL_GDT_LIMIT = gdt_limit;
    asm!("lgdt [{ptr}]", ptr = in(reg) &GdtPacked { limit: gdt_limit, base: &raw const GDT_RAW as u64 } as *const _ as u64);

    // Far jump to reload CS
    asm!(
        "push {sel}",
        "lea rax, [rip + 2f]",
        "push rax",
        "retfq",
        "2:",
        sel = in(reg) 0x08u64,
    );
    asm!("mov ds, ax", "mov es, ax", "mov ss, ax", in("ax") 0x10u16);

    // Set IDT entries for exceptions 0..31 using per-vector stubs
    let stubs: [unsafe extern "C" fn(); 32] = [
        exc_stub_0,  exc_stub_1,  exc_stub_2,  exc_stub_3,
        exc_stub_4,  exc_stub_5,  exc_stub_6,  exc_stub_7,
        exc_stub_8,  exc_stub_9,  exc_stub_10, exc_stub_11,
        exc_stub_12, exc_stub_13, exc_stub_14, exc_stub_15,
        exc_stub_16, exc_stub_17, exc_stub_18, exc_stub_19,
        exc_stub_20, exc_stub_21, exc_stub_22, exc_stub_23,
        exc_stub_24, exc_stub_25, exc_stub_26, exc_stub_27,
        exc_stub_28, exc_stub_29, exc_stub_30, exc_stub_31,
    ];
    for i in 0..32 {
        let addr = stubs[i] as *const () as u64;
        IDT[i].set_handler(addr, 0x08);
    }

    let idt_base = &raw const IDT as *const _ as u64;
    let idt_limit = (core::mem::size_of::<IdtEntry>() * 256 - 1) as u16;
    asm!("lidt [{ptr}]", ptr = in(reg) &GdtPacked { limit: idt_limit, base: idt_base } as *const _ as u64);

    pic_remap();
}

#[repr(C, packed)]
pub struct GdtPacked {
    pub limit: u16,
    pub base: u64,
}
