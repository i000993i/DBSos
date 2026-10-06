#!/bin/sh
# DBSos VirtualBox setup — VM под стандарты, которые поддерживает ядро.
#
# Соответствие железу VBox <-> драйверы DBSos:
#   EFI firmware      — наш BOOTX64.EFI (без EFI грузиться не будет!)
#   Intel PRO/1000 MT — e1000 (82540EM), DHCP из коробки
#   ICH9 SATA/AHCI    — ahci (в доработке, ESP лучше на IDE/VDI)
#   Intel HD Audio    — audio-probe (HDA детект)
#   xHCI USB          — usb-probe
#   COM1 -> file      — serial-лог для отладки (присылайте его с багами!)
#
# Использование:
#   ./scripts/vbox_setup.sh [имя-ВМ]   # создаёт/перенастраивает + цепляет ESP
#   VBoxManage startvm "DBSos"
#
# Как это работает:
#   VirtualBox EFI не умеет загружать El Torito из hybrid ISO.
#   Вместо этого мы конвертируем esp.img -> esp.vdi и цепляем как SATA-диск.
#   EFI firmware находит GPT+FAT16 на VDI и загружает BOOTX64.EFI.

set -e
cd "$(dirname "$0")/.."

VM="${1:-DBSos}"
LOG="$PWD/vbox_serial.log"
ESP_IMG="$PWD/esp.img"
VDI="$PWD/esp.vdi"

if [ ! -f "$ESP_IMG" ]; then
  echo "[vbox] $ESP_IMG not found — run scripts/mk_esp.py first"
  exit 1
fi

# --- 1. Конвертация esp.img -> esp.vdi ---
echo "[vbox] converting esp.img -> esp.vdi (VirtualBox disk format)..."
rm -f "$VDI"
VBoxManage convertfromraw "$ESP_IMG" "$VDI" --format VDI

# --- 2. Создание/настройка VM ---
if ! VBoxManage list vms | grep -q "\"$VM\""; then
  echo "[vbox] creating VM \"$VM\""
  VBoxManage createvm --name "$VM" --ostype "Linux_64" --register
fi

echo "[vbox] applying DBSos-standard settings to \"$VM\""
VBoxManage modifyvm "$VM" \
  --firmware efi \
  --chipset ich9 \
  --memory 512 \
  --cpus 2 \
  --boot1 disk --boot2 none --boot3 none --boot4 none \
  --nic1 nat \
  --nictype1 82540EM \
  --audiocontroller hda \
  --usbxhci on \
  --uart1 0x3F8 4 \
  --uartmode1 file "$LOG"

# --- 3. SATA-контроллер с ESP как загрузочный диск ---
VBoxManage storagectl "$VM" --name "SATA" --add sata --controller IntelAhci 2>/dev/null || true
VBoxManage storageattach "$VM" \
  --storagectl "SATA" --port 0 --device 0 --type hdd --medium "$VDI"

# --- 4. IDE-контроллер (пустой, на будущее) ---
VBoxManage storagectl "$VM" --name "IDE" --add ide --controller PIIX4 2>/dev/null || true

echo "[vbox] done. Serial log -> $LOG"
echo "[vbox] start: VBoxManage startvm \"$VM\""
echo "[vbox]"
echo "[vbox] boot order: SATA (esp.vdi) -> EFI firmware -> BOOTX64.EFI"
