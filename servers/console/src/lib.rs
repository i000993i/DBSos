#![no_std]

/// Offset from code page start where the cap_idx is stored as u64 little-endian.
pub const CAP_IDX_CODE_OFFSET: usize = 35;

/// Test function to verify library linking from kernel.
pub unsafe fn test_lib(dst: *mut u8) -> u64 {
    dst.add(0).write(0x90);
    dst.add(1).write(0x90);
    dst.add(2).write(0x90);
    dst.add(3).write(0x90);
    42
}

/// Emit `mov reg64, imm64` (REX.W + B8+reg).
unsafe fn emit_mov_imm64(dst: *mut u8, off: usize, reg: u8, val: u64) -> usize {
    let rex = 0x48 | ((reg >> 3) & 1);
    dst.add(off).write(rex);
    dst.add(off + 1).write(0xB8 | (reg & 7));
    for j in 0..8 { dst.add(off + 2 + j).write((val >> (j * 8)) as u8); }
    10
}

/// Emit `mov reg32, imm32` (REX.W + C7 C0+reg).
unsafe fn emit_mov_imm32(dst: *mut u8, off: usize, reg: u8, val: u32) -> usize {
    let rex = 0x48 | ((reg >> 3) & 1);
    dst.add(off).write(rex);
    dst.add(off + 1).write(0xC7);
    dst.add(off + 2).write(0xC0 | (reg & 7));
    for j in 0..4 { dst.add(off + 3 + j).write((val >> (j * 8)) as u8); }
    7
}

/// Emit `syscall` (0F 05).
unsafe fn emit_syscall(dst: *mut u8, off: usize) -> usize {
    dst.add(off).write(0x0F);
    dst.add(off + 1).write(0x05);
    2
}

/// Write userspace console server code into a 4KB page at `dst`.
///
/// The cap_idx is embedded at `dst + CAP_IDX_CODE_OFFSET` (8 bytes, u64 LE).
/// The kernel writes the real cap_idx there after spawning the task.
pub unsafe fn write_user_code_console(dst: *mut u8) -> usize {
    let entry: u64 = 0x100003000;
    let str_off: usize = 0x100;
    let str_virt: u64 = entry + str_off as u64;

    let mut off: usize = 0;

    // Startup message string data
    let startup = b"[CONSOLE] server ready\r\n";
    for (i, &b) in startup.iter().enumerate() {
        dst.add(str_off + i).write(b);
    }

    // mov rdx, str_virt
    off += emit_mov_imm64(dst, off, 2, str_virt);

    // mov r8d, len
    off += emit_mov_imm32(dst, off, 8, startup.len() as u32);

    // mov eax, 20   (SYS_LOG_WRITE)
    off += emit_mov_imm32(dst, off, 0, 20);

    // syscall
    off += emit_syscall(dst, off);

    // Loop start
    // sub rsp, 80
    dst.add(off).write(0x48); off += 1;
    dst.add(off).write(0x81); off += 1;
    dst.add(off).write(0xEC); off += 1;
    for i in 0..4 { dst.add(off + i).write((80u32 >> (i * 8)) as u8); }
    off += 4;

    // mov rdx, cap_idx   (SYS_IPC_RECV arg1)
    off += emit_mov_imm64(dst, off, 2, 0); // placeholder, patched later

    // mov r8, rsp        (arg2 = buffer)
    dst.add(off).write(0x49); off += 1;
    dst.add(off).write(0x89); off += 1;
    dst.add(off).write(0xE0); off += 1;

    // mov r9, 0          (arg3)
    off += emit_mov_imm64(dst, off, 9, 0);

    // mov eax, 12        (SYS_IPC_RECV)
    off += emit_mov_imm32(dst, off, 0, 12);

    // syscall
    off += emit_syscall(dst, off);

    // mov rdx, rsp       (buffer for SYS_LOG_WRITE)
    dst.add(off).write(0x48); off += 1;
    dst.add(off).write(0x89); off += 1;
    dst.add(off).write(0xE2); off += 1;

    // mov r8d, 64        (length)
    off += emit_mov_imm32(dst, off, 8, 64);

    // mov eax, 20        (SYS_LOG_WRITE)
    off += emit_mov_imm32(dst, off, 0, 20);

    // syscall
    off += emit_syscall(dst, off);

    // add rsp, 80
    dst.add(off).write(0x48); off += 1;
    dst.add(off).write(0x81); off += 1;
    dst.add(off).write(0xC4); off += 1;
    for i in 0..4 { dst.add(off + i).write((80u32 >> (i * 8)) as u8); }
    off += 4;

    // jmp loop (short)
    let disp = (26i64 - off as i64 - 2) as i8;
    dst.add(off).write(0xEB); off += 1;
    dst.add(off).write(disp as u8); off += 1;

    off
}
