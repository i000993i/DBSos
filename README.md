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

> **Цель проекта** — создать полноценную операционную систему на Rust с поддержкой программ Linux и Windows 10. Приоритеты: высокая безопасность и высокая производительность.

---

## Скриншоты

### Экран загрузки

<p align="center">
  <img src="README/photo-1.png" alt="DBSos Boot Screen" width="800">
  <br>
  <i>Экран загрузки DBSos с логотипом "DBS" и промптом оболочки</i>
</p>

### Интерактивная оболочка

<p align="center">
  <img src="README/photo-2.png" alt="DBSos Shell" width="800">
  <br>
  <i>Список команд (help), выполнение ping (ARP), системная информация (info)</i>
</p>

---

## Быстрый старт

### Требования

| Компонент | Версия / Путь |
|-----------|---------------|
| **Rust** | Nightly, target `x86_64-unknown-uefi` |
| **QEMU** | `qemu-system-x86_64` |
| **OVMF** | `edk2-x86_64-code.fd` |
| **OS** | Linux / Windows (PowerShell) |

### 1. Установите Rust и target

```bash
# Установка Rust (если ещё нет)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Добавление target для UEFI
rustup target add x86_64-unknown-uefi
```

### 2. Установите QEMU

```bash
# Linux (Debian/Ubuntu)
sudo apt install qemu-system-x86 ovmf

# Linux (Arch)
sudo pacman -S qemu-full edk2-ovmf

# Windows
# Скачайте с https://www.qemu.org/download/
```

### 3. Соберите ядро

```bash
# Клонируйте репозиторий
git clone https://github.com/i000993i/DBSos.git
cd DBSos

# Соберите ядро
cargo build -p dbsos-kernel --target x86_64-unknown-uefi
```

### 4. Подготовьте ESP (EFI System Partition)

```bash
mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-uefi/debug/dbsos-kernel.efi esp/EFI/BOOT/BOOTX64.EFI
```

### 5. Запустите в QEMU

```bash
qemu-system-x86_64 \
  -machine q35 \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/ovmf/OVMF_CODE.fd \
  -drive file=fat:rw:esp,format=raw \
  -drive file=nvme_disk.img,if=none,id=nvme0,format=raw \
  -device nvme,serial=deadbeef,drive=nvme0 \
  -m 256M \
  -nic user,model=e1000 \
  -nographic \
  -no-reboot
```

> **Примечание:** Путь к OVMF может отличаться. Проверьте: `find / -name "OVMF_CODE.fd" 2>/dev/null`

### Альтернатива: PowerShell (Windows)

```powershell
# Полная сборка + создание диска + запуск
.\scripts\bootstrap.ps1 -Run

# Только сборка (без запуска)
.\scripts\bootstrap.ps1

# Пересборка диска (очистка ESP)
.\scripts\bootstrap.ps1 -Clean
```

---

## Архитектура

```
┌─────────────────────────────────────────────────────┐
│                    Limine Bootloader                 │
│               (UEFI Application)                     │
├─────────────────────────────────────────────────────┤
│                    UEFI Firmware                     │
│         (GOP, Memory Map, ACPI Tables)               │
├─────────────────────────────────────────────────────┤
│  Ring 0 (Kernel)                                    │
│  ┌───────────────────────────────────────────────┐  │
│  │  init() — точка входа ядра                    │  │
│  │  ├─ UEFI boot services                         │  │
│  │  ├─ Memory + VM (PML4)                         │  │
│  │  ├─ ACPI + Drivers (AHCI/NVMe)                 │  │
│  │  ├─ ExitBootServices                           │  │
│  │  ├─ GDT/IDT/PIC/LAPIC                          │  │
│  │  ├─ IPC + Scheduler + Timer                    │  │
│  │  ├─ Syscall (SYSCALL/SYSRET)                   │  │
│  │  ├─ FS tests + Network tests                   │  │
│  │  └─ Shell (interactive loop)                   │  │
│  └───────────────────────────────────────────────┘  │
│                                                      │
│  Ring 3 (User Tasks)                                │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐          │
│  │ Console  │  │ e1000    │  │ ELF      │          │
│  │ Server   │  │ Driver   │  │ Binary   │          │
│  └──────────┘  └──────────┘  └──────────┘          │
│        ↕ IPC (capabilities)                         │
├─────────────────────────────────────────────────────┤
│  Hardware / QEMU Emulation                          │
│  LAPIC, HPET, NVMe, AHCI, e1000 NIC, PS/2          │
└─────────────────────────────────────────────────────┘
```

### Ключевые принципы

