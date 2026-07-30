/// DriverManager — регистрация и инициализация драйверов

use super::traits::*;
use super::uart;

static mut DRIVERS: &[DriverEntry] = &[];

pub fn register(drivers: &'static [DriverEntry]) {
    unsafe { DRIVERS = drivers; }
}

pub fn init_all() {
    let drivers = unsafe { DRIVERS };
    uart::write_str("\r\n=== DBSos Drivers ===\r\n");

    for (i, driver) in drivers.iter().enumerate() {
        let status = driver.init();
        let tag = match status {
            DriverStatus::Ok => "OK",
            DriverStatus::Unsupported => "SKIP",
            DriverStatus::Error(e) => e,
        };
        uart::write_str("  [");
        uart::putchar(b'0' + (i / 10) as u8);
        uart::putchar(b'0' + (i % 10) as u8);
        uart::write_str("] ");
        uart::write_str(driver.name());
        uart::write_str(" ");
        uart::write_str(tag);
        uart::write_str("\r\n");
    }
    uart::write_str("=== End Drivers ===\r\n");
}
