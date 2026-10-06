//! Криптография для защиты запомненных Wi-Fi-подключений (no_std, без зависимостей).
//!
//! - SHA-256 (FIPS 180-4), HMAC-SHA256 (RFC 2104), PBKDF2 (RFC 8018, как в WPA2)
//! - AES-128 (FIPS 197) ECB-block + CBC с PKCS#7 (шифрование PSK в покое)
//! Ключ для хранилища выводится PBKDF2 из MAC-адреса машины (стабильный секрет
//! устройства) — пароли не лежат открытым текстом в /etc/net.conf.
//! Проверка: `connect test` гоняет KAT (NIST/RFC векторы).

// ── SHA-256 ─────────────────────────────────────────────────────────

const SHA_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_compress(h: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = ((block[i * 4] as u32) << 24) | ((block[i * 4 + 1] as u32) << 16)
            | ((block[i * 4 + 2] as u32) << 8) | (block[i * 4 + 3] as u32);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }
    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
        (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA_K[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        hh = g; g = f; f = e; e = d.wrapping_add(t1);
        d = c; c = b; b = a; a = t1.wrapping_add(t2);
    }
    h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e); h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g); h[7] = h[7].wrapping_add(hh);
}

pub fn sha256(msg: &[u8], out: &mut [u8; 32]) {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    let mut off = 0;
    while off + 64 <= msg.len() {
        let mut b = [0u8; 64];
        b.copy_from_slice(&msg[off..off + 64]);
        sha256_compress(&mut h, &b);
        off += 64;
    }
    // Добивка: данные + 0x80 + нули + длина (всегда влезает в 1-2 блока)
    let rem = msg.len() - off;
    let mut last = [0u8; 128];
    last[..rem].copy_from_slice(&msg[off..]);
    last[rem] = 0x80;
    let total = rem + 1 + 8;
    let blocks = if total <= 64 { 1 } else { 2 };
    for i in 0..8 {
        last[blocks * 64 - 8 + i] = (bit_len >> (56 - i * 8)) as u8;
    }
    for bi in 0..blocks {
        let mut b = [0u8; 64];
        b.copy_from_slice(&last[bi * 64..bi * 64 + 64]);
        sha256_compress(&mut h, &b);
    }
    for i in 0..8 {
        out[i * 4] = (h[i] >> 24) as u8;
        out[i * 4 + 1] = (h[i] >> 16) as u8;
        out[i * 4 + 2] = (h[i] >> 8) as u8;
        out[i * 4 + 3] = h[i] as u8;
    }
}

// ── HMAC-SHA256 / PBKDF2 ────────────────────────────────────────────

pub fn hmac_sha256(key: &[u8], msg: &[u8], out: &mut [u8; 32]) {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let mut h = [0u8; 32];
        sha256(key, &mut h);
        k[..32].copy_from_slice(&h);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 { ipad[i] ^= k[i]; opad[i] ^= k[i]; }
    // inner = sha(ipad || msg)
    let mut inner_in = [0u8; 128];
    inner_in[..64].copy_from_slice(&ipad);
    let mut inner = [0u8; 32];
    if msg.len() <= 64 {
        inner_in[64..64 + msg.len()].copy_from_slice(msg);
        let mut h: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
            0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
        ];
        // inner_in фиксированно 64+msg байт — хешируем через общий путь:
        // собираем во временный буфер (msg ≤ 64, итого ≤ 128)
        let tot = 64 + msg.len();
        let mut tmp = [0u8; 128];
        tmp[..tot].copy_from_slice(&inner_in[..tot]);
        sha256_stream(&tmp[..tot], &mut inner, &mut h);
    } else {
        // длинное msg: sha(ipad) || sha(msg) нельзя склеить наивно —
        // считаем inner = sha(ipad||msg) через два compress-прохода вручную:
        // (реализация ниже через sha256_two_part)
        sha256_two_part(&ipad, msg, &mut inner);
    }
    // outer = sha(opad || inner)
    let mut outer_in = [0u8; 96];
    outer_in[..64].copy_from_slice(&opad);
    outer_in[64..96].copy_from_slice(&inner);
    sha256(&outer_in, out);
}

// SHA-256 поверх заранее заданного IV (для HMAC с длинным msg)
fn sha256_stream(msg: &[u8], out: &mut [u8; 32], h: &mut [u32; 8]) {
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    let mut off = 0;
    while off + 64 <= msg.len() {
        let mut b = [0u8; 64];
        b.copy_from_slice(&msg[off..off + 64]);
        sha256_compress(h, &b);
        off += 64;
    }
    let rem = msg.len() - off;
    let mut last = [0u8; 128];
    last[..rem].copy_from_slice(&msg[off..]);
    last[rem] = 0x80;
    let total = rem + 1 + 8;
    let blocks = if total <= 64 { 1 } else { 2 };
    for i in 0..8 {
        last[blocks * 64 - 8 + i] = (bit_len >> (56 - i * 8)) as u8;
    }
    for bi in 0..blocks {
        let mut b = [0u8; 64];
        b.copy_from_slice(&last[bi * 64..bi * 64 + 64]);
        sha256_compress(h, &b);
    }
    for i in 0..8 {
        out[i * 4] = (h[i] >> 24) as u8;
        out[i * 4 + 1] = (h[i] >> 16) as u8;
        out[i * 4 + 2] = (h[i] >> 8) as u8;
        out[i * 4 + 3] = h[i] as u8;
    }
}

