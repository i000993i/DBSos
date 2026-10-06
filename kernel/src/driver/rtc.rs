/// CMOS RTC (Real-Time Clock) — MC146818 compatible
///
/// Reads real date/time from hardware via I/O ports 0x70 (address) / 0x71 (data).

use crate::driver::uart;

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

const RTC_SEC: u8 = 0x00;
const RTC_MIN: u8 = 0x02;
const RTC_HOUR: u8 = 0x04;
const RTC_DAY: u8 = 0x07;
const RTC_MONTH: u8 = 0x08;
const RTC_YEAR: u8 = 0x09;
const RTC_STATUS_A: u8 = 0x0A;
const RTC_STATUS_B: u8 = 0x0B;

#[derive(Clone, Copy, Default)]
pub struct RtcTime {
    pub second: u8,
    pub minute: u8,
    pub hour: u8,
    pub day: u8,
    pub month: u8,
    pub year: u16,
}

fn cmos_read(reg: u8) -> u8 {
    unsafe {
        crate::io::outb(CMOS_ADDR, reg);
        crate::io::inb(CMOS_DATA)
    }
}

fn bcd_to_bin(val: u8) -> u8 {
    (val & 0x0F) + (val >> 4) * 10
}

fn wait_update() {
    while cmos_read(RTC_STATUS_A) & 0x80 != 0 {
        core::hint::spin_loop();
    }
}

/// Read current RTC date/time — waits for UIP=0 and reads atomically
pub fn read() -> RtcTime {
    let status_b = cmos_read(RTC_STATUS_B);
    let is_binary = status_b & 0x04 != 0;
    let is_24h = status_b & 0x02 != 0;

    wait_update();
    let mut rtc = RtcTime {
        second: cmos_read(RTC_SEC),
        minute: cmos_read(RTC_MIN),
        hour: cmos_read(RTC_HOUR),
        day: cmos_read(RTC_DAY),
        month: cmos_read(RTC_MONTH),
        year: cmos_read(RTC_YEAR) as u16,
    };
    // Check UIP didn't go high during read — if so, retry
    if cmos_read(RTC_STATUS_A) & 0x80 != 0 {
        wait_update();
        rtc.second = cmos_read(RTC_SEC);
        rtc.minute = cmos_read(RTC_MIN);
        rtc.hour = cmos_read(RTC_HOUR);
        rtc.day = cmos_read(RTC_DAY);
        rtc.month = cmos_read(RTC_MONTH);
        rtc.year = cmos_read(RTC_YEAR) as u16;
    }

    let raw_hour = rtc.hour;
    if !is_binary {
        rtc.second = bcd_to_bin(rtc.second);
        rtc.minute = bcd_to_bin(rtc.minute);
        // For 12h mode, handle PM bit before BCD conversion
        let hour_pm = if !is_24h { raw_hour & 0x80 } else { 0 };
        let hour_bcd = raw_hour & 0x7F;
        rtc.hour = bcd_to_bin(hour_bcd);
        if !is_24h && hour_pm != 0 {
            rtc.hour = (rtc.hour % 12) + 12;
            if rtc.hour == 24 { rtc.hour = 12; }
        } else if !is_24h && rtc.hour == 12 {
            rtc.hour = 0;
        }
        rtc.day = bcd_to_bin(rtc.day);
        rtc.month = bcd_to_bin(rtc.month);
        rtc.year = bcd_to_bin(rtc.year as u8) as u16;
    } else if !is_24h {
        if raw_hour & 0x80 != 0 {
            rtc.hour = (raw_hour & 0x7F) + 12;
            if rtc.hour == 24 { rtc.hour = 12; }
        }
        if rtc.hour == 12 { rtc.hour = 0; }
    }

    rtc.year += 2000;
    rtc
}

fn num_to_str(mut v: u8, buf: &mut [u8]) -> usize {
    if v == 0 { buf[0] = b'0'; return 1; }
    let mut tmp = [0u8; 3];
    let mut n = 0;
    while v > 0 { tmp[n] = b'0' + (v % 10); v /= 10; n += 1; }
    let mut i = 0;
    while i < n { buf[i] = tmp[n - 1 - i]; i += 1; }
    n
}

/// Московское время: Europe/Moscow = UTC+3 круглый год (без DST с 2014).
/// QEMU/железо отдаёт RTC в UTC — прибавляем 3 часа с переносом даты.
pub const MSK_OFFSET_HOURS: u8 = 3;

fn is_leap(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(month: u8, year: u16) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap(year) { 29 } else { 28 },
        _ => 30,
    }
}

/// RTC в московском времени (UTC+3).
pub fn read_msk() -> RtcTime {
    let mut t = read();
    t.hour += MSK_OFFSET_HOURS;
    if t.hour >= 24 {
        t.hour -= 24;
        t.day += 1;
        if t.day > days_in_month(t.month, t.year) {
            t.day = 1;
            t.month += 1;
            if t.month > 12 {
                t.month = 1;
                t.year += 1;
            }
        }
    }
    t
}

pub fn init() {    let t = read();
    let mut buf = [0u8; 24];
    let mut i = 0;

    let mut tmp = [0u8; 4];
    let n = num_to_str(t.hour, &mut tmp);
    let mut j = 0; while j < n { buf[i] = tmp[j]; i += 1; j += 1; }
    buf[i] = b':'; i += 1;
    let n = num_to_str(t.minute, &mut tmp);
    let mut j = 0; while j < n { buf[i] = tmp[j]; i += 1; j += 1; }
    buf[i] = b':'; i += 1;
    let n = num_to_str(t.second, &mut tmp);
    let mut j = 0; while j < n { buf[i] = tmp[j]; i += 1; j += 1; }
    buf[i] = b' '; i += 1;
    let n = num_to_str(t.day, &mut tmp);
    let mut j = 0; while j < n { buf[i] = tmp[j]; i += 1; j += 1; }
    buf[i] = b'.'; i += 1;
    let n = num_to_str(t.month, &mut tmp);
    let mut j = 0; while j < n { buf[i] = tmp[j]; i += 1; j += 1; }
    buf[i] = b'.'; i += 1;
    let mut tmp2 = [0u8; 4];
    tmp2[0] = b'0' + (t.year / 1000) as u8;
    tmp2[1] = b'0' + ((t.year / 100) % 10) as u8;
    tmp2[2] = b'0' + ((t.year / 10) % 10) as u8;
    tmp2[3] = b'0' + (t.year % 10) as u8;
    let mut j = 0; while j < 4 { buf[i] = tmp2[j]; i += 1; j += 1; }
    buf[i] = b'\n'; i += 1;

    uart::write_str("[RTC] ");
    uart::write_str(core::str::from_utf8(&buf[..i]).unwrap_or("?"));
}
