# Return to user mode restoring a complete TrapFrame.
#
#   userret(frame: *const TrapFrame /* a0 */, kstack_top: usize /* a1 */)
#
# Unlike enter_user (which starts a program with a blank slate), this restores
# every general register plus sepc/sstatus from a saved frame, then srets. A
# forked child uses it to resume exactly where its parent was when it called
# fork — same registers, same pc — differing only in a0 (set to 0 by the
# caller before this runs).
#
# TrapFrame offsets must match trap.rs / kernelvec.S: x1..x31 at 0..240 in
# index order, sepc at 248, sstatus at 256.

.section .text
.global userret
userret:
    csrw    sscratch, a1            # top of kernel trap stack for the next trap
    ld      t0, 248(a0)
    csrw    sepc, t0
    ld      t0, 256(a0)
    csrw    sstatus, t0

    # Restore everything except sp (x2, offset 8) and a0 (x10, offset 72),
    # which we still need: a0 holds the frame pointer, and sp must be loaded
    # last so the kernel stack stays usable until the final moment.
    ld      ra, 0(a0)
    ld      gp, 16(a0)
    ld      tp, 24(a0)
    ld      t0, 32(a0)
    ld      t1, 40(a0)
    ld      t2, 48(a0)
    ld      s0, 56(a0)
    ld      s1, 64(a0)
    ld      a1, 80(a0)
    ld      a2, 88(a0)
    ld      a3, 96(a0)
    ld      a4, 104(a0)
    ld      a5, 112(a0)
    ld      a6, 120(a0)
    ld      a7, 128(a0)
    ld      s2, 136(a0)
    ld      s3, 144(a0)
    ld      s4, 152(a0)
    ld      s5, 160(a0)
    ld      s6, 168(a0)
    ld      s7, 176(a0)
    ld      s8, 184(a0)
    ld      s9, 192(a0)
    ld      s10, 200(a0)
    ld      s11, 208(a0)
    ld      t3, 216(a0)
    ld      t4, 224(a0)
    ld      t5, 232(a0)
    ld      t6, 240(a0)

    ld      sp, 8(a0)               # user sp (a0 still valid, points at frame)
    ld      a0, 72(a0)              # a0 last
    sret
