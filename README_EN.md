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
  Boots directly via <b>UEFI</b> (no separate bootloader)
</p>

> **Project goal** — build a full-featured OS in Rust with Linux binary support, graphical desktop, and Linux API compatibility.

---

## Screenshots

<p align="center">
  <img src="README/photo-1.png" alt="DBSos Boot Screen" width="800">
  <br><i>Boot screen with "DBS" logo and shell prompt</i>
</p>

<p align="center">
  <img src="README/photo-2.png" alt="DBSos Shell" width="800">
  <br><i>GNOME Shell desktop (F1 — desktop, F2 — text console)</i>
</p>

---

## Quick Start

```bash
# 1. Install Rust + target
rustup target add x86_64-unknown-uefi

# 2. Clone repository
git clone https://github.com/i000993i/DBSos.git
cd DBSos

# 3. Build and run (Linux/macOS)
./run.sh

# Or manually:
cargo build -p dbsos-kernel --target x86_64-unknown-uefi
python3 scripts/mk_esp.py        # Creates ESP image (GPT+FAT16)
# Then launch QEMU (see below)
```

### Running in QEMU

```bash
qemu-system-x86_64 \
  -machine q35 \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
  -drive file=esp.img,format=raw,id=espblk,if=none \
  -device ide-hd,drive=espblk \
  -drive file=nvme_disk.img,if=none,id=nvme0,format=raw \
  -device nvme,serial=deadbeef,drive=nvme0 \
  -display gtk \
  -m 256M -smp 2 \
  -serial stdio -monitor none \
  -nic user,model=e1000
```

> OVMF path: `find / -name "OVMF_CODE*.fd" 2>/dev/null`
> Set `OVMF=/path/to/OVMF_CODE.fd ./run.sh` if OVMF is not in the default location.

### Controls

- **F1** — graphical desktop (GNOME Shell)
- **F2** — text console (System Console)
- In text console: `gui` — return to desktop