// sha(a || b) для произвольных длин (a ≤ 64, b любое): честный streaming.
fn sha256_two_part(a: &[u8], b: &[u8], out: &mut [u8; 32]) {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let total_len = (a.len() + b.len()) as u64;
    let bit_len = total_len.wrapping_mul(8);
    // Собираем поток: a, затем b кусками по 64, затем добивка.
    // Упрощение без аллокаций: обрабатываем a+b через кольцевой остаток.
    let mut buf = [0u8; 64];
    let mut fill = 0usize;
    let feed = |h: &mut [u32; 8], data: &[u8], buf: &mut [u8; 64], fill: &mut usize| {
        let mut o = 0;
        while o < data.len() {
            let take = (64 - *fill).min(data.len() - o);
            buf[*fill..*fill + take].copy_from_slice(&data[o..o + take]);
            *fill += take;
            o += take;
            if *fill == 64 {
                let mut blk = [0u8; 64];
                blk.copy_from_slice(&buf[..]);
                sha256_compress(h, &blk);
                *fill = 0;
            }
        }
    };
    feed(&mut h, a, &mut buf, &mut fill);
    feed(&mut h, b, &mut buf, &mut fill);
    // Добивка с ОБЩЕЙ длиной: 0x80, нули до ≡56 (mod 64), затем 8 байт длины.
    // fill всегда < 64 (feed сжимает полные блоки).
    let k = (56 + 64 - (fill % 64)) % 64;
    let total_pad = 1 + k + 8;
    let mut pad = [0u8; 128];
    pad[0] = 0x80;
    for i in 0..8 {
        pad[1 + k + i] = (bit_len >> (56 - i * 8)) as u8;
    }
    feed(&mut h, &pad[..total_pad], &mut buf, &mut fill);
    debug_assert!(fill == 0);
    for i in 0..8 {
        out[i * 4] = (h[i] >> 24) as u8;
        out[i * 4 + 1] = (h[i] >> 16) as u8;
        out[i * 4 + 2] = (h[i] >> 8) as u8;
        out[i * 4 + 3] = h[i] as u8;
    }
}

/// PBKDF2-HMAC-SHA256, вывод до 32 байт (dkLen ≤ 32 — хватает на AES-128).
pub fn pbkdf2(password: &[u8], salt: &[u8], iters: u32, out: &mut [u8]) {
    let dklen = out.len().min(32);
    // U1 = HMAC(pw, salt || 0x00000001)
    let mut msg = [0u8; 64];
    let sl = salt.len().min(60);
    msg[..sl].copy_from_slice(&salt[..sl]);
    msg[sl] = 0; msg[sl + 1] = 0; msg[sl + 2] = 0; msg[sl + 3] = 1;
    let mlen = sl + 4;
    let mut u = [0u8; 32];
    hmac_sha256(password, &msg[..mlen], &mut u);
    let mut acc = u;
    let mut i = 1u32;
    while i < iters {
        let mut v = [0u8; 32];
        hmac_sha256(password, &u, &mut v);
        u = v;
        for k in 0..32 { acc[k] ^= u[k]; }
        i += 1;
    }
    out[..dklen].copy_from_slice(&acc[..dklen]);
}

// ── AES-128 ─────────────────────────────────────────────────────────

const AES_SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const AES_RSBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

const AES_RCON: [u8; 10] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36];

fn aes_key_expand(key: &[u8; 16], rk: &mut [u8; 176]) {
    rk[..16].copy_from_slice(key);
    let mut i = 16usize;
    let mut rci = 0usize;
    while i < 176 {
        let mut t = [rk[i - 4], rk[i - 3], rk[i - 2], rk[i - 1]];
        if i % 16 == 0 {
            // RotWord + SubWord + Rcon
            let u = t[0];
            t[0] = AES_SBOX[t[1] as usize] ^ AES_RCON[rci];
            t[1] = AES_SBOX[t[2] as usize];
            t[2] = AES_SBOX[t[3] as usize];
            t[3] = AES_SBOX[u as usize];
            rci += 1;
        }
        for k in 0..4 {
            rk[i] = rk[i - 16] ^ t[k];
            i += 1;
        }
    }
}

