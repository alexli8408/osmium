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
"#
);

unsafe extern "C" {
    fn user_prog_greeter();
    fn user_prog_trespasser();
}

pub fn greeter_addr() -> usize {
    user_prog_greeter as *const () as usize
}

pub fn trespasser_addr() -> usize {
    user_prog_trespasser as *const () as usize
}
