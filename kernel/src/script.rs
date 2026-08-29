/// Интерпретатор shell-скриптов (.sh)
///
/// Поддерживает:
///   - комментарии (#)
///   - переменные: export VAR=val, $VAR, $?, $$
///   - пайпы: cmd1 | cmd2 (пока без реального pipe I/O)
///   - перенаправление: cmd > file, cmd < file, cmd >> file
///   - условный: if CMD; then ... elif CMD; then ... else ... fi
///   - цикл: for VAR in A B C; do ... done
///   - выход: exit [code]

use crate::vfs;

// ── Переменные окружения ───────────────────────────────────────────
const MAX_VARS: usize = 64;
const MAX_NAME: usize = 32;
const MAX_VAL: usize = 128;

static mut ENV_NAMES: [[u8; MAX_NAME]; MAX_VARS] = [[0; MAX_NAME]; MAX_VARS];
static mut ENV_VALS: [[u8; MAX_VAL]; MAX_VARS] = [[0; MAX_VAL]; MAX_VARS];
static mut ENV_COUNT: usize = 0;
static mut LAST_EXIT_CODE: u32 = 0;

pub fn set_var(name: &[u8], value: &[u8]) {
    unsafe {
        for i in 0..ENV_COUNT {
            if name_eq(&ENV_NAMES[i], name) {
                copy_slice(&mut ENV_VALS[i], value);
                return;
            }
        }
        if ENV_COUNT >= MAX_VARS { return; }
        copy_slice(&mut ENV_NAMES[ENV_COUNT], name);
        copy_slice(&mut ENV_VALS[ENV_COUNT], value);
        ENV_COUNT += 1;
    }
}

pub fn get_var(name: &[u8]) -> Option<&'static [u8]> {
    unsafe {
        for i in 0..ENV_COUNT {
            if name_eq(&ENV_NAMES[i], name) {
                return Some(nul_term(&ENV_VALS[i]));
            }
        }
    }
    None
}

pub fn set_last_exit(code: u32) {
    unsafe { LAST_EXIT_CODE = code; }
}

pub fn last_exit() -> u32 {
    unsafe { LAST_EXIT_CODE }
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

fn nul_term(s: &[u8]) -> &[u8] {
    let len = clen(s);
    &s[..len]
}

fn copy_slice(dst: &mut [u8], src: &[u8]) {
    let n = src.len().min(dst.len() - 1);
    for i in 0..n { dst[i] = src[i]; }
    if n < dst.len() { dst[n] = 0; }
}

// ── Замена переменных ─────────────────────────────────────────────

fn expand_vars(input: &[u8], output: &mut [u8]) -> usize {
    let mut oi = 0;
    let mut i = 0;
    while i < input.len() && oi < output.len() - 1 {
        if input[i] == b'$' && i + 1 < input.len() {
            i += 1;
            match input[i] {
                b'?' => {
                    let v = last_exit();
                    let s = u32_to_str(v);
                    for &c in s.iter() {
                        if c == 0 { break; }
                        if oi < output.len() - 1 { output[oi] = c; oi += 1; }
                    }
                    i += 1;
                }
                b'$' => {
                    let s = u32_to_str(1);
                    for &c in s.iter() {
                        if c == 0 { break; }
                        if oi < output.len() - 1 { output[oi] = c; oi += 1; }
                    }
                    i += 1;
                }
                b'0'..=b'9' => {
                    i += 1;
                }
                b'{' => {
                    i += 1;
                    let start = i;
                    while i < input.len() && input[i] != b'}' { i += 1; }
                    if i < input.len() {
                        let name = &input[start..i];
                        i += 1;
                        if let Some(val) = get_var(name) {
                            for &c in val {
                                if oi < output.len() - 1 { output[oi] = c; oi += 1; }
                            }
                        }
                    }
                }
                b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                    let start = i;
                    while i < input.len() && (input[i].is_ascii_alphanumeric() || input[i] == b'_') {
                        i += 1;
                    }
                    let name = &input[start..i];
                    if let Some(val) = get_var(name) {
                        for &c in val {
                            if oi < output.len() - 1 { output[oi] = c; oi += 1; }
                        }
                    }
                }
                _ => {
                    if oi < output.len() - 1 { output[oi] = b'$'; oi += 1; }
                }
            }
        } else {
            output[oi] = input[i];
            oi += 1;
            i += 1;
        }
    }
    output[oi] = 0;
    oi
}

