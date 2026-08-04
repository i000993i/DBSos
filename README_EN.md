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
  <b>English</b> | <a href="README.md">Русский</a>
</p>

<h1 align="center">DBSos</h1>

<p align="center">
  <b>Custom microkernel operating system</b><br>
  Written in <a href="https://www.rust-lang.org/">Rust</a> for <b>x86_64</b> architecture<br>
  Boots via <b>UEFI</b> using the <a href="https://github.com/limine-bootloader/limine">Limine</a> bootloader
</p>

> **Project goal** — create a full-featured operating system in Rust with support for Linux and Windows 10 programs. Priorities: high security and high performance.

---

## Screenshots

### Boot Screen

<p align="center">
  <img src="README/photo-1.png" alt="DBSos Boot Screen" width="800">
  <br>
  <i>DBSos boot screen with "DBS" logo and shell prompt</i>
</p>

### Interactive Shell

<p align="center">
  <img src="README/photo-2.png" alt="DBSos Shell" width="800">
  <br>
  <i>Command list (help), ping execution (ARP), system info (info)</i>
</p>

---

## Quick Start

### Requirements

| Component | Version / Path |
|-----------|----------------|
| **Rust** | Nightly, target `x86_64-unknown-uefi` |
| **QEMU** | `qemu-system-x86_64` |
| **OVMF** | `edk2-x86_64-code.fd` |
| **OS** | Linux / Windows (PowerShell) |

### 1. Install Rust and target

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add UEFI target
rustup target add x86_64-unknown-uefi
```

### 2. Install QEMU

```bash
# Linux (Debian/Ubuntu)
sudo apt install qemu-system-x86 ovmf

# Linux (Arch)
sudo pacman -S qemu-full edk2-ovmf

# Windows
# Download from https://www.qemu.org/download/
```

### 3. Build the kernel

```bash
# Clone the repository
git clone https://github.com/i000993i/DBSos.git
cd DBSos

# Build the kernel
cargo build -p dbsos-kernel --target x86_64-unknown-uefi
```

### 4. Prepare ESP (EFI System Partition)

```bash
mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-uefi/debug/dbsos-kernel.efi esp/EFI/BOOT/BOOTX64.EFI
```

### 5. Run in QEMU

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

> **Note:** OVMF path may vary. Check: `find / -name "OVMF_CODE.fd" 2>/dev/null`

### Alternative: PowerShell (Windows)

```powershell
# Full build + disk creation + run
.\scripts\bootstrap.ps1 -Run

# Build only (no run)
.\scripts\bootstrap.ps1

# Rebuild disk (clean ESP)
.\scripts\bootstrap.ps1 -Clean
```

---

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                    Limine Bootloader                 │
│               (UEFI Application)                     │
├──────────────────────────────────────────────────────┤
│                    UEFI Firmware                     │
│         (GOP, Memory Map, ACPI Tables)               │
├──────────────────────────────────────────────────────┤
│  Ring 0 (Kernel)                                     │
│  ┌───────────────────────────────────────────────┐   │
│  │  init() — kernel entry point                  │   │
│  │  ├─ UEFI boot services                        │   │
│  │  ├─ Memory + VM (PML4)                        │   │
│  │  ├─ ACPI + Drivers (AHCI/NVMe)                │   │
│  │  ├─ ExitBootServices                          │   │
│  │  ├─ GDT/IDT/PIC/LAPIC                         │   │
│  │  ├─ IPC + Scheduler + Timer                   │   │
│  │  ├─ Syscall (SYSCALL/SYSRET)                  │   │
│  │  ├─ FS tests + Network tests                  │   │
│  │  └─ Shell (interactive loop)                  │   │
│  └───────────────────────────────────────────────┘   │
│                                                      │
│  Ring 3 (User Tasks)                                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐            │
│  │ Console  │  │ e1000    │  │ ELF      │            │
│  │ Server   │  │ Driver   │  │ Binary   │            │
│  └──────────┘  └──────────┘  └──────────┘            │
│        ↕ IPC (capabilities)                          │
├──────────────────────────────────────────────────────┤
│  Hardware / QEMU Emulation                           │
│  LAPIC, HPET, NVMe, AHCI, e1000 NIC, PS/2            │
└──────────────────────────────────────────────────────┘
```

