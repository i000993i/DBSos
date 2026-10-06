//! .deb фундамент: ar-архив + tar-парсеры (no_std).
//!
//! .deb = ar-контейнер (debian-binary, control.tar.*, data.tar.*).
//! Здесь: поиск члена ar по имени + поиск файла в tar (ustar).
//! gzip/xz/zstd распаковка data — следующий этап (пока берём несжатый
//! data.tar, валидный по Debian policy). KAT на синтетике в self_test().

/// Найти члена ar по имени. Возвращает срез данных.
pub fn ar_find<'a>(archive: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    if archive.len() < 8 || &archive[..8] != b"!<arch>\n" {
        return None;
    }
    let mut off = 8usize;
    while off + 60 <= archive.len() {
        let hdr = &archive[off..off + 60];
        if &hdr[58..60] != b"`\n" {
            return None;
        }
        // имя: до '/' или пробела
        let mut nlen = 16;
        for i in 0..16 {
            if hdr[i] == b'/' || hdr[i] == b' ' || hdr[i] == 0 {
                nlen = i;
                break;
            }
        }
        // размер: десятичный ASCII, пробелы
        let mut size: usize = 0;
        for i in 48..58 {
            let c = hdr[i];
            if c >= b'0' && c <= b'9' {
                size = size * 10 + (c - b'0') as usize;
            }
        }
        let data_start = off + 60;
        // GNU-таблица имён (//) и длинные имена (/N) — пропускаем как opaque
        if hdr[0] == b'/' {
            off = data_start + ((size + 1) & !1);
            continue;
        }
        if nlen == name.len() && &hdr[..nlen] == name {
            if data_start + size > archive.len() {
                return None;
            }
            return Some(&archive[data_start..data_start + size]);
        }
        off = data_start + ((size + 1) & !1);
    }
    None
}

fn octal(b: &[u8]) -> usize {
    let mut v = 0usize;
    for &c in b {
        if c == 0 || c == b' ' {
            break;
        }
        if c < b'0' || c > b'7' {
            return 0;
        }
        v = v * 8 + (c - b'0') as usize;
    }
    v
}

/// Найти файл в tar (ustar) по пути. Возвращает срез данных.
pub fn tar_find<'a>(archive: &'a [u8], path: &[u8]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 512 <= archive.len() {
        let hdr = &archive[off..off + 512];
        if hdr.iter().all(|&c| c == 0) {
            return None; // два нулевых блока = конец
        }
        // имя: [0..100] + prefix [345..500] (ustar)
        let mut name_len = 0;
        while name_len < 100 && hdr[name_len] != 0 {
            name_len += 1;
        }
        let name = &hdr[..name_len];
        // склейка с prefix для длинных путей
        let full: &[u8];
        let mut joined = [0u8; 256];
        let plen = {
            let mut pl = 0;
            while pl < 155 && hdr[345 + pl] != 0 {
                pl += 1;
            }
            pl
        };
        if plen > 0 {
            let mut j = 0;
            joined[..plen].copy_from_slice(&hdr[345..345 + plen]);
            j += plen;
            joined[j] = b'/';
            j += 1;
            joined[j..j + name_len].copy_from_slice(name);
            j += name_len;
            full = &joined[..j];
        } else {
            full = name;
        }
        let size = octal(&hdr[124..136]);
        let data_start = off + 512;
        let data_end = data_start + size;
        if data_end > archive.len() {
            return None;
        }
        let is_file = hdr[156] == 0 || hdr[156] == b'0';
        if is_file && full == path {
            return Some(&archive[data_start..data_end]);
        }
        off = data_end + ((512 - (size % 512)) % 512);
    }
    None
}

