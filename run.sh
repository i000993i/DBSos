#!/bin/sh
# Build, deploy EFI, recreate disk image and run DBSos in QEMU (no reboot).
set -e
cd "$(dirname "$0")"

cargo build -p dbsos-kernel --target x86_64-unknown-uefi
mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-uefi/debug/dbsos-kernel.efi esp/EFI/BOOT/BOOTX64.EFI

python3 scripts/mk_image.py 2>/dev/null || :  # image already present/recreated above

Ovmf="${OVMF:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
echo "[run] OVMF=$Ovmf"
exec timeout 30 qemu-system-x86_64 \
  -machine q35 \
  -drive if=pflash,format=raw,readonly=on,file="$Ovmf" \
  -drive file=fat:rw:"$PWD/esp",format=raw \
  -drive file="$PWD/nvme_disk.img",if=none,id=nvme0,format=raw \
  -device nvme,serial=deadbeef,drive=nvme0 \
  -nographic -no-reboot -m 256M \
  -nic user,model=e1000