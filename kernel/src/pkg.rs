/// Простейший пакетный менеджер (pkg).
///
/// Команды:
///   pkg install NAME  — скачать и установить пакет
///   pkg remove NAME   — удалить пакет
///   pkg list          — список установленных пакетов
///   pkg update        — обновить список пакетов
///
/// Формат пакета: архив .tar.gz (пока просто gzip нет — храним как есть).
/// Репозиторий: JSON-индекс на HTTP-сервере.
///
/// Структура на диске:
///   /pkg/registry.txt     — список доступных пакетов (имя URL)
///   /pkg/installed/       — установленные пакеты
///   /pkg/installed/NAME/  — содержимое пакета

use crate::driver::uart;
use crate::vfs;
use crate::wget;

fn uart_print(s: &str) { uart::write_str(s); }

const REGISTRY_URL: &[u8] = b"10.0.2.2:8080/packages.txt";
const PKG_DIR: &[u8] = b"/pkg";
const INSTALLED_DIR: &[u8] = b"/pkg/installed";

// ── Утилиты ───────────────────────────────────────────────────────

fn ensure_dir(path: &[u8]) {
    if !vfs::is_dir(path) {
        vfs::mkdir(path);
    }
}

fn _u32_to_str(v: u32) -> [u8; 12] {
    let mut buf = [0u8; 12];
    if v == 0 { buf[0] = b'0'; return buf; }
    let mut tmp = [0u8; 12];
    let mut n = 0;
    let mut val = v;
    while val > 0 { tmp[n] = b'0' + (val % 10) as u8; val /= 10; n += 1; }
    let mut j = 0;
    while j < n { buf[j] = tmp[n - 1 - j]; j += 1; }
    buf
}

/// Прочитать файл в буфер, вернуть длину.
fn read_file_buf(path: &[u8], buf: &mut [u8]) -> usize {
    let fd = vfs::open(path, 0);
    if fd < 0 { return 0; }
    let mut total = 0;
    let mut tmp = [0u8; 4096];
    loop {
        let n = vfs::read(fd, &mut tmp);
        if n <= 0 { break; }
        let n = n as usize;
        let copy = n.min(buf.len() - total);
        for i in 0..copy { buf[total + i] = tmp[i]; }
        total += copy;
    }
    vfs::close(fd);
    total
}

/// Записать буфер в файл.
fn write_file_buf(path: &[u8], data: &[u8]) -> bool {
    let ffd = vfs::open(path, 0x200 | 0x100); // O_CREAT | O_WRONLY
    if ffd < 0 { return false; }
    vfs::write(ffd, data);
    vfs::close(ffd);
    true
}

// ── Реестр пакетов ────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct RegistryEntry {
    name: [u8; 64],
    url: [u8; 256],
    version: [u8; 16],
}

struct Registry {
    entries: [RegistryEntry; 64],
    count: usize,
}

fn parse_registry(data: &[u8]) -> Registry {
    let mut reg = Registry {
        entries: [RegistryEntry {
            name: [0u8; 64],
            url: [0u8; 256],
            version: [0u8; 16],
        }; 64],
        count: 0,
    };

    // Формат: NAME VERSION URL\n
    let mut i = 0;
    while i < data.len() && reg.count < 64 {
        // Пропускаем пустые строки
        while i < data.len() && (data[i] == b'\n' || data[i] == b'\r') { i += 1; }
        if i >= data.len() { break; }

        // Читаем имя
        let mut j = 0;
        while i < data.len() && data[i] != b' ' && data[i] != b'\t' {
            if j < 63 { reg.entries[reg.count].name[j] = data[i]; j += 1; }
            i += 1;
        }
        reg.entries[reg.count].name[j] = 0;

        // Пропускаем пробелы
        while i < data.len() && (data[i] == b' ' || data[i] == b'\t') { i += 1; }

        // Читаем версию
        j = 0;
        while i < data.len() && data[i] != b' ' && data[i] != b'\t' && data[i] != b'\n' {
            if j < 15 { reg.entries[reg.count].version[j] = data[i]; j += 1; }
            i += 1;
        }
        reg.entries[reg.count].version[j] = 0;

        // Пропускаем пробелы
        while i < data.len() && (data[i] == b' ' || data[i] == b'\t') { i += 1; }

        // Читаем URL
        j = 0;
        while i < data.len() && data[i] != b'\n' && data[i] != b'\r' {
            if j < 255 { reg.entries[reg.count].url[j] = data[i]; j += 1; }
            i += 1;
        }
        reg.entries[reg.count].url[j] = 0;

        if reg.entries[reg.count].name[0] != 0 {
            reg.count += 1;
        }
    }
    reg
}

