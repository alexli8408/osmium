//! Embedded user-mode programs.
//!
//! These are hand-written RISC-V assembly, linked into the kernel image but
//! placed in the `.user` section, which the VM maps R-X **with the U bit**:
//! U-mode can execute it, S-mode cannot (RISC-V forbids S-mode execution of
//! U pages). They talk to the kernel exclusively through `ecall`.
//!
//! Syscall ABI (must match syscall.rs): number in a7, args in a0/a1,
//! return value in a0.

use core::arch::global_asm;

global_asm!(
    r#"
.section .user, "ax", @progbits

# --- greeter: prints via SYS_WRITE, naps via SYS_SLEEP_MS, exits cleanly ---
.global user_prog_greeter
.align 4
user_prog_greeter:
    li      s0, 3                   # three rounds
1:
    la      a0, 2f
    li      a1, 3f - 2f
    li      a7, 1                   # SYS_WRITE
    ecall
    li      a0, 30
    li      a7, 5                   # SYS_SLEEP_MS
    ecall
    addi    s0, s0, -1
    bnez    s0, 1b

    li      a7, 4                   # SYS_GETPID
    ecall
    # pid is a single digit for this demo; turn it into ASCII in-place on
    # the stack and write it.
    addi    sp, sp, -16
    addi    a0, a0, 48
    sb      a0, 0(sp)
    li      a0, 10                  # '\n'
    sb      a0, 1(sp)
    mv      a0, sp
    li      a1, 2
    li      a7, 1                   # SYS_WRITE
    ecall
    addi    sp, sp, 16

    li      a0, 0
    li      a7, 2                   # SYS_EXIT
    ecall
2:  .ascii  "user: hello from U-mode via ecall! my pid is "
3:

# --- trespasser: tries to read kernel memory; must die by page fault ---
.global user_prog_trespasser
.align 4
user_prog_trespasser:
    la      a0, 4f
    li      a1, 5f - 4f
    li      a7, 1                   # SYS_WRITE
    ecall
    li      t0, 0x80000000          # kernel text: mapped without PTE_U
    ld      t1, 0(t0)               # load page fault -> kernel kills us
    # Never reached; if it were, this exit would report failure.
    li      a0, 1
    li      a7, 2                   # SYS_EXIT
    ecall
4:  .ascii  "user: trespasser about to touch kernel memory...\n"
5:

# --- badsys: feeds hostile arguments to syscalls; the kernel must reject
#     them (return -1) and stay alive, so this program runs to a clean exit ---
.global user_prog_badsys
.align 4
user_prog_badsys:
    # A U-mode breakpoint: the kernel must report it, step past it, and
    # resume us here. If it instead faulted reading our instruction (SUM
    # bug) the kernel would panic and nothing below would run.
    ebreak

    # SYS_WRITE with a pointer past the Sv39 user range (1<<38). A kernel
    # that doesn't bounds-check would panic in the page-table walk.
    li      a0, 0x4000000000
    li      a1, 8
    li      a7, 1                   # SYS_WRITE
    ecall
    addi    a0, a0, 1               # want -1 (0xffff...ffff); a0+1 == 0 if so
    bnez    a0, 1f                  # nonzero -> not rejected -> fail

    # SYS_WRITE with a pointer that wraps (ptr + len overflows usize).
    li      a0, -8
    li      a1, 64
    li      a7, 1
    ecall
    addi    a0, a0, 1
    bnez    a0, 1f

    # SYS_SLEEP_MS with a huge value: must saturate, not overflow-panic.
    # (Uses a modest value so the demo doesn't actually stall for ages.)
    li      a0, 5
    li      a7, 5                   # SYS_SLEEP_MS
    ecall

    la      a0, 6f
    li      a1, 7f - 6f
    li      a7, 1
    ecall
    li      a0, 0
    li      a7, 2                   # SYS_EXIT (success)
    ecall
1:
    la      a0, 8f
    li      a1, 9f - 8f
    li      a7, 1
    ecall
    li      a0, 1
    li      a7, 2                   # SYS_EXIT (failure)
    ecall
6:  .ascii  "user: badsys survived; kernel rejected hostile args\n"
7:
8:  .ascii  "user: badsys FAILED -- kernel did not reject an arg\n"
9:
"#
);

unsafe extern "C" {
    fn user_prog_greeter();
    fn user_prog_trespasser();
    fn user_prog_badsys();
}

pub fn greeter_addr() -> usize {
    user_prog_greeter as *const () as usize
}

pub fn trespasser_addr() -> usize {
    user_prog_trespasser as *const () as usize
}

pub fn badsys_addr() -> usize {
    user_prog_badsys as *const () as usize
}