fn u32_to_str(v: u32) -> [u8; 12] {
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

// ── Чтение файла ──────────────────────────────────────────────────

struct FileData {
    buf: [u8; 8192],
    len: usize,
}

fn read_file(path: &[u8]) -> Option<FileData> {
    let fd = vfs::open(path, 0);
    if fd < 0 { return None; }
    let mut data = FileData { buf: [0u8; 8192], len: 0 };
    let mut tmp = [0u8; 4096];
    loop {
        let n = vfs::read(fd, &mut tmp);
        if n <= 0 { break; }
        let n = n as usize;
        let copy = n.min(data.buf.len() - data.len);
        for i in 0..copy { data.buf[data.len + i] = tmp[i]; }
        data.len += copy;
    }
    vfs::close(fd);
    if data.len == 0 { None } else { Some(data) }
}

// ── Токенизатор ───────────────────────────────────────────────────

struct Tokens {
    argv: [[u8; 128]; 16],
    argc: usize,
    redir_out: [u8; 128],
    redir_out_append: bool,
    redir_in: [u8; 128],
}

fn tokenize(line: &[u8]) -> Tokens {
    let mut t = Tokens {
        argv: [[0u8; 128]; 16],
        argc: 0,
        redir_out: [0u8; 128],
        redir_out_append: false,
        redir_in: [0u8; 128],
    };
    let mut i = 0;
    let mut in_quote = false;
    let mut quote_char = b'"';

    while i < line.len() {
        while i < line.len() && (line[i] == b' ' || line[i] == b'\t') { i += 1; }
        if i >= line.len() { break; }
        if line[i] == b'#' { break; }

        // > or >>
        if line[i] == b'>' && !in_quote {
            i += 1;
            if i < line.len() && line[i] == b'>' {
                t.redir_out_append = true;
                i += 1;
            }
            while i < line.len() && line[i] == b' ' { i += 1; }
            let start = i;
            while i < line.len() && line[i] != b' ' && line[i] != b'\t' && line[i] != b'\n' && line[i] != b'|' {
                i += 1;
            }
            copy_slice(&mut t.redir_out, &line[start..i]);
            continue;
        }

        // <
        if line[i] == b'<' && !in_quote {
            i += 1;
            while i < line.len() && line[i] == b' ' { i += 1; }
            let start = i;
            while i < line.len() && line[i] != b' ' && line[i] != b'\t' && line[i] != b'\n' && line[i] != b'|' {
                i += 1;
            }
            copy_slice(&mut t.redir_in, &line[start..i]);
            continue;
        }

        // |
        if line[i] == b'|' && !in_quote { break; }

        // quote
        if (line[i] == b'"' || line[i] == b'\'') && !in_quote {
            quote_char = line[i];
            in_quote = true;
            i += 1;
        }

        // token
        if t.argc >= 16 { break; }
        let mut ti = 0;
        while i < line.len() {
            if in_quote {
                if line[i] == quote_char { in_quote = false; i += 1; break; }
            } else {
                if line[i] == b' ' || line[i] == b'\t' || line[i] == b'|' || line[i] == b'>' || line[i] == b'<' {
                    break;
                }
            }
            if ti < 127 { t.argv[t.argc][ti] = line[i]; ti += 1; }
            i += 1;
        }
        t.argv[t.argc][ti] = 0;
        t.argc += 1;
    }
    t
}

// ── Безопасное чтение C-строки ────────────────────────────────────

unsafe fn _cstr<'a>(ptr: *const u8) -> &'a [u8] {
    let mut len = 0;
    while *ptr.add(len) != 0 && len < 255 { len += 1; }
    core::slice::from_raw_parts(ptr, len)
}

// ── Выполнение одной команды ───────────────────────────────────────

fn exec_single(argv: &[[u8; 128]], argc: usize,
               redir_out: &[u8], redir_append: bool,
               _redir_in: &[u8]) -> u32
{
    if argc == 0 { return 0; }
    let cmd = &argv[0][..clen(&argv[0])];

    // cd
    if cmd == b"cd" {
        let arg = if argc > 1 { &argv[1][..clen(&argv[1])] } else { b"" };
        crate::shell::cd_pub(arg);
        return 0;
    }

    // echo
    if cmd == b"echo" {
        let mut out = [0u8; 512];
        let mut oi = 0;
        for j in 1..argc {
            let word = &argv[j][..clen(&argv[j])];
            if j > 1 && oi < out.len() - 1 { out[oi] = b' '; oi += 1; }
            for &c in word {
                if oi < out.len() - 1 { out[oi] = c; oi += 1; }
            }
        }
        out[oi] = 0;
        if !redir_out.is_empty() && redir_out[0] != 0 {
            let ffd = vfs::open(redir_out, 0x200 | 0x100);
            if ffd >= 0 {
                if redir_append { vfs::lseek(ffd, 0, 2); }
                vfs::write(ffd, &out[..oi]);
                vfs::close(ffd);
            }
        } else {
            crate::shell::print_str_pub(&out[..oi]);
        }
        return 0;
    }

    // export
    if cmd == b"export" {
        if argc > 1 {
            let eq = &argv[1][..clen(&argv[1])];
            if let Some(pos) = eq.iter().position(|&c| c == b'=') {
                set_var(&eq[..pos], &eq[pos + 1..]);
            }
        }
        return 0;
    }

    // exit
    if cmd == b"exit" {
        if argc > 1 {
            return parse_u32(&argv[1][..clen(&argv[1])]);
        }
        return 1;
    }

    // pwd
    if cmd == b"pwd" {
        let cwd = crate::shell::cwd_get_public();
        crate::shell::print_str_pub(cwd);
        crate::shell::print_str_pub(b"\r\n");
        return 0;
    }

    // ls
    if cmd == b"ls" {
        let path = if argc > 1 { &argv[1][..clen(&argv[1])] } else { b"/" };
        if !redir_out.is_empty() && redir_out[0] != 0 {
            let mut listing = [0u8; 4096];
            let n = vfs::readdir_to_buf(path, &mut listing);
            let ffd = vfs::open(redir_out, 0x200 | 0x100);
            if ffd >= 0 {
                vfs::write(ffd, &listing[..n]);
                vfs::close(ffd);
            }
        } else {
            crate::shell::ls_pub(path);
        }
        return 0;
    }

    // cat
    if cmd == b"cat" {
        if argc < 2 { return 1; }
        let path = &argv[1][..clen(&argv[1])];
        if !redir_out.is_empty() && redir_out[0] != 0 {
            let fd = vfs::open(path, 0);
            if fd < 0 { return 1; }
            let ffd = vfs::open(redir_out, 0x200 | 0x100);
            if ffd < 0 { vfs::close(fd); return 1; }
            let mut buf = [0u8; 4096];
            loop {
                let n = vfs::read(fd, &mut buf);
                if n <= 0 { break; }
                vfs::write(ffd, &buf[..n as usize]);
            }
            vfs::close(fd);
            vfs::close(ffd);
            return 0;
        }
        let fd = vfs::open(path, 0);
        if fd < 0 { return 1; }
        let mut buf = [0u8; 4096];
        loop {
            let n = vfs::read(fd, &mut buf);
            if n <= 0 { break; }
            crate::shell::print_str_pub(&buf[..n as usize]);
        }
        vfs::close(fd);
        return 0;
    }

    // mkdir
    if cmd == b"mkdir" {
        if argc < 2 { return 1; }
        return if vfs::mkdir(&argv[1][..clen(&argv[1])]) { 0 } else { 1 };
    }

    // rm
    if cmd == b"rm" {
        if argc < 2 { return 1; }
        return if vfs::unlink(&argv[1][..clen(&argv[1])]) { 0 } else { 1 };
    }

    // rmdir
    if cmd == b"rmdir" {
        if argc < 2 { return 1; }
        return if vfs::rmdir(&argv[1][..clen(&argv[1])]) { 0 } else { 1 };
    }

    // write
    if cmd == b"write" {
        if argc < 3 { return 1; }
        let path = &argv[1][..clen(&argv[1])];
        let content = &argv[2][..clen(&argv[2])];
        let ffd = vfs::open(path, 0x200 | 0x100);
        if ffd < 0 { return 1; }
        vfs::write(ffd, content);
        vfs::close(ffd);
        return 0;
    }

    // tcp test
    if cmd == b"tcp" && argc > 1 && argv[1][..clen(&argv[1])] == *b"test" {
        crate::driver::tcp::test_stack();
        return 0;
    }

    // nvme info
    if cmd == b"nvme" && argc > 1 && argv[1][..clen(&argv[1])] == *b"info" {
        if unsafe { crate::driver::nvme::INIT } {
            crate::shell::print_str_pub(b"NVMe: initialized\r\n");
        } else {
            crate::shell::print_str_pub(b"NVMe: not present\r\n");
        }
        return 0;
    }

    // ping, dhcp, mem, time, info, help
    if cmd == b"ping" { crate::shell::cmd_ping_pub(); return 0; }
    if cmd == b"dhcp" { crate::shell::cmd_dhcp_pub(); return 0; }
    if cmd == b"mem" { crate::shell::cmd_mem_pub(); return 0; }
    if cmd == b"time" { crate::shell::cmd_time_pub(); return 0; }
    if cmd == b"info" { crate::shell::cmd_info_pub(); return 0; }
    if cmd == b"help" { crate::shell::cmd_help_pub(); return 0; }
    if cmd == b"wget" {
        if argc > 1 {
            let url = &argv[1][..clen(&argv[1])];
            let file = if argc > 2 { &argv[2][..clen(&argv[2])] } else { b"" };
            crate::wget::wget(url, file);
        }
        return 0;
    }
    if cmd == b"pkg" {
        if argc > 1 {
            let subcmd = &argv[1][..clen(&argv[1])];
            crate::pkg::pkg_main(subcmd);
        }
        return 0;
    }
    if cmd == b"reboot" { crate::acpi::reboot(); return 0; }
    if cmd == b"poweroff" { crate::acpi::shutdown(); return 0; }

    // Внешняя программа — пробуем /bin/CMD
    let mut path_buf = [0u8; 128];
    let mut pi = 0;
    for &c in b"/bin/" {
        if pi < 127 { path_buf[pi] = c; pi += 1; }
    }
    let cmd_name = &argv[0][..clen(&argv[0])];
    for &c in cmd_name {
        if pi < 127 { path_buf[pi] = c; pi += 1; }
    }
    path_buf[pi] = 0;

    let elf_fd = vfs::open(&path_buf[..pi], 0);
    if elf_fd >= 0 {
        vfs::close(elf_fd);
        crate::elf::load_and_spawn(&path_buf[..pi]);
        return 0;
    }
    let elf_fd = vfs::open(cmd_name, 0);
    if elf_fd >= 0 {
        vfs::close(elf_fd);
        crate::elf::load_and_spawn(cmd_name);
        return 0;
    }

    crate::shell::print_str_pub(b"command not found: ");
    crate::shell::print_str_pub(cmd_name);
    crate::shell::print_str_pub(b"\r\n");
    1
}

fn parse_u32(s: &[u8]) -> u32 {
    let mut v = 0u32;
    for &c in s {
        if c >= b'0' && c <= b'9' {
            v = v.wrapping_mul(10).wrapping_add((c - b'0') as u32);
        }
    }
    v
}

// ── Выполнение строки ─────────────────────────────────────────────

fn exec_line(raw: &[u8]) -> u32 {
    // Убираем комментарии
    let line = if let Some(pos) = raw.iter().position(|&c| c == b'#') {
        &raw[..pos]
    } else {
        raw
    };

    // Trim
    let line = trim(line);
    if line.is_empty() { return 0; }

    // Замена переменных
    let mut expanded = [0u8; 1024];
    let elen = expand_vars(line, &mut expanded);
    let line = &expanded[..elen];

    // Пайпы (пока просто разбиваем и выполняем по очереди)
    let mut parts: [[u8; 256]; 8] = [[0u8; 256]; 8];
    let mut part_count = 0;
    {
        let mut i = 0;
        while i < line.len() && part_count < 8 {
            let mut j = 0;
            while i < line.len() && line[i] != b'|' {
                if j < 255 { parts[part_count][j] = line[i]; j += 1; }
                i += 1;
            }
            parts[part_count][j] = 0;
            part_count += 1;
            if i < line.len() { i += 1; }
        }
    }

    if part_count > 1 {
        let mut code = 0u32;
        for p in 0..part_count {
            code = exec_single_line(&parts[p]);
        }
        return code;
    }

    exec_single_line(line)
}

fn exec_single_line(line: &[u8]) -> u32 {
    let line = trim(line);
    if line.is_empty() { return 0; }

    // if / for
    if starts_with_word(line, b"if") { return exec_if(line); }
    if starts_with_word(line, b"for") { return exec_for(line); }

    let t = tokenize(line);
    let code = exec_single(&t.argv, t.argc, &t.redir_out, t.redir_out_append, &t.redir_in);
    set_last_exit(code);
    code
}

// ── if/then/elif/else/fi ──────────────────────────────────────────

fn exec_if(line: &[u8]) -> u32 {
    // Пропускаем "if"
    let after_if = skip_first_word(line);
    let then_pos = find_keyword(after_if, b"then");
    if then_pos == 0 { return 1; }

    let condition = trim(&after_if[..then_pos]);
    let body_start = skip_keyword_text(&after_if[then_pos..], b"then");

    // Ищем elif / else / fi
    let (elif_pos, else_pos, fi_pos) = find_if_parts(&after_if[then_pos..]);

    // Выполняем условие
    let cond_code = exec_line(condition);

    if cond_code == 0 {
        // then-ветка
        let body_end = if elif_pos > 0 { elif_pos } else if else_pos > 0 { else_pos } else { fi_pos };
        let body = trim(&after_if[then_pos..][body_start..body_end]);
        if body.is_empty() { return 0; }
        return exec_block(body);
    }

    // elif
    if elif_pos > 0 {
        let elif_text = &after_if[then_pos..][elif_pos..];
        let elif_then = find_keyword(elif_text, b"then");
        if elif_then > 0 {
            let elif_cond = trim(&elif_text[..elif_then]);
            let elif_body_start = skip_keyword_text(&elif_text[elif_then..], b"then");
            let elif_body_end = if else_pos > 0 { else_pos - elif_pos } else { fi_pos - elif_pos };
            let elif_body = trim(&elif_text[elif_then..][elif_body_start..elif_body_end]);
            let c2 = exec_line(elif_cond);
            if c2 == 0 && !elif_body.is_empty() {
                return exec_block(elif_body);
            }
        }
    }

    // else
    if else_pos > 0 {
        let else_text = &after_if[then_pos..][else_pos..];
        let else_body = trim(&else_text[4..]); // skip "else"
        let else_body = trim_end_fi(else_body);
        if !else_body.is_empty() {
            return exec_block(else_body);
        }
    }

    0
}

fn exec_block(body: &[u8]) -> u32 {
    let mut last_code = 0u32;
    let mut i = 0;
    let b = trim(body);
    while i < b.len() {
        while i < b.len() && (b[i] == b'\n' || b[i] == b';' || b[i] == b'\r') { i += 1; }
        if i >= b.len() { break; }
        let start = i;
        while i < b.len() && b[i] != b'\n' && b[i] != b';' { i += 1; }
        let line = trim(&b[start..i]);
        if !line.is_empty() { last_code = exec_line(line); }
    }
    last_code
}

// ── for/in/do/done ────────────────────────────────────────────────

fn exec_for(line: &[u8]) -> u32 {
    let after_for = skip_first_word(line);

    // Имя переменной
    let var_name = first_word(after_for);
    if var_name.is_empty() { return 1; }
    let after_var = skip_first_word(after_for);

    // Пропускаем "in"
    let after_in = skip_first_word(trim(after_var));

    // Список до "do"
    let do_pos = find_keyword(after_in, b"do");
    if do_pos == 0 { return 1; }
    let list_str = trim(&after_in[..do_pos]);
    let body_start = skip_keyword_text(&after_in[do_pos..], b"do");

    // Тело до "done"
    let done_pos = find_keyword(&after_in[do_pos..], b"done");
    let body = if done_pos > 0 {
        trim(&after_in[do_pos..][body_start..do_pos])
    } else {
        trim(&after_in[do_pos..][body_start..])
    };

    // Разбиваем список
    let mut items: [[u8; 64]; 32] = [[0u8; 64]; 32];
    let mut item_count = 0;
    {
        let mut i = 0;
        let ls = trim(list_str);
        while i < ls.len() && item_count < 32 {
            while i < ls.len() && ls[i] == b' ' { i += 1; }
            if i >= ls.len() { break; }
            let mut j = 0;
            while i < ls.len() && ls[i] != b' ' {
                if j < 63 { items[item_count][j] = ls[i]; j += 1; }
                i += 1;
            }
            items[item_count][j] = 0;
            item_count += 1;
        }
    }

    // Выполняем
    for idx in 0..item_count {
        set_var(var_name, &items[idx][..clen(&items[idx])]);
        if !body.is_empty() { exec_block(body); }
    }
    last_exit()
}

// ── Утилиты парсинга ─────────────────────────────────────────────

fn trim<'a>(s: &'a [u8]) -> &'a [u8] {
    let mut start = 0;
    while start < s.len() && matches!(s[start], b' ' | b'\t' | b'\n' | b'\r' | b';') { start += 1; }
    let mut end = s.len();
    while end > start && matches!(s[end - 1], b' ' | b'\t' | b'\n' | b'\r' | b';') { end -= 1; }
    &s[start..end]
}