fn reg_find(reg: &Registry, name: &[u8]) -> Option<usize> {
    for i in 0..reg.count {
        if name_eq(&reg.entries[i].name, name) {
            return Some(i);
        }
    }
    None
}

fn name_eq(a: &[u8], b: &[u8]) -> bool {
    let alen = clen(a);
    let blen = clen(b);
    if alen != blen { return false; }
    &a[..alen] == &b[..blen]
}

fn clen(s: &[u8]) -> usize {
    s.iter().position(|&c| c == 0).unwrap_or(s.len())
}

// ── Установка пакета ──────────────────────────────────────────────

fn pkg_install(name: &[u8]) -> bool {
    ensure_dir(PKG_DIR);
    ensure_dir(INSTALLED_DIR);

    // Проверяем, уже установлен
    let mut pkg_path = [0u8; 128];
    let mut pi = 0;
    for &c in INSTALLED_DIR { if pi < 127 { pkg_path[pi] = c; pi += 1; } }
    if pi < 127 { pkg_path[pi] = b'/'; pi += 1; }
    for &c in name { if pi < 127 { pkg_path[pi] = c; pi += 1; } }
    pkg_path[pi] = 0;

    if vfs::is_dir(&pkg_path[..pi]) {
        uart_print("[pkg] already installed: ");
        crate::shell::print_str_pub(name);
        uart_print("\r\n");
        return true;
    }

    // Загружаем реестр
    let mut reg_data = [0u8; 16384];
    let reg_path = b"/pkg/registry.txt";

    // Если реестра нет — скачиваем
    if vfs::open(reg_path, 0) < 0 {
        uart_print("[pkg] downloading registry...\r\n");
        if !wget::wget(REGISTRY_URL, reg_path) {
            uart_print("[pkg] failed to download registry\r\n");
            return false;
        }
    }

    let reg_len = read_file_buf(reg_path, &mut reg_data);
    if reg_len == 0 {
        uart_print("[pkg] empty registry\r\n");
        return false;
    }

    let reg = parse_registry(&reg_data[..reg_len]);

    // Ищем пакет
    let idx = match reg_find(&reg, name) {
        Some(i) => i,
        None => {
            uart_print("[pkg] package not found: ");
            crate::shell::print_str_pub(name);
            uart_print("\r\n");
            return false;
        }
    };

    let entry = &reg.entries[idx];
    let url = &entry.url[..clen(&entry.url)];

    // Создаём директорию пакета
    vfs::mkdir(&pkg_path[..pi]);

    // Скачиваем файлы пакета
    // Пока поддерживаем один файл: URL/name (архив)
    uart_print("[pkg] downloading ");
    crate::shell::print_str_pub(name);
    uart_print("...\r\n");

    // Формируем путь для сохранения: /pkg/installed/NAME/content.bin
    let mut save_path = [0u8; 128];
    let mut si = 0;
    for &c in &pkg_path[..pi] { if si < 127 { save_path[si] = c; si += 1; } }
    if si < 127 { save_path[si] = b'/'; si += 1; }
    for &c in b"content.bin" { if si < 127 { save_path[si] = c; si += 1; } }
    save_path[si] = 0;

    if !wget::wget(url, &save_path[..si]) {
        uart_print("[pkg] download failed\r\n");
        // Удаляем пустую директорию
        vfs::rmdir(&pkg_path[..pi]);
        return false;
    }

    // Сохраняем метаданные
    let mut meta_path = [0u8; 128];
    let mut mi = 0;
    for &c in &pkg_path[..pi] { if mi < 127 { meta_path[mi] = c; mi += 1; } }
    if mi < 127 { meta_path[mi] = b'/'; mi += 1; }
    for &c in b"meta.txt" { if mi < 127 { meta_path[mi] = c; mi += 1; } }
    meta_path[mi] = 0;

    let mut meta = [0u8; 256];
    let mut mlen = 0;
    for &c in b"name=" {
        if mlen < 255 { meta[mlen] = c; mlen += 1; }
    }
    for &c in name {
        if mlen < 255 { meta[mlen] = c; mlen += 1; }
    }
    if mlen < 255 { meta[mlen] = b'\n'; mlen += 1; }
    for &c in b"version=" {
        if mlen < 255 { meta[mlen] = c; mlen += 1; }
    }
    let ver = &entry.version[..clen(&entry.version)];
    for &c in ver {
        if mlen < 255 { meta[mlen] = c; mlen += 1; }
    }
    if mlen < 255 { meta[mlen] = b'\n'; mlen += 1; }
    write_file_buf(&meta_path[..mi], &meta[..mlen]);

    uart_print("[pkg] installed: ");
    crate::shell::print_str_pub(name);
    uart_print(" v");
    crate::shell::print_str_pub(ver);
    uart_print("\r\n");
    true
}

