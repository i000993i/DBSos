//! AF_UNIX socketpair для ядра: локальный транспорт без сети.
//!
//! Независимая таблица (8 пар, ring-буферы 2KB на направление), non-blocking:
//! send возвращает сколько влезло, recv — сколько есть. Фундамент для
//! будущей привязки AF_UNIX-сокетов в syscall-слое и Wayland-сокета.
//! KAT: pair → send → recv → close гоняется в RS-Kernel-Test.

const MAX_PAIRS: usize = 8;
const RING: usize = 2048;

#[derive(Clone, Copy)]
struct OneWay {
    buf: [u8; RING],
    head: usize, // чтение
    len: usize,  // занято
}

impl OneWay {
    const fn empty() -> Self {
        OneWay { buf: [0; RING], head: 0, len: 0 }
    }
    fn push(&mut self, data: &[u8]) -> usize {
        let free = RING - self.len;
        let n = data.len().min(free);
        for i in 0..n {
            self.buf[(self.head + self.len + i) % RING] = data[i];
        }
        self.len += n;
        n
    }
    fn pop(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.len);
        for i in 0..n {
            out[i] = self.buf[(self.head + i) % RING];
        }
        self.head = (self.head + n) % RING;
        self.len -= n;
        n
    }
}

#[derive(Clone, Copy)]
struct Pair {
    used: bool,
    a_to_b: OneWay,
    b_to_a: OneWay,
    a_open: bool,
    b_open: bool,
}

impl Pair {
    const fn empty() -> Self {
        Pair { used: false, a_to_b: OneWay::empty(), b_to_a: OneWay::empty(), a_open: false, b_open: false }
    }
}

static mut PAIRS: [Pair; MAX_PAIRS] = [Pair::empty(); MAX_PAIRS];

/// Хендл: (pair_idx << 1) | end (0=a, 1=b).
fn split(h: usize) -> Option<(usize, usize)> {
    let p = h >> 1;
    let e = h & 1;
    if p >= MAX_PAIRS { return None; }
    unsafe {
        if !PAIRS[p].used { return None; }
        if (e == 0 && !PAIRS[p].a_open) || (e == 1 && !PAIRS[p].b_open) {
            return None;
        }
    }
    Some((p, e))
}

/// Создать пару. Возвращает (ha, hb).
pub fn pair() -> Option<(usize, usize)> {
    unsafe {
        for i in 0..MAX_PAIRS {
            if !PAIRS[i].used {
                PAIRS[i] = Pair::empty();
                PAIRS[i].used = true;
                PAIRS[i].a_open = true;
                PAIRS[i].b_open = true;
                return Some((i << 1, (i << 1) | 1));
            }
        }
        None
    }
}

pub fn send(h: usize, data: &[u8]) -> usize {
    let (p, e) = match split(h) {
        Some(v) => v,
        None => return 0,
    };
    unsafe {
        if e == 0 {
            PAIRS[p].a_to_b.push(data)
        } else {
            PAIRS[p].b_to_a.push(data)
        }
    }
}

pub fn recv(h: usize, out: &mut [u8]) -> usize {
    let (p, e) = match split(h) {
        Some(v) => v,
        None => return 0,
    };
    unsafe {
        if e == 0 {
            PAIRS[p].b_to_a.pop(out)
        } else {
            PAIRS[p].a_to_b.pop(out)
        }
    }
}

pub fn close(h: usize) {
    let p = h >> 1;
    let e = h & 1;
    unsafe {
        if p >= MAX_PAIRS || !PAIRS[p].used {
            return;
        }
        if e == 0 {
            PAIRS[p].a_open = false;
        } else {
            PAIRS[p].b_open = false;
        }
        if !PAIRS[p].a_open && !PAIRS[p].b_open {
            PAIRS[p] = Pair::empty();
        }
    }
}

/// KAT: pair → send/recv в обе стороны → wrap-around → close. 0 = ok.
pub fn self_test() -> u32 {
    let (a, b) = match pair() {
        Some(v) => v,
        None => return 1,
    };
    // a -> b
    let msg = b"unix-socket-test-123";
    if send(a, msg) != msg.len() {
        return 2;
    }
    let mut buf = [0u8; 32];
    if recv(b, &mut buf) != msg.len() {
        return 4;
    }
    if buf[..msg.len()] != *msg {
        return 8;
    }
    // b -> a, два куска (частичный recv)
    if send(b, b"hello world") != 11 {
        return 16;
    }
    let mut p1 = [0u8; 5];
    let mut p2 = [0u8; 8];
    if recv(a, &mut p1) != 5 || &p1 != b"hello" {
        return 32;
    }
    if recv(a, &mut p2) != 6 || &p2[..6] != b" world" {
        return 64;
    }
    // пустой recv
    if recv(a, &mut p2) != 0 {
        return 128;
    }
    close(a);
    close(b);
    // после close — тишина
    if send(a, b"x") != 0 || recv(b, &mut buf) != 0 {
        return 256;
    }
    0
}
