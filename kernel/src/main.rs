#![no_std]
#![no_main]

use core::panic::PanicInfo;
use uefi::prelude::*;

#[entry]
fn efi_main() -> Status {
    dbsos_kernel::init();
    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