### Key Principles

| Principle | Description |
|-----------|-------------|
| **Microkernel** | Minimal kernel (memory, IPC, scheduler). Servers run in Ring 3 |
| **Capability-based** | Resource access only through capabilities (IPC, MMIO, shared memory) |
| **no_std** | Kernel does not use Rust's standard library |
| **Paging 4-level** | Full x86-64 page table support (PML4 → PDPT → PD → PT) |
| **Preemptive** | Preemptive multitasking based on LAPIC timer (~100 Hz) |
| **ELF loading** | User binaries loaded from FAT file system |

---

## Kernel Components

### `memory.rs` — Physical Memory

| Component | Description |
|-----------|-------------|
| **PhysAllocator** | Bitmap allocator for physical pages |
| **Sources** | UEFI Memory Map (MemoryType::CONVENTIONAL) |
| **palloc()** | Allocate one page (4 KB) |
| **palloc_n(n)** | Allocate `n` contiguous pages |
| **pfree(phys)** | Free page, return to pool |
| **Bitmap** | Placed at start of found memory region, does not overlap with allocated memory |

### `vm.rs` — Virtual Memory (x86-64 Paging)

| Component | Description |
|-----------|-------------|
| **PML4** | 4-level page table: PML4 → PDPT → PD → PT → 4KB |
| **map_page()** | Map phys → virt, with automatic split 2MB → 4KB |
| **create_address_space()** | Create new PML4 with identity map |
| **switch_to()** | Load CR3 to switch address space |
| **clone_kernel_mappings()** | Clone kernel high-half (256-511) |
| **identity_map_2mb()** | Identity mapping via 2MB huge pages |
| **destroy_address_space()** | Free all page table pages |

**PTE flags:** `PRESENT`, `WRITABLE`, `USER`, `WRITE_THROUGH`, `CACHE_DISABLE`, `ACCESSED`, `DIRTY`, `HUGE`, `GLOBAL`, `NX`

### `scheduler.rs` — Scheduler

| Component | Description |
|-----------|-------------|
| **Task** | Task structure: state, stack, sp, id, IPC state, PML4, GDT, TSS, FPU buffer |
| **TaskState** | `Free`, `Ready`, `Running`, `Exited`, `BlockedSend`, `BlockedRecv` |
| **spawn()** | Create Ring 0 task (cooperative) |
| **spawn_user()** | Create Ring 3 task (user) |
| **reschedule()** | Context switch point (called from timer ISR) |
| **yield_now()** | Cooperative switch |
| **exit()** | Terminate task, free resources |
| **FPU save/restore** | `fxsave64` / `fxrstor64` for coprocessor state |
| **LAPIC timer** | ~100 Hz, vector 0x20, periodic mode |

**IRETQ frame** (for QEMU): `SS`, `RSP`, `RFLAGS`, `CS`, `RIP` + 15 GPRs (`RAX..R15`)

### `interrupts.rs` — Interrupts

| Component | Description |
|-----------|-------------|
| **IDT** | 256 entries, per-vector stubs for 0..31 |
| **GDT** | 6 segments: null, kernel code, kernel data, user data, user code, user data (SYSRET) |
| **PIC remap** | IRQ0-7 → 0x20-0x27, IRQ8-15 → 0x28-0x2F |
| **exception_common** | Unified exception handler with register dump |
| **timer_stub** | LAPIC timer handler (IRQ0), calls `reschedule()` |

### `syscall.rs` — System Calls (SYSCALL / SYSRET)

| MSR | Value | Purpose |
|-----|-------|---------|
| **IA32_EFER** | SCE=1 | Enable SYSCALL |
| **IA32_STAR** | 0x0008_0020_0000_0000 | CS/SS selectors |
| **IA32_LSTAR** | syscall_stub | Entry point |
| **IA32_FMASK** | Clear IF/DF/TF | RFLAGS mask on entry |

**System Call Table**