/// Собрать минимальный ar-член в буфер (для KAT). Возвращает длину.
fn ar_emit(buf: &mut [u8], name: &[u8], data: &[u8]) -> usize {
    let mut o = 0;
    buf[o..o + 8].copy_from_slice(b"!<arch>\n");
    o += 8;
    // header 60 байт
    for i in 0..16 {
        buf[o + i] = if i < name.len() { name[i] } else { b' ' };
    }
    if name.len() < 16 {
        buf[o + name.len()] = b'/';
    }
    o += 16;
    // mtime/uid/gid/mode пробелами+нули
    for i in 0..12 + 6 + 6 + 8 {
        buf[o + i] = b' ';
    }
    o += 12 + 6 + 6 + 8;
    // size decimal width 10
    let mut sz = data.len();
    let mut digits = [b' '; 10];
    let mut di = 10;
    if sz == 0 {
        di -= 1;
        digits[di] = b'0';
    }
    while sz > 0 {
        di -= 1;
        digits[di] = b'0' + (sz % 10) as u8;
        sz /= 10;
    }
    buf[o..o + 10].copy_from_slice(&digits);
    o += 10;
    buf[o] = b'`';
    buf[o + 1] = b'\n';
    o += 2;
    buf[o..o + data.len()].copy_from_slice(data);
    o += data.len();
    if data.len() % 2 == 1 {
        buf[o] = b'\n';
        o += 1;
    }
    o
}

/// Собрать ustar-блок файла в буфер (для KAT). Возвращает длину.
fn tar_emit(buf: &mut [u8], path: &[u8], data: &[u8]) -> usize {
    // Заголовок ровно 512 байт, сначала нули
    for i in 0..512 {
        buf[i] = 0;
    }
    // name[0..100]
    let nl = path.len().min(100);
    buf[..nl].copy_from_slice(&path[..nl]);
    // size: octal, 11 цифр + NUL (ведущие нули парсер пропускает корректно)
    let mut sz = data.len();
    for i in 0..11 {
        buf[124 + i] = b'0';
    }
    buf[135] = 0;
    let mut di = 11usize;
    while sz > 0 && di > 0 {
        di -= 1;
        buf[124 + di] = b'0' + (sz % 8) as u8;
        sz /= 8;
    }
    buf[156] = b'0'; // regular file
    buf[257..262].copy_from_slice(b"ustar");
    // данные + добивка нулями до 512
    let mut o = 512;
    buf[o..o + data.len()].copy_from_slice(data);
    o += data.len();
    while o % 512 != 0 {
        buf[o] = 0;
        o += 1;
    }
    o
}

/// KAT: синтетический ar (2 члена) + tar (2 файла). 0 = ok.
pub fn self_test() -> u32 {
    let mut ar = [0u8; 256];
    // второй вызов должен дописать, а не перезаписать: собираем вручную
    let l1 = ar_emit(&mut ar, b"a.txt", b"AAAA");
    // ar_emit пишет магию заново — для второго члена сдвигаем вручную:
    let mut ar2 = [0u8; 256];
    ar2[..l1].copy_from_slice(&ar[..l1]);
    // дописать второй член без магии: эмулируем через ar_emit во временный + склейка
    let mut tmp = [0u8; 128];
    let lt = ar_emit(&mut tmp, b"b.txt", b"BBBBB");
    // tmp = magic(8) + member; копируем member часть
    ar2[l1..l1 + lt - 8].copy_from_slice(&tmp[8..lt]);
    let total = l1 + lt - 8;
    let ar = &ar2[..total];
    match ar_find(ar, b"a.txt") {
        Some(d) if d == b"AAAA" => {}
        _ => return 1,
    }
    match ar_find(ar, b"b.txt") {
        Some(d) if d == b"BBBBB" => {}
        _ => return 2,
    }
    if ar_find(ar, b"nope").is_some() {
        return 4;
    }
    if ar_find(b"garbage", b"a.txt").is_some() {
        return 8;
    }
    // tar (4096: два файла + место под два нулевых блока конца архива)
    let mut tar = [0u8; 2048];
    let t1 = tar_emit(&mut tar, b"./x.txt", b"1234");
    let mut tar2 = [0u8; 4096];
    tar2[..t1].copy_from_slice(&tar[..t1]);
    let t2 = tar_emit(&mut tar2[t1..], b"./d/y.txt", b"56789");
    let total2 = t1 + t2;
    // два нулевых блока в конце уже есть (буфер нулевой)
    let tar = &tar2[..total2 + 1024];
    match tar_find(tar, b"./x.txt") {
        Some(d) if d == b"1234" => {}
        _ => return 16,
    }
    match tar_find(tar, b"./d/y.txt") {
        Some(d) if d == b"56789" => {}
        _ => return 32,
    }
    if tar_find(tar, b"./nope").is_some() {
        return 64;
    }
    0
}
