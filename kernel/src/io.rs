/// Низкоуровневый доступ к портам I/O (x86 in/out)

pub unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value);
}

pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", out("al") value, in("dx") port);
    value
}

pub unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") value);
}

pub unsafe fn outl(port: u16, value: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") value);
}

pub unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    core::arch::asm!("in eax, dx", out("eax") value, in("dx") port);
    value
}

pub unsafe fn mmio_read32(addr: *const u32) -> u32 {
    let value: u32;
    core::arch::asm!("mov eax, [rcx]", out("eax") value, in("rcx") addr);
    value
}

pub unsafe fn mmio_write32(addr: *mut u32, value: u32) {
    core::arch::asm!("mov [rcx], eax", in("rcx") addr, in("eax") value);
}