// ── Удаление пакета ───────────────────────────────────────────────

fn pkg_remove(name: &[u8]) -> bool {
    let mut pkg_path = [0u8; 128];
    let mut pi = 0;
    for &c in INSTALLED_DIR { if pi < 127 { pkg_path[pi] = c; pi += 1; } }
    if pi < 127 { pkg_path[pi] = b'/'; pi += 1; }
    for &c in name { if pi < 127 { pkg_path[pi] = c; pi += 1; } }
    pkg_path[pi] = 0;

    if !vfs::is_dir(&pkg_path[..pi]) {
        uart_print("[pkg] not installed: ");
        crate::shell::print_str_pub(name);
        uart_print("\r\n");
        return false;
    }

    // Удаляем файлы внутри директории
    let mut entries = [crate::vfs::DirEntry {
        name: [0u8; crate::vfs::MAX_NAME],
        is_dir: false,
        size: 0,
    }; 16];
    let n = vfs::readdir(&pkg_path[..pi], &mut entries);
    for i in 0..n as usize {
        if entries[i].name[0] != 0 {
            let mut fpath = [0u8; 128];
            let mut fi = 0;
            for &c in &pkg_path[..pi] { if fi < 127 { fpath[fi] = c; fi += 1; } }
            if fi < 127 { fpath[fi] = b'/'; fi += 1; }
            let nlen = entries[i].name.iter().position(|&c| c == 0).unwrap_or(32);
            for j in 0..nlen { if fi < 127 { fpath[fi] = entries[i].name[j]; fi += 1; } }
            fpath[fi] = 0;
            vfs::unlink(&fpath[..fi]);
        }
    }

    // Удаляем саму директорию
    vfs::rmdir(&pkg_path[..pi]);

    uart_print("[pkg] removed: ");
    crate::shell::print_str_pub(name);
    uart_print("\r\n");
    true
}

// ── Список установленных ──────────────────────────────────────────