| Number | Name | Arg1 | Arg2 | Arg3 | Return |
|--------|------|------|------|------|--------|
| 0 | `SYS_EXIT` | exit_code | — | — | EXIT_MAGIC |
| 1 | `SYS_IPC_SEND_LEGACY` | dst_id | val | — | 0 / IPC_ERR |
| 2 | `SYS_IPC_RECV_LEGACY` | src_id | — | — | val / IPC_ERR |
| 3 | `SYS_GETPID` | — | — | — | pid |
| 4 | `SYS_WAIT` | pid (0=any) | status_ptr | — | pid / 0 |
| 5 | `SYS_KILL` | pid | signal | — | 1 / 0 |
| 6 | `SYS_FORK` | — | — | — | child_pid / 0 / -1 |
| 7 | `SYS_GETPGID` | pid (0=current) | — | — | pgid |
| 8 | `SYS_SETPGID` | pid (0=current) | pgid | — | 0 / IPC_ERR |
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

### `ipc.rs` — Inter-Process Communication

| Component | Description |
|-----------|-------------|
| **Message** | Message structure: `msg_type`, `dst_port`, `src_port`, `data[8]`, `length` |
| **send_with_cap()** | Send via capability with blocking |
| **recv_with_cap()** | Receive via capability with blocking |
| **deliver_message()** | Fast-path delivery (sender → waiting receiver) |
| **self-send** | Automatic routing via `PENDING[]` |
| **broadcast()** | Broadcast messaging |
| **Shared Memory** | Zero-copy mapping via capabilities |

### `cap.rs` — Capability Manager

| Component | Description |
|-----------|-------------|
| **Cap** | `cap_type`, `server_id`, `rights`, `data` |
| **CapTable** | 64 slots per task |
| **alloc()** | Allocate capability for current task |
| **duplicate()** | Copy capability to another task |
| **transfer()** | Transfer with revocation from sender |
| **free_all()** | Free all task capabilities |

**Types:** `IpcTarget` (1), `SharedMem` (2), `Irq` (3), `IpcReply` (4)
**Rights:** `CAP_SEND` (1), `CAP_RECV` (2), `CAP_READ` (4), `CAP_WRITE` (8)

### `timer.rs` — Timers

| Component | Description |
|-----------|-------------|
| **HPET** | High Precision Event Timer @ 0xFED00000, 10 MHz |
| **usleep(us)** | Busy-wait based on HPET counter |
| **millis()** | Milliseconds from HPET |
| **ticks()** | Raw HPET ticks |
| **LAPIC timer** | ~100 Hz, used for preemptive scheduling |

### `fs.rs` — FAT16 Filesystem

| Function | Description |
|----------|-------------|
| **ls()** | List root directory |
| **ls_path(path)** | List directory by path |
| **cat(name)** | Print file content to UART |
| **cat_path(path)** | Same, with path support |
| **read_file()** | Read file into buffer |
| **mkdir(path)** | Create directory |
| **write_file(path, data)** | Write file (create + overwrite) |
| **rm(path)** | Delete file |
| **rmdir(path)** | Delete empty directory |
| **VFAT LFN** | Long File Names support (up to 255 characters) |
| **Path resolution** | `/parent/child/file` — recursive resolution |

### `elf.rs` — ELF Loader

| Component | Description |
|-----------|-------------|
| **load_and_spawn()** | Load ELF from FAT, map into memory, create Ring 3 task |
| **Header parsing** | ELF64, `ET_EXEC`, `PT_LOAD` segments |
| **Mapping** | `PTE_USER | PTE_WRITABLE` for code and data |

### `display.rs` — UEFI GOP Display

| Component | Description |
|-----------|-------------|
| **draw_str()** | Print string with color (RGB 888) |
| **draw_rect()** | Fill rectangle |
| **clear_screen()** | Clear screen |
| **set_cursor()** | Set cursor position (column, row) |

### `console.rs` — Console Output

| Component | Description |
|-----------|-------------|
| **write_str()** | Print string to display (via GOP) |
| **putchar()** | Print single character |
| **clear_screen()** | Clear screen |
| **screen_cols()** | Screen width in characters |
| **draw_rect()** | Draw rectangle |
| **draw_logo()** | Draw DBSos logo (3x scale, gold on dark red) |

### `acpi.rs` — ACPI

| Component | Description |
|-----------|-------------|
| **RSDP lookup** | Search in UEFI config tables (ACPI2_GUID priority) |
| **copy_tables()** | Copy tables before ExitBootServices |
| **init()** | Parse RSDT/XSDT, FADT |
| **reboot()** | ACPI reboot via RESET_REG |
| **shutdown()** | ACPI poweroff |

