#![no_std]
#![no_main]

use core::panic::PanicInfo;
use uefi::prelude::*;

#[entry]
fn efi_main() -> Status {
    // Earliest probe: if OVMF loads this EFI at all, we get 'K' on COM1.
    // Bounded raw write (no waiting on LSR) so an uninitialized port cannot hang.
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b'K'); }
    dbsos_kernel::init();
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    dbsos_kernel::driver::uart::write_str("\r\n[PANIC]\r\n");
    loop {}
}
