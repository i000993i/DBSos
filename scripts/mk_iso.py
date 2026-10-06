#!/usr/bin/env python3
"""Build a better DBSos ISO: hybrid El Torito EFI + GPT, real ISO9660 tree.

Improvements over v1:
  - Hybrid disk: protective MBR + GPT in the ISO system area, so the same
    image boots both as CD-ROM (El Torito) and as USB/HDD (dd to stick).
  - Real ISO9660 tree: PVD, L/M path tables, root dir with README.TXT.
  - Verified layout, asserts on every structure size.

Layout (2048-byte CD sectors):
  0..15      system area: LBA0 protective MBR, LBA1 GPT header (512-B units!),
             LBA2..33 GPT entries, rest zeros
  16         PVD (root at 22)
  17         Boot Record (El Torito, catalog at 19)
  18         Terminator
  19         Boot Catalog (EFI entry -> IMAGE_SECTOR)
  20         L path table
  21         M path table
  22         Root dir ('.', '..', 'README.TXT;1')
  23         README.TXT content
  24..       EFI boot image = esp.img verbatim
  tail       backup GPT entries + backup GPT header (512-B units)

Boot CD:  qemu-system-x86_64 -cdrom dbsos.iso -boot d
Boot USB: dd if=dbsos.iso of=/dev/sdX bs=4M status=progress && boot via UEFI
"""
import os
import struct
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
ESP_IMG = os.path.join(HERE, "..", "esp.img")
ISO = os.path.join(HERE, "..", "dbsos.iso")
SECTOR = 2048

CATALOG_SECTOR = 19
PATH_L_SECTOR = 20
PATH_M_SECTOR = 21
ROOT_SECTOR = 22
README_SECTOR = 23
IMAGE_SECTOR = 24  # CD sectors (2048 B)

README_TEXT = (
    b"DBSos boot media\r\n"
    b"================\r\n"
    b"CD boot  : El Torito EFI entry (platform 0xEF).\r\n"
    b"USB boot : hybrid GPT, partition 1 = EFI System.\r\n"
    b"QEMU     : qemu-system-x86_64 -cdrom dbsos.iso -boot d\r\n"
    b"VirtualBox: attach as optical drive, EFI enabled.\r\n"
)

EFI_TYPE_GUID = bytes.fromhex("C12A7328F81F11D2BA4B00A0C93EC93B")
DISK_GUID = bytes.fromhex("00112233445566778899AABBCCDDEEFF")
PART_GUID = bytes.fromhex("FFEEDDCCBBAA99887766554433221100")


def dir_record(extent, size, flags, name):
    nl = len(name)
    rec_len = 33 + nl + (1 if nl % 2 == 0 else 0)
    r = bytearray(rec_len)
    r[0] = rec_len
    r[1] = 0
    struct.pack_into("<I", r, 2, extent)
    struct.pack_into(">I", r, 6, extent)
    struct.pack_into("<I", r, 10, size)
    struct.pack_into(">I", r, 14, size)
    r[25] = flags
    r[32] = nl
    r[33:33 + nl] = name
    return bytes(r)


def pvd(vol_sectors):
    s = bytearray(SECTOR)
    s[0] = 1
    s[1:6] = b"CD001"
    s[6] = 1
    s[8:40] = b"DBSOS_ISO".ljust(32, b" ")
    s[80:84] = struct.pack("<I", vol_sectors)
    s[84:88] = struct.pack(">I", vol_sectors)
    root = dir_record(ROOT_SECTOR, SECTOR, 2, b"\x00")
    s[156:156 + len(root)] = root
    assert len(s) == SECTOR
    return bytes(s)


def boot_record():
    s = bytearray(SECTOR)
    s[0] = 0
    s[1:6] = b"CD001"
    s[6] = 1
    s[7:30] = b"EL TORITO SPECIFICATION"
    s[71:75] = struct.pack("<I", CATALOG_SECTOR)
    assert len(s) == SECTOR
    return bytes(s)


