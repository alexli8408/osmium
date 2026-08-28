/*
 * Linker script for the Osmium kernel on qemu-system-riscv64 -machine virt.
 *
 * QEMU's virt machine puts DRAM at 0x8000_0000 and, with -bios none, starts
 * every hart at exactly that address. The .text.entry section (boot assembly)
 * must therefore be the first byte of the image.
 *
 * Section boundaries are exported as symbols and page-aligned so the VM code
 * can map each region with the tightest possible permissions:
 *   .text       R-X  (kernel code)
 *   .user       R-X + U  (embedded user-mode programs; never executed by S-mode)
 *   .rodata     R--
 *   .data/.bss  RW-
 */

OUTPUT_ARCH(riscv)
ENTRY(_entry)

BASE_ADDRESS = 0x80000000;

SECTIONS
{
    . = BASE_ADDRESS;
    __kernel_start = .;

    .text : {
        __text_start = .;
        KEEP(*(.text.entry))
        *(.text .text.*)
        . = ALIGN(4K);
        __text_end = .;
    }

    /* User-mode code lives in its own pages so they can carry the U bit. */
    .user : {
        __user_start = .;
        *(.user .user.*)
        . = ALIGN(4K);
        __user_end = .;
    }

    .rodata : {
        __rodata_start = .;
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
        . = ALIGN(4K);
        __rodata_end = .;
    }

    .data : {
        __data_start = .;
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    .bss : {
        . = ALIGN(16);
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    }

    . = ALIGN(4K);
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame .eh_frame_hdr)
    }
}
