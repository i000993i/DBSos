ENTRY(start)

KERNEL_OFFSET = 0x100000;

SECTIONS {
    . = KERNEL_OFFSET;

    .multiboot : ALIGN(4K) {
        *(.multiboot)
    }

    .text : ALIGN(4K) {
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
    }

    .bss : ALIGN(16) {
        *(COMMON)
        *(.bss .bss.*)
        . += 1M;
        _stack_end = .;
        . += 64K;
        _stack_start = .;
    }

    /DISCARD/ : {
        *(.comment)
        *(.note.*)
    }
}