def terminator():
    s = bytearray(SECTOR)
    s[0] = 255
    s[1:6] = b"CD001"
    s[6] = 1
    assert len(s) == SECTOR
    return bytes(s)


def catalog(image_sectors):
    s = bytearray(SECTOR)
    s[0] = 0x01
    s[1] = 0xEF
    s[4:27] = b"DBSos".ljust(23, b"\x00")
    s[30] = 0x55
    s[31] = 0xAA
    total = 0
    for i in range(0, 32, 2):
        total = (total + struct.unpack_from("<H", s, i)[0]) & 0xFFFF
    struct.pack_into("<H", s, 28, (-total) & 0xFFFF)
    s[32] = 0x88
    s[33] = 0x00
    s[34:36] = struct.pack("<H", 0)
    s[36] = 0xEF
    s[37] = 0x00
    s[38:40] = struct.pack("<H", image_sectors)
    s[40:44] = struct.pack("<I", IMAGE_SECTOR)
    assert len(s) == SECTOR
    return bytes(s)


def path_table(be):
    s = bytearray(SECTOR)
    rec = bytearray(10)
    rec[0] = 1
    rec[1] = 0
    if be:
        struct.pack_into(">I", rec, 2, ROOT_SECTOR)
        struct.pack_into(">H", rec, 6, 1)
    else:
        struct.pack_into("<I", rec, 2, ROOT_SECTOR)
        struct.pack_into("<H", rec, 6, 1)
    rec[8] = 0
    rec[9] = 0
    s[0:10] = rec
    assert len(s) == SECTOR
    return bytes(s)


def root_dir(readme_sectors):
    s = bytearray(SECTOR)
    o = 0
    for name, extent, size in (
        (b"\x00", ROOT_SECTOR, SECTOR),
        (b"\x01", ROOT_SECTOR, SECTOR),
        (b"README.TXT;1", README_SECTOR, readme_sectors * SECTOR),
    ):
        r = dir_record(extent, size, 2 if name in (b"\x00", b"\x01") else 0, name)
        s[o:o + len(r)] = r
        o += len(r)
    return bytes(s)


def gpt_header(my_lba, alt_lba, first_use, last_use, entries_lba, crc_entries):
    """512-байтный сектор с 92-байтным GPT-заголовком. Смещения — как в
    проверенном mk_esp.py (им грузится OVMF)."""
    h = bytearray(512)
    h[0:8] = b"EFI PART"
    h[8:12] = struct.pack("<I", 0x00010000)
    h[12:16] = struct.pack("<I", 92)
    h[20:24] = struct.pack("<I", 0)
    h[24:32] = struct.pack("<Q", my_lba)
    h[32:40] = struct.pack("<Q", alt_lba)
    h[40:48] = struct.pack("<Q", first_use)
    h[48:56] = struct.pack("<Q", last_use)
    h[56:72] = DISK_GUID
    h[72:80] = struct.pack("<Q", entries_lba)
    h[80:84] = struct.pack("<I", 128)
    h[84:88] = struct.pack("<I", 128)
    h[88:92] = struct.pack("<I", crc_entries)
    crc = zlib.crc32(bytes(h[0:92])) & 0xFFFFFFFF
    h[16:20] = struct.pack("<I", crc)
    return bytes(h)


