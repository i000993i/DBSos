#!/bin/sh
# Diagnostic: verify OVMF + serial output works at all, then test GPT ESP boot.
set -e
cd "$(dirname "$0")"

Ovmf="${OVMF:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
if [ ! -f "$Ovmf" ]; then
  echo "[diag] OVMF not found at $Ovmf"
  exit 1
fi

echo "=== Test 1: OVMF boot (no disk, 5s timeout) ==="
timeout 5 qemu-system-x86_64 \
  -machine q35 -m 64M \
  -drive if=pflash,format=raw,readonly=on,file="$Ovmf" \
  -nographic -serial stdio -monitor none \
  -display none 2>/dev/null || true
echo ""
echo "=== Test 1 done ==="

echo ""
echo "=== Test 2: OVMF + GPT ESP on AHCI (10s timeout) ==="
timeout 10 qemu-system-x86_64 \
  -machine q35 -m 256M \
  -drive if=pflash,format=raw,readonly=on,file="$Ovmf" \
  -drive if=none,id=espblk,format=raw,file="$PWD/esp.img" \
  -device ich9-ahci,id=ahci \
  -device ide-hd,drive=espblk,bus=ahci.0 \
  -nographic -no-reboot \
  -serial stdio -monitor none \
  -display none 2>/dev/null || true
echo ""
echo "=== Test 2 done ==="