fn xtime(x: u8) -> u8 {
    if x & 0x80 != 0 { (x << 1) ^ 0x1b } else { x << 1 }
}
fn mul2(x: u8) -> u8 { xtime(x) }
fn mul3(x: u8) -> u8 { xtime(x) ^ x }
fn mul9(x: u8) -> u8 { xtime(xtime(xtime(x))) ^ x }
fn mul11(x: u8) -> u8 { xtime(xtime(xtime(x)) ^ x) ^ x }
fn mul13(x: u8) -> u8 { xtime(xtime(xtime(x) ^ x)) ^ x }
fn mul14(x: u8) -> u8 { xtime(xtime(xtime(x) ^ x) ^ x) }

/// Один блок AES-128 encrypt. Состояние column-major как в FIPS.
pub fn aes128_encrypt_block(key: &[u8; 16], pt: &[u8; 16], ct: &mut [u8; 16]) {
    let mut rk = [0u8; 176];
    aes_key_expand(key, &mut rk);
    let mut s = *pt;
    for i in 0..16 { s[i] ^= rk[i]; }
    // Ровно 10 раундов: 1-9 полные, 10-й без MixColumns (FIPS 197 §5.1)
    for round in 1..11 {
        for i in 0..16 { s[i] = AES_SBOX[s[i] as usize]; }
        // ShiftRows
        let mut t = s;
        t[1] = s[5]; t[5] = s[9]; t[9] = s[13]; t[13] = s[1];
        t[2] = s[10]; t[10] = s[2]; t[6] = s[14]; t[14] = s[6];
        t[3] = s[15]; t[15] = s[11]; t[11] = s[7]; t[7] = s[3];
        s = t;
        if round != 10 {
            for c in 0..4 {
                let (a0, a1, a2, a3) = (s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]);
                s[4 * c] = mul2(a0) ^ mul3(a1) ^ a2 ^ a3;
                s[4 * c + 1] = a0 ^ mul2(a1) ^ mul3(a2) ^ a3;
                s[4 * c + 2] = a0 ^ a1 ^ mul2(a2) ^ mul3(a3);
                s[4 * c + 3] = mul3(a0) ^ a1 ^ a2 ^ mul2(a3);
            }
        }
        let off = round * 16;
        for i in 0..16 { s[i] ^= rk[off + i]; }
    }
    *ct = s;
}

/// Один блок AES-128 decrypt (Inverse Cipher).
pub fn aes128_decrypt_block(key: &[u8; 16], ct: &[u8; 16], pt: &mut [u8; 16]) {
    let mut rk = [0u8; 176];
    aes_key_expand(key, &mut rk);
    let mut s = *ct;
    for i in 0..16 { s[i] ^= rk[160 + i]; }
    for round in (0..10).rev() {
        // InvShiftRows
        let mut t = s;
        t[1] = s[13]; t[13] = s[9]; t[9] = s[5]; t[5] = s[1];
        t[2] = s[10]; t[10] = s[2]; t[6] = s[14]; t[14] = s[6];
        t[3] = s[7]; t[7] = s[11]; t[11] = s[15]; t[15] = s[3];
        s = t;
        for i in 0..16 { s[i] = AES_RSBOX[s[i] as usize]; }
        let off = round * 16;
        for i in 0..16 { s[i] ^= rk[off + i]; }
        if round != 0 {
            for c in 0..4 {
                let (a0, a1, a2, a3) = (s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]);
                s[4 * c] = mul14(a0) ^ mul11(a1) ^ mul13(a2) ^ mul9(a3);
                s[4 * c + 1] = mul9(a0) ^ mul14(a1) ^ mul11(a2) ^ mul13(a3);
                s[4 * c + 2] = mul13(a0) ^ mul9(a1) ^ mul14(a2) ^ mul11(a3);
                s[4 * c + 3] = mul11(a0) ^ mul13(a1) ^ mul9(a2) ^ mul14(a3);
            }
        }
    }
    *pt = s;
}

// ── CBC + PKCS#7 ────────────────────────────────────────────────────

/// Шифровать data (len ≤ out.len()-16) в out, возвращает длину с паддингом.
pub fn aes_cbc_encrypt(key: &[u8; 16], iv: &[u8; 16], data: &[u8], out: &mut [u8]) -> Option<usize> {
    let pad = 16 - (data.len() % 16);
    let total = data.len() + pad;
    if total > out.len() || total == 0 { return None; }
    let mut prev = *iv;
    let mut o = 0;
    let mut i = 0;
    while i < total {
        let mut blk = [0u8; 16];
        for k in 0..16 {
            let b = if i + k < data.len() { data[i + k] } else { pad as u8 };
            blk[k] = b ^ prev[k];
        }
        let mut ct = [0u8; 16];
        aes128_encrypt_block(key, &blk, &mut ct);
        out[o..o + 16].copy_from_slice(&ct);
        prev = ct;
        o += 16;
        i += 16;
    }
    Some(total)
}

