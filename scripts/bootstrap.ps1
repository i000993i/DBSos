param(
    [switch]$Run,
    [switch]$Clean
)

$DBSos = Split-Path -Parent $PSScriptRoot
$Qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
$Ovmf = "C:\Program Files\qemu\share\edk2-x86_64-code.fd"

# Clean
if ($Clean) {
    cargo clean -p dbsos-kernel
    Remove-Item -Recurse -Force "$DBSos\esp" -ErrorAction SilentlyContinue
}

# Build kernel
Write-Host "=== Building DBSos kernel ===" -ForegroundColor Cyan
cargo build -p dbsos-kernel --target x86_64-unknown-uefi
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed!" -ForegroundColor Red
    exit 1
}

# Prepare ESP
Write-Host "=== Preparing ESP ===" -ForegroundColor Cyan
New-Item -ItemType Directory -Path "$DBSos\esp\EFI\BOOT" -Force | Out-Null
Copy-Item "$DBSos\target\x86_64-unknown-uefi\debug\dbsos-kernel.efi" `
         "$DBSos\esp\EFI\BOOT\BOOTX64.EFI" -Force

Write-Host "DBSos kernel ready: $(Get-Item "$DBSos\esp\EFI\BOOT\BOOTX64.EFI" | Select-Object -ExpandProperty Length) bytes" -ForegroundColor Green

# Prepare NVMe disk image (64 MB, MBR + FAT16)
$NvmeImg = "$DBSos\nvme_disk.img"
Write-Host "=== Creating NVMe disk image ===" -ForegroundColor Yellow

$MbrSig = 0xAA55
$PartStart = 2048       # partition starts at LBA 2048
$PartSectors = 129024   # partition is 63 MB

# FAT16 parameters for the partition
$Bps = 512
$Spc = 4                # sectors per cluster
$Reserved = 1           # reserved sectors (just the boot sector)
$FatCount = 2
$RootEnt = 512          # root directory entries
$RootSectors = $RootEnt * 32 / $Bps  # 32 sectors
$FatSectors = 126       # computed from cluster math
$TotalSectors = $PartSectors
$Media = 0xF8
$SecPerTrack = 32
$NumHeads = 64

# Boot sector data (sector = PartStart)
function Write-Ascii($w, $s) {
    $bytes = [System.Text.Encoding]::ASCII.GetBytes($s)
    $w.Write($bytes, 0, $bytes.Length)
}

function Write-Bpb {
    param($w, $lba)
    $w.Seek($lba * $Bps, [System.IO.SeekOrigin]::Begin) | Out-Null
    # Jump instruction
    $w.WriteByte(0xEB); $w.WriteByte(0x3C); $w.WriteByte(0x90)
    # OEM name
    Write-Ascii $w "MSDOS5.0"
    # BPB
    $w.Write([System.BitConverter]::GetBytes([uint16]$Bps), 0, 2)           # bps
    $w.WriteByte($Spc)                                                       # spc
    $w.Write([System.BitConverter]::GetBytes([uint16]$Reserved), 0, 2)       # reserved
    $w.WriteByte($FatCount)                                                  # fats
    $w.Write([System.BitConverter]::GetBytes([uint16]$RootEnt), 0, 2)        # root entries
    $w.Write([System.BitConverter]::GetBytes([uint16]0), 0, 2)               # total sectors small (0 means use large)
    $w.WriteByte($Media)                                                     # media
    $w.Write([System.BitConverter]::GetBytes([uint16]$FatSectors), 0, 2)     # sectors per FAT
    $w.Write([System.BitConverter]::GetBytes([uint16]$SecPerTrack), 0, 2)    # sectors per track
    $w.Write([System.BitConverter]::GetBytes([uint16]$NumHeads), 0, 2)       # heads
    $w.Write([System.BitConverter]::GetBytes([uint32]0), 0, 4)               # hidden sectors
    $w.Write([System.BitConverter]::GetBytes([uint32]$TotalSectors), 0, 4)   # total sectors large
    # Extended BPB
    $w.WriteByte(0x80)                                                       # drive number
    $w.WriteByte(0x00)                                                       # reserved
    $w.WriteByte(0x29)                                                       # boot signature
    $w.Write([System.BitConverter]::GetBytes([uint32]0x12345678), 0, 4)       # volume serial
    Write-Ascii $w "NVME DISK  "                                              # volume label (11 bytes)
    Write-Ascii $w "FAT16   "                                                 # FS type (8 bytes)
    # Boot code (just padding with zeros, boot signature at end of sector)
    $w.Seek($lba * $Bps + 0x1FE, [System.IO.SeekOrigin]::Begin) | Out-Null
    $w.WriteByte(0x55); $w.WriteByte(0xAA)
}

function Write-Fat {
    param($w, $lba, $numSectors, $dataClusters)
    $w.Seek($lba * $Bps, [System.IO.SeekOrigin]::Begin) | Out-Null
    # FAT[0] = media descriptor + 0xFF
    $w.WriteByte($Media); $w.WriteByte(0xFF); $w.WriteByte(0xFF); $w.WriteByte(0xFF)
    # FAT[1] = EOC (0xFFFF)
    $w.WriteByte(0xFF); $w.WriteByte(0xFF); $w.WriteByte(0xFF); $w.WriteByte(0xFF)
    # Initialize all remaining entries as free (0x0000)
    $remaining = $numSectors * $Bps - 8
    $zeros = New-Object byte[] $remaining
    $w.Write($zeros, 0, $remaining)
}

function Write-DirEntry {
    param($w, $name8, $ext3, $attr, $cluster, $size)
    Write-Ascii $w $name8                                     # name (8)
    Write-Ascii $w $ext3                                      # ext (3)
    $w.WriteByte($attr)                                       # attr
    $w.WriteByte(0)                                           # reserved (NT)
    $w.WriteByte(0)                                           # ctime tenths
    $w.Write([System.BitConverter]::GetBytes([uint16]0), 0, 2) # ctime
    $w.Write([System.BitConverter]::GetBytes([uint16]0), 0, 2) # cdate
    $w.Write([System.BitConverter]::GetBytes([uint16]0), 0, 2) # adate
    $w.Write([System.BitConverter]::GetBytes([uint16]0), 0, 2) # cluster high (FAT32, zero for FAT16)
    $w.Write([System.BitConverter]::GetBytes([uint16]0), 0, 2) # mtime
    $w.Write([System.BitConverter]::GetBytes([uint16]0), 0, 2) # mdate
    $w.Write([System.BitConverter]::GetBytes([uint16]$cluster), 0, 2)  # cluster low
    $w.Write([System.BitConverter]::GetBytes([uint32]$size), 0, 4)    # size
}

if (Test-Path $NvmeImg) { Remove-Item -Force $NvmeImg }
$sw = [System.IO.File]::Create($NvmeImg)
$sw.SetLength(64 * 1024 * 1024)

# Write MBR at LBA 0
$sw.Seek(0x1BE, [System.IO.SeekOrigin]::Begin) | Out-Null
$sw.WriteByte(0x00)                                            # status
$sw.WriteByte(0x00); $sw.WriteByte(0x00); $sw.WriteByte(0x01)  # CHS start
$sw.WriteByte(0x06)                                            # partition type FAT16
$sw.WriteByte(0x00); $sw.WriteByte(0x00); $sw.WriteByte(0x00)  # CHS end (unused)
$sw.Write([System.BitConverter]::GetBytes([uint32]$PartStart), 0, 4)
$sw.Write([System.BitConverter]::GetBytes([uint32]$PartSectors), 0, 4)
$sw.Seek(0x1FE, [System.IO.SeekOrigin]::Begin) | Out-Null
$sw.WriteByte(0x55); $sw.WriteByte(0xAA)

# Write FAT16 BPB at partition start
Write-Bpb $sw $PartStart

# Compute data area
$DataStart = $PartStart + $Reserved + $FatCount * $FatSectors + $RootSectors
$DataClusters = ($PartSectors - $Reserved - $FatCount * $FatSectors - $RootSectors) / $Spc

# Write FAT1 at partition + 1
Write-Fat $sw ($PartStart + $Reserved) $FatSectors $DataClusters

# Write FAT2
Write-Fat $sw ($PartStart + $Reserved + $FatSectors) $FatSectors $DataClusters

# Write root directory at partition + 1 + 2*FAT_sectors
$RootLba = $PartStart + $Reserved + $FatCount * $FatSectors
$sw.Seek($RootLba * $Bps, [System.IO.SeekOrigin]::Begin) | Out-Null

# Volume label entry (standard 32-byte FAT entry)
Write-Ascii $sw "NVME DISK  "  # name (8) + ext (3)
$sw.WriteByte(0x08)           # attr: VOLUME_ID
$sw.WriteByte(0)              # reserved (NT)
$sw.WriteByte(0)              # ctime tenths
$sw.Write(([byte[]]@(0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0)), 0, 18)  # remaining 18 bytes (total 32)

# HELLO.TXT entry
Write-DirEntry $sw "HELLO   " "TXT" 0x20 3 13  # cluster=3, size=13, attr=ARCHIVE
$sw.WriteByte(0)  # directory terminator

# Write HELLO.TXT content at cluster 3 (data_start + (3-2)*spc)
$HelloData = [System.Text.Encoding]::ASCII.GetBytes("Hello NVMe!" + [char]13 + [char]10)
$ClusterLba = $DataStart + (3 - 2) * $Spc
$sw.Seek($ClusterLba * $Bps, [System.IO.SeekOrigin]::Begin) | Out-Null
$sw.Write($HelloData, 0, $HelloData.Length)

# Update FAT to mark cluster 3 as EOC (final cluster of file)
$FatOff = 3 * 2  # cluster 3, 2 bytes per entry
$Fat1Start = ($PartStart + $Reserved) * $Bps
$sw.Seek($Fat1Start + $FatOff, [System.IO.SeekOrigin]::Begin) | Out-Null
$sw.WriteByte(0xFF); $sw.WriteByte(0xFF)  # EOC

# Also update FAT2
$Fat2Start = ($PartStart + $Reserved + $FatSectors) * $Bps
$sw.Seek($Fat2Start + $FatOff, [System.IO.SeekOrigin]::Begin) | Out-Null
$sw.WriteByte(0xFF); $sw.WriteByte(0xFF)  # EOC

# --- Create /test directory and hello.elf ---
function New-MinimalElf {
    $code = [byte[]]@(
        0x48,0xc7,0xc0,0x14,0x00,0x00,0x00,  # mov rax, 20 (SYS_LOG_WRITE)
        0x48,0x8d,0x15,0x18,0x00,0x00,0x00,  # lea rdx, [rip+24] -> msg
        0x49,0xc7,0xc0,0x16,0x00,0x00,0x00,  # mov r8, 22 (length)
        0x0f,0x05,                             # syscall
        0x48,0xc7,0xc0,0x00,0x00,0x00,0x00,  # mov rax, 0 (SYS_EXIT)
        0x48,0x31,0xff,                        # xor rdi, rdi
        0x48,0x31,0xd2,                        # xor rdx, rdx (exit code = 0)
        0x0f,0x05                              # syscall
    )
    $msg = [byte[]]@(0x48,0x65,0x6c,0x6c,0x6f,0x20,0x66,0x72,0x6f,0x6d,0x20,0x75,0x73,0x65,0x72,0x20,0x45,0x4c,0x46,0x21,0x0d,0x0a)
    $off = 0x78
    $file_size = $off + $code.Length + $msg.Length
    $elf = New-Object byte[] $file_size

    # ELF header at offset 0
    [System.Buffer]::BlockCopy([byte[]]@(
        0x7f,0x45,0x4c,0x46,0x02,0x01,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0x02,0x00,0x3e,0x00,0x01,0x00,0x00,0x00,
        0x78,0x00,0x40,0x00,0x00,0x00,0x00,0x00,
        0x40,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0x00,0x00,0x00,0x00,0x40,0x00,0x38,0x00,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00
    ), 0, $elf, 0, 64)

    # Program header at offset 64
    [System.BitConverter]::GetBytes([uint32]1).CopyTo($elf, 64)
    [System.BitConverter]::GetBytes([uint32]5).CopyTo($elf, 68)
    [System.BitConverter]::GetBytes([uint64]0).CopyTo($elf, 72)
    [System.BitConverter]::GetBytes([uint64]0x400000).CopyTo($elf, 80)
    [System.BitConverter]::GetBytes([uint64]0x400000).CopyTo($elf, 88)
    [System.BitConverter]::GetBytes([uint64]$file_size).CopyTo($elf, 96)
    [System.BitConverter]::GetBytes([uint64]$file_size).CopyTo($elf, 104)
    [System.BitConverter]::GetBytes([uint64]0x1000).CopyTo($elf, 112)

    [System.Buffer]::BlockCopy($code, 0, $elf, $off, $code.Length)
    [System.Buffer]::BlockCopy($msg, 0, $elf, $off + $code.Length, $msg.Length)
    return $elf
}

# --- Minimal x86-64 assembler (для parent.elf / child.elf) ---
function New-Asm {
    [pscustomobject]@{
        code  = New-Object 'System.Collections.Generic.List[byte]'
        rel32 = @{}
        rel8  = @{}
        label = @{}
        cur   = 0
    }
}
function Add-AsmBytes($asm, [byte[]]$b) {
    foreach ($x in $b) { $asm.code.Add($x) }
    $asm.cur += $b.Length
}
function Asm-MovImm64($asm, $reg, [uint64]$val) {
    # PS5.1: литерал 0xFFFFFFFF парсится как [int32]-1 и ломает -band,
    # поэтому маску 32 бит задаём десятичной как [uint64].
    $M32 = [uint64]4294967295
    $hi32 = ($val -shr 32) -band $M32
    $lo32 = $val -band $M32
    $sign = (($lo32 -band [uint64]2147483648) -ne 0)
    $useShort = (($hi32 -eq 0) -and (-not $sign)) -or (($hi32 -eq $M32) -and $sign)
    if ($useShort) {
        $rex = 0x48; if ($reg -ge 8) { $rex = $rex -bor 1 }
        $modrm = 0xC0 -bor ($reg -band 7)
        Add-AsmBytes $asm ([byte[]]@($rex, 0xC7, $modrm,
            ($lo32 -band 0xFF), (($lo32 -shr 8) -band 0xFF),
            (($lo32 -shr 16) -band 0xFF), (($lo32 -shr 24) -band 0xFF)))
    } else {
        $rex = 0x48; if ($reg -ge 8) { $rex = $rex -bor 1 }
        $b = New-Object byte[] 10
        $b[0] = $rex; $b[1] = 0xB8 -bor ($reg -band 7)
        for ($j = 0; $j -lt 8; $j++) { $b[2 + $j] = ($val -shr ($j * 8)) -band 0xFF }
        Add-AsmBytes $asm $b
    }
}
function Asm-Xor($asm, $reg) {
    $rex = 0x48
    if ($reg -ge 8) { $rex = $rex -bor 5 }
    $modrm = 0xC0 -bor (($reg -band 7) -shl 3) -bor ($reg -band 7)
    Add-AsmBytes $asm ([byte[]]@($rex, 0x31, $modrm))
}
function Asm-LeaRip($asm, $reg, $labelName) {
    $rex = 0x48; if ($reg -ge 8) { $rex = $rex -bor 4 }
    $modrm = 0x05 -bor (($reg -band 7) -shl 3)
    Add-AsmBytes $asm ([byte[]]@($rex, 0x8D, $modrm, 0, 0, 0, 0))
    $asm.rel32[$labelName] = $asm.cur - 4
}
function Asm-Syscall($asm) { Add-AsmBytes $asm ([byte[]]@(0x0F, 0x05)) }
function Asm-CmpImm8($asm, $reg, $imm) {
    $rex = 0x48; if ($reg -ge 8) { $rex = $rex -bor 1 }
    $modrm = 0xC0 -bor (7 -shl 3) -bor ($reg -band 7)
    Add-AsmBytes $asm ([byte[]]@($rex, 0x83, $modrm, ($imm -band 0xFF)))
}
function Asm-Je($asm, $labelName)  { Add-AsmBytes $asm ([byte[]]@(0x74, 0)); $asm.rel8[$labelName] = $asm.cur - 1 }
function Asm-Jne($asm, $labelName) { Add-AsmBytes $asm ([byte[]]@(0x75, 0)); $asm.rel8[$labelName] = $asm.cur - 1 }
function Asm-Label($asm, $name)    { $asm.label[$name] = $asm.cur }
function Asm-MovFromMem($asm, $dstReg, $baseReg) {
    $rex = 0x48
    if ($dstReg -ge 8) { $rex = $rex -bor 4 }
    if ($baseReg -ge 8) { $rex = $rex -bor 1 }
    $modrm = 0x00 -bor (($dstReg -band 7) -shl 3) -bor ($baseReg -band 7)
    Add-AsmBytes $asm ([byte[]]@($rex, 0x8B, $modrm))
}
function Resolve-Asm($asm) {
    foreach ($k in $asm.rel32.Keys) {
        $pos = $asm.rel32[$k]
        $disp = $asm.label[$k] - ($pos + 4)
        for ($j = 0; $j -lt 4; $j++) { $asm.code[$pos + $j] = ($disp -shr ($j * 8)) -band 0xFF }
    }
    foreach ($k in $asm.rel8.Keys) {
        $pos = $asm.rel8[$k]
        $disp = $asm.label[$k] - ($pos + 1)
        if ($disp -lt -128 -or $disp -gt 127) { throw "rel8 out of range: $k" }
        $asm.code[$pos] = $disp -band 0xFF
    }
    , $asm.code.ToArray()
}

function New-Elf([byte[]]$stream) {
    $off = 0x78
    $file_size = $off + $stream.Length
    $elf = New-Object byte[] $file_size
    [System.Buffer]::BlockCopy([byte[]]@(
        0x7f,0x45,0x4c,0x46,0x02,0x01,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0x02,0x00,0x3e,0x00,0x01,0x00,0x00,0x00,
        0x78,0x00,0x40,0x00,0x00,0x00,0x00,0x00,
        0x40,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0x00,0x00,0x00,0x00,0x40,0x00,0x38,0x00,0x01,0x00,0x00,0x00,0x00,0x00,0x00,0x00
    ), 0, $elf, 0, 64)
    [System.BitConverter]::GetBytes([uint32]1).CopyTo($elf, 64)
    [System.BitConverter]::GetBytes([uint32]5).CopyTo($elf, 68)
    [System.BitConverter]::GetBytes([uint64]0).CopyTo($elf, 72)
    [System.BitConverter]::GetBytes([uint64]0x400000).CopyTo($elf, 80)
    [System.BitConverter]::GetBytes([uint64]0x400000).CopyTo($elf, 88)
    [System.BitConverter]::GetBytes([uint64]$file_size).CopyTo($elf, 96)
    [System.BitConverter]::GetBytes([uint64]$file_size).CopyTo($elf, 104)
    [System.BitConverter]::GetBytes([uint64]0x1000).CopyTo($elf, 112)
    [System.Buffer]::BlockCopy($stream, 0, $elf, $off, $stream.Length)
    return $elf
}

# parent.elf: wait(child, exit 7) через SYS_WAIT, kill(bogus) через SYS_KILL, exit 0.
function New-ParentElf {
    $asm = New-Asm
    $msg1b = [System.Text.Encoding]::ASCII.GetBytes("parent: wait child...`r`n")
    $msg2b = [System.Text.Encoding]::ASCII.GetBytes("parent: wait+kill OK`r`n")
    # print msg1
    Asm-MovImm64 $asm 0 20
    Asm-LeaRip $asm 2 "msg1"
    Asm-MovImm64 $asm 8 $msg1b.Length
    Asm-Syscall $asm
    # SYS_WAIT(0, &status)
    Asm-MovImm64 $asm 0 4
    Asm-Xor $asm 2
    Asm-LeaRip $asm 8 "status"
    Asm-Syscall $asm
    # reaped pid == 0 -> fail1
    Asm-CmpImm8 $asm 0 0
    Asm-Je $asm "fail1"
    # status == 7 else fail2
    Asm-MovFromMem $asm 1 8
    Asm-CmpImm8 $asm 1 7
    Asm-Jne $asm "fail2"
    # SYS_KILL(0xFFFFFFFFFFFFFFFF, 9) -> 0
    Asm-MovImm64 $asm 0 5
    Asm-MovImm64 $asm 2 ([uint64]::MaxValue)
    Asm-MovImm64 $asm 8 9
    Asm-Syscall $asm
    Asm-CmpImm8 $asm 0 0
    Asm-Jne $asm "fail3"
    # print msg2
    Asm-MovImm64 $asm 0 20
    Asm-LeaRip $asm 2 "msg2"
    Asm-MovImm64 $asm 8 $msg2b.Length
    Asm-Syscall $asm
    # exit(0)
    Asm-MovImm64 $asm 0 0
    Asm-MovImm64 $asm 2 0
    Asm-Syscall $asm
    # fail paths: exit(1/2/3)
    Asm-Label $asm "fail1"
    Asm-MovImm64 $asm 0 0
    Asm-MovImm64 $asm 2 1
    Asm-Syscall $asm
    Asm-Label $asm "fail2"
    Asm-MovImm64 $asm 0 0
    Asm-MovImm64 $asm 2 2
    Asm-Syscall $asm
    Asm-Label $asm "fail3"
    Asm-MovImm64 $asm 0 0
    Asm-MovImm64 $asm 2 3
    Asm-Syscall $asm
    # data
    Asm-Label $asm "msg1";   Add-AsmBytes $asm $msg1b
    Asm-Label $asm "msg2";   Add-AsmBytes $asm $msg2b
    Asm-Label $asm "status"; Add-AsmBytes $asm (New-Object byte[] 8)
    New-Elf (Resolve-Asm $asm)
}

# child.elf: print + exit(7)
function New-ChildElf {
    $asm = New-Asm
    $msgb = [System.Text.Encoding]::ASCII.GetBytes("child exit(7)`r`n")
    Asm-MovImm64 $asm 0 20
    Asm-LeaRip $asm 2 "msg"
    Asm-MovImm64 $asm 8 $msgb.Length
    Asm-Syscall $asm
    Asm-MovImm64 $asm 0 0
    Asm-MovImm64 $asm 2 7
    Asm-Syscall $asm
    Asm-Label $asm "msg"; Add-AsmBytes $asm $msgb
    New-Elf (Resolve-Asm $asm)
}

# forktest.elf: fork() → ребёнок exit(42), родитель wait → проверить code 42 → exit(0).
function New-ForkElf {
    $asm = New-Asm
    $msgc = [System.Text.Encoding]::ASCII.GetBytes("fork child exit(42)`r`n")
    $msgp = [System.Text.Encoding]::ASCII.GetBytes("fork parent: wait+check OK`r`n")
    # SYS_FORK(6)
    Asm-MovImm64 $asm 0 6
    Asm-Syscall $asm
    # RAX==0 → ребёнок; иначе родитель
    Asm-CmpImm8 $asm 0 0
    Asm-Je $asm "child"
    # родитель: SYS_WAIT(0, &status)
    Asm-MovImm64 $asm 0 4
    Asm-Xor $asm 2
    Asm-LeaRip $asm 8 "status"
    Asm-Syscall $asm
    # reaped pid == 0 -> fail1
    Asm-CmpImm8 $asm 0 0
    Asm-Je $asm "fail1"
    # status == 42 else fail2
    Asm-MovFromMem $asm 1 8
    Asm-CmpImm8 $asm 1 42
    Asm-Jne $asm "fail2"
    # print + exit(0)
    Asm-MovImm64 $asm 0 20
    Asm-LeaRip $asm 2 "msgp"
    Asm-MovImm64 $asm 8 $msgp.Length
    Asm-Syscall $asm
    Asm-MovImm64 $asm 0 0
    Asm-MovImm64 $asm 2 0
    Asm-Syscall $asm
    # ребёнок: print + exit(42)
    Asm-Label $asm "child"
    Asm-MovImm64 $asm 0 20
    Asm-LeaRip $asm 2 "msgc"
    Asm-MovImm64 $asm 8 $msgc.Length
    Asm-Syscall $asm
    Asm-MovImm64 $asm 0 0
    Asm-MovImm64 $asm 2 42
    Asm-Syscall $asm
    # fail paths: exit(1/2)
    Asm-Label $asm "fail1"
    Asm-MovImm64 $asm 0 0
    Asm-MovImm64 $asm 2 1
    Asm-Syscall $asm
    Asm-Label $asm "fail2"
    Asm-MovImm64 $asm 0 0
    Asm-MovImm64 $asm 2 2
    Asm-Syscall $asm
    # data
    Asm-Label $asm "msgc";  Add-AsmBytes $asm $msgc
    Asm-Label $asm "msgp";  Add-AsmBytes $asm $msgp
    Asm-Label $asm "status"; Add-AsmBytes $asm (New-Object byte[] 8)
    New-Elf (Resolve-Asm $asm)
}

# Allocate cluster 4 for /test dir, cluster 5 for hello.elf,
# cluster 6 for parent.elf, cluster 7 for child.elf, cluster 8 for forktest.elf
$TestCluster = 4
$ElfCluster = 5
$ParentCluster = 6
$ChildCluster = 7
$ForkCluster = 8
$ElfBytes = New-MinimalElf
$ParentBytes = New-ParentElf
$ChildBytes = New-ChildElf
$ForkBytes = New-ForkElf

# Write /test directory cluster
$sw.Seek(($DataStart + ($TestCluster - 2) * $Spc) * $Bps, [System.IO.SeekOrigin]::Begin) | Out-Null
Write-DirEntry $sw ".       " "   " 0x10 $TestCluster 0   # .
Write-DirEntry $sw "..      " "   " 0x10 0 0              # .. (root = cluster 0)
Write-DirEntry $sw "HELLO   " "ELF" 0x20 $ElfCluster $ElfBytes.Length
Write-DirEntry $sw "PARENT  " "ELF" 0x20 $ParentCluster $ParentBytes.Length
Write-DirEntry $sw "CHILD   " "ELF" 0x20 $ChildCluster $ChildBytes.Length
Write-DirEntry $sw "FORKTEST" "ELF" 0x20 $ForkCluster $ForkBytes.Length
$sw.WriteByte(0)  # directory terminator

# Write hello.elf content at cluster 5
$sw.Seek(($DataStart + ($ElfCluster - 2) * $Spc) * $Bps, [System.IO.SeekOrigin]::Begin) | Out-Null
$sw.Write($ElfBytes, 0, $ElfBytes.Length)

# Write parent.elf content at cluster 6
$sw.Seek(($DataStart + ($ParentCluster - 2) * $Spc) * $Bps, [System.IO.SeekOrigin]::Begin) | Out-Null
$sw.Write($ParentBytes, 0, $ParentBytes.Length)

# Write child.elf content at cluster 7
$sw.Seek(($DataStart + ($ChildCluster - 2) * $Spc) * $Bps, [System.IO.SeekOrigin]::Begin) | Out-Null
$sw.Write($ChildBytes, 0, $ChildBytes.Length)

# Write forktest.elf content at cluster 8
$sw.Seek(($DataStart + ($ForkCluster - 2) * $Spc) * $Bps, [System.IO.SeekOrigin]::Begin) | Out-Null
$sw.Write($ForkBytes, 0, $ForkBytes.Length)

# Add /test directory entry in root dir (replace old terminator at offset 64)
$sw.Seek($RootLba * $Bps + 64, [System.IO.SeekOrigin]::Begin) | Out-Null
Write-DirEntry $sw "TEST    " "   " 0x10 $TestCluster 0
$sw.WriteByte(0)  # new terminator at offset 96

# Mark clusters 4,5,6,7,8 as EOC in FAT1 and FAT2
foreach ($fatBase in @($Fat1Start, $Fat2Start)) {
    foreach ($cl in @($TestCluster, $ElfCluster, $ParentCluster, $ChildCluster, $ForkCluster)) {
        $sw.Seek($fatBase + $cl * 2, [System.IO.SeekOrigin]::Begin) | Out-Null
        $sw.WriteByte(0xFF); $sw.WriteByte(0xFF)
    }
}

$sw.Close()

Write-Host "NVMe image created with FAT16 + HELLO.TXT + /test/{hello,parent,child,forktest}.elf" -ForegroundColor Green

# Run in QEMU
if ($Run) {
    Write-Host "=== Starting QEMU ===" -ForegroundColor Cyan
    cmd /c "`"$Qemu`" -machine q35 -drive if=pflash,format=raw,readonly=on,file=`"$Ovmf`" -drive file=fat:rw:`"$DBSos\esp`",format=raw -drive file=`"$NvmeImg`",if=none,id=nvme0,format=raw -device nvme,serial=deadbeef,drive=nvme0 -nographic -no-reboot -m 256M -nic user,model=e1000 -d int -D qemu.log"
}
