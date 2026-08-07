<p align="center">
  <img src="https://img.shields.io/badge/Rust-Nightly-orange?style=for-the-badge&logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/Arch-x86__64-blue?style=for-the-badge&logo=amd" alt="x86_64">
  <img src="https://img.shields.io/badge/Boot-UEFI-green?style=for-the-badge" alt="UEFI">
  <img src="https://img.shields.io/badge/License-CC%20BY--SA%204.0-lightgrey?style=for-the-badge" alt="License">
  <img src="https://img.shields.io/badge/Status-Active-brightgreen?style=for-the-badge" alt="Status">
  <img src="https://img.shields.io/badge/Build-Passing-brightgreen?style=for-the-badge" alt="Build">
  <img src="https://img.shields.io/badge/Version-v0.1-blue?style=for-the-badge" alt="Version">
  <img src="https://img.shields.io/badge/Author-i000993i-purple?style=for-the-badge" alt="Author">
</p>

<p align="center">
  <a href="README_EN.md">English</a> | <b>Русский</b>
</p>

<h1 align="center">DBSos</h1>

<p align="center">
  <b>Собственная операционная система с микроядерной архитектурой</b><br>
  Написана на <a href="https://www.rust-lang.org/">Rust</a> для архитектуры <b>x86_64</b><br>
  Загружается через <b>UEFI</b> с использованием загрузчика <a href="https://github.com/limine-bootloader/limine">Limine</a>
</p>

> **Цель проекта** — создать полноценную ОС на Rust с поддержкой программ Linux и Windows 10. Приоритеты: безопасность и производительность.

---

## Скриншоты

<p align="center">
  <img src="README/photo-1.png" alt="DBSos Boot Screen" width="800">
  <br><i>Экран загрузки с логотипом "DBS" и промптом оболочки</i>
</p>

<p align="center">
  <img src="README/photo-2.png" alt="DBSos Shell" width="800">
  <br><i>Команды help, ping (ARP), info</i>
</p>

---

## Быстрый старт

```bash
# 1. Установите Rust + target
rustup target add x86_64-unknown-uefi

# 2. Клонируйте и соберите
git clone https://github.com/i000993i/DBSos.git
cd DBSos
cargo build -p dbsos-kernel --target x86_64-unknown-uefi

# 3. Подготовьте ESP
mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-uefi/debug/dbsos-kernel.efi esp/EFI/BOOT/BOOTX64.EFI

# 4. Запустите в QEMU
qemu-system-x86_64 \
  -machine q35 \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/ovmf/OVMF_CODE.fd \
  -drive file=fat:rw:esp,format=raw \
  -drive file=nvme_disk.img,if=none,id=nvme0,format=raw \
  -device nvme,serial=deadbeef,drive=nvme0 \
  -m 256M -nic user,model=e1000 -nographic -no-reboot
```

> Путь к OVMF может отличаться: `find / -name "OVMF_CODE.fd" 2>/dev/null`

> Тест TCP: в отдельном терминале на хосте запустите `python3 scripts/tcp_echo_server.py 8000`, затем в оболочке DBSos выполните `tcp 10.0.2.2 8000 hello`. QEMU (slirp `10.0.2.2`) пробрасывает соединение на loopback хоста.

### Windows (PowerShell)

```powershell
.\scripts\bootstrap.ps1 -Run    # сборка + запуск
.\scripts\bootstrap.ps1          # только сборка
```

---

## Архитектура

```
┌─────────────────────────────────────────────────┐
│  Ring 3 (User)                                  │
│  ┌───────────┐ ┌───────────┐ ┌───────────┐      │
│  │ Console   │ │ e1000     │ │ ELF       │      │
│  │ Server    │ │ Driver    │ │ Binary    │      │
│  └─────┬─────┘ └─────┬─────┘ └─────┬─────┘      │
│        └──────────────┼─────────────┘            │
│                       │ IPC (capabilities)       │
├───────────────────────┼─────────────────────────┤
│  Ring 0 (Kernel)      ▼                         │
│  ┌──────────────────────────────────────────┐   │
│  │ Memory │ VM │ Scheduler │ Syscall │ FS   │   │
│  │ UART │ PCI │ AHCI │ NVMe │ e1000 │ PS/2 │   │
│  └──────────────────────────────────────────┘   │
│  Boot: Limine → UEFI → ExitBootServices → Shell │
└─────────────────────────────────────────────────┘
```

