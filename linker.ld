/*
 * Linker script for the Osmium kernel on qemu-system-riscv64 -machine virt.
 *
 * The kernel is an OpenSBI supervisor payload. DRAM starts at 0x8000_0000;
 * the firmware owns the first 2 MiB and enters the next stage in S-mode at
 * the conventional payload address 0x8020_0000, which .text.entry (the boot
 * assembly) must occupy as the first byte of the image.
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

BASE_ADDRESS = 0x80200000;

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