/// Расшифровать CBC, снять PKCS#7. Возвращает длину открытого текста.
pub fn aes_cbc_decrypt(key: &[u8; 16], iv: &[u8; 16], data: &[u8], out: &mut [u8]) -> Option<usize> {
    if data.is_empty() || data.len() % 16 != 0 || data.len() > out.len() + 16 { return None; }
    let mut prev = *iv;
    let mut o = 0;
    let mut i = 0;
    while i < data.len() {
        let mut cb = [0u8; 16];
        cb.copy_from_slice(&data[i..i + 16]);
        let mut pt = [0u8; 16];
        aes128_decrypt_block(key, &cb, &mut pt);
        for k in 0..16 { pt[k] ^= prev[k]; }
        prev = cb;
        if o + 16 <= out.len() {
            out[o..o + 16].copy_from_slice(&pt);
        } else {
            return None;
        }
        o += 16;
        i += 16;
    }
    // PKCS#7 проверка
    let pad = out[o - 1] as usize;
    if pad == 0 || pad > 16 { return None; }
    for k in 0..pad {
        if out[o - 1 - k] as usize != pad { return None; }
    }
    Some(o - pad)
}

// ── KAT (Known Answer Tests) ────────────────────────────────────────

fn hex_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    let mut d = 0u8;
    for i in 0..a.len() { d |= a[i] ^ b[i]; }
    d == 0
}

/// 0 = все тесты прошли. Биты ошибок: 1=SHA256, 2=AES-enc, 4=AES-dec, 8=CBC, 16=HMAC, 32=PBKDF2.
pub fn self_test() -> u32 {
    let mut err = 0u32;
    // SHA-256("abc") — FIPS 180-4
    {
        let mut d = [0u8; 32];
        sha256(b"abc", &mut d);
        let exp: [u8; 32] = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde,
            0x5d, 0xae, 0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
            0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
        ];
        if !hex_eq(&d, &exp) { err |= 1; }
    }
    // AES-128 NIST FIPS-197 Appendix B
    {
        let key: [u8; 16] = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
                             0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f];
        let pt: [u8; 16] = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
                            0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let exp: [u8; 16] = [0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30,
                             0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4, 0xc5, 0x5a];
        let mut ct = [0u8; 16];
        aes128_encrypt_block(&key, &pt, &mut ct);
        if !hex_eq(&ct, &exp) { err |= 2; }
        let mut back = [0u8; 16];
        aes128_decrypt_block(&key, &ct, &mut back);
        if !hex_eq(&back, &pt) { err |= 4; }
    }
    // CBC round-trip
    {
        let key = [0x2bu8; 16];
        let iv = [0x0fu8; 16];
        let msg = b"DBSos wifi test message!";
        let mut enc = [0u8; 48];
        let mut dec = [0u8; 48];
        match aes_cbc_encrypt(&key, &iv, msg, &mut enc) {
            Some(el) => match aes_cbc_decrypt(&key, &iv, &enc[..el], &mut dec) {
                Some(dl) => {
                    if dl != msg.len() || !hex_eq(&dec[..dl], msg) { err |= 8; }
                }
                None => err |= 8,
            },
            None => err |= 8,
        }
    }
    // HMAC-SHA256 RFC 4231 Test 1: key=20x0b, data="Hi There"
    // (сверено локально с Python hashlib: b0344c61...2e32cff7)
    {
        let key = [0x0bu8; 20];
        let mut m = [0u8; 32];
        hmac_sha256(&key, b"Hi There", &mut m);
        let exp: [u8; 32] = [
            0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf,
            0xce, 0xaf, 0x0b, 0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83,
            0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c, 0x2e, 0x32, 0xcf, 0xf7,
        ];
        if !hex_eq(&m, &exp) { err |= 16; }
    }
    // PBKDF2-HMAC-SHA256 c=1: password="password", salt="salt" (RFC 7914 §A.1,
    // сверено локально с Python hashlib — полный 32-байтный вектор)
    {
        let mut dk = [0u8; 32];
        pbkdf2(b"password", b"salt", 1, &mut dk);
        let exp: [u8; 32] = [
            0x12, 0x0f, 0xb6, 0xcf, 0xfc, 0xf8, 0xb3, 0x2c, 0x43, 0xe7, 0x22,
            0x52, 0x56, 0xc4, 0xf8, 0x37, 0xa8, 0x65, 0x48, 0xc9, 0x2c, 0xcc,
            0x35, 0x48, 0x08, 0x05, 0x98, 0x7c, 0xb7, 0x0b, 0xe1, 0x7b,
        ];
        if !hex_eq(&dk, &exp) { err |= 32; }
    }
    err
}