def build_gpt(total_lba512, part_start512, part_end512):
    """MBR + primary hdr/entries + backup entries/hdr. Всё в 512-единицах."""
    mbr = bytearray(512)
    mbr[446] = 0x00
    mbr[450] = 0xEE
    struct.pack_into("<I", mbr, 454, 1)
    struct.pack_into("<I", mbr, 458, min(total_lba512 - 1, 0xFFFFFFFF))
    mbr[510] = 0x55
    mbr[511] = 0xAA
    entries = bytearray(128 * 128)
    entries[0:16] = EFI_TYPE_GUID
    entries[16:32] = PART_GUID
    struct.pack_into("<Q", entries, 32, part_start512)
    struct.pack_into("<Q", entries, 40, part_end512)
    struct.pack_into("<Q", entries, 48, 0)
    name = "EFI System Partition".encode("utf-16-le").ljust(72, b"\x00")
    entries[56:128] = name[:72]
    crc_e = zlib.crc32(bytes(entries)) & 0xFFFFFFFF
    first_use = 34
    last_use = total_lba512 - 34 - 1
    primary = gpt_header(1, total_lba512 - 1, first_use, last_use, 2, crc_e)
    backup = gpt_header(total_lba512 - 1, 1, first_use, last_use,
                        total_lba512 - 33, crc_e)
    return bytes(mbr), primary, bytes(entries), backup, bytes(entries)


def main():
    if not os.path.exists(ESP_IMG):
        print(f"ERROR: {ESP_IMG} not found — run scripts/mk_esp.py first")
        sys.exit(1)
    with open(ESP_IMG, "rb") as f:
        esp = f.read()
    if len(esp) % SECTOR:
        esp += b"\x00" * (SECTOR - len(esp) % SECTOR)
    image_sectors = len(esp) // SECTOR
    if image_sectors > 0xFFFF:
        print("ERROR: ESP image too big for El Torito sector count")
        sys.exit(1)
    vol_sectors = IMAGE_SECTOR + image_sectors

    pv = bytearray(pvd(vol_sectors))
    struct.pack_into("<I", pv, 132, 10)
    struct.pack_into(">I", pv, 140, 10)
    struct.pack_into("<I", pv, 148, PATH_L_SECTOR)
    struct.pack_into(">I", pv, 152, PATH_M_SECTOR)
    assert len(pv) == SECTOR

    ltables, mtables = path_table(False), path_table(True)
    assert len(ltables) == SECTOR and len(mtables) == SECTOR

    # README контент (1 сектор)
    readme = README_TEXT + b"\x00" * (SECTOR - len(README_TEXT) % SECTOR)
    readme = readme[:SECTOR]
    rdir = root_dir(1)

    # Гибридная GPT (512-байтные LBA поверх ISO-образа):
    # ESP начинается на CD-секторе IMAGE_SECTOR -> LBA512 = IMAGE_SECTOR*4.
    part_start512 = IMAGE_SECTOR * 4
    esp_sectors512 = image_sectors * 4
    # хвост: backup entries (32 LBA) + backup header (1 LBA) + 1 запас
    total_lba512 = part_start512 + esp_sectors512 + 34
    part_end512 = part_start512 + esp_sectors512 - 1
    assert part_end512 <= total_lba512 - 34 - 1, "partition overlaps backup GPT"
    mbr, phdr, pent, bhdr, bent = build_gpt(total_lba512, part_start512,
                                            part_end512)

    with open(ISO, "wb") as f:
        # system area 0..15 (CD-сектора): MBR/GPT поверх нулей
        sysarea = bytearray(16 * SECTOR)
        sysarea[0:512] = mbr
        sysarea[512:1024] = phdr
        sysarea[1024:1024 + len(pent)] = pent
        f.write(bytes(sysarea))
        f.write(bytes(pv))
        f.write(boot_record())
        f.write(terminator())
        f.write(catalog(image_sectors))
        f.write(ltables)
        f.write(mtables)
        f.write(rdir)
        f.write(readme)
        f.write(esp)
        # backup GPT в хвост (512-единицы)
        f.seek(total_lba512 * 512 - 33 * 512)
        f.write(bent)
        f.write(bhdr)
    size = os.path.getsize(ISO)
    assert size == total_lba512 * 512, (size, total_lba512 * 512)
    print(f"OK: {ISO} ({vol_sectors} CD sectors + hybrid GPT, {size} bytes)")
    print("Boot CD : qemu-system-x86_64 -cdrom dbsos.iso -boot d")
    print("Boot USB: dd if=dbsos.iso of=/dev/sdX bs=4M status=progress")


if __name__ == "__main__":
    main()
