#!/usr/bin/env python3
"""Build a GPT ESP disk image from the esp/ tree (no external tools required).

OVMF refuses to offer the raw `fat:rw:` VVFAT on SATA/AHCI as a boot device and
falls through to PXE. A real GPT disk with an EFI System Partition (FAT16) is
reliably enumerated and booted.

Layout (512-byte sectors):
  LBA 0        protective MBR (partition type 0xEE)
  LBA 1        primary GPT header
  LBA 2..33    GPT partition entries (128 x 128 B)
  LBA 2048..   EFI System Partition (FAT16 filesystem, payload from esp/)
  LBA -33..    backup GPT partition entries
  LBA -1       backup GPT header
"""
import os, struct, sys, zlib

SRC = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "esp"))
IMG = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "esp.img"))
SIZE = 16 * 1024 * 1024            # 16 MB
LBA = 512

PART_START = 2048
PART_ENTRY_LBA = 2
PART_ENTRIES = 128
PART_ENTRY_SZ = 128
LAST_LBA = SIZE // LBA - 1         # backup GPT header
FIRST_USE = PART_START
LAST_USE = LAST_LBA - 33           # reserve 32 entries + 1 header
PART_LAST = LAST_USE               # partition spans PART_START..LAST_USE
PART_SECTORS = PART_LAST - PART_START + 1

ESP_TYPE_GUID = bytes.fromhex("C12A7328F81F11D2BA4B00A0C93EC93B")
DISK_GUID = bytes.fromhex("00112233445566778899AABBCCDDEEFF")
PART_GUID = bytes.fromhex("FFEEDDCCBBAA99887766554433221100")

BPS = 512; SPC = 1; RESERVED = 1; FATS = 2; ROOT_ENT = 512
ROOT_SEC = ROOT_ENT * 32 // BPS    # 32


def compute_fat_sec(part_sectors):
    lo, hi = 1, part_sectors
    while lo < hi:
        mid = (lo + hi) // 2
        clusters = (part_sectors - RESERVED - FATS * mid - ROOT_SEC) // SPC
        if (clusters + 2) * 2 <= mid * BPS:
            hi = mid
        else:
            lo = mid + 1
    return lo


FAT_SEC = compute_fat_sec(PART_SECTORS)
DATA_START = PART_START + RESERVED + FATS * FAT_SEC + ROOT_SEC
DATA_CLUSTERS = (PART_SECTORS - RESERVED - FATS * FAT_SEC - ROOT_SEC) // SPC
assert DATA_CLUSTERS < 65524, "FAT16 cluster overflow"


def le16(v): return struct.pack("<H", v)
def le32(v): return struct.pack("<I", v)
def le64(v): return struct.pack("<Q", v)


def cluster_lba(c):
    return DATA_START + (c - 2) * SPC


def to_83(name):
    base, _, ext = name.partition(".")
    base = base[:8].upper().ljust(8)
    ext = ext[:3].upper().ljust(3)
    return base.encode("ascii"), ext.encode("ascii")


def dir_entry(b8, b3, attr, cluster, size):
    return (
        b8 + b3              # 0..10 name+ext
        + bytes([attr])      # 11
        + b"\x00"            # 12 NT rsvd
        + b"\x00" * 7        # 13..19 ctime etc.
        + le16(0)            # 20-21 cluster high
        + b"\x00" * 4        # 22-25 time/date
        + le16(cluster)      # 26-27 cluster low
        + le32(size)         # 28-31 size
    )


def walk_tree():
    """Return (dirs, files): every directory and file under SRC (deterministic)."""
    dirs, files = [], []
    for dirpath, dirnames, filenames in sorted(os.walk(SRC)):
        rel = os.path.relpath(dirpath, SRC)
        if rel == ".":
            rel = ""
        for d in sorted(dirnames):
            dirs.append((os.path.join(rel, d) if rel else d, True, b""))
        for f in sorted(filenames):
            with open(os.path.join(dirpath, f), "rb") as fh:
                data = fh.read()
            p = os.path.join(rel, f) if rel else f
            files.append((p, False, data))
    return dirs, files