| Принцип | Описание |
|---------|----------|
| **Микроядро** | Минимальное ядро (память, IPC, планировщик). Серверы работают в Ring 3 |
| **Capability-based** | Доступ к ресурсам только через capabilities (IPC, MMIO, shared memory) |
| **no_std** | Ядро не использует стандартную библиотеку Rust |
| **Paging 4-level** | Полная поддержка x86-64 page tables (PML4 → PDPT → PD → PT) |
| **Preemptive** | Вытесняющая многозадачность на базе LAPIC timer (~100 Hz) |
| **ELF loading** | Пользовательские бинарники загружаются из FAT-файловой системы |

---

## Компоненты ядра

### `memory.rs` — Физическая память

| Компонент | Описание |
|-----------|----------|
| **PhysAllocator** | Bitmap-аллокатор физических страниц |
| **Источники** | UEFI Memory Map (MemoryType::CONVENTIONAL) |
| **palloc()** | Выделение одной страницы (4 KB) |
| **palloc_n(n)** | Выделение `n` подряд идущих страниц |
| **pfree(phys)** | Освобождение страницы, возврат в пул |
| **Bitmap** | Размещается в начале найденного memory region, не пересекается с выделенной памятью |

### `vm.rs` — Виртуальная память (x86-64 Paging)

| Компонент | Описание |
|-----------|----------|
| **PML4** | 4-уровневая страница: PML4 → PDPT → PD → PT → 4KB |
| **map_page()** | Маппинг phys → virt, с автоматическим split 2MB → 4KB |
| **create_address_space()** | Создание нового PML4 с identity map |
| **switch_to()** | Загрузка CR3 для переключения address space |
| **clone_kernel_mappings()** | Клонирование kernel high-half (256-511) |
| **identity_map_2mb()** | Identity mapping через 2MB huge pages |
| **destroy_address_space()** | Освобождение всех page table pages |

**PTE flags:** `PRESENT`, `WRITABLE`, `USER`, `WRITE_THROUGH`, `CACHE_DISABLE`, `ACCESSED`, `DIRTY`, `HUGE`, `GLOBAL`, `NX`

### `scheduler.rs` — Планировщик

| Компонент | Описание |
|-----------|----------|
| **Task** | Структура задачи: state, stack, sp, id, IPC state, PML4, GDT, TSS, FPU buffer |
| **TaskState** | `Free`, `Ready`, `Running`, `Exited`, `BlockedSend`, `BlockedRecv` |
| **spawn()** | Создание задачи Ring 0 (кооперативной) |
| **spawn_user()** | Создание задачи Ring 3 (пользовательской) |
| **reschedule()** | Точка переключения контекста (вызывается из ISR таймера) |
| **yield_now()** | Кооперативное переключение |
| **exit()** | Завершение задачи, освобождение ресурсов |
| **FPU save/restore** | `fxsave64` / `fxrstor64` для сохранения состояния сопроцессора |
| **LAPIC timer** | ~100 Hz, вектор 0x20, periodic mode |

**IRETQ frame** (для QEMU): `SS`, `RSP`, `RFLAGS`, `CS`, `RIP` + 15 GPRs (`RAX..R15`)

### `interrupts.rs` — Прерывания

| Компонент | Описание |
|-----------|----------|
| **IDT** | 256 entry, per-vector stubs для 0..31 |
| **GDT** | 6 сегментов: null, kernel code, kernel data, user data, user code, user data (SYSRET) |
| **PIC remap** | IRQ0-7 → 0x20-0x27, IRQ8-15 → 0x28-0x2F |
| **exception_common** | Единый обработчик исключений с дампом регистров |
| **timer_stub** | Обработчик LAPIC timer (IRQ0), вызывает `reschedule()` |

### `syscall.rs` — Системные вызовы (SYSCALL / SYSRET)

| MSR | Значение | Назначение |
|-----|----------|------------|
| **IA32_EFER** | SCE=1 | Включение SYSCALL |
| **IA32_STAR** | 0x0008_0020_0000_0000 | CS/SS селекторы |
| **IA32_LSTAR** | syscall_stub | Entry point |
| **IA32_FMASK** | Clear IF/DF/TF | Маска RFLAGS на входе |

**Таблица системных вызовов**