fn starts_with_word(s: &[u8], word: &[u8]) -> bool {
    if s.len() < word.len() { return false; }
    if &s[..word.len()] != word { return false; }
    if s.len() == word.len() { return true; }
    !s[word.len()].is_ascii_alphanumeric()
}

fn first_word(s: &[u8]) -> &[u8] {
    let s = trim(s);
    let mut i = 0;
    while i < s.len() && s[i] != b' ' && s[i] != b'\t' && s[i] != b'\n' && s[i] != b';' { i += 1; }
    &s[..i]
}

fn skip_first_word<'a>(s: &'a [u8]) -> &'a [u8] {
    let s = trim(s);
    let mut i = 0;
    while i < s.len() && s[i] != b' ' && s[i] != b'\t' && s[i] != b'\n' && s[i] != b';' { i += 1; }
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b';') { i += 1; }
    if i < s.len() { &s[i..] } else { &s[s.len()..] }
}

fn skip_keyword_text<'a>(s: &'a [u8], keyword: &[u8]) -> usize {
    let mut i = 0;
    while i < s.len() && i < keyword.len() { i += 1; }
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b';') { i += 1; }
    i
}

fn find_keyword(s: &[u8], keyword: &[u8]) -> usize {
    let mut i = 0;
    while i < s.len() {
        if s[i] == keyword[0] && i + keyword.len() <= s.len() {
            if &s[i..i + keyword.len()] == keyword {
                let before_ok = i == 0 || !s[i - 1].is_ascii_alphanumeric();
                let after_pos = i + keyword.len();
                let after_ok = after_pos >= s.len() || !s[after_pos].is_ascii_alphanumeric();
                if before_ok && after_ok { return i; }
            }
        }
        i += 1;
    }
    0
}

