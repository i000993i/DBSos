#!/usr/bin/env python3
"""Resolve a runtime RIP from the QEMU log to an offset in the kernel image.

The kernel (x86_64-unknown-uefi debug PE) is loaded by UEFI at some base B.
A runtime address A maps to an image file offset when we know B.  We print
the section RVAs (preferred base 0x140000000) so we can relate RIP to a
section once the runtime load base is known.
"""
import struct, sys

PE = 'target/x86_64-unknown-uefi/debug/dbsos-kernel.efi'
d = open(PE, 'rb').read()

e_lfanew = struct.unpack_from('<I', d, 0x3C)[0]
coff = e_lfanew + 4
machine, nsecs = struct.unpack_from('<HH', d, coff)
opt_size, = struct.unpack_from('<H', d, coff + 16)
opt = coff + 20
magic, = struct.unpack_from('<H', d, opt)
if magic == 0x20B:
    image_base, = struct.unpack_from('<Q', d, opt + 24)
elif magic == 0x10B:
    image_base, = struct.unpack_from('<I', d, opt + 28)
else:
    print('unknown optional magic'); raise SystemExit(1)

print(f'machine={machine:#x} nsecs={nsecs} opt={opt_size} magic={magic:#x} image_base=0x{image_base:x}')

sec_off = opt + opt_size
secs = []
for i in range(nsecs):
    o = sec_off + i * 40
    name = d[o:o+8].rstrip(b'\0').decode('ascii', 'replace')
    vsize, rva, raw_size, raw_ptr = struct.unpack_from('<IIII', d, o + 8)
    secs.append((name, rva, raw_size))
    print(f'  {name:6s} rva=0x{rva:x} (pref=0x{image_base+rva:x}) raw=0x{raw_ptr:x} fsz=0x{raw_size:x} vsz=0x{vsize:x}')

# A page-fault RIP in the log: 0xD7F4D4C.  Kernel text lives in the first big
# section.  Using ILP: offset_in_text = RIP - runtime_base_of_text.
print('RIP to symbolize: 0x712F4D4C and 0xD7F4D4C')
txt_rva = next((rva for n, rva, _ in secs if n in ('.text', 'text', 'PECOFF')), None)
if txt_rva is not None:
    print(f'text rva=0x{txt_rva:x}')
    for a in (0x712F4D4C, 0xD7F4D4C):
        # if runtime base B, RIP = B + text_rva + off  => off = a
        print(f'  if base such that RIP in text: offset_in_text = 0x{a - (0x712F4D4C):x} (probe)')