| Номер | Имя | Arg1 | Arg2 | Arg3 | Возврат |
|-------|-----|------|------|------|---------|
| 0 | `SYS_EXIT` | exit_code | — | — | EXIT_MAGIC |
| 1 | `SYS_IPC_SEND_LEGACY` | dst_id | val | — | 0 / IPC_ERR |
| 2 | `SYS_IPC_RECV_LEGACY` | src_id | — | — | val / IPC_ERR |
| 3 | `SYS_GETPID` | — | — | — | pid |
| 4 | `SYS_WAIT` | pid (0=любой) | status_ptr | — | pid / 0 |
| 5 | `SYS_KILL` | pid | signal | — | 1 / 0 |
| 6 | `SYS_FORK` | — | — | — | child_pid / 0 / -1 |
| 7 | `SYS_GETPGID` | pid (0=текущий) | — | — | pgid |
| 8 | `SYS_SETPGID` | pid (0=текущий) | pgid | — | 0 / IPC_ERR |
| 9 | `SYS_KILLPG` | pgid | signal | — | count |
| 11 | `SYS_IPC_SEND` | cap_idx | msg_ptr | — | IPC_OK / IPC_ERR |
| 12 | `SYS_IPC_RECV` | cap_idx | buf_ptr | — | IPC_OK / IPC_ERR |
| 13 | `SYS_CAP_GRANT` | dst_task_id | cap_idx | — | new_cap_idx |
| 14 | `SYS_CAP_ATTACH_IRQ` | cap_idx | irq_num | — | 0 / IPC_ERR |
| 15 | `SYS_SHMEM_MAP` | cap_idx | virt_addr | — | 0 / IPC_ERR |
| 16 | `SYS_SHMEM_CREATE` | pages | — | — | cap_idx / IPC_ERR |
| 17 | `SYS_MMIO_MAP` | phys_addr | size | virt_addr | 0 / IPC_ERR |
| 18 | `SYS_PCI_READ` | bdf | offset | — | value |
| 19 | `SYS_PCI_WRITE` | bdf | offset | value | 0 |
| 20 | `SYS_LOG_WRITE` | str_ptr | len | — | 0 |
| 21 | `SYS_CAP_GET_DATA` | cap_idx | — | — | cap.data |
| 22 | `SYS_SIGNAL` | sig | handler | — | prev_handler |
| 23 | `SYS_SIGRETURN` | — | — | — | — |
| 24 | `SYS_SLEEP` | ms | — | — | 0 |

### `ipc.rs` — Межпроцессное взаимодействие

| Компонент | Описание |
|-----------|----------|
| **Message** | Структура сообщения: `msg_type`, `dst_port`, `src_port`, `data[8]`, `length` |
| **send_with_cap()** | Отправка через capability с blocking |
| **recv_with_cap()** | Получение через capability с blocking |
| **deliver_message()** | Fast-path delivery (sender → waiting receiver) |
| **self-send** | Автоматический routing через `PENDING[]` |
| **broadcast()** | Широковещательная рассылка |
| **Shared Memory** | Zero-copy маппинг через capabilities |

### `cap.rs` — Capability Manager

| Компонент | Описание |
|-----------|----------|
| **Cap** | `cap_type`, `server_id`, `rights`, `data` |
| **CapTable** | 64 слота на задачу |
| **alloc()** | Выделение capability для текущей задачи |
| **duplicate()** | Копирование capability в другую задачу |
| **transfer()** | Передача с отзывом у отправителя |
| **free_all()** | Освобождение всех capabilities задачи |

**Types:** `IpcTarget` (1), `SharedMem` (2), `Irq` (3), `IpcReply` (4)
**Rights:** `CAP_SEND` (1), `CAP_RECV` (2), `CAP_READ` (4), `CAP_WRITE` (8)

### `timer.rs` — Таймеры

| Компонент | Описание |
|-----------|----------|
| **HPET** | High Precision Event Timer @ 0xFED00000, 10 MHz |
| **usleep(us)** | Busy-wait на основе HPET counter |
| **millis()** | Миллисекунды от HPET |
| **ticks()** | Сырые тики HPET |
| **LAPIC timer** | ~100 Hz, используется для preemptive scheduling |

### `fs.rs` — FAT16 Filesystem

| Функция | Описание |
|---------|----------|
| **ls()** | Listing root directory |
| **ls_path(path)** | Listing directory по пути |
| **cat(name)** | Вывод содержимого файла на UART |
| **cat_path(path)** | То же, с поддержкой путей |
| **read_file()** | Чтение файла в буфер |
| **mkdir(path)** | Создание директории |
| **write_file(path, data)** | Запись файла (создание + перезапись) |
| **rm(path)** | Удаление файла |
| **rmdir(path)** | Удаление пустой директории |
| **VFAT LFN** | Поддержка Long File Names (до 255 символов) |
| **Path resolution** | `/parent/child/file` — рекурсивное разрешение |

### `elf.rs` — ELF Loader

| Компонент | Описание |
|-----------|----------|
| **load_and_spawn()** | Загрузка ELF из FAT, маппинг в память, создание задачи Ring 3 |
| **Header parsing** | ELF64, `ET_EXEC`, `PT_LOAD` segments |
| **Mapping** | `PTE_USER | PTE_WRITABLE` для кода и данных |

