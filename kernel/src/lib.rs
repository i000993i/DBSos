#![no_std]
#![allow(static_mut_refs)]

pub mod cap;
pub mod display;
pub mod driver;
pub mod fat_driver;
pub mod fs;
pub mod fs_server;
pub mod heap;
pub mod interrupts;
pub mod io;
pub mod ipc;
pub mod memory;
pub mod scheduler;
pub mod shell;
pub mod timer;
pub mod vfs;
pub mod vm;
pub mod font;
pub mod console;
pub mod syscall;
pub mod acpi;
pub mod elf;
pub mod script;
pub mod wget;
pub mod pkg;
pub mod gui;
pub mod linux;
pub mod wayland;

fn uart_print(s: &str) { driver::uart::write_str(s); }

fn uart_hex(mut val: u64) {
    if val == 0 { driver::uart::putchar(b'0'); return; }
    let mut buf = [0u8; 16];
    let mut i = 0;
    while val > 0 {
        let nib = (val & 0xF) as u8;
        buf[i] = if nib < 10 { b'0' + nib } else { b'A' + nib - 10 };
        val >>= 4;
        i += 1;
    }
    while i > 0 { i -= 1; driver::uart::putchar(buf[i]); }
}

fn uart_dec(mut val: u64) {
    if val == 0 { driver::uart::putchar(b'0'); return; }
    let mut buf = [0u8; 20];
    let mut i = 0;
    while val > 0 { buf[i] = b'0' + (val % 10) as u8; val /= 10; i += 1; }
    while i > 0 { i -= 1; driver::uart::putchar(buf[i]); }
}

pub fn init() {
    uefi::helpers::init().unwrap();

    ipc::init();
    memory::init();
    timer::init();
    driver::init();

    let free = memory::free_count();
    uart_print("[MEM] free pages: ");
    uart_dec(free as u64);
    uart_print("\r\n");

    let p1 = memory::palloc();
    let p2 = memory::palloc();
    if p1 != 0 && p2 != 0 {
        uart_print("[MEM] palloc test: ");
        uart_dec(p1);
        uart_print(" ");
        uart_dec(p2);
        uart_print("\r\n");
        memory::pfree(p1);
        memory::pfree(p2);
    }

    // Тест таймера
    let t0 = timer::millis();
    let c0 = timer::ticks();
    timer::usleep(10_000);
    let c1 = timer::ticks();
    let t1 = timer::millis();
    let dt = t1 - t0;
    uart_print("[TIMER] ticks: ");
    uart_dec(c0);
    uart_print(" -> ");
    uart_dec(c1);
    uart_print(", delta ticks: ");
    uart_dec(c1 - c0);
    uart_print(" (expect 100k), delta ms: ");
    uart_dec(dt);
    uart_print("\r\n");

    uart_print("[GOP] init...\r\n");
    display::init();
    uart_print("[GOP] draw_str...\r\n");
    display::draw_str(10, 40, "DBSos v0.1", 0xFF, 0xFF, 0x00);
    uart_print("[GOP] done\r\n");

    // Save RSDP address from UEFI config table before ExitBootServices
    // Prefer ACPI2_GUID (v2, has XSDT) over ACPI_GUID (v1)
    uart_print("[ACPI] scanning config tables...\r\n");
    uefi::system::with_config_table(|entries| {
        let mut found = 0u64;
        for e in entries {
            if e.guid == uefi::table::cfg::ACPI2_GUID {
                found = e.address as u64;
                break;
            }
        }
        if found == 0 {
            for e in entries {
                if e.guid == uefi::table::cfg::ACPI_GUID {
                    found = e.address as u64;
                    break;
                }
            }
        }
        if found != 0 {
            unsafe { acpi::set_rsdp(found); }
            uart_print("[ACPI] RSDP saved at 0x");
            uart_hex(found);
            uart_print("\r\n");
        } else {
            uart_print("[ACPI] RSDP not in UEFI config\r\n");
        }
    });

    // Copy ACPI table data before ExitBootServices
    uart_print("[ACPI] copy_tables...\r\n");
    unsafe { acpi::copy_tables(); }
    uart_print("[ACPI] copy done\r\n");

    uart_print("[CPU] ExitBootServices...\r\n");
    unsafe { let _ = uefi::boot::exit_boot_services(None); }
    uart_print("[CPU] Bare-metal mode\r\n");

    uart_print("[CPU] GDT/IDT/PIC...\r\n");
    unsafe { interrupts::init(); }
    uart_print("[CPU] Interrupts ready\r\n");

    unsafe { vm::init(); }

    // Kernel heap — must come after VM init (needs page table mapping)
    // 256 initial pages = 1 MiB heap
    unsafe { heap::init(256); }

    // VFS — virtual filesystem layer
    crate::vfs::init();

    // FAT driver — mount at "/"
    crate::fat_driver::init();

    acpi::init();

    // SMP disabled: AP goes wild after SIPI (todo: fix trampoline)
    // scheduler::smp::init();

    unsafe { syscall::init(); }

    // Scheduler + essential services
    scheduler::init();
    scheduler::lapic_timer_init();
    crate::driver::ps2::init(); // PS/2 keyboard (IRQ1)
    crate::driver::mouse::init(); // PS/2 mouse (IRQ12)
    crate::driver::uart::enable_irq(); // UART serial RX (IRQ4)

    // Launch graphical desktop
    crate::gui::run();
}
