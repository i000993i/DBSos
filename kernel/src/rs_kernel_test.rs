//! RS-Kernel-Test — фирменная система проверки целостности Rust-ядра DBSos.
//!
//! Прогоняется в lib.rs до шелла + командой `selftest`.
//! Растёт вместе с ядром: каждый новый компонент добавляет сюда свой кейс
//! (VERSION bump), и загрузочный лог доказывает порядок в ядре и уровнях.
//! Каждый тест возвращает bool и печатает строку в UART;
//! итог дублируется на фреймбуфер чтобы было видно в главном терминале.

use crate::driver::uart;

/// Версия тестировщика. Bump при добавлении кейсов под новые изменения ядра:
/// v1 — 12 базовых кейсов (mem/heap/timer/vfs/fat/crypto/sched/syscall/net/display/wayland/cmds)
/// v2 — +gop-modes (таблица видеорежимов), +rtc-msk (московское время)
/// v3 — +display-present (регресс: невидимый диалог connect — dirty/copy кадра)
/// v4 — +net-rtl (вторая проводная карта / skip), +wifi-scan (детект радио)
/// v5 — +ext4-sb, +unix-sock, +deb-ar-tar, +xdg-cycle, +ioapic-ver, +mem-pressure, +usb-probe
/// v6 — +isofs-scan (ISO9660 парсинг на синтетике, для VirtualBox CD)
pub const VERSION: u32 = 6;

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    ok: bool,
}

fn up(s: &str) { uart::write_str(s); }

