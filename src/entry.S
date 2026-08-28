# Osmium boot entry.
#
# With `-bios none`, QEMU's virt machine resets every hart into machine mode
# at 0x8000_0000, which the linker script pins to this code. Registers hold
# nothing useful except a0 = hartid (also in mhartid) and a1 = DTB pointer.
#
# Hart 0 zeroes .bss (the ELF loader does not), points sp at the boot stack,
# and calls the Rust M-mode setup routine `start`. Secondary harts park in a
# wfi loop; the kernel is single-hart for now.

.option norvc

.section .text.entry
.global _entry
_entry:
    csrr    t0, mhartid
    bnez    t0, park

    # Zero .bss. Both symbols are 16-byte aligned (linker.ld), so 8-byte
    # stores never overrun. The boot stack lives in .bss but nothing is on
    # it yet -- sp is only set after this loop.
    la      t0, __bss_start
    la      t1, __bss_end
1:
    bgeu    t0, t1, 2f
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       1b
2:
    la      sp, __boot_stack_top
    call    start

park:
    wfi
    j       park

# Boot stack for hart 0. Lives in .bss (so it is inside the kernel image and
# gets RW mapping for free); 64 KiB is plenty for boot + scheduler context.
.section .bss.stack
.align 4
.global __boot_stack
__boot_stack:
    .space 4096 * 16
.global __boot_stack_top
__boot_stack_top:
