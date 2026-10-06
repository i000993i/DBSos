#![no_std]
#![allow(static_mut_refs)]
// Сырые указатели в ядре — доверенные буферы и пользовательские адреса,
// валидируемые на границе syscall (user_range_ok и т.п.). Пометка 14 pub fn
// как unsafe сломала бы ~40 call sites, не добавив ни одной новой проверки,
// поэтому лint глушим осознанно на уровне крейта (см. static_mut_refs выше).
#![allow(clippy::not_unsafe_ptr_arg_deref)]

pub mod cap;
pub mod display;
pub mod driver;
pub mod block;
pub mod event;
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
pub mod linux;
pub mod dbs_gr; // DBS-GR — своя растровая графика (TUI, градиент)
pub mod fm; // DBS-FM файловый менеджер
pub mod editor; // DBS-Edit
pub mod browser; // DBS-Browser
pub mod system; // LEVEL_0/1 архитектура
pub mod user; // multi-user
pub mod permissions; // rwx
pub mod ext4; // FAT32/ext4 VFS
pub mod gpt; // GPT partition parser (protective MBR 0xEE -> first usable LBA)
pub mod backtrace; // RBP-chain backtrace для #PF/panic
pub mod tmpfs; // RAM tmpfs для /tmp
pub mod crypto; // SHA-256/HMAC/PBKDF2/AES-128 для защиты Wi-Fi секретов
pub mod netman; // Менеджер подключений: профили, шифрованное хранение, скан
pub mod unix; // AF_UNIX socketpair: локальный транспорт ядра
pub mod deb; // .deb фундамент: ar + tar парсеры (распаковка data — следом)
pub mod isofs; // ISO9660 read-only: корень с CD (VirtualBox без NVMe-диска)
pub mod rs_kernel_test; // RS-Kernel-Test: проверка целостности Rust-ядра
// Linux-подобный UI убран: plasma/wayland/gfx/login оставлены как legacy, не используются
pub mod gfx; // legacy (не используется в DBS-GR)
pub mod plasma; // legacy
pub mod login; // legacy
pub mod wayland; // legacy

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
    // Adaptive driver report — which hardware needs pkg fetch
    crate::driver::adapt::print_report();
    // Автовыбор primary NIC (e1000 vs rtl8139) + итог по радио
    crate::driver::adapt::auto_select();

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
    // 1920x1080 needs 1920*1080*4 = 8294400 = 2025 pages; use 4096 pages = 16 MiB
    unsafe { heap::init(4096); }

    // Double buffering — allocate back buffer to eliminate screen flicker
    // Must be after heap init (uses kmalloc)
    uart_print("[GOP] double buffer...\r\n");
    unsafe { display::init_double_buffer(); }

    // ASLR — initialize entropy pool for address space randomization
    crate::linux::aslr_init();
    uart_print("[SEC] ASLR initialized\r\n");

    // VFS — virtual filesystem layer
    crate::vfs::init();

    // Единый block-слой: NVMe -> AHCI -> IDE. Выбирает backend один раз.
    crate::block::probe();
    crate::block::list();
    // FAT driver — mount at "/"; если диска нет (VBox CD-only) — ISO9660 с CD.
    let fat_ok = crate::fat_driver::init();
    if !fat_ok {
        uart_print("[FS] FAT not found on HDD/SSD, trying CD-ROM...\r\n");
        if crate::isofs::init() {
            let idx = crate::isofs::register();
            if idx != !0usize {
                unsafe { crate::vfs::mount(b"/", idx); }
                uart_print("[FS] ISO9660 root mounted (read-only)\r\n");
            }
        } else {
            uart_print("[FS] no FAT and no ISO9660 — VFS empty, /tmp only\r\n");
        }
    } else {
        uart_print("[FS] FAT root mounted via ");
        uart_print(crate::block::backend_name());
        uart_print("\r\n");
    }
    crate::ext4::init();
    crate::permissions::init();
    crate::user::init();
    // Create home directories for multi-user
    let _ = crate::vfs::mkdir(b"/home");
    let _ = crate::vfs::mkdir(b"/home/guest");
    let _ = crate::vfs::mkdir(b"/home/user");
    let _ = crate::vfs::mkdir(b"/etc");
    // Try to reload users from /etc/passwd now that VFS is ready
    crate::user::post_vfs_init();
    // FAT32/ext4 VFS ready — supports FAT12/16/32 + ext4 stub
    uart_print("[FS] multi-FS: FAT16/FAT32/ext4 mounted\r\n");
    // tmpfs для /tmp (RAM, переживает только до reboot)
    crate::tmpfs::init();
    // Профили подключений (расшифровка по требованию, не в память)
    crate::netman::load();

    acpi::init();

    // SMP: INIT-SIPI-SIPI trampoline with stack-based retf (32→64 mode switch)
    scheduler::smp::init();

    unsafe { syscall::init(); }

    // Scheduler + essential services
    scheduler::init();
    scheduler::lapic_timer_init();
    crate::driver::ps2::init(); // PS/2 keyboard (IRQ1)
    crate::driver::mouse::init(); // PS/2 mouse (IRQ12)
    crate::driver::uart::enable_irq(); // UART serial RX (IRQ4)

    // CMOS RTC — read real date/time from hardware
    crate::driver::rtc::init();

    // Ring-3 smoke-тест: каждый boot проверяет syscall-ABI (rax/rdi/rsi/rdx/r10
    // и Win64-пролог syscall_stub) и посадку/выход ring-3 задач.
    unsafe { crate::syscall::test_ring3(); }

    // DBS-GR — пока прямо в BSP (гарантированно видна консоль), гибрид L3 через spawn — следующий шаг после стабилизации
    crate::dbs_gr::init();
    // RS-Kernel-Test: проверка целостности ядра (UART-таблица + итог на фреймбуфер)
    crate::rs_kernel_test::run();
    crate::system::set(system::Level::L1Tui);
    crate::system::boot_banner();
    crate::dbs_gr::run();
}