def allocate_clusters(items, fat):
    """Assign cluster runs for every item; returns {rel: (cluster, size)}."""
    base = 2
    plan = {}
    for rel, is_dir, data in items:
        if is_dir:
            nclusters = 1
            plan[rel] = (base, 0)
        else:
            nclusters = max(1, (len(data) + BPS - 1) // BPS)
            plan[rel] = (base, len(data))
        for i in range(nclusters):
            fat[base + i] = 0xFFFF if i == nclusters - 1 else base + i + 1
        base += nclusters
    return plan, base


def build():
    dirs, files = walk_tree()
    fat = [0xFFF8, 0xFFFF] + [0] * (DATA_CLUSTERS)
    plan, _ = allocate_clusters(dirs + files, fat)

    def entries_for(parent_rel):
        out = []
        for rel, is_dir, _ in dirs:
            if os.path.dirname(rel) == parent_rel:
                out.append((rel, True))
        for rel, _, _ in files:
            if os.path.dirname(rel) == parent_rel:
                out.append((rel, False))
        return sorted(out, key=lambda it: it[0])

    with open(IMG, "wb") as f:
        f.truncate(SIZE)

        # ---- protective MBR ----
        mbr = bytearray(512)
        mbr[510:512] = b"\x55\xAA"
        mbr[0x1BE:0x1BE + 16] = (
            bytes([0x00, 0x00, 0x02, 0x00])       # status + CHS start
            + bytes([0xEE])                        # GUID partition
            + bytes([0xFF, 0xFF, 0xFF])            # CHS end
            + le32(1) + le32(SIZE // LBA - 1)
        )
        f.seek(0)
        f.write(mbr)

        # ---- GPT partition entries (primary) ----
        entries = bytearray(PART_ENTRIES * PART_ENTRY_SZ)
        e = bytearray(128)
        e[0:16] = ESP_TYPE_GUID
        e[16:32] = PART_GUID
        e[32:40] = le64(PART_START)
        e[40:48] = le64(PART_LAST)
        e[48:56] = le64(0)
        name = "EFI System Partition".encode("utf-16-le").ljust(72, b"\x00")
        e[56:128] = name[:72]
        entries[0:128] = e
        part_entries_crc = zlib.crc32(bytes(entries)) & 0xFFFFFFFF
        f.seek(PART_ENTRY_LBA * LBA)
        f.write(entries)

        # ---- GPT header (primary) ----
        hdr = bytearray(512)
        hdr[0:8] = b"EFI PART"
        hdr[8:12] = le32(0x00010000)
        hdr[12:16] = le32(92)
        hdr[20:24] = le32(0)
        hdr[24:32] = le64(1)
        hdr[32:40] = le64(LAST_LBA)
        hdr[40:48] = le64(FIRST_USE)
        hdr[48:56] = le64(LAST_USE)
        hdr[56:72] = DISK_GUID
        hdr[72:80] = le64(PART_ENTRY_LBA)
        hdr[80:84] = le32(PART_ENTRIES)
        hdr[84:88] = le32(PART_ENTRY_SZ)
        hdr[88:92] = le32(part_entries_crc)
        crc = zlib.crc32(hdr[0:92]) & 0xFFFFFFFF
        hdr[16:20] = le32(crc)
        f.seek(LBA)
        f.write(hdr)

        # ---- FAT16 filesystem inside the partition ----
        f.seek(PART_START * LBA)
        f.write(b"\xEB\x3C\x90"); f.write(b"MSDOS5.0")
        f.write(le16(BPS)); f.write(bytes([SPC]))
        f.write(le16(RESERVED)); f.write(bytes([FATS]))
        f.write(le16(ROOT_ENT)); f.write(le16(0))
        f.write(bytes([0xF8])); f.write(le16(FAT_SEC))
        f.write(le16(32)); f.write(le16(1))
        f.write(le32(PART_START))                            # hidden sectors = partition start LBA
        f.write(le32(PART_SECTORS))
        f.write(bytes([0x80, 0x00, 0x29])); f.write(le32(0xDEADBEEF))
        f.write(b"NO NAME    "); f.write(b"FAT16   ")
        f.seek(PART_START * LBA + 0x1FE); f.write(b"\x55\xAA")

        # FAT copies
        fat_bytes = b"".join(le16(v & 0xFFFF) for v in fat)
        fat_lba = PART_START + RESERVED
        for i in range(FATS):
            f.seek((fat_lba + i * FAT_SEC) * LBA)
            f.write(fat_bytes)

        # root directory: entries for root-level items
        root_lba = PART_START + RESERVED + FATS * FAT_SEC
        f.seek(root_lba * LBA)
        for rel, is_dir in entries_for(""):
            b8, b3 = to_83(os.path.basename(rel))
            attr = 0x10 if is_dir else 0x20
            f.write(dir_entry(b8, b3, attr, plan[rel][0], plan[rel][1] if not is_dir else 0))
        f.write(b"\x00")

        # directory blocks (root dirs and subdirs alike)
        for rel, _, _ in dirs:
            cl = plan[rel][0]
            f.seek(cluster_lba(cl) * LBA)
            blk = bytearray(512)
            blk[0:32] = dir_entry(b".       ", b"   ", 0x10, cl, 0)
            parent_rel = os.path.dirname(rel)
            parent_cl = plan[parent_rel][0] if parent_rel else 0
            blk[32:64] = dir_entry(b"..      ", b"   ", 0x10, parent_cl, 0)
            off = 64
            for r2, is_dir in entries_for(rel):
                b8, b3 = to_83(os.path.basename(r2))
                attr = 0x10 if is_dir else 0x20
                blk[off:off + 32] = dir_entry(b8, b3, attr, plan[r2][0],
                                              plan[r2][1] if not is_dir else 0)
                off += 32
            if off < 512:
                blk[off] = 0
            f.write(blk)

        # file contents
        for rel, _, data in files:
            cl = plan[rel][0]
            f.seek(cluster_lba(cl) * LBA)
            f.write(data)

        # ---- backup GPT ----
        bk_ent_lba = LAST_LBA - 32
        f.seek(bk_ent_lba * LBA)
        f.write(entries)
        bh = bytearray(512)
        bh[0:8] = b"EFI PART"
        bh[8:12] = le32(0x00010000)
        bh[12:16] = le32(92)
        bh[20:24] = le32(0)
        bh[24:32] = le64(LAST_LBA)
        bh[32:40] = le64(1)
        bh[40:48] = le64(FIRST_USE)
        bh[48:56] = le64(LAST_USE)
        bh[56:72] = DISK_GUID
        bh[72:80] = le64(bk_ent_lba)
        bh[80:84] = le32(PART_ENTRIES)
        bh[84:88] = le32(PART_ENTRY_SZ)
        bh[88:92] = le32(part_entries_crc)
        crc = zlib.crc32(bh[0:92]) & 0xFFFFFFFF
        bh[16:20] = le32(crc)
        f.seek(LAST_LBA * LBA)
        f.write(bh)

    print(f"OK: {IMG} ({SIZE // 1024} KB, partition {PART_START}..{PART_LAST} LBA)")
    print(f"FAT_SEC={FAT_SEC} DATA_START_LBA={DATA_START} DATA_CLUSTERS={DATA_CLUSTERS}")
    for rel, is_dir, _ in sorted(dirs + files, key=lambda it: it[0]):
        kind = "dir " if is_dir else "file"
        print(f"  {kind} {rel}: cluster {plan[rel][0]} size {plan[rel][1] or 0}")


if __name__ == "__main__":
    if not os.path.isdir(SRC):
        print(f"error: source dir not found: {SRC}")
        sys.exit(1)
    build()