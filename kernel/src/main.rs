#![no_std]
// При cargo test (host) харнесс генерирует свой main.
#![cfg_attr(not(test), no_main)]
#![allow(unused_imports, dead_code)]

#[cfg(not(test))]
use core::panic::PanicInfo;
#[cfg(not(test))]
use core::fmt::Write;
use uefi::prelude::*;

#[entry]
fn efi_main() -> Status {
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'K'); }
    dbsos_kernel::init();
    loop {}
}

#[cfg(not(test))]
#[allow(dead_code)]
struct UartWriter;

#[cfg(not(test))]
impl core::fmt::Write for UartWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        dbsos_kernel::driver::uart::write_str(s);
        Ok(())
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    use dbsos_kernel::driver::uart;
    uart::write_str("\r\n=== PANIC ===\r\n");
    if let Some(loc) = info.location() {
        uart::write_str("  at ");
        uart::write_str(loc.file());
        uart::write_str(":");
        let mut buf = [0u8; 20];
        let mut n = loc.line() as u64;
        let mut i = 0;
        if n == 0 { buf[0] = b'0'; i = 1; }
        while n > 0 { buf[i] = b'0' + (n % 10) as u8; n /= 10; i += 1; }
        while i > 0 { i -= 1; uart::putchar(buf[i]); }
        uart::write_str("\r\n");
    }
    // Print message via fmt::Display
    uart::write_str("  msg: ");
    let _ = write!(&mut UartWriter, "{}", info.message());
    uart::write_str("\r\n");
    unsafe { dbsos_kernel::backtrace::dump_current(12); }
    uart::write_str("=== END ===\r\n");
    loop {}
}