### `driver/` — Driver Subsystem

| Module | Description |
|--------|-------------|
| **traits.rs** | Abstract interface: `Driver` trait with `name()`, `device_type()`, `init()` methods |
| **manager.rs** | Driver registry, registration and initialization |
| **mod.rs** | Driver list: UART, PCI, e1000 |

---

## Drivers

### `uart.rs` — Serial Port (16550A)

| Parameter | Value |
|-----------|-------|
| **Port** | COM1: `0x3F8` |
| **Baud** | 115200 |
| **Functions** | `write_str()`, `putchar()`, `poll_char()` |
| **Usage** | Primary debug and output channel |

### `pci.rs` — PCI Bus

| Function | Description |
|----------|-------------|
| **read32(bus, dev, func, offset)** | PCI config space read |
| **write32(bus, dev, func, offset, val)** | PCI config space write |
| **validate_mmio()** | Validate MMIO region (only known PCI BARs) |

### `net.rs` — Intel e1000e NIC (82574L)

| Component | Description |
|-----------|-------------|
| **MMIO BAR0** | 0x1000–0x2FFF (256 registers) |
| **TX Ring** | 16 descriptors (16 bytes each, legacy format) |
| **RX Ring** | 16 descriptors (16 bytes each, legacy format) |
| **send_packet()** | Send packet via TX ring |
| **poll()** | Poll RX ring for incoming packets |
| **send_icmp_ping()** | Generate and send ICMP Echo Request |
| **send_arp_request()** | Generate and send ARP request |
| **tx_test()** | TX loopback test |
| **rx_software_test()** | Inject fake ARP into RX ring |
| **dump_rx_state()** | Debug: RX ring state |

**MAC address:** `52:54:00:12:34:56` (QEMU default)
**Gateway:** `10.0.2.1` (QEMU user-mode NAT)

### `ahci.rs` — AHCI SATA

| Parameter | Value |
|-----------|-------|
| **MMIO BAR** | PCI config read BAR0 |
| **BPS** | 512 bytes per sector |
| **SPC** | 8 sectors per cluster |
| **FATS** | 2 FAT copies |
| **ROOT_ENT** | 512 root directory entries |
| **RESERVED** | 1 reserved sector |
| **FAT_SZ** | 126 sectors per FAT |
| **read_fat_sector()** | Read sector via AHCI command list |
| **write_fat_sector()** | Write sector via AHCI |

### `nvme.rs` — NVMe SSD

| Parameter | Value |
|-----------|-------|
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
| **read_fat_sector()** | Wrapper for FS |
| **write_fat_sector()** | Wrapper for FS |

### `ps2.rs` — PS/2 Keyboard

| Function | Description |
|----------|-------------|
| **init()** | Initialize PS/2 controller (IRQ1) |
| **poll_char()** | Non-blocking scancode read |
| **Handler** | IRQ1 → Decode → UART echo |

---

## User Mode (Ring 3)

### Entry/Exit Mechanism

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

### Creating a Ring 3 Task

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

### Test Tasks

| Task | Description | Entry |
|------|-------------|-------|
| **Sender (A)** | Ring 3 IPC sender: LOG_WRITE + infinite loop | `0x100000000` |
| **Receiver (B)** | Ring 3 IPC receiver: LOG_WRITE + infinite loop | `0x100001000` |
| **e1000 driver** | Userspace NIC driver: PCI config read → MMIO map → MAC/STATUS read | `0x100002000` |
| **Console server** | Ring 3 IPC server: IPC_RECV → LOG_WRITE → loop | `0x100003000` |

---

## File System

### Supported Operations

```
ls                    # list root
ls /path              # list directory
cat file              # print file content
cat /path/to/file     # print with path
mkdir /path           # create directory
write /path "text"    # write file
rm /path              # delete file
rmdir /path           # delete empty directory
```

### NVMe Image Structure

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

## Interactive Shell

### Commands

