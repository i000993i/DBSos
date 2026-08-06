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
    "  add rsp, 16", // skip vec + error_code; iretq consumes RIP/CS/RFLAGS[/SS/RSP]
    "  iretq",
);

// CPUID helper: reads CPUID leaf in EAX, returns EBX value through pointer in RDI
// Must be global_asm because LLVM reserves RBX and inline asm can't touch it.
core::arch::global_asm!(
    ".globl cpuid_get_ebx",
    "cpuid_get_ebx:",
    "  push rbx",
    "  mov eax, edi",
    "  xor ecx, ecx",
    "  cpuid",
    "  mov [rsi], rbx",
    "  pop rbx",
    "  ret",
);

extern "C" {
    fn cpuid_get_ebx(leaf: u32, out: *mut u64);
}

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
    // Page fault: vector 14 — handle specially
    if vector == 14 {
        let cr2: u64;
        unsafe { asm!("mov {}, cr2", out(reg) cr2); }
        let cr3: u64;
        unsafe { asm!("mov {}, cr3", out(reg) cr3); }
        // Page fault error code:
        //   bit0 P  = 0 -> page not present
        //   bit1 RW = 1 -> write
        //   bit2 US = 1 -> access came from CPL 3 (user mode)
        //             = 0 -> access came from CPL < 3 (supervisor/kernel)
        // CS here is the *selector*, not the faulting CPL.  The USER bit is the
        // authoritative signal of which privilege level performed the access.
        let user = error_code & 4 != 0;
        let present = error_code & 1 != 0;

        // Demand paging: a not-present USER fault inside a registered VMA is
        // serviced by allocating a zeroed page and mapping it; returning from
        // the handler makes the exception stub iretq retry the faulting
        // instruction.  Successfully serviced faults return silently — the
        // verbose header + page-table walk below only run for genuinely
        // unhandled faults.
        if user && !present {
            unsafe {
            let cur = crate::scheduler::CURRENT;
            if let Some(vma) = crate::scheduler::vma::find(cur, cr2) {
                if vma.kind == crate::scheduler::vma::VMA_STACK
                    || vma.kind == crate::scheduler::vma::VMA_HEAP
                    || vma.kind == crate::scheduler::vma::VMA_DATA
                {
                    let page = crate::memory::palloc();
                    if page != 0 {
                        core::ptr::write_bytes(page as *mut u8, 0, 4096);
                        let vaddr = cr2 & !0xFFFu64;
                        let r = crate::vm::map_page(
                            crate::scheduler::TASKS[cur].pml4,
                            page, vaddr,
                            vma.flags,
                        );
                        if r == 0 {
                            return;
                        }
                    }
                    crate::driver::uart::write_str("\r\n  -> demand-map FAILED (OOM) at 0x");
                    hex64(cr2 & !0xFFFu64);
                    crate::driver::uart::write_str("\r\n");
                }
            }
            }
        }

        crate::driver::uart::write_str("\r\n!!! #PF vec=14 ec=");
        hex32(error_code as u32);
        crate::driver::uart::write_str(" CR2=");
        hex64(cr2);
        crate::driver::uart::write_str(" CR3=");
        hex64(cr3);
        crate::driver::uart::write_str(" RIP=");
        hex64(saved_rip);
        crate::driver::uart::write_str(" CS=");
        hex64(saved_cs);
        crate::driver::uart::write_str(" [");
        if !present { crate::driver::uart::write_str("PRESENT "); }
        if error_code & 2 != 0 { crate::driver::uart::write_str("WRITE "); }
        if user { crate::driver::uart::write_str("USER "); } else { crate::driver::uart::write_str("kernel-access "); }
        if error_code & 8 != 0 { crate::driver::uart::write_str("RSVD "); }
        if error_code & 16 != 0 { crate::driver::uart::write_str("FETCH "); }
        crate::driver::uart::write_str("]\r\n");

        // Diagnose: which task / address space is active, and walk the page
        // table rooted at CR3 for the faulting address so we can see where the
        // mapping is missing.
        {
            let hex = b"0123456789ABCDEF";
            unsafe {
            let cur = crate::scheduler::CURRENT;
            crate::driver::uart::write_str("  task[");
            crate::driver::uart::putchar(hex[((cur >> 4) & 0xF) as usize]);
            crate::driver::uart::putchar(hex[(cur & 0xF) as usize]);
            crate::driver::uart::write_str("].id=");
            hex64(crate::scheduler::TASKS[cur].id);
            crate::driver::uart::write_str(" ring3=");
            if crate::scheduler::TASKS[cur].ring3 { crate::driver::uart::write_str("Y"); } else { crate::driver::uart::write_str("N"); }
            crate::driver::uart::write_str(" task.pml4=");
            hex64(crate::scheduler::TASKS[cur].pml4 as u64);
            crate::driver::uart::write_str("\r\n");

            // Walk the page table rooted at the faulting cr3 (read via identity map).
            let p4 = cr3 as *const u64;
            let e0 = *p4.add((cr2 >> 39) as usize & 0x1FF);
            crate::driver::uart::write_str("  walk[PML4E=");
            hex64(e0);
            if e0 & 1 != 0 {
                let p3 = (e0 & 0xFFFF_FFFF_F000) as *const u64;
                let e1 = *p3.add((cr2 >> 30) as usize & 0x1FF);
                crate::driver::uart::write_str("][PDPTE=");
                hex64(e1);
                if e1 & 1 != 0 {
                    if e1 & (1 << 7) != 0 {
                        crate::driver::uart::write_str("][1GB HUGE]\r\n");
                    } else {
                        let p2 = (e1 & 0xFFFF_FFFF_F000) as *const u64;
                        // DEBUG: dump PD page contents (PDPTE points at the PD page)
                        {
                            let ph = b"0123456789ABCDEF";
                            crate::driver::uart::write_str("\r\n  PDphys=0x");
                            let v = p2 as u64;
                            for i in (0..16).rev() { crate::driver::uart::putchar(ph[((v >> (i*4)) & 0xF) as usize]); }
                            for k in 0..4 {
                                let vv = *p2.add(k);
                                crate::driver::uart::write_str(" PD[");
                                crate::driver::uart::putchar(ph[((k >> 8) & 0xF) as usize]);
                                crate::driver::uart::putchar(ph[((k >> 4) & 0xF) as usize]);
                                crate::driver::uart::putchar(ph[(k & 0xF) as usize]);
                                crate::driver::uart::write_str("]=");
                                for i in (0..16).rev() { crate::driver::uart::putchar(ph[((vv >> (i*4)) & 0xF) as usize]); }
                            }
                            crate::driver::uart::write_str("\r\n");
                        }
                        let e2 = *p2.add((cr2 >> 21) as usize & 0x1FF);
                        crate::driver::uart::write_str("][PDE=");
                        hex64(e2);
                        if e2 & 1 != 0 {
                            if e2 & (1 << 7) != 0 {
                                crate::driver::uart::write_str("][2MB HUGE]\r\n");
                            } else {
                                // DEBUG: dump the first few PD entries at the break point
                                let hd2 = (e2 & 0xFFFF_FFFF_F000) as *const u64;
                                let ph = b"0123456789ABCDEF";
                                crate::driver::uart::write_str("\r\n  PDphys=0x");
                                { let v = hd2 as u64; for i in (0..16).rev(){ crate::driver::uart::putchar(ph[((v>>(i*4))&0xF) as usize]); } }
                                for k in 0..4 {
                                    let v = *hd2.add(k);
                                    crate::driver::uart::write_str(" PD[");
                                    crate::driver::uart::putchar(ph[((k>>8)&0xF) as usize]);
                                    crate::driver::uart::putchar(ph[((k>>4)&0xF) as usize]);
                                    crate::driver::uart::putchar(ph[(k&0xF) as usize]);
                                    crate::driver::uart::write_str("]=");
                                    for i in (0..16).rev(){ crate::driver::uart::putchar(ph[((v>>(i*4))&0xF) as usize]); }
                                }
                                crate::driver::uart::write_str("\r\n");
                                let p1 = (e2 & 0xFFFF_FFFF_F000) as *const u64;
                                let e3 = *p1.add((cr2 >> 12) as usize & 0x1FF);
                                crate::driver::uart::write_str("][PTE=");
                                hex64(e3);
                                if e3 & 1 != 0 {
                                    crate::driver::uart::write_str("][phys=");
                                    hex64(e3 & 0xFFFF_FFFF_F000 | (cr2 & 0xFFF));
                                }
                                crate::driver::uart::write_str("]\r\n");
                            }
                        } else {
                            crate::driver::uart::write_str("][-]\r\n");
                        }
                    }
                } else {
                    crate::driver::uart::write_str("][-]\r\n");
                }
            } else {
                crate::driver::uart::write_str("][-]\r\n");
            }
            }
        }

        // A real user-mode fault (US bit set) means the user process itself
        // dereferenced a bad address: terminate that process only.
        // A supervisor-access fault (US bit clear) is a kernel bug (e.g. wrong
        // CR3 while reading user memory): do NOT kill a task for it — halt.
        if user {
            crate::driver::uart::write_str("  -> user-mode #PF, terminating process");
            unsafe {
                let hex = b"0123456789ABCDEF";
                let cur = crate::scheduler::CURRENT;
                let t = &crate::scheduler::TASKS[cur];
                crate::driver::uart::write_str(" (vma_count=");
                crate::driver::uart::putchar(hex[(t.vma_count & 0xF) as usize]);
                for i in 0..t.vma_count as usize {
                    crate::driver::uart::write_str(" v[");
                    crate::driver::uart::putchar(hex[(i & 0xF) as usize]);
                    crate::driver::uart::write_str("]=");
                    hex64(t.vmas[i].start);
                    crate::driver::uart::write_str("-");
                    hex64(t.vmas[i].end);
                    crate::driver::uart::write_str("k=");
                    crate::driver::uart::putchar(hex[(t.vmas[i].kind & 0xF) as usize]);
                }
                crate::driver::uart::write_str(")\r\n");
            }
            crate::scheduler::exit();
        } else {
            crate::driver::uart::write_str("  -> KERNEL #PF (supervisor access), HALTED\r\n");
            loop { core::hint::spin_loop(); }
        }
    }

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

    crate::driver::uart::write_str("[CPU] enabling WP...\r\n");
    // CR0.WP (bit 16): Write Protect — kernel cannot write user pages
    {
        let mut cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0);
        cr0 |= 1u64 << 16;
        asm!("mov cr0, {}", in(reg) cr0);
    }

    // Before enabling SMEP, clear PTE_USER on all kernel identity-mapped pages.
    // UEFI marks all pages as user-accessible; SMEP requires kernel pages to have US=0.
    // Temporarily disable WP so we can modify page table entries.
    crate::driver::uart::write_str("[CPU] clearing PTE_USER...\r\n");
    {
        let mut cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0);
        cr0 &= !(1u64 << 16); // disable WP for page table edits
        asm!("mov cr0, {}", in(reg) cr0);
    }
    let cr3: u64;
    asm!("mov {}, cr3", out(reg) cr3);
    let pml4 = cr3 as *mut u64;
    let mut cleared: u64 = 0;
    let mut made_writable: u64 = 0;
    const PTE_ADDR: u64 = 0x000FFFFFFFFFF000; // bits 12-51
    const PTE_RW: u64 = 1 << 1; // Read/Write bit

    for i in 0..512 {
        let e0 = *pml4.add(i);
        if e0 & 1 == 0 { continue; }
        // Ensure PML4->PDPT entries are writable (for future page table edits)
        if e0 & PTE_RW == 0 { *pml4.add(i) = e0 | PTE_RW; made_writable += 1; }
        let pdpt = (e0 & PTE_ADDR) as *mut u64;
        for j in 0..512 {
            let e1 = *pdpt.add(j);
            if e1 & 1 == 0 { continue; }
            if e1 & (1 << 7) != 0 {
                if e1 & (1 << 2) != 0 { *pdpt.add(j) = e1 & !(1u64 << 2); cleared += 1; }
                continue;
            }
            if e1 & PTE_RW == 0 { *pdpt.add(j) = e1 | PTE_RW; made_writable += 1; }
            let pd = (e1 & PTE_ADDR) as *mut u64;
            for k in 0..512 {
                let e2 = *pd.add(k);
                if e2 & 1 == 0 { continue; }
                if e2 & (1 << 7) != 0 {
                    if e2 & (1 << 2) != 0 { *pd.add(k) = e2 & !(1u64 << 2); cleared += 1; }
                    continue;
                }
                if e2 & PTE_RW == 0 { *pd.add(k) = e2 | PTE_RW; made_writable += 1; }
                let pt = (e2 & PTE_ADDR) as *mut u64;
                for l in 0..512 {
                    let e3 = *pt.add(l);
                    if e3 & 1 == 0 { continue; }
                    if e3 & (1 << 2) != 0 { *pt.add(l) = e3 & !(1u64 << 2); cleared += 1; }
                    if e3 & PTE_RW == 0 { *pt.add(l) = e3 | PTE_RW; made_writable += 1; }
                }
            }
        }
    }

    // Re-enable WP
    {
        let mut cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0);
        cr0 |= 1u64 << 16;
        asm!("mov cr0, {}", in(reg) cr0);
    }
    // Flush TLB
    asm!("mov cr3, {}", in(reg) cr3);
    fn print_u64(mut v: u64) {
        if v == 0 { crate::driver::uart::putchar(b'0'); return; }
        let mut buf = [0u8; 20];
        let mut n = 0;
        while v > 0 { buf[n] = b'0' + (v % 10) as u8; v /= 10; n += 1; }
        while n > 0 { n -= 1; crate::driver::uart::putchar(buf[n]); }
    }
    crate::driver::uart::write_str("[CPU] cleared PTE_USER=");
    print_u64(cleared);
    crate::driver::uart::write_str(" made_RW=");
    print_u64(made_writable);
    crate::driver::uart::write_str("\r\n");

    // Flush TLB
    asm!("mov cr3, {}", in(reg) cr3);

    crate::driver::uart::write_str("[CPU] enabling SMEP...\r\n");
    let hex = b"0123456789ABCDEF";

    // Check SMEP support via CPUID leaf 7, EBX bit 7
    let cpuid7_ebx: u32;
    unsafe {
        let mut store: u64 = 0;
        cpuid_get_ebx(7, &mut store);
        cpuid7_ebx = store as u32;
    }
    let smep_supported = (cpuid7_ebx >> 7) & 1 == 1;

    crate::driver::uart::write_str("[CPU] CPUID.7:EBX=");
    for i in (0..8).rev() {
        crate::driver::uart::putchar(hex[((cpuid7_ebx >> (i*4)) & 0xF) as usize]);
    }
    crate::driver::uart::write_str(" SMEP=");
    crate::driver::uart::putchar(if smep_supported { b'Y' } else { b'N' });
    crate::driver::uart::write_str("\r\n");
    if smep_supported {
        // CR4.SMEP (bit 20): Supervisor Mode Execution Prevention
        let mut cr4: u64;
        unsafe { asm!("mov {}, cr4", out(reg) cr4); }
        crate::driver::uart::write_str("[CPU] CR4 before=");
        crate::driver::uart::putchar(hex[((cr4 >> 60) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 56) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 52) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 48) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 44) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 40) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 36) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 32) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 28) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 24) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 20) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 16) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 12) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 8) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[((cr4 >> 4) & 0xF) as usize]);
        crate::driver::uart::putchar(hex[(cr4 & 0xF) as usize]);
        crate::driver::uart::write_str("\r\n");
        cr4 |= 1u64 << 20;
        unsafe { asm!("mov cr4, {}", in(reg) cr4); }
        crate::driver::uart::write_str("[CPU] SMEP enabled\r\n");
    } else {
        crate::driver::uart::write_str("[CPU] SMEP not supported, skipping\r\n");
    }

    crate::driver::uart::write_str("[CPU] Security: WP+SMEP done\r\n");
}

#[repr(C, packed)]
pub struct GdtPacked {
    pub limit: u16,
    pub base: u64,
}