---

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│  Ring 3 (User)                                           │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐           │
│  │ Linux ELF  │ │ Wayland    │ │ GUI Apps   │           │
│  │ Binaries   │ │ Compositor │ │ (Terminal, │           │
│  │            │ │            │ │  Editor,   │           │
│  │            │ │            │ │  Files)    │           │
│  └─────┬──────┘ └─────┬──────┘ └─────┬──────┘           │
│        └──────────────┼──────────────┘                   │
│                       │ IPC (capabilities)               │
├───────────────────────┼──────────────────────────────────┤
│  Ring 0 (Kernel)      ▼                                  │
│  ┌───────────────────────────────────────────────────┐   │
│  │ Core: Memory │ VM │ Scheduler │ Syscall │ VFS     │   │
│  │ Drivers: UART │ PCI │ NVMe │ e1000 │ PS/2 │ HPET │   │
│  │ Display: GOP Framebuffer │ GNOME Shell Desktop     │   │
│  │ Linux Compat: ELF Loader │ Syscall Translation     │   │
│  └───────────────────────────────────────────────────┘   │
│  Boot: UEFI → ExitBootServices → Shell / GUI             │
└──────────────────────────────────────────────────────────┘
```

**Key principles:**
- **Microkernel** — minimal kernel, servers in Ring 3
- **Capability-based** — resource access through capabilities
- **no_std** — no Rust standard library (~20,600 lines)
- **Preemptive** — multitasking via LAPIC timer (~100 Hz)
- **Demand Paging** — process heap/stack mapped on demand via VMAs
- **Linux Compatibility** — Linux ELF binary loader, syscall translation

---

## Drivers & Subsystems

### Device Drivers

| Driver | Description | Status |
|--------|-------------|--------|
| **UART 16550** | Serial port COM1, IRQ4 | ✅ |
| **PCI** | Bus scanner, config space | ✅ |
| **NVMe** | SSD/NVMe disks, admin/IO queues | ✅ |
| **AHCI/SATA** | SATA disks (AT API) | ⚠️ Hangs on port_init |
| **e1000** | Gigabit Ethernet, DMA TX/RX, ARP/ICMP | ✅ |
| **PS/2 Keyboard** | Scancode set 1 → ASCII, IRQ1 | ✅ |
| **PS/2 Mouse** | 3-byte packets, IRQ12 | ✅ |
| **HPET Timer** | High-precision timer 10 MHz | ✅ |
| **LAPIC Timer** | Local APIC timer ~100 Hz | ✅ |

### Network Stack

| Component | Description | Status |
|-----------|-------------|--------|
| **Ethernet** | e1000 DMA rings, MAC/PHY | ✅ |
| **ARP** | ARP table, IP→MAC resolution | ✅ |
| **IPv4** | IP framing, routing | ✅ |
| **ICMP** | Ping (echo request/reply) | ✅ |
| **UDP** | User Datagram Protocol | ✅ |
| **DHCP** | DISCOVER→OFFER→REQUEST→ACK | ✅ |
| **DNS** | A record queries (UDP:53) | ✅ |
| **TCP** | 3-way handshake, retransmission, FIN | ✅ |

### Filesystem

| Component | Description | Status |
|-----------|-------------|--------|
| **VFS** | Virtual filesystem, mount/fd tables | ✅ |
| **FAT12/16/32** | BPB parsing, cluster chains | ✅ |
| **VFAT LFN** | Long File Name support | ✅ |
| **fs_server** | File server in Ring 3 (IPC) | ✅ |

### Graphical Interface

| Component | Description | Status |
|-----------|-------------|--------|
| **UEFI GOP** | Framebuffer 1280×800 BGRx | ✅ |
| **GNOME Shell** | Top panel, Dash dock, Activities overview | ✅ |
| **Window Manager** | Drag/resize, z-order, decorations | ✅ |
| **Cursor** | Save/restore overlay, 12×18 pixels | ✅ |
| **Wayland** | Protocol, surfaces, compositor loop | ⚠️ Skeleton |
| **egui** | Immediate-mode GUI (custom, no_std) | ✅ |

### Kernel

| Component | Description | Status |
|-----------|-------------|--------|
| **Physical Memory** | Bitmap allocator, palloc/pfree | ✅ |
| **Virtual Memory** | 4-level paging (PML4→PDPT→PD→PT) | ✅ |
| **Heap** | First-fit free-list, 16-byte align | ✅ |
| **Scheduler** | Round-robin, preemptive (LAPIC) | ✅ |
| **Context Switch** | Save/restore registers, ring 0↔3 | ✅ |
| **GDT/IDT/PIC** | Full interrupt setup | ✅ |
| **Syscall** | ~30 syscalls (read/write/open/mmap...) | ✅ |
| **IPC** | Capability-based, send/recv/broadcast | ✅ |
| **Demand Paging** | VMA, page fault handler, lazy allocation | ✅ |
| **ACPI** | RSDP/XSDT/FADT/MADT, reboot/shutdown | ✅ |

### Linux Compatibility Layer

| Component | Description | Status |
|-----------|-------------|--------|
| **ELF Loader** | Linux x86_64 ELF binary loading | ✅ |
| **Syscall Translation** | 80+ Linux syscalls → DBSos operations | ✅ |
| **LinuxTask** | Process with Linux-compatible ABI | ✅ |
| **Memory (VMA)** | mmap/brk/demand paging Linux-style | ⚠️ Basic |

---

## Shell Commands

| Command | Description | Command | Description |
|---------|-------------|---------|-------------|
| `help` | Show help | `tcp IP PORT TXT` | TCP client |
| `info` | System info | `wget URL` | HTTP/1.0 download |
| `mem` | Memory stats | `pkg install NAME` | Install package |
| `time` | Timer test | `pkg remove NAME` | Remove package |
| `clear` | Clear screen | `pkg list` | List packages |
| `ls [PATH]` | List files | `pkg update` | Update registry |
| `cat PATH` | Print file | `exec PATH` | Run ELF binary |
| `mkdir PATH` | Create dir | `ping` | ARP ping gateway |
| `write PATH TXT` | Write file | `nvme info` | NVMe disk info |
| `rm PATH` | Delete file | `reboot` | ACPI reboot |
| `cp SRC DST` | Copy file | `poweroff` | ACPI shutdown |
| `mv SRC DST` | Move file | `net` | Network settings |

---

## Current Status

### Working ✅ (30+ components)

| Component | Component | Component |
|-----------|-----------|-----------|
| UEFI Boot (direct) | FAT12/16/32 (R/W) | NVMe Driver |
| Physical Memory (bitmap) | VFAT LFN | e1000 NIC (ARP/ICMP) |
| Virtual Memory (4-level paging) | VFS mount/fd | PS/2 Keyboard + Mouse |
| Preemptive Scheduler | ELF Loader | UART 16550A |
| GDT / IDT / PIC / LAPIC | IPC (capabilities) | HPET Timer |
| SYSCALL / SYSRET (~30) | Shared Memory | ACPI (reboot/shutdown) |
| Demand Paging (VMA) | TCP/IP Stack | DHCP / DNS |
| Interactive Shell (15+ cmds) | FS Server (Ring 3) | GNOME Shell Desktop |
| Script Interpreter | Package Manager | Display (GOP framebuffer) |

### Recent Fixes

- ✅ **e1000 EOI fix** — IRQ11 now correctly terminates on PIC slave
- ✅ **Mouse IRQ fix** — cli/sti during init, data reporting enabled
- ✅ **57 warnings** → 0 warnings
- ✅ **Shell redesign** — GNOME-style interface, save/restore cursor

### In Progress ⚠️

| Component | Issue | Priority |
|-----------|-------|----------|
| Mouse IRQ (after init) | Testing with e1000 fix | 🔴 High |
| AHCI/SATA | Hangs on port_init | 🔴 High |
| FAT mount | Can't find FAT on SATA/NVMe | 🔴 High |
| Screen flicker | No double buffering | 🟡 Medium |
| Multi-core (SMP) | AP wild after SIPI | 🟡 Medium |
| Wayland Compositor | Basic, no clients | 🟢 Low |
| Linux ELF binaries | Testing with real binaries | 🟢 Low |
| Backtrace | Not implemented | 🟢 Low |

---

## Project Structure

```
DBSos/
├── kernel/
│   └── src/
│       ├── lib.rs              # Kernel entry point
│       ├── main.rs             # UEFI entry point
│       ├── driver/             # Drivers (14 files, ~4,400 lines)
│       │   ├── pci.rs          # PCI bus scan
│       │   ├── nvme.rs         # NVMe SSD
│       │   ├── net.rs          # e1000 Ethernet
│       │   ├── tcp.rs          # TCP/IP stack
│       │   ├── ps2.rs          # PS/2 keyboard
│       │   ├── mouse.rs        # PS/2 mouse
│       │   └── ...
│       ├── gui/                # GUI (8 files, ~2,200 lines)
│       │   ├── mod.rs          # Main loop, app launcher
│       │   ├── shell.rs        # GNOME Shell desktop
│       │   ├── wm.rs           # Window manager
│       │   ├── render.rs       # Drawing primitives
│       │   ├── egui.rs         # Immediate-mode GUI
│       │   └── ...
│       ├── linux/              # Linux compatibility (5 files, ~1,400 lines)
│       │   ├── elf.rs          # Linux ELF loader
│       │   ├── syscall.rs      # Linux syscall translation
│       │   └── task.rs         # Linux process management
│       ├── wayland/            # Wayland compositor (6 files, ~950 lines)
│       ├── scheduler/          # Scheduler (10 files, ~1,400 lines)
│       ├── syscall/            # Syscall interface (5 files, ~1,700 lines)
│       └── ...
├── scripts/
│   ├── mk_esp.py              # Create ESP image (GPT+FAT16)
│   ├── mk_image.py            # Create data disk (NVMe)
│   └── tcp_echo_server.py     # TCP echo server for testing
├── run.sh                     # Build + Deploy + QEMU
└── esp.img                    # Boot ESP image
```

**Total:** 72 files, ~20,600 lines of Rust

---

## Author and License

**Author:** i000993i | **License:** [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/deed.en)

Use, modify, distribute (including commercially). Condition: credit author and show changes. New versions must use the same license.

---

## Sources

| Resource | Link |
|----------|------|
| x86 SDM | [Intel](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html) |
| UEFI Spec | [uefi.org](https://uefi.org/specifications) |
| Rust no_std | [Rustonomicon](https://doc.rust-lang.org/nomicon/) |
| FAT16 | [Microsoft](https://learn.microsoft.com/en-us/windows/win32/filesystem/long-fat-file-system) |
| e1000 | [Intel](https://www.intel.com/content/dam/www/public/us/en/documents/pci-ide-interrupt-manual.pdf) |
| NVMe | [nvmexpress.org](https://nvmexpress.org/specifications/) |
| ACPI | [acpi.info](https://acpi.info/specifications.htm) |
| Wayland | [wayland.freedesktop.org](https://wayland.freedesktop.org/docs/html/) |
| Linux ABI | [man7.org](https://man7.org/linux/man-pages/dir_section_2.html) |
| QEMU | [Documentation](https://www.qemu.org/docs/) |

---

<p align="center">
  <i>DBSos v0.1 — Built with ❤️ and Rust</i><br>
  <sub>Author: i000993i | License: CC BY-SA 4.0 | 72 files, ~20,600 lines of Rust</sub>
</p>