**Ключевые принципы:**
- **Микроядро** — минимальное ядро, серверы в Ring 3
- **Capability-based** — доступ к ресурсам через capabilities
- **no_std** — ядро без стандартной библиотеки
- **Preemptive** — вытесняющая многозадачность (LAPIC timer ~100 Hz)
- **Demand paging** — куча/стек процессов мапятся по требованию через VMA
- **ELF loading** — пользовательские бинарники из FAT-файловой системы

---

## Команды оболочки

| Команда | Описание | Команда | Описание |
|---------|----------|---------|----------|
| `help` | Справка | `exec PATH` | Запуск ELF |
| `info` | Информация о системе | `ping` | ARP ping gateway |
| `mem` | Статистика памяти | `nvme info` | Информация о NVMe |
| `time` | Тест таймера | `nvme read LBA` | Чтение секторов |
| `clear` | Очистить экран | `nvme write LBA` | Запись секторов |
| `sleep N` | Задержка N мс | `reboot` | ACPI reboot |
| `ls [PATH]` | Список файлов | `poweroff` | ACPI shutdown |
| `cat PATH` | Содержимое файла | | |
| `mkdir PATH` | Создать директорию | | |
| `write PATH TXT` | Записать файл | | |
| `rm PATH` | Удалить файл | | |
| `rmdir PATH` | Удалить директорию | | |
| `tcp IP PORT TXT` | TCP-стек (клиент) | | |

---

## Текущее состояние

### Реализовано

| Компонент | Статус | Компонент | Статус |
|-----------|--------|-----------|--------|
| UEFI Boot (Limine) | ✅ | FAT16 FS (read/write) | ✅ |
| Physical Memory (bitmap) | ✅ | VFAT LFN | ✅ |
| Virtual Memory (4-level paging) | ✅ | NVMe Driver | ✅ |
| Preemptive Scheduler | ✅ | AHCI SATA Driver | ✅ |
| GDT / IDT / PIC / LAPIC | ✅ | e1000 NIC (ARP/ICMP) | ✅ |
| SYSCALL / SYSRET (24 calls) | ✅ | PS/2 Keyboard | ✅ |
| IPC (capabilities) | ✅ | UART (16550A) | ✅ |
| Shared Memory (zero-copy) | ✅ | ELF Loader | ✅ |
| ACPI (reboot/shutdown) | ✅ | Ring 3 Tasks | ✅ |
| HPET Timer | ✅ | Display (GOP) | ✅ |
| Interactive Shell (15+ cmds) | ✅ | PCI Bus | ✅ |
| Demand Paging (VMA) | ✅ | IRQ-safe Allocator | ✅ |
| TCP/IP Stack (ARP) | ✅ | e1000 NIC | ✅ |

### В доработке

| Компонент | Приоритет |
|-----------|-----------|
| FS Server (Ring 3) | Средний |
| DHCP / DNS | Средний |
| Multi-core (SMP) | Средний |
| Backtrace | Низкий |
| CI/CD | Низкий |

---

## Автор и лицензия

**Автор:** i000993i | **Лицензия:** [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/deed.ru)

Можно использовать, изменять, распространять (в т.ч. коммерчески). Условие: указывать автора и показывать изменения. Новые версии — под той же лицензией.

---

## Источники

| Ресурс | Ссылка |
|--------|--------|
| x86 SDM | [Intel](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html) |
| UEFI Spec | [uefi.org](https://uefi.org/specifications) |
| Limine | [GitHub](https://github.com/limine-bootloader/limine) |
| Rust no_std | [Rustonomicon](https://doc.rust-lang.org/nomicon/) |
| FAT16 | [Microsoft](https://learn.microsoft.com/en-us/windows/win32/filesystem/long-fat-file-system) |
| e1000 | [Intel](https://www.intel.com/content/dam/www/public/us/en/documents/pci-ide-interrupt-manual.pdf) |
| ACPI | [acpi.info](https://acpi.info/specifications.htm) |
| QEMU | [Documentation](https://www.qemu.org/docs/) |

---

<p align="center">
  <i>DBSos v0.1 — Built with ❤️ and Rust</i><br>
  <sub>Author: i000993i | License: CC BY-SA 4.0</sub>
</p>
