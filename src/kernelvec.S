# Supervisor trap vector.
#
# stvec points here (direct mode) for every S-mode trap: exceptions,
# interrupts, and ecalls from U-mode. The handler builds a TrapFrame on the
# kernel stack, calls the Rust dispatcher, then restores and srets.
#
# Stack discipline (the sscratch protocol):
#   - While the hart runs in S-mode, sscratch == 0 and sp is already a
#     kernel stack, so the frame goes right below the interrupted sp.
#   - Before sret-ing to U-mode, sscratch is loaded with the top of the
#     process's kernel trap stack. A trap from U-mode swaps it in, because
#     the user sp can be garbage or hostile.
#
# TrapFrame layout (must match trap.rs): slots 0..30 are x1..x31 in index
# order (ra, sp, gp, tp, t0-t2, s0-s1, a0-a7, s2-s11, t3-t6), slot 31 is
# sepc, slot 32 is sstatus. 33 slots, frame padded to 272 for 16-alignment.

.altmacro

# Save every GP register except sp (x2), whose slot the two entry paths
# fill differently.
.macro SAVE_GP
    sd      ra, 0(sp)
    sd      gp, 16(sp)
    sd      tp, 24(sp)
    sd      t0, 32(sp)
    sd      t1, 40(sp)
    sd      t2, 48(sp)
    sd      s0, 56(sp)
    sd      s1, 64(sp)
    sd      a0, 72(sp)
    sd      a1, 80(sp)
    sd      a2, 88(sp)
    sd      a3, 96(sp)
    sd      a4, 104(sp)
    sd      a5, 112(sp)
    sd      a6, 120(sp)
    sd      a7, 128(sp)
    sd      s2, 136(sp)
    sd      s3, 144(sp)
    sd      s4, 152(sp)
    sd      s5, 160(sp)
    sd      s6, 168(sp)
    sd      s7, 176(sp)
    sd      s8, 184(sp)
    sd      s9, 192(sp)
    sd      s10, 200(sp)
    sd      s11, 208(sp)
    sd      t3, 216(sp)
    sd      t4, 224(sp)
    sd      t5, 232(sp)
    sd      t6, 240(sp)
.endm

.macro LOAD_GP
    ld      ra, 0(sp)
    ld      gp, 16(sp)
    ld      tp, 24(sp)
    ld      t0, 32(sp)
    ld      t1, 40(sp)
    ld      t2, 48(sp)
    ld      s0, 56(sp)
    ld      s1, 64(sp)
    ld      a0, 72(sp)
    ld      a1, 80(sp)
    ld      a2, 88(sp)
    ld      a3, 96(sp)
    ld      a4, 104(sp)
    ld      a5, 112(sp)
    ld      a6, 120(sp)
    ld      a7, 128(sp)
    ld      s2, 136(sp)
    ld      s3, 144(sp)
    ld      s4, 152(sp)
    ld      s5, 160(sp)
    ld      s6, 168(sp)
    ld      s7, 176(sp)
    ld      s8, 184(sp)
    ld      s9, 192(sp)
    ld      s10, 200(sp)
    ld      s11, 208(sp)
    ld      t3, 216(sp)
    ld      t4, 224(sp)
    ld      t5, 232(sp)
    ld      t6, 240(sp)
.endm

.section .text
.align 4
.global kernelvec
kernelvec:
    csrrw   sp, sscratch, sp
    bnez    sp, .Lfrom_user

.Lfrom_kernel:
    # sp was swapped with sscratch's 0; swap back to the interrupted sp
    # (and leave sscratch = 0).
    csrrw   sp, sscratch, sp
    addi    sp, sp, -272
    SAVE_GP
    addi    t0, sp, 272            # interrupted kernel sp
    sd      t0, 8(sp)
    j       .Lsave_csrs

.Lfrom_user:
    # sp = top of this process's kernel trap stack; sscratch = user sp.
    addi    sp, sp, -272
    SAVE_GP
    csrr    t0, sscratch
    sd      t0, 8(sp)              # interrupted user sp
    csrw    sscratch, zero         # back in the kernel: mark S-mode

.Lsave_csrs:
    csrr    t0, sepc
    sd      t0, 248(sp)
    csrr    t0, sstatus
    sd      t0, 256(sp)

    mv      a0, sp                 # &mut TrapFrame
    call    kerneltrap

    # sepc/sstatus may have been modified (e.g. syscall return path).
    ld      t0, 248(sp)
    csrw    sepc, t0
    ld      t1, 256(sp)
    csrw    sstatus, t1

    # Returning to U-mode (saved SPP == 0)? Re-arm sscratch with this trap
    # stack's top for the next user trap.
    andi    t1, t1, 0x100          # SSTATUS_SPP
    bnez    t1, .Lrestore
    addi    t0, sp, 272
    csrw    sscratch, t0

.Lrestore:
    LOAD_GP
    ld      sp, 8(sp)              # base register is read before writeback
    sret
