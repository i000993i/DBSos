//! Backtrace через RBP-цепочку — было в TODO как "Backtrace не реализован".
//!
//! Безопасный: только volatile-чтения, валидация указателей,
//! кап 16 фреймов, не аллоцирует, не паникует.

fn hex(v: u64) {
    if v == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut b = [0u8; 16]; let mut i = 0;
    let mut x = v;
    while x > 0 { let n = (x & 0xF) as u8; b[i] = if n < 10 { b'0' + n } else { b'A' + n - 10 }; x >>= 4; i += 1; }
    while i > 0 { i -= 1; crate::driver::uart::putchar(b[i]); }
}

fn dec(mut v: u64) {
    if v == 0 { crate::driver::uart::putchar(b'0'); return; }
    let mut b = [0u8; 20]; let mut i = 0;
    while v > 0 { b[i] = b'0' + (v % 10) as u8; v /= 10; i += 1; }
    while i > 0 { i -= 1; crate::driver::uart::putchar(b[i]); }
}

#[inline(always)]
fn current_rbp() -> u64 {
    let r: u64;
    unsafe { core::arch::asm!("mov {}, rbp", out(reg) r, options(nomem, nostack, preserves_flags)); }
    r
}

fn valid_frame(p: u64) -> bool {
    if p == 0 || (p & 7) != 0 { return false; }
    // Каноничный low-half или high-half
    let low = p < 0x0000_8000_0000_0000;
    let high = p >= 0xFFFF_8000_0000_0000;
    low || high
}

fn valid_ret(r: u64) -> bool {
    if r == 0 { return true; } // нулевой ret — конец цепочки, печатаем и стоп
    valid_frame(r)
}

/// Печать backtrace от текущего RBP. Вызывать из #PF/panic.
pub unsafe fn dump_current(max: usize) {
    let cap = max.min(16);
    crate::driver::uart::write_str("  backtrace (rbp chain):\r\n");
    let mut rbp = current_rbp();
    for i in 0..cap {
        if !valid_frame(rbp) { break; }
        // [rbp] = prev rbp, [rbp+8] = return RIP
        let prev = core::ptr::read_volatile(rbp as *const u64);
        let ret = core::ptr::read_volatile((rbp + 8) as *const u64);
        if !valid_ret(ret) { break; } // мусор (нет frame pointers) — дальше не идём
        crate::driver::uart::write_str("    #"); dec(i as u64);
        crate::driver::uart::write_str(" rbp=0x"); hex(rbp);
        crate::driver::uart::write_str(" ret=0x"); hex(ret);
        crate::driver::uart::write_str("\r\n");
        if prev == 0 || prev == rbp { break; }
        // Стек растёт вниз — фреймы идут вверх. Если next <= current — цикл/мусор.
        if prev <= rbp && rbp < 0xFFFF_8000_0000_0000 { break; }
        if prev <= rbp { break; }
        rbp = prev;
    }
}

/// Backtrace от явного RBP (для #PF где RBP обработчика уже другой).
pub unsafe fn dump_from(mut rbp: u64, max: usize) {
    let cap = max.min(16);
    crate::driver::uart::write_str("  backtrace:\r\n");
    for i in 0..cap {
        if !valid_frame(rbp) { break; }
        let prev = core::ptr::read_volatile(rbp as *const u64);
        let ret = core::ptr::read_volatile((rbp + 8) as *const u64);
        if !valid_ret(ret) { break; }
        crate::driver::uart::write_str("    #"); dec(i as u64);
        crate::driver::uart::write_str(" ret=0x"); hex(ret);
        crate::driver::uart::write_str("\r\n");
        if prev == 0 || prev == rbp || prev <= rbp { break; }
        rbp = prev;
    }
}