### `display.rs` — UEFI GOP Display

| Компонент | Описание |
|-----------|----------|
| **draw_str()** | Вывод строки с цветом (RGB 888) |
| **draw_rect()** | Заливка прямоугольника |
| **clear_screen()** | Очистка экрана |
| **set_cursor()** | Установка курсора (колонка, строка) |

### `console.rs` — Консольный вывод

| Компонент | Описание |
|-----------|----------|
| **write_str()** | Вывод строки на дисплей (через GOP) |
| **putchar()** | Вывод одного символа |
| **clear_screen()** | Очистка |
| **screen_cols()** | Ширина экрана в символах |
| **draw_rect()** | Рисование прямоугольника |
| **draw_logo()** | Отрисовка логотипа DBSos (3x scale, gold on dark red) |

### `acpi.rs` — ACPI

| Компонент | Описание |
|-----------|----------|
| **RSDP lookup** | Поиск в UEFI config tables (ACPI2_GUID приоритет) |
| **copy_tables()** | Копирование таблиц до ExitBootServices |
| **init()** | Парсинг RSDT/XSDT, FADT |
| **reboot()** | ACPI reboot через RESET_REG |
| **shutdown()** | ACPI poweroff |

### `driver/` — Подсистема драйверов

| Модуль | Описание |
|--------|----------|
| **traits.rs** | Абстрактный интерфейс: `Driver` trait с методами `name()`, `device_type()`, `init()` |
| **manager.rs** | Реестр драйверов, регистрация и инициализация |
| **mod.rs** | Список драйверов: UART, PCI, e1000 |

---

## Драйверы

### `uart.rs` — Serial Port (16550A)

| Параметр | Значение |
|----------|----------|
| **Port** | COM1: `0x3F8` |
| **Baud** | 115200 |
| **Функции** | `write_str()`, `putchar()`, `poll_char()` |
| **Использование** | Основной канал отладки и вывода |

### `pci.rs` — PCI Bus

| Функция | Описание |
|---------|----------|
| **read32(bus, dev, func, offset)** | PCI config space read |
| **write32(bus, dev, func, offset, val)** | PCI config space write |
| **validate_mmio()** | Проверка MMIO region (только известные PCI BARs) |

### `net.rs` — Intel e1000e NIC (82574L)

| Компонент | Описание |
|-----------|----------|
| **MMIO BAR0** | 0x1000–0x2FFF (256 registers) |
| **TX Ring** | 16 descriptors (16 bytes each, legacy format) |
| **RX Ring** | 16 descriptors (16 bytes each, legacy format) |
| **send_packet()** | Отправка пакета через TX ring |
| **poll()** | Polling RX ring для incoming packets |
| **send_icmp_ping()** | Генерация и отправка ICMP Echo Request |
| **send_arp_request()** | Генерация и отправка ARP request |
| **tx_test()** | Тест TX loopback |
| **rx_software_test()** | Инъекция fake ARP в RX ring |
| **dump_rx_state()** | Debug: состояние RX ring |

**MAC адрес:** `52:54:00:12:34:56` (QEMU default)
**Gateway:** `10.0.2.1` (QEMU user-mode NAT)

### `ahci.rs` — AHCI SATA

| Параметр | Значение |
|----------|----------|
| **MMIO BAR** | PCI config read BAR0 |
| **BPS** | 512 bytes per sector |
| **SPC** | 8 sectors per cluster |
| **FATS** | 2 FAT copies |
| **ROOT_ENT** | 512 root directory entries |
| **RESERVED** | 1 reserved sector |
| **FAT_SZ** | 126 sectors per FAT |
| **read_fat_sector()** | Чтение сектора через AHCI command list |
| **write_fat_sector()** | Запись сектора через AHCI |

### `nvme.rs` — NVMe SSD

| Параметр | Значение |
|----------|----------|
| **BAR** | PCI config read BAR0 |
| **NSID** | 1 |
| **LBA_SIZE** | 512 bytes |
| **LBAs** | 129024 (63 MB) |
| **BPS** | 512 |
| **SPC** | 4 |
| **FATS** | 2 |
| **ROOT_ENT** | 512 |
| **RESERVED** | 1 |
| **FAT_SZ** | 126 |
| **read_sectors()** | NVMe Admin + I/O submission |
| **write_sectors()** | NVMe write submission |
| **read_fat_sector()** | Обёртка для FS |
| **write_fat_sector()** | Обёртка для FS |

### `ps2.rs` — PS/2 Keyboard