| Command | Description |
|---------|-------------|
| `help` | Show help |
| `info` | System information |
| `mem` | Memory statistics |
| `time` | Timer test (10ms delay) |
| `clear` | Clear screen |
| `sleep N` | Block for N ms (scheduler) |
| `ls [PATH]` | List directory |
| `cat PATH` | Print file content |
| `mkdir PATH` | Create directory |
| `write PATH TEXT` | Write text file |
| `rm PATH` | Delete file |
| `rmdir PATH` | Delete directory |
| `exec PATH` | Run ELF binary |
| `ping` | ARP ping gateway (10.0.2.1) |
| `nvme info` | NVMe disk information |
| `nvme read LBA [COUNT]` | Read sectors |
| `nvme write LBA [COUNT]` | Write sectors |
| `reboot` | ACPI reboot |
| `poweroff` | ACPI shutdown |

---

## QEMU Parameters

| Flag | Description |
|------|-------------|
| `-machine q35` | Q35 chipset |
| `-drive if=pflash` | UEFI firmware (OVMF) |
| `-drive file=fat:rw:esp` | ESP with BOOTX64.EFI |
| `-device nvme` | NVMe disk |
| `-nic user,model=e1000` | e1000 NIC (user-mode NAT, gateway 10.0.2.1) |
| `-m 256M` | 256 MB RAM |
| `-nographic` | No GUI, serial only |
| `-no-reboot` | Do not reboot on panic |
| `-d int -D qemu.log` | Interrupt logging |

---

## Boot Testing

During initialization, the kernel performs auto-tests:

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

## ABI and Servers

### Message Structure

```rust
pub struct Message {
    pub src_port: u16,    // Sender port
    pub dst_port: u16,    // Destination port
    pub msg_type: u16,    // MsgType enum
    pub flags: u16,       // Flags (IPC_NONBLOCK, IPC_SHMEM)
    pub length: u16,      // Data length (0-64)
    pub shmem_cap: u16,   // Shared-mem capability index (0 = none)
    pub data: [u8; 64],   // Inline payload (64 bytes)
}
```

### Ports

| Port | Value | Purpose |
|------|-------|---------|
| `PORT_KERNEL` | 0 | Kernel |
| `PORT_CONSOLE` | 1 | Console server |

### IPC Error Codes

| Code | Value | Description |
|------|-------|-------------|
| `IPC_OK` | 0 | Success |
| `IPC_ERR_BAD_CAP` | -1 | Invalid capability |
| `IPC_ERR_NO_SERVER` | -2 | Server not found |
| `IPC_ERR_TIMEOUT` | -3 | Timeout |
| `IPC_ERR_NO_MEM` | -4 | Out of memory |
| `IPC_ERR_DENIED` | -5 | No permissions (bad capability) |

---

## Boot Diagram

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

## Architecture Diagram

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

## Current Status

### Implemented

| Component | Status | Notes |
|-----------|--------|-------|
| **UEFI Boot** | ✅ | Boot via Limine + OVMF |
| **Physical Memory** | ✅ | Bitmap allocator, palloc/pfree/palloc_n |
| **Virtual Memory** | ✅ | 4-level paging, huge pages, split |
| **Scheduler** | ✅ | Preemptive, LAPIC timer, FPU save/restore |
| **Interrupts** | ✅ | GDT, IDT, PIC remap, exception handlers |
| **Syscall** | ✅ | SYSCALL/SYSRET, 13+ system calls |
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

### In Progress

| Component | Status | Priority | Notes |
|-----------|--------|----------|-------|
| **Page Fault Handler** | ⚠️ | High | Exception stub exists, no specialized handler |
| **FS Server (Ring 3)** | ⚠️ | Medium | FAT works in ring-0, needs migration to ring-3 |
| **TCP/IP Stack** | ❌ | High | Only ARP + ICMP, no TCP/UDP |
| **DHCP/DNS** | ❌ | Medium | Automatic network configuration |
| **Multi-core (SMP)** | ❌ | Medium | Single core, no IPI |
| **Backtrace** | ❌ | Low | No stack trace output on panic |
| **CI/CD** | ❌ | Low | No build automation |

---

## Author and License

**Author:** i000993i

This project is licensed under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/deed.en).

| | Description |
|---|-------------|
| ✅ **Allowed** | Use, modify, distribute, commercial use |
| ⚠️ **Condition** | Must credit author (i000993i) and show changes made |
| ⚠️ **Distribution** | New versions must be under the same license (ShareAlike) |

---

## Sources

| Resource | Link |
|----------|------|
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
