// x86-64 code emission helpers for ring-3 test tasks

pub unsafe fn emit_mov_imm64(dst: *mut u8, off: usize, reg: u8, val: u64) -> usize {
    if val as i64 as u64 == val && (val & 0xFFFFFFFF80000000 == 0 || val & 0xFFFFFFFF80000000 == 0xFFFFFFFF80000000) {
        let rex = 0x48 | ((reg >> 3) & 1);
        dst.add(off).write(rex); dst.add(off+1).write(0xC7);
        dst.add(off+2).write(0xC0 | (reg & 7));
        for j in 0..4 { dst.add(off+3+j).write((val >> (j*8)) as u8); }
        7
    } else {
        let rex = 0x48 | ((reg >> 3) & 1);
        dst.add(off).write(rex); dst.add(off+1).write(0xB8 | (reg & 7));
        for j in 0..8 { dst.add(off+2+j).write((val >> (j*8)) as u8); }
        10
    }
}

pub unsafe fn emit_mov_imm32(dst: *mut u8, off: usize, reg: u8, val: u32) -> usize {
    let rex = 0x48 | ((reg >> 3) & 1);
    dst.add(off).write(rex); dst.add(off+1).write(0xC7);
    dst.add(off+2).write(0xC0 | (reg & 7));
    for j in 0..4 { dst.add(off+3+j).write((val >> (j*8)) as u8); }
    7
}

pub unsafe fn emit_syscall(dst: *mut u8, off: usize) -> usize {
    dst.add(off).write(0x0F); dst.add(off+1).write(0x05); 2
}

pub unsafe fn emit_mov_r64(dst: *mut u8, off: usize, d: u8, s: u8) -> usize {
    let rex = 0x48
        | (if s > 7 { 1 << 2 } else { 0 })
        | (if d > 7 { 1 << 0 } else { 0 });
    dst.add(off).write(rex);
    dst.add(off+1).write(0x89);
    dst.add(off+2).write(0xC0 | ((s & 7) << 3) | (d & 7));
    3
}

pub unsafe fn emit_mmio_read32(dst: *mut u8, off: usize, dst_reg: u8, base: u8, disp: u32) -> usize {
    let mut p = off;
    let rex = (if dst_reg > 7 { 1 << 2 } else { 0 }) | (if base > 7 { 1 << 0 } else { 0 });
    if rex != 0 { dst.add(p).write(0x40 | rex); p += 1; }
    dst.add(p).write(0x8B); p += 1;
    dst.add(p).write(0x80 | ((dst_reg & 7) << 3) | (base & 7)); p += 1;
    for j in 0..4 { dst.add(p+j).write((disp >> (j*8)) as u8); } p += 4;
    p - off
}

pub unsafe fn emit_print(dst: *mut u8, off: usize, str_addr: u64, len: u32) -> usize {
    let mut p = off;
    p += emit_mov_imm64(dst, p, 2, str_addr);
    p += emit_mov_imm32(dst, p, 8, len);
    p += emit_mov_imm32(dst, p, 0, 20);
    p += emit_syscall(dst, p);
    p - off
}

pub unsafe fn emit_syscall3(dst: *mut u8, off: usize, num: u64, arg1: u64, arg2: u64, arg3: u64) -> usize {
    let mut p = off;
    p += emit_mov_imm64(dst, p, 8, arg2);
    p += emit_mov_imm64(dst, p, 9, arg3);
    p += emit_mov_imm64(dst, p, 2, arg1);
    p += emit_mov_imm64(dst, p, 0, num);
    p += emit_syscall(dst, p);
    p - off
}