| Функция | Описание |
|---------|----------|
| **init()** | Инициализация PS/2 controller (IRQ1) |
| **poll_char()** | Non-blocking чтение сканкода |
| **Handler** | IRQ1 → Decode → UART echo |

---

## Пользовательский режим (Ring 3)

### Механизм входа/выхода

```
Ring 3 (User)                          Ring 0 (Kernel)
─────────────                          ─────────────
SYSCALL (0F 05)  ──────────────────►  syscall_stub:
  RAX = syscall number                  mov rsp, sys_krsp
  RDX = arg1                              push 15 GPRs
  R8  = arg2                              call syscall_rust_entry
  R9  = arg3                              pop 15 GPRs
                                         iretq (restore user RIP, CS, RFLAGS, RSP, SS)
◄──────────────────────────────────  SYSRET / iretq
  RAX = return value
```

### Создание задачи Ring 3

```
1. create_user_task_env()
   ├─ palloc: code, stack, GDT, TSS, PML4
   ├─ prepare_user_pml4()
   │   ├─ clone_high_half (kernel space)
   │   └─ identity_map_2mb (user space 0-4GB, PTE_USER)
   └─ map code + stack at canonical addresses

2. setup_user_gdt_tss()
   ├─ Write GDT: null, kernel code(0x08), kernel data(0x10),
   │              user data(0x18), user code(0x20), user data SYSRET(0x28)
   └─ Write TSS: LTR, RSP0 = kernel stack top

3. spawn_user(entry, rsp, pml4, gdt, tss, code_phys, stack_phys)
   └─ Build IRETQ frame: SS, user RSP, RFLAGS, CS, RIP + 15 GPRs
```

### Тестовые задачи

| Задача | Описание | Entry |
|--------|----------|-------|
| **Sender (A)** | Ring 3 IPC sender: LOG_WRITE + infinite loop | `0x100000000` |
| **Receiver (B)** | Ring 3 IPC receiver: LOG_WRITE + infinite loop | `0x100001000` |
| **e1000 driver** | Userspace NIC driver: PCI config read → MMIO map → MAC/STATUS read | `0x100002000` |
| **Console server** | Ring 3 IPC server: IPC_RECV → LOG_WRITE → loop | `0x100003000` |

---

## Файловая система

### Поддерживаемые операции

```
ls                    # listing root
ls /path              # listing directory
cat file              # print file content
cat /path/to/file     # print with path
mkdir /path           # create directory
write /path "text"    # write file
rm /path              # delete file
rmdir /path           # delete empty directory
```

### Структура NVMe-образа

```
nvme_disk.img (64 MB)
├── MBR (LBA 0)
│   └── Partition: FAT16, LBA 2048, 63 MB
├── Boot Sector (LBA 2048)
│   └── BPB: BPS=512, SPC=4, FATs=2, RootEnt=512
├── FAT1 (126 sectors)
├── FAT2 (126 sectors)
├── Root Directory (32 sectors, 512 entries)
│   ├── HELLO.TXT (cluster 3, "Hello NVMe!\r\n")
│   ├── TEST/ (cluster 4, directory)
│   │   ├── . (self)
│   │   ├── .. (root)
│   │   └── HELLO.ELF (cluster 5, 22 bytes, Ring 3 test binary)
│   └── NVME DISK (volume label)
└── Data Area (clusters 3+)
```

---

## Интерактивная оболочка

### Команды

| Команда | Описание |
|---------|----------|
| `help` | Показать справку |
| `info` | Информация о системе |
| `mem` | Статистика памяти |
| `time` | Тест таймера (10ms delay) |
| `clear` | Очистить экран |
| `sleep N` | Задержка N мс (планировщик) |
| `ls [PATH]` | Listing директории |
| `cat PATH` | Вывод содержимого файла |
| `mkdir PATH` | Создание директории |
| `write PATH TEXT` | Запись текстового файла |
| `rm PATH` | Удаление файла |
| `rmdir PATH` | Удаление директории |
| `exec PATH` | Запуск ELF-бинарника |
| `ping` | ARP ping gateway (10.0.2.1) |
| `nvme info` | Информация о NVMe диске |
| `nvme read LBA [COUNT]` | Чтение секторов |
| `nvme write LBA [COUNT]` | Запись секторов |
| `reboot` | ACPI reboot |
| `poweroff` | ACPI shutdown |

---

## Параметры QEMU

| Флаг | Описание |
|------|----------|
| `-machine q35` | Q35 чипсет |
| `-drive if=pflash` | UEFI firmware (OVMF) |
| `-drive file=fat:rw:esp` | ESP с BOOTX64.EFI |
| `-device nvme` | NVMe диск |
| `-nic user,model=e1000` | e1000 NIC (user-mode NAT, gateway 10.0.2.1) |
| `-m 256M` | 256 МБ RAM |
| `-nographic` | Без GUI, только serial |
| `-no-reboot` | Не перезагружать при panic |
| `-d int -D qemu.log` | Логирование прерываний |

