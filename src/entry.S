# Osmium boot entry.
#
# OpenSBI enters the kernel here in *supervisor* mode, at the payload
# address the linker script pins this code to, with:
#   a0 = hartid   (mhartid is M-mode-only; this is how S-mode learns it)
#   a1 = pointer to the flattened device tree
# Machine-mode setup — trap delegation, PMP, counter access — has already
# been done by the firmware. Secondary harts are parked in OpenSBI's HSM
# state machine and never arrive here unless explicitly started.
#
# Zero .bss (the ELF loader does not, and RAM survives a warm reboot),
# point sp at the boot stack, stash the hartid in tp, and enter Rust.
# The .bss loop uses only t-registers, so a0/a1 flow through to kmain
# as its (hartid, dtb) arguments.

.option norvc

.section .text.entry
.global _entry
_entry:
    la      t0, __bss_start
    la      t1, __bss_end
1:
    bgeu    t0, t1, 2f
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       1b
2:
    la      sp, __boot_stack_top
    mv      tp, a0
    call    kmain

# kmain never returns; trap if it somehow does.
3:
    wfi
    j       3b

# Boot stack for the boot hart. Lives in .bss (so it is inside the kernel
# image and gets RW mapping for free); 64 KiB is plenty for boot + the
# scheduler context.
.section .bss.stack
.align 4
.global __boot_stack
__boot_stack:
    .space 4096 * 16
.global __boot_stack_top
__boot_stack_top:
