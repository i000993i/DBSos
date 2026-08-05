#!/usr/bin/env python3
"""Recreate the DBSos 64MB FAT16 NVMe disk image (mirrors bootstrap.ps1 geometry).

Layout (LBA):
  0                MBR (partition 1 = FAT16 type 0x06)
  PART_START=2048  FAT16 BPB (VBR)
  +RESERVED        FAT1
  +RESERVED+FAT_SEC  FAT2
  root_lba         root directory (32 sectors)
  DATA_START(@ cluster 2) data area

Geometry: BpS 512, SpC 4, Reserved 1, FATs 2, RootEnt 512 (=32 sec), FatSectors 126.
"""
import os, struct, sys

IMG = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "nvme_disk.img"))
SIZE = 64 * 1024 * 1024

BPS=512; SPC=4; RESERVED=1; FATS=2; ROOT_ENT=512
ROOT_SEC = ROOT_ENT*32//BPS          # 32
FAT_SEC = 126
MEDIA = 0xF8
PART_START = 2048
PART_SECTORS = 129024
DATA_START = PART_START + RESERVED + FATS*FAT_SEC + ROOT_SEC
DATA_CLUSTERS = (PART_SECTORS - RESERVED - FATS*FAT_SEC - ROOT_SEC)//SPC

def le16(v): return struct.pack('<H', v)
def le32(v): return struct.pack('<I', v)

def dir_entry(f, name8, ext3, attr, cluster, size):
    f.write(name8.encode().ljust(8, b' '))
    f.write(ext3.encode().ljust(3, b' '))
    f.write(bytes([attr]))           # 11 attr
    f.write(b'\x00')                 # 12 NT rsvd
    f.write(b'\x00')                 # 13 ctime tenths
    f.write(b'\x00\x00\x00\x00\x00\x00')  # 14-19 ctime/cdate/adate
    f.write(le16(0))                 # 20-21 cluster high
    f.write(b'\x00\x00\x00\x00')     # 22-25 mtime/mdate
    f.write(le16(cluster))           # 26-27 cluster low
    f.write(le32(size))              # 28-31 size

def cluster_lba(c):
    return DATA_START + (c-2)*SPC

def minimal_elf():
    code = bytes([
        0x48,0xc7,0xc0,0x14,0x00,0x00,0x00,  # mov rax, 20 (SYS_LOG_WRITE)
        0x48,0x8d,0x15,0x18,0x00,0x00,0x00,  # lea rdx,[rip+24] -> msg
        0x49,0xc7,0xc0,0x16,0x00,0x00,0x00,  # mov r8, 22 (len)
        0x0f,0x05,                           # syscall
        0x48,0xc7,0xc0,0x00,0x00,0x00,0x00,  # mov rax,0 (exit)
        0x48,0x31,0xff,                       # xor rdi,rdi
        0x48,0x31,0xd2,                       # xor rdx,rdx
        0x0f,0x05,                            # syscall
    ])
    msg = b'Hello from user ELF!\r\n'
    off = 0x78
    file_sz = off + len(code) + len(msg)
    elf = bytearray(file_sz)
    hdr = bytes([
        0x7f,0x45,0x4c,0x46,0x02,0x01,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0x02,0x00,0x3e,0x00,0x01,0x00,0x00,0x00,
        0x78,0x00,0x40,0x00,0x00,0x00,0x00,0x00,
        0x40,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0x00,0x00,0x00,0x00,0x40,0x00,0x38,0x00,
        0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
    ])
    elf[0:64] = hdr
    struct.pack_into('<IIQQQQQQ', elf, 64, 1, 5, 0, 0x400000, 0x400000, file_sz, file_sz, 0x1000)
    elf[off:off+len(code)] = code
    elf[off+len(code):off+len(code)+len(msg)] = msg
    return bytes(elf)

def write_bpb(f):
    f.seek(PART_START*BPS)
    f.write(b'\xEB\x3C\x90'); f.write(b'MSDOS5.0')
    f.write(le16(BPS)); f.write(bytes([SPC]))
    f.write(le16(RESERVED)); f.write(bytes([FATS]))
    f.write(le16(ROOT_ENT)); f.write(le16(0))
    f.write(bytes([MEDIA])); f.write(le16(FAT_SEC))
    f.write(le16(32)); f.write(le16(64))
    f.write(le32(0)); f.write(le32(PART_SECTORS))
    f.write(bytes([0x80,0x00,0x29])); f.write(le32(0x12345678))
    f.write(b'NVME DISK  '); f.write(b'FAT16   ')
    f.seek(PART_START*BPS+0x1FE); f.write(b'\x55\xAA')

def write_fat(f, lba):
    f.seek(lba*BPS)
    f.write(bytes([MEDIA,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF,0xFF]))

def main():
    elf = minimal_elf()
    with open(IMG, 'wb') as f:
        f.truncate(SIZE)
        # MBR
        f.seek(0x1BE)
        f.write(bytes([0x00,0x00,0x00,0x01]))   # status+CHS start
        f.write(bytes([0x06]))                  # FAT16
        f.write(bytes([0x00,0x00,0x00]))         # CHS end
        f.write(le32(PART_START)); f.write(le32(PART_SECTORS))
        f.seek(0x1FE); f.write(b'\x55\xAA')
        write_bpb(f)
        write_fat(f, PART_START+RESERVED)
        write_fat(f, PART_START+RESERVED+FAT_SEC)

        # root directory
        root_lba = PART_START + RESERVED + FATS*FAT_SEC
        f.seek(root_lba*BPS)
        f.write(b'NVME DISK  '); f.write(bytes([0x08])); f.write(b'\x00'*20)   # volume label (32 bytes)
        dir_entry(f, 'HELLO', 'TXT', 0x20, 3, 13)
        dir_entry(f, 'TEST', '   ', 0x10, 4, 0)
        f.write(b'\x00')

        # HELLO.TXT content at cluster 3
        f.seek(cluster_lba(3)*BPS); f.write(b'Hello NVMe!\r\n')

        # /test dir cluster 4
        f.seek(cluster_lba(4)*BPS)
        dir_entry(f, '.', '   ', 0x10, 4, 0)
        dir_entry(f, '..', '   ', 0x10, 0, 0)
        dir_entry(f, 'HELLO  ', 'ELF', 0x20, 5, len(elf))
        dir_entry(f, 'PARENT ', 'ELF', 0x20, 6, len(elf))
        dir_entry(f, 'CHILD  ', 'ELF', 0x20, 7, len(elf))
        dir_entry(f, 'FORKT  ', 'ELF', 0x20, 8, len(elf))
        f.write(b'\x00')

        # ELF contents at clusters 5..8
        for cl in (5,6,7,8):
            f.seek(cluster_lba(cl)*BPS); f.write(elf)

        # mark clusters 3..8 EOC in FAT1 and FAT2
        for base in (PART_START+RESERVED, PART_START+RESERVED+FAT_SEC):
            for cl in (3,4,5,6,7,8):
                f.seek(base*BPS + cl*2); f.write(b'\xFF\xFF')

    print(f"OK: {IMG}")
    print(f"DATA_START_LBA={DATA_START} DATA_CLUSTERS={DATA_CLUSTERS}")
    print(f"root_lba={PART_START+RESERVED+FATS*FAT_SEC}")

if __name__ == '__main__':
    main()