---

## Тестирование при загрузке

При инициализации ядро выполняет авто-тесты:

```
[MEM] free pages: 61280
[MEM] palloc test: 0x158000 0x159000
[TIMER] ticks: 0 -> 100000, delta ticks: 100000 (expect 100k), delta ms: 10
[GOP] init...
[ACPI] RSDP saved at 0x...
[CPU] ExitBootServices...
[CPU] Bare-metal mode
[CPU] GDT/IDT/PIC...
[CPU] Interrupts ready
[VM] CR3=0x...
[VM] map test: phys=0x... lookup=0x... OK
[BOOT] FAT ls:
         HELLO     13 bytes
  [DIR]   TEST
[BOOT] FAT cat NVVARS:
NVME DISK
--- end ---
[BOOT] FS test: mkdir /test
[FS] mkdir OK
[BOOT] FS test: write /test/hello.txt
[FS] write OK
[BOOT] Ping gateway via ICMP...
[NET] sending ARP...
[BOOT] Ping done
=== Cooperative test ===
[A] start
[MAIN] spawned id=1 sp=... [RIP]=... [CS]=... [RFL]=...
[MAIN] before yield
[A] resumed
[MAIN] first back
[MAIN] second back
=== done ===
=== Preemption test ===
[PREEMPT] 4 workers spawned, waiting 2s...
  Worker 0: 12345678 iterations
  Worker 1: 12345678 iterations
  Worker 2: 12345678 iterations
  Worker 3: 12345678 iterations
=== Preemption done ===
[RING3] test start
[RING3] spawned: A=1 B=2
[E1000] spawning userspace driver...
[E1000] spawned task id=3
[CONSOLE] spawning server...
[CONSOLE] spawned id=4 cap=0
```

---

## ABI и серверы

### Структура Message

```rust
pub struct Message {
    pub src_port: u16,    // Порт отправителя
    pub dst_port: u16,    // Порт назначения
    pub msg_type: u16,    // MsgType enum
    pub flags: u16,       // Флаги (IPC_NONBLOCK, IPC_SHMEM)
    pub length: u16,      // Длина данных (0-64)
    pub shmem_cap: u16,   // Индекс shared-mem capability (0 = нет)
    pub data: [u8; 64],   // Inline payload (64 bytes)
}
```

### Ports

| Port | Значение | Назначение |
|------|----------|------------|
| `PORT_KERNEL` | 0 | Ядро |
| `PORT_CONSOLE` | 1 | Консольный сервер |

### IPC Error Codes

| Code | Значение | Описание |
|------|----------|----------|
| `IPC_OK` | 0 | Успех |
| `IPC_ERR_BAD_CAP` | -1 | Неверный capability |
| `IPC_ERR_NO_SERVER` | -2 | Сервер не найден |
| `IPC_ERR_TIMEOUT` | -3 | Таймаут |
| `IPC_ERR_NO_MEM` | -4 | Нет памяти |
| `IPC_ERR_DENIED` | -5 | Нет прав (bad capability) |

---

## Диаграмма загрузки

```
UEFI efi_main()
    │
    ▼
dbsos_kernel::init()
    │
    ├─ uefi::helpers::init()
    │
    ├─ ipc::init()                    // Capability manager
    ├─ memory::init()                  // UEFI memory map → bitmap allocator
    ├─ timer::init()                   // HPET @ 0xFED00000
    ├─ driver::init()                  // Register: UART, PCI, e1000
    │
    ├─ [TEST] memory palloc
    ├─ [TEST] timer usleep(10000)
    │
    ├─ driver::ahci::init()           // SATA controller
    ├─ driver::nvme::init()           // NVMe controller
    │
    ├─ display::init()                // UEFI GOP
    ├─ display::draw_str("DBSos v0.1")
    │
    ├─ ACPI: save RSDP from UEFI config
    ├─ ACPI: copy_tables()
    │
    ├─ uefi::boot::exit_boot_services()
    │
    ├─ interrupts::init()             // GDT, IDT, PIC remap
    ├─ vm::init()                     // CR3 save
    ├─ acpi::init()                   // FADT parsing
    │
    ├─ [TEST] VM: create + clone + map
    │
    ├─ syscall::init()                // MSR setup (LSTAR, STAR, FMASK)
    │
    ├─ driver::net::tx_test()
    ├─ fs::ls()                       // FAT listing
    ├─ fs::cat(b"NVVARS")
    ├─ fs::mkdir / fs::write / fs::rm / fs::rmdir
    │
    ├─ driver::net::rx_software_test()
    ├─ driver::net::send_icmp_ping([10,0,2,1])
    │
    ├─ scheduler::init()
    ├─ ipc::tests::run_test()
    ├─ ipc::shmem_test()
    ├─ scheduler::test()              // Cooperative test
    │
    ├─ scheduler::lapic_timer_init()  // ~100 Hz preemptive
    ├─ driver::ps2::init()            // PS/2 keyboard IRQ1
    │
    ├─ scheduler::preempt_test()      // 4 workers, 2s
    │
    ├─ syscall::test_ring3()          // Sender, Receiver, e1000, Console
    │
    ├─ elf::load_and_spawn("/test/hello.elf")
    │
    └─ shell::run()                   // Interactive loop
```