fn pkg_list() {
    ensure_dir(PKG_DIR);
    ensure_dir(INSTALLED_DIR);

    let mut entries = [crate::vfs::DirEntry {
        name: [0u8; crate::vfs::MAX_NAME],
        is_dir: false,
        size: 0,
    }; 64];
    let n = vfs::readdir(INSTALLED_DIR, &mut entries);

    if n == 0 {
        uart_print("[pkg] no packages installed\r\n");
        return;
    }

    uart_print("Installed packages:\r\n");
    for i in 0..n as usize {
        if entries[i].is_dir && entries[i].name[0] != 0 {
            uart_print("  ");
            crate::shell::print_str_pub(&entries[i].name);

            // Читаем версию из meta.txt
            let mut meta_path = [0u8; 128];
            let mut pi = 0;
            for &c in INSTALLED_DIR { if pi < 127 { meta_path[pi] = c; pi += 1; } }
            if pi < 127 { meta_path[pi] = b'/'; pi += 1; }
            let nlen = entries[i].name.iter().position(|&c| c == 0).unwrap_or(32);
            for j in 0..nlen { if pi < 127 { meta_path[pi] = entries[i].name[j]; pi += 1; } }
            if pi < 127 { meta_path[pi] = b'/'; pi += 1; }
            for &c in b"meta.txt" { if pi < 127 { meta_path[pi] = c; pi += 1; } }
            meta_path[pi] = 0;

            let mut mbuf = [0u8; 512];
            let mlen = read_file_buf(&meta_path[..pi], &mut mbuf);
            if mlen > 0 {
                // Ищем строку version=
                if let Some(pos) = find_in_buf(&mbuf[..mlen], b"version=") {
                    let vstart = pos + 8;
                    let vend = mbuf[vstart..].iter().position(|&c| c == b'\n' || c == 0).unwrap_or(mlen - vstart);
                    uart_print(" v");
                    crate::shell::print_str_pub(&mbuf[vstart..vstart + vend]);
                }
            }
            uart_print("\r\n");
        }
    }
}

fn find_in_buf(buf: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || buf.len() < needle.len() { return None; }
    for i in 0..=buf.len() - needle.len() {
        if &buf[i..i + needle.len()] == needle {
            return Some(i);
        }
    }
    None
}

// ── Обновление реестра ────────────────────────────────────────────

fn pkg_update() {
    ensure_dir(PKG_DIR);
    let reg_path = b"/pkg/registry.txt";
    // Удаляем старый реестр
    vfs::unlink(reg_path);
    uart_print("[pkg] downloading registry...\r\n");
    if wget::wget(REGISTRY_URL, reg_path) {
        uart_print("[pkg] registry updated\r\n");
    } else {
        uart_print("[pkg] failed to update registry\r\n");
    }
}

// ── Публичный API ──────────────────────────────────────────────────

pub fn pkg_main(args: &[u8]) {
    // Парсим: pkg SUBCMD [NAME]
    let mut i = 0;
    while i < args.len() && (args[i] == b' ' || args[i] == b'\t') { i += 1; }
    let start = i;
    while i < args.len() && args[i] != b' ' && args[i] != b'\t' { i += 1; }
    let subcmd = &args[start..i];

    while i < args.len() && (args[i] == b' ' || args[i] == b'\t') { i += 1; }
    let name_start = i;
    while i < args.len() && args[i] != b' ' && args[i] != b'\t' && args[i] != b'\n' { i += 1; }
    let name = &args[name_start..i];

    if subcmd == b"install" {
        if name.is_empty() {
            uart_print("Usage: pkg install NAME\r\n");
            return;
        }
        pkg_install(name);
    } else if subcmd == b"remove" || subcmd == b"rm" {
        if name.is_empty() {
            uart_print("Usage: pkg remove NAME\r\n");
            return;
        }
        pkg_remove(name);
    } else if subcmd == b"list" || subcmd == b"ls" {
        pkg_list();
    } else if subcmd == b"update" {
        pkg_update();
    } else {
        uart_print("Usage: pkg [install|remove|list|update] [NAME]\r\n");
        uart_print("  pkg install NAME  - download and install package\r\n");
        uart_print("  pkg remove NAME   - remove installed package\r\n");
        uart_print("  pkg list          - list installed packages\r\n");
        uart_print("  pkg update        - refresh package registry\r\n");
    }
}