fn find_if_parts(s: &[u8]) -> (usize, usize, usize) {
    let mut elif_pos = 0;
    let mut else_pos = 0;
    let mut fi_pos = 0;
    let mut depth = 0u32;
    let mut i = 0;
    while i < s.len() {
        if starts_with_word(&s[i..], b"if") { depth += 1; i += 2; }
        else if starts_with_word(&s[i..], b"fi") {
            if depth == 0 { fi_pos = i; break; }
            depth -= 1; i += 2;
        }
        else if starts_with_word(&s[i..], b"elif") && depth == 1 && elif_pos == 0 { elif_pos = i; i += 4; }
        else if starts_with_word(&s[i..], b"else") && depth == 1 && else_pos == 0 { else_pos = i; i += 4; }
        else { i += 1; }
    }
    if fi_pos == 0 { fi_pos = s.len(); }
    (elif_pos, else_pos, fi_pos)
}

fn trim_end_fi<'a>(s: &'a [u8]) -> &'a [u8] {
    let s = trim(s);
    if s.len() >= 2 && &s[s.len() - 2..] == b"fi" {
        &s[..s.len() - 2]
    } else { s }
}

// ── Публичный API ──────────────────────────────────────────────────

pub fn run_script(path: &[u8]) -> u32 {
    let data = match read_file(path) {
        Some(d) => d,
        None => {
            crate::shell::print_str_pub(b"script: cannot open ");
            crate::shell::print_str_pub(path);
            crate::shell::print_str_pub(b"\r\n");
            return 1;
        }
    };
    let content = &data.buf[..data.len];
    exec_block(content)
}