---

## Диаграмма архитектуры

```
┌──────────────────────────────────────────────────────────────────┐
│                        DBSos Architecture                        │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────┐   ┌─────────────┐   ┌─────────────┐             │
│  │   USER      │   │   USER      │   │   USER      │             │
│  │   Ring 3    │   │   Ring 3    │   │   Ring 3    │             │
│  │  Console    │   │  e1000      │   │  ELF Binary │             │
│  │  Server     │   │  Driver     │   │  (spawned)  │             │
│  └──────┬──────┘   └──────┬──────┘   └──────┬──────┘             │
│         │                 │                  │                   │
│         └─────────────────┼──────────────────┘                   │
│                           │ IPC (capabilities)                   │
│                    ┌──────▼──────┐                               │
│                    │    IPC      │                               │
│                    │   Manager   │                               │
│                    └──────┬──────┘                               │
│                           │                                      │
│  ┌────────────────────────▼─────────────────────────────────┐    │
│  │                    KERNEL (Ring 0)                       │    │
│  │                                                          │    │
│  │  ┌────────┐  ┌────────┐  ┌──────────┐  ┌────────────┐    │    │
│  │  │ Memory │  │  VM    │  │Scheduler │  │  Interrupts│    │    │
│  │  │ Bitmap │  │PML4/   │  │ Task     │  │ GDT/IDT/   │    │    │
│  │  │ Alloc  │  │Pages   │  │ States   │  │ LAPIC/HPET │    │    │
│  │  └───┬────┘  └────┬───┘  └────┬─────┘  └──────┬─────┘    │    │
│  │      │            │           │                │         │    │
│  │  ┌───▼────────────▼───────────▼────────────────▼────┐    │    │
│  │  │              Core Subsystem                      │    │    │
│  │  │  ┌─────────┐  ┌────────┐  ┌────────┐  ┌───────┐  │    │    │
│  │  │  │  Syscall│  │  FS    │  │  ACPI  │  │  ELF  │  │    │    │
│  │  │  │ (0F 05) │  │ FAT16  │  │ Tables │  │ Loader│  │    │    │
│  │  │  └─────────┘  └────────┘  └────────┘  └───────┘  │    │    │
│  │  └──────────────────────────────────────────────────┘    │    │
│  │                                                          │    │
│  │  ┌──────────────────────────────────────────────────┐    │    │
│  │  │              Driver Subsystem                    │    │    │
│  │  │  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐    │    │    │
│  │  │  │ UART │ │ PCI  │ │ AHCI │ │ NVMe │ │ e1000│    │    │    │
│  │  │  │ 16550│ │ Bus  │ │ SATA │ │ SSD  │ │ NIC  │    │    │    │
│  │  │  └──────┘ └──────┘ └──────┘ └──────┘ └──────┘    │    │    │
│  │  │  ┌──────┐                                        │    │    │
│  │  │  │ PS/2 │                                        │    │    │
│  │  │  │ KB   │                                        │    │    │
│  │  │  └──────┘                                        │    │    │
│  │  └──────────────────────────────────────────────────┘    │    │
│  │                                                          │    │
│  │  ┌──────────────────────────────────────────────────┐    │    │
│  │  │           Display & Console                      │    │    │
│  │  │  GOP (UEFI) + VGA Font 8x16 + Logo               │    │    │
│  │  └──────────────────────────────────────────────────┘    │    │
│  └──────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐    │
│  │                    Shell (Interactive)                   │    │
│  │  help │ mem │ time │ clear │ info │ ls │ cat │ mkdir     │    │
│  │  write │ rm │ rmdir │ exec │ ping │ nvme │ reboot        │    │
│  └──────────────────────────────────────────────────────────┘    │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

---

## Текущее состояние

### Реализовано

| Компонент | Статус | Примечания |
|-----------|--------|------------|
| **UEFI Boot** | ✅ | Загрузка через Limine + OVMF |
| **Physical Memory** | ✅ | Bitmap allocator, palloc/pfree/palloc_n |
| **Virtual Memory** | ✅ | 4-level paging, huge pages, split |
| **Scheduler** | ✅ | Preemptive, LAPIC timer, FPU save/restore |
| **Interrupts** | ✅ | GDT, IDT, PIC remap, exception handlers |
| **Syscall** | ✅ | SYSCALL/SYSRET, 13+ системных вызовов |
| **IPC** | ✅ | Capabilities, blocking send/recv, shared memory |
| **Capability Manager** | ✅ | 64 slots/task, alloc/duplicate/transfer |
| **FAT16 FS** | ✅ | Read/write/mkdir/rm/rmdir, VFAT LFN |
| **NVMe Driver** | ✅ | Admin + I/O submission, read/write sectors |
| **AHCI Driver** | ✅ | SATA command list, read/write sectors |
| **e1000 NIC** | ✅ | TX/RX rings, ARP, ICMP ping |
| **PS/2 Keyboard** | ✅ | IRQ1, poll_char |
| **UART** | ✅ | COM1, 115200 baud |
| **PCI Bus** | ✅ | Config read/write, MMIO validation |
| **HPET Timer** | ✅ | 10 MHz, usleep/millis/ticks |
| **ACPI** | ✅ | RSDP/XSDT, reboot/shutdown |
| **ELF Loader** | ✅ | ET_EXEC, PT_LOAD, spawn Ring 3 |
| **Ring 3 Tasks** | ✅ | Console server, e1000 driver, test binaries |
| **Display** | ✅ | GOP, draw_str, draw_rect, logo |
| **Shell** | ✅ | 15+ commands, PS/2 + UART input |
| **Preemption Test** | ✅ | 4 workers, LAPIC timer |
| **Cooperative Test** | ✅ | spawn/yield/exit |
| **Shared Memory** | ✅ | Zero-copy test |
| **Spinlock** | ✅ | AtomicBool + irqsave, SpinGuard |

### В доработке

| Компонент | Статус | Приоритет | Примечания |
|-----------|--------|-----------|------------|
| **Page Fault Handler** | ⚠️ | Высокий | Есть exception stub, нет специализированного обработчика |
| **FS Server (Ring 3)** | ⚠️ | Средний | FAT работает в ring-0, нужен перенос в ring-3 |
| **TCP/IP Stack** | ❌ | Высокий | Только ARP + ICMP, нет TCP/UDP |
| **DHCP/DNS** | ❌ | Средний | Автоматическая настройка сети |
| **Multi-core (SMP)** | ❌ | Средний | Одно ядро, нет IPI |
| **Backtrace** | ❌ | Низкий | Нет вывода stack trace при panic |
| **CI/CD** | ❌ | Низкий | Нет автоматизации сборки |

---

## Автор и лицензия

**Автор:** i000993i

Этот проект лицензирован по [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/deed.ru).

| | Описание |
|---|----------|
| ✅ **Можно** | использовать, изменять, распространять, коммерческое использование |
| ⚠️ **Условие** | указывать автора (i000993i) и показывать внесённые изменения |
| ⚠️ **Распространение** | новые версии должны быть под той же лицензией (ShareAlike) |

---

## Источники

| Ресурс | Ссылка |
|--------|--------|
| **x86 SDM** | [Intel 64 and IA-32 Architectures Software Developer's Manual](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html) |
| **UEFI Spec** | [Unified Extensible Firmware Interface](https://uefi.org/specifications) |
| **Limine** | [GitHub](https://github.com/limine-bootloader/limine) |
| **Rust no_std** | [The Rustonomicon](https://doc.rust-lang.org/nomicon/) |
| **FAT16** | [Microsoft FAT32 Spec](https://learn.microsoft.com/en-us/windows/win32/filesystem/long-fat-file-system) |
| **e1000** | [Intel 82540EM/82545EM Programming Manual](https://www.intel.com/content/dam/www/public/us/en/documents/pci-ide-interrupt-manual.pdf) |
| **HPET** | [Intel HPET Spec](https://www.intel.com/content/dam/develop/external/us/en/documents/hpet-1-0a-spec.pdf) |
| **ACPI** | [ACPI Specification](https://acpi.info/specifications.htm) |
| **QEMU** | [QEMU Documentation](https://www.qemu.org/docs/) |

---

<p align="center">
  <i>DBSos v0.1 — Built with ❤️ and Rust</i><br>
  <sub>Author: i000993i | License: CC BY-SA 4.0</sub>
</p>
