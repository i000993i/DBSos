pub mod traits;
pub mod manager;
pub mod uart;
pub mod pci;
pub mod net;
pub mod tcp;
pub mod udp;
pub mod dhcp;
pub mod dns;
pub mod ahci;
pub mod nvme;
pub mod ps2;
pub mod mouse;

use traits::*;

static DRIVERS: &[DriverEntry] = &[
    &uart::UartDriver,
    &pci::PciBusDriver,
    &net::E1000Driver,
    &nvme::NvmeDriver,
    // ahci disabled: hangs in port_init (SATA controller unresponsive on QEMU)
    // ps2 removed: initialized later after IDT/PIC is ready (lib.rs:262)
];

pub fn init() {
    // E1000 driver нужно создать с mmio_base
    // Пока что используем detect в самой структуре
    // TODO: передавать mmio_base через регистрацию драйвера
    manager::register(DRIVERS);
    manager::init_all();
}
