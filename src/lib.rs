#![no_std]

pub mod cap;
pub mod display;
pub mod driver;
pub mod fs;
pub mod interrupts;
pub mod io;
pub mod ipc;
pub mod memory;
pub mod scheduler;
pub mod shell;
pub mod timer;
pub mod vm;
pub mod font;
pub mod console;
pub mod syscall;
pub mod acpi;
pub mod elf;

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

    crate::driver::ahci::init();
    crate::driver::nvme::init();

    uart_print("[GOP] init...\r\n");
    display::init();
    display::draw_str(10, 40, "DBSos v0.1", 0xFF, 0xFF, 0x00);

    // Save RSDP address from UEFI config table before ExitBootServices
    // Prefer ACPI2_GUID (v2, has XSDT) over ACPI_GUID (v1)
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
    unsafe { acpi::copy_tables(); }

    uart_print("[CPU] ExitBootServices...\r\n");
    unsafe { let _ = uefi::boot::exit_boot_services(None); }
    uart_print("[CPU] Bare-metal mode\r\n");

    uart_print("[CPU] GDT/IDT/PIC...\r\n");
    unsafe { interrupts::init(); }
    uart_print("[CPU] Interrupts ready\r\n");

    unsafe { vm::init(); }

    acpi::init();

    // Test VM: create new address space, clone kernel, map a page, switch back
    unsafe {
        let new_pml4 = vm::create_address_space();
        if !new_pml4.is_null() {
            vm::clone_kernel_mappings(vm::current_pml4() as *mut u64, new_pml4);
            let test_phys = memory::palloc();
            if test_phys != 0 {
                let test_virt = 0x1000000u64; // 16 MB — safe unused area
                vm::map_page(new_pml4, test_phys, test_virt, vm::PTE_WRITABLE);
                let lookup = vm::virt_to_phys(new_pml4, test_virt);
                uart_print("[VM] map test: phys=");
                uart_hex(test_phys);
                uart_print(" lookup=");
                uart_hex(lookup);
                if lookup == test_phys {
                    uart_print(" OK\r\n");
                } else {
                    uart_print(" MISMATCH\r\n");
                }
            }
        }
    }

    unsafe { syscall::init(); }

    // Test NIC TX after EBS
    driver::net::tx_test();

    // Auto-test FAT ls
    uart_print("[BOOT] FAT ls:\r\n");
    crate::fs::ls();

    // Auto-test FAT cat
    uart_print("[BOOT] FAT cat NVVARS:\r\n");
    crate::fs::cat(b"NVVARS");
    uart_print("\r\n--- end ---\r\n");

    // Test FS operations
    uart_print("[BOOT] FS test: mkdir /test\r\n");
    crate::fs::mkdir(b"/test");
    uart_print("[BOOT] FS test: write /test/hello.txt\r\n");
    crate::fs::write_file(b"/test/hello.txt", b"Hello from NVMe write!\r\n");
    crate::fs::rm(b"/test/hello.txt");
    crate::fs::rmdir(b"/test");

    // Software RX test (inject fake ARP into RX ring)
    driver::net::rx_software_test();

    // Auto-test network ping (send ICMP + poll for reply)
    uart_print("[BOOT] Ping gateway via ICMP...\r\n");
    let gw: [u8; 4] = [10, 0, 2, 1];
    driver::net::send_icmp_ping(gw);
    // Wait with delay to let QEMU process the TX packet
    let deadline = timer::millis() + 3000;
    while timer::millis() < deadline {
        driver::net::poll();
        timer::usleep(5000); // 5ms delay to yield to QEMU
    }
    // Debug: check RX ring state
    driver::net::dump_rx_state();
    uart_print("[BOOT] Ping done\r\n");

    // Scheduler + multitasking test
    scheduler::init();
    // IPC test needs scheduler ready
    ipc::tests::run_test();
    // Shared memory zero-copy test
    ipc::shmem_test();
    scheduler::test();

    // Enable preemptive multitasking (LAPIC timer)
    scheduler::lapic_timer_init();
    crate::driver::ps2::init(); // PS/2 keyboard (IRQ1)

    // Preemption test — 4 worker threads running under LAPIC timer
    scheduler::preempt_test();

    // Ring-3 user-mode test
    unsafe { syscall::test_ring3(); }

    // ELF loading from subdirectory test
    let result = crate::elf::load_and_spawn(b"/test/hello.elf");
    if result == 0 { uart_print("[BOOT] spawn FAIL\r\n"); } else { uart_print("[BOOT] spawn OK\r\n"); }

    shell::run();
}
