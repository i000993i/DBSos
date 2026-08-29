#!/bin/sh
# Build, deploy EFI, recreate disk image and run DBSos in QEMU (no reboot).
# SMP trace to qemu.log:  SMP_DEBUG=1 ./run.sh
set -e
cd "$(dirname "$0")"

cargo build -p dbsos-kernel --target x86_64-unknown-uefi
mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-uefi/debug/dbsos-kernel.efi esp/EFI/BOOT/BOOTX64.EFI
rm -f esp/NvVars  # leftover written by QEMU's fat:rw mode, not needed

python3 scripts/mk_image.py 2>/dev/null || :  # data disk (NVMe)
python3 scripts/mk_esp.py                      # boot ESP (GPT+FAT16)

Ovmf="${OVMF:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
if [ ! -f "$Ovmf" ]; then
  echo "[run] ERROR: OVMF not found at $Ovmf"
  echo "[run] find it with: find / -name 'OVMF_CODE*.fd' 2>/dev/null"
  echo "[run] then run:      OVMF=/path/to/OVMF_CODE.fd ./run.sh"
  exit 1
fi
echo "[run] OVMF=$Ovmf"

QEMU_LOG=""
if [ -n "$SMP_DEBUG" ]; then
  rm -f qemu.log
  QEMU_LOG="-d cpu_reset -D qemu.log"
  echo "[run] SMP_DEBUG=1 -> cpu_reset trace in qemu.log"
fi

rm -f qemu_console.log
echo "[run] Starting QEMU... serial output below."
echo "[run] Type 'gui' in the OS shell to launch desktop."
echo "[run] Session log saved to qemu_console.log"
echo ""
script -q -c "timeout 120 qemu-system-x86_64 \
  -machine q35 \
  -drive if=pflash,format=raw,readonly=on,file=\"$Ovmf\" \
  -drive file=\"$PWD/esp.img\",format=raw,id=espblk,if=none \
  -device ide-hd,drive=espblk \
  -drive file=\"$PWD/nvme_disk.img\",if=none,id=nvme0,format=raw \
  -device nvme,serial=deadbeef,drive=nvme0 \
  -display gtk \
  -m 256M \
  -smp 2 \
  -serial stdio \
  -monitor none \
  $QEMU_LOG \
  -nic user,model=e1000" qemu_console.log
echo ""
echo "[run] Session saved to qemu_console.log"
if [ -s qemu_console.log ]; then
  echo "===== last 100 lines ====="
  tail -100 qemu_console.log
fi
if [ -n "$SMP_DEBUG" ]; then
  echo "===== qemu.log tail ====="
  tail -80 qemu.log 2>/dev/null || echo "(no qemu.log)"
fi
exit $rc