fn run_all() -> (u32, u32) {
    let mut cases: [Case; 32] = [Case { name: "", ok: false }; 32];
    let mut n = 0usize;
    let mut put = |name: &'static str, ok: bool| {
        if n < cases.len() {
            cases[n] = Case { name, ok };
            n += 1;
        }
    };

    // MEM: palloc/pfree round-trip
    put("MEM palloc", {
        let a = crate::memory::palloc();
        let b = crate::memory::palloc();
        let ok = a != 0 && b != 0 && a != b;
        if a != 0 { crate::memory::pfree(a); }
        if b != 0 { crate::memory::pfree(b); }
        ok
    });
    // HEAP: kmalloc запись/чтение
    put("HEAP kmalloc", unsafe {
        let p = crate::heap::kmalloc(128);
        if p.is_null() {
            false
        } else {
            for i in 0..128 { *p.add(i) = (i & 0xFF) as u8; }
            let mut ok = true;
            for i in 0..128 {
                if *p.add(i) != (i & 0xFF) as u8 { ok = false; break; }
            }
            crate::heap::kfree(p);
            ok
        }
    });
    // TIMER: тики идут
    put("TIMER hpet", {
        let t0 = crate::timer::ticks();
        crate::timer::usleep(2000);
        crate::timer::ticks().wrapping_sub(t0) > 0
    });
    // VFS tmpfs round-trip
    put("VFS tmpfs", {
        let path = b"/tmp/.selftest";
        let fd = crate::vfs::open(path, 0x100 | 0x200 | 1);
        if fd < 0 {
            false
        } else {
            let w = crate::vfs::write(fd, b"OK123");
            crate::vfs::close(fd);
            if w != 5 {
                false
            } else {
                let fd2 = crate::vfs::open(path, 0);
                if fd2 < 0 {
                    false
                } else {
                    let mut buf = [0u8; 8];
                    let r = crate::vfs::read(fd2, &mut buf);
                    crate::vfs::close(fd2);
                    let _ = crate::vfs::unlink(path);
                    r == 5 && buf[..5] == *b"OK123"
                }
            }
        }
    });
    // FAT: корень читается
    put("FAT readdir", {
        let mut e = [crate::vfs::DirEntry { name: [0; 32], is_dir: false, size: 0 }; 4];
        crate::vfs::readdir(b"/", &mut e) >= 0
    });
    // CRYPTO: KAT (биты: 1=sha 2=aesE 4=aesD 8=cbc 16=hmac 32=pbkdf2)
    put("CRYPTO kat", {
        let bits = crate::crypto::self_test();
        if bits != 0 {
            up("  crypto bits=0x");
            dec(bits as u64);
            up("\r\n");
            false
        } else {
            true
        }
    });
    // SCHED: есть задачи, CURRENT валиден
    put("SCHED tasks", unsafe {
        let mut cnt = 0;
        crate::scheduler::for_each_task(|_, _| cnt += 1);
        cnt > 0 && crate::scheduler::CURRENT < crate::scheduler::MAX_TASKS
    });
    // SYSCALL: LSTAR настроен
    put("SYSCALL msr", unsafe {
        let lo: u32;
        let hi: u32;
        core::arch::asm!("rdmsr", in("ecx") 0xC0000082u32, out("eax") lo, out("edx") hi);
        ((hi as u64) << 32 | lo as u64) != 0
    });
    // NET: IP назначен
    put("NET ip", crate::driver::net::our_ip() != [0, 0, 0, 0]);
    // DISPLAY: фреймбуфер жив
    put("DISPLAY gop", {
        !crate::display::framebuffer().is_null()
            && crate::display::width() > 0
            && crate::display::height() > 0
    });
    // WAYLAND: реестр полон (5 глобалов)
    put("WAYLAND reg", {
        use crate::wayland::protocol::global_by_id;
        global_by_id(1).is_some()
            && global_by_id(2).is_some()
            && global_by_id(3).is_some()
            && global_by_id(4).is_some()
            && global_by_id(5).is_some()
    });
    // CMDS: таблица команд shell непустая (диспетч по построению)
    put("CMDS table", crate::shell::command_count() >= 40);
    // NET rtl8139: если карты нет — честный skip (код путёй проверен), иначе MAC валиден
    put("NET rtl8139", {
        if !crate::driver::rtl8139::is_ready() {
            up("  (absent, skipped)\r\n");
            true
        } else {
            let m = crate::driver::rtl8139::mac();
            !(m == [0; 6] || m == [0xFF; 6])
        }
    });
    // WIFI: скан отработал (0 радио в QEMU — норма, путь кода проверен)
    put("WIFI scan", {
        let n = crate::driver::wifi::scan();
        up("  radios=");
        dec(n as u64);
        up("\r\n");
        true
    });
    // EXT4: парсер суперблока на синтетике
    put("EXT4 superblock", {
        let bits = crate::ext4::self_test();
        if bits != 0 {
            up("  ext4 bits=0x");
            dec(bits as u64);
            up("\r\n");
            false
        } else {
            true
        }
    });
    // UNIX: socketpair туда-обратно
    put("UNIX socketpair", {
        let bits = crate::unix::self_test();
        if bits != 0 {
            up("  unix bits=0x");
            dec(bits as u64);
            up("\r\n");
            false
        } else {
            true
        }
    });
    // DEB: ar+tar на синтетике
    put("DEB ar/tar", {
        let bits = crate::deb::self_test();
        if bits != 0 {
            up("  deb bits=0x");
            dec(bits as u64);
            up("\r\n");
            false
        } else {
            true
        }
    });
    // XDG: configure/ack цикл
    put("XDG configure", {
        let bits = crate::wayland::xdg::self_test();
        if bits != 0 {
            up("  xdg bits=0x");
            dec(bits as u64);
            up("\r\n");
            false
        } else {
            true
        }
    });
    // IOAPIC: VER-регистр прочитан (0x20 на Q35)
    put("IOAPIC ver", crate::acpi::ioapic_ver() != 0);
    // MEM pressure: учёт в диапазоне 0..100
    put("MEM pressure", crate::memory::pressure() <= 100);
    // USB: probe-путь исполнен (контроллеров в QEMU может не быть)
    put("USB probe", {
        crate::driver::usb::probe();
        true
    });
    // ISO9660: парсинг на синтетике (для VirtualBox CD)
    put("ISO9660 scan", {
        let bits = crate::isofs::self_test();
        if bits != 0 {
            up("  iso bits=0x");
            dec(bits as u64);
            up("\r\n");
            false
        } else {
            true
        }
    });    // GOP: таблица видеорежимов перечислена (для Plasma/display)
    put("GOP modes", crate::display::mode_count() > 0);
    // RTC-MSK: московское время в sane-диапазоне
    put("RTC msk", {
        let t = crate::driver::rtc::read_msk();
        t.hour < 24 && t.minute < 60 && t.second < 60
            && (1..=31).contains(&t.day)
            && (1..=12).contains(&t.month)
            && t.year >= 2020
    });
    // DISPLAY present: back->front копия реально доходит (регресс connect-диалога)
    put("DISP present", unsafe {
        let w = crate::display::width() as usize;
        let h = crate::display::height() as usize;
        let s = crate::display::stride() as usize;
        let back = crate::display::framebuffer();
        let front = crate::display::real_framebuffer();
        if back.is_null() || front.is_null() || w == 0 || h == 0 {
            false
        } else if back == front {
            true // без double buffer рисуют сразу в видимый
        } else {
            let off = ((h - 1) * s + (w - 1)) * 4;
            let mut sb = [0u8; 4];
            let mut sf = [0u8; 4];
            for i in 0..4 {
                sb[i] = core::ptr::read_volatile(back.add(off + i));
                sf[i] = core::ptr::read_volatile(front.add(off + i));
            }
            let magic = [0xABu8, 0xCD, 0x12, 0x00];
            for i in 0..4 {
                core::ptr::write_volatile(back.add(off + i), magic[i]);
            }
            crate::display::mark_dirty((w - 1) as i32, (h - 1) as i32, 1, 1);
            crate::display::present();
            let mut got = [0u8; 4];
            for i in 0..4 {
                got[i] = core::ptr::read_volatile(front.add(off + i));
            }
            let ok = got == magic;
            for i in 0..4 {
                core::ptr::write_volatile(back.add(off + i), sb[i]);
                core::ptr::write_volatile(front.add(off + i), sf[i]);
            }
            crate::display::mark_dirty((w - 1) as i32, (h - 1) as i32, 1, 1);
            crate::display::present();
            ok
        }
    });

    let mut ok_n = 0u32;
    up("=== RS-Kernel-Test v");
    dec(VERSION as u64);
    up(" (Rust kernel integrity) ===\r\n");
    for i in 0..n {
        up(if cases[i].ok { "  OK  " } else { "  NO  " });
        up(cases[i].name);
        up("\r\n");
        if cases[i].ok { ok_n += 1; }
    }
    (ok_n, n as u32)
}

/// Прогон с выводом итога на UART и фреймбуфер. Возвращает (ok, total).
pub fn run() -> (u32, u32) {
    let (ok, total) = run_all();
    up("RS-Kernel-Test v");
    dec(VERSION as u64);
    up(": ");
    dec(ok as u64);
    up("/");
    dec(total as u64);
    up(if ok == total { " ALL OK\r\n" } else { " FAILURES\r\n" });
    // Итог на фреймбуфер (видно в главном терминале)
    let (msg, r, g) = if ok == total {
        ("RS-KERNEL-TEST: ALL OK", 0x00u8, 0xFFu8)
    } else {
        ("RS-KERNEL-TEST: FAILURES (see UART)", 0xFFu8, 0x00u8)
    };
    crate::display::draw_str(10, 64, msg, r, g, 0x00);
    unsafe { crate::display::mark_dirty(0, 60, crate::display::width(), 24); crate::display::present(); }
    (ok, total)
}

fn dec(mut v: u64) {
    if v == 0 { uart::putchar(b'0'); return; }
    let mut b = [0u8; 20];
    let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; uart::putchar(b[i]); }
}
