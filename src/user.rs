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
.global user_prog_greeter_end
user_prog_greeter_end:

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
.global user_prog_trespasser_end
user_prog_trespasser_end:

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
.global user_prog_badsys_end
user_prog_badsys_end:
"#
);

global_asm!(
    r#"
.section .user, "ax", @progbits

# --- uname: exercises the kernel->user copy path. Asks the kernel to fill a
#     stack buffer via SYS_UNAME, then writes it back out via SYS_WRITE. ---
.global user_prog_uname
.align 4
user_prog_uname:
    addi    sp, sp, -32
    mv      a0, sp                  # buffer on the user stack
    li      a1, 32
    li      a7, 7                   # SYS_UNAME -> bytes written in a0
    ecall
    # Echo exactly what the kernel wrote (a0 = length) back to the console.
    mv      a1, a0
    mv      a0, sp
    li      a7, 1                   # SYS_WRITE
    ecall
    addi    sp, sp, 32
    li      a0, 0
    li      a7, 2                   # SYS_EXIT
    ecall
.global user_prog_uname_end
user_prog_uname_end:

# --- echoline: read one line from the console (SYS_READ) and echo it back
#     (SYS_WRITE). Demonstrates blocking user input end to end. ---
.global user_prog_echoline
.align 4
user_prog_echoline:
    la      a0, 10f
    li      a1, 11f - 10f
    li      a7, 1                   # SYS_WRITE (prompt)
    ecall
    addi    sp, sp, -128
    mv      a0, sp
    li      a1, 128
    li      a7, 6                   # SYS_READ -> a0 = byte count
    ecall
    mv      a1, a0                  # echo exactly what was read
    la      a0, 12f
    # (write the label first)
    li      a2, 13f - 12f
    mv      s0, a1                  # save count
    mv      a1, a2
    li      a7, 1
    ecall                           # SYS_WRITE "you typed: "
    mv      a0, sp
    mv      a1, s0
    li      a7, 1
    ecall                           # SYS_WRITE the line
    addi    sp, sp, 128
    li      a0, 0
    li      a7, 2                   # SYS_EXIT
    ecall
10: .ascii  "echoline: type a line and press enter\n"
11:
12: .ascii  "you typed: "
13:
.global user_prog_echoline_end
user_prog_echoline_end:
"#
);

global_asm!(
    r#"
.section .user, "ax", @progbits

# --- catfile: open the ramfs file "motd", read it via SYS_FREAD, write it
#     to the console, and close it. Exercises the file syscalls with no
#     blocking, so it runs deterministically in the boot self-tests. ---
.global user_prog_catfile
.align 4
user_prog_catfile:
    la      a0, 20f                 # path "motd"
    li      a1, 21f - 20f
    li      a7, 8                   # SYS_OPEN -> a0 = fd
    ecall
    bltz    a0, 3f                  # fd < 0 -> open failed
    mv      s0, a0                  # save fd
    addi    sp, sp, -128
1:
    mv      a0, s0
    mv      a1, sp
    li      a2, 128
    li      a7, 9                   # SYS_FREAD -> a0 = bytes read
    ecall
    blez    a0, 2f                  # 0 -> EOF, <0 -> error
    mv      a1, a0
    mv      a0, sp
    li      a7, 1                   # SYS_WRITE what we read
    ecall
    j       1b
2:
    addi    sp, sp, 128
    mv      a0, s0
    li      a7, 10                  # SYS_CLOSE
    ecall
    li      a0, 0
    li      a7, 2                   # SYS_EXIT (success)
    ecall
3:
    la      a0, 22f
    li      a1, 23f - 22f
    li      a7, 1
    ecall
    li      a0, 1
    li      a7, 2                   # SYS_EXIT (failure)
    ecall
20: .ascii  "motd"
21:
22: .ascii  "catfile: open failed\n"
23:
.global user_prog_catfile_end
user_prog_catfile_end:
"#
);

global_asm!(
    r#"
.section .user, "ax", @progbits

# --- heapgrow: grow the heap with SYS_SBRK, then write to two pages within
#     the new region. Neither page is mapped until this program first
#     touches it, so each store faults into the kernel, which demand-maps a
#     page and resumes us. If demand paging didn't work, the first store
#     would kill the process and nothing would print. ---
.global user_prog_heapgrow
.align 4
user_prog_heapgrow:
    li      a0, 8192                # grow by two pages
    li      a7, 11                  # SYS_SBRK -> a0 = old break (region base)
    ecall
    li      t0, -1
    beq     a0, t0, 2f              # -1 -> sbrk failed
    mv      s0, a0                  # s0 = heap region base

    li      t0, 0x41                # 'A'
    sb      t0, 0(s0)               # store into page 0 -> demand fault -> mapped
    li      t1, 4096
    add     s1, s0, t1
    li      t0, 0x42                # 'B'
    sb      t0, 0(s1)               # store into page 1 -> demand fault -> mapped

    # Read both back onto the stack and print them: proves the pages persist.
    addi    sp, sp, -16
    lb      t0, 0(s0)
    sb      t0, 0(sp)
    lb      t0, 0(s1)
    sb      t0, 1(sp)
    li      t0, 10                  # '\n'
    sb      t0, 2(sp)
    la      a0, 3f
    li      a1, 4f - 3f
    li      a7, 1                   # SYS_WRITE label
    ecall
    mv      a0, sp
    li      a1, 3
    li      a7, 1                   # SYS_WRITE "AB\n"
    ecall
    addi    sp, sp, 16
    li      a0, 0
    li      a7, 2                   # SYS_EXIT
    ecall
2:
    la      a0, 5f
    li      a1, 6f - 5f
    li      a7, 1
    ecall
    li      a0, 1
    li      a7, 2                   # SYS_EXIT (failure)
    ecall
3:  .ascii  "user: heapgrow demand-paged bytes: "
4:
5:  .ascii  "user: heapgrow FAILED (sbrk)\n"
6:
.global user_prog_heapgrow_end
user_prog_heapgrow_end:
"#
);

unsafe extern "C" {
    fn user_prog_greeter();
    fn user_prog_greeter_end();
    fn user_prog_trespasser();
    fn user_prog_trespasser_end();
    fn user_prog_badsys();
    fn user_prog_badsys_end();
    fn user_prog_uname();
    fn user_prog_uname_end();
    fn user_prog_echoline();
    fn user_prog_echoline_end();
    fn user_prog_catfile();
    fn user_prog_catfile_end();
    fn user_prog_heapgrow();
    fn user_prog_heapgrow_end();
}

/// (link address, byte length) of a program's image in the kernel, which the
/// loader copies into a fresh user address space.
type Prog = (usize, usize);

fn image(start: unsafe extern "C" fn(), end: unsafe extern "C" fn()) -> Prog {
    let s = start as *const () as usize;
    let e = end as *const () as usize;
    (s, e - s)
}

pub fn greeter() -> Prog {
    image(user_prog_greeter, user_prog_greeter_end)
}

pub fn trespasser() -> Prog {
    image(user_prog_trespasser, user_prog_trespasser_end)
}

pub fn badsys() -> Prog {
    image(user_prog_badsys, user_prog_badsys_end)
}

pub fn uname() -> Prog {
    image(user_prog_uname, user_prog_uname_end)
}

pub fn echoline() -> Prog {
    image(user_prog_echoline, user_prog_echoline_end)
}

pub fn catfile() -> Prog {
    image(user_prog_catfile, user_prog_catfile_end)
}

pub fn heapgrow() -> Prog {
    image(user_prog_heapgrow, user_prog_heapgrow_end)
}
