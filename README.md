# Osmium

A RISC-V (RV64GC) operating system kernel written from scratch in Rust — no
external crates, no `libc`, no `unsafe` shortcuts around real hardware
protocols. It boots on QEMU's `virt` machine as an **OpenSBI supervisor
payload** — the same firmware contract Linux uses — and ends up running
preemptively-scheduled processes in user mode behind hardware memory
protection.

(Earlier in its history the kernel *was* its own firmware: it started at
the reset vector in machine mode with `-bios none` and did its own trap
delegation, PMP, and timer setup before dropping to S-mode. The git
history keeps that phase; the transition to OpenSBI replaced all of it
with the SBI firmware interface below.)

> Osmium: the densest naturally occurring metal. Chemical symbol **Os**.

## Features

- **OpenSBI supervisor payload** — enters in S-mode at `0x8020_0000` with
  the standard firmware handoff (`a0` = hartid, `a1` = device tree), like a
  production RISC-V OS; the DTB the firmware hands over is header-parsed
  and reserved out of the page allocator
- **An SBI client** (`sbi.rs`) — the ecall calling convention plus the
  Base (discovery/probing), TIME, and SRST extensions; the kernel
  identifies its firmware at boot and requests M-mode services through it
- **NS16550A UART driver** with polled transmit and interrupt-driven receive
- **Trap handling** — a single S-mode vector for exceptions, interrupts, and
  syscalls, with an `sscratch` protocol that safely takes traps arriving
  from either kernel or user code
- **Timer interrupts** at 100 Hz via `sbi_set_timer` — arming through the
  firmware also acknowledges the pending interrupt
- **Physical page allocator** — intrusive free list threaded through the
  free pages themselves; zero metadata overhead, poison-on-free
- **Sv39 virtual memory** — three-level page tables built by hand. The
  kernel is identity-mapped with least privilege per section (text R-X,
  rodata R--, data RW-); user processes get their own address spaces.
- **Kernel heap** — first-fit, address-ordered, coalescing free-list
  allocator implementing `GlobalAlloc`, so the kernel uses `Box`, `Vec`,
  and `String` from the `alloc` crate
- **PLIC driver** with the claim/complete protocol routing device IRQs
- **Kernel threads** — 16 KiB heap-allocated stacks, forged initial
  contexts, cooperative `yield` plus timer-driven **preemptive round-robin
  scheduling**, xv6-style `sleep`/`wakeup` channels with lost-wakeup-proof
  check-then-sleep
- **Synchronization primitives** — a counting semaphore and a bounded
  blocking channel (`sync.rs`) built on sleep/wakeup, exercised by a
  producer/consumer self-test that only passes if hand-off under
  preemption loses and duplicates nothing
- **Per-process address spaces** — every user process gets its own page
  table and a program loader copies its image into private pages at a
  fixed virtual address; the kernel map is shared into every table so
  traps run without swapping first, and each process's pages are freed on
  exit. All demo programs run at the same VA in isolated spaces.
- **Demand-paged user heap** — `sbrk` extends the heap region without
  mapping anything; the page-fault handler is *constructive*, mapping a
  page on first touch and resuming the process. Faults outside the heap
  (e.g. a stray kernel-memory access) stay fatal.
- **`fork()` / `exec()` / `wait()`** — the Unix process model. `fork`
  clones a process's whole address space (child returns 0, parent gets the
  pid), the child resuming at the exact fork-return point via a full
  register-frame restore; `exec` replaces a process's image with a named
  program in a fresh address space; `wait` blocks on a child. A user
  program demonstrates the canonical fork-then-exec-in-the-child pattern.
- **User mode** — processes drop to U-privilege with `sret`, call back into
  the kernel through `ecall` syscalls (`write`, `read`, `exit`, `yield`,
  `getpid`, `sleep_ms`, `uname`, `open`, `fread`, `close`, `sbrk`, `fork`,
  `wait`, `exec`), and get
  **killed on faults instead of taking the kernel down**. Both copy
  directions are validated against the calling process's address space:
  `copy_from_user`/`copy_to_user` reject wrapping or out-of-range pointers
  and prove every touched page carries the U bit (and, for writes, is
  writable) before touching it under `sstatus.SUM` — a syscall can't be
  tricked into reading or writing kernel memory
- **Process control** — `join(pid)` blocks until a process finishes, so
  the shell can hand the console to a user program and take it back
- **In-memory filesystem** — a flat read-only ramfs (`fs.rs`) with an
  offset-based read path, per-process file-descriptor tables, and the
  `open`/`fread`/`close` syscalls on top
- **Interactive shell** on the serial console — `ps`, `mem`, `uptime`,
  `ls`, `cat FILE`, run-a-user-program (`greet`/`fault`/`echoline`/
  `uname`/`heapgrow`/`fork`), `poweroff`/`reboot`
- **A userland shell** (`ush`) running as a U-mode process — prompts,
  reads a line, and launches the named program with fork/exec/wait, all
  through syscalls; a real userland on top of the kernel
- **Boot self-tests** for every subsystem, checked in CI by booting the
  kernel headless in QEMU

## Demo

```
OpenSBI v1.7
   ____                    _____ ____ _____
  / __ \                  / ____|  _ \_   _|
 | |  | |_ __   ___ _ __ | (___ | |_) || |
 | |  | | '_ \ / _ \ '_ \ \___ \|  _ < | |
 | |__| | |_) |  __/ | | |____) | |_) || |_
  \____/| .__/ \___|_| |_|_____/|____/_____|
[... firmware banner ...]

osmium kernel booting on hart 0 (dtb at 0x87e00000)
  sbi: v3.0 via OpenSBI (impl version 0x10007)
  kernel image: 0x80200000..0x80230000 (192 KiB)
  traps: stvec -> kernelvec
  traps: survived an ebreak round-trip
  dtb: reserving 6084 bytes at 0x87e00000
  kalloc: 30158 free pages (117 MiB)
  kalloc: alloc/free self-test ok
  vm: sv39 paging on (root table 0x87ffe000)
  vm: translation self-test ok
  heap: 8192 KiB free, Box/Vec/String self-test ok
  fs: ramfs mounted, 3 files
  plic: routing uart irq 10 to hart 0
  timer: 100 Hz ticks running, uptime 197 ms
  proc: entering scheduler
  thread ping (pid 1): round 0
user: trespasser about to touch kernel memory...
trap: killing pid 9: load page fault at sepc=0x40000018 stval=0x80200000
osmium 0.1.0 riscv64
welcome to osmium -- a RISC-V kernel in Rust
user: badsys survived; kernel rejected hostile args
user: hello from U-mode via ecall! my pid is 6
syscall: pid 6 exited with code 0
  proc: preemptive multitasking verified (busy0=1136497, busy1=1235739)

ALL BOOT TESTS PASSED

osmium shell -- 'help' lists commands
osmium> ls
      45  motd
      97  readme
      40  cpuinfo
osmium> cat motd
welcome to osmium -- a RISC-V kernel in Rust
osmium> ps
  PID   NAME         STATE
  8     init         Running
osmium> mem
  phys: 30609 free pages (119 MiB of 128 MiB DRAM)
  heap: 8175 KiB free of 8192 KiB
osmium> ush
osh$ uname
osmium 0.1.0 riscv64
osh$ nope
osh: unknown command (try greeter/uname/heapgrow/forker or q)
osh$ q
osmium>
```

The two U-mode programs at boot are the system's own proof: the *greeter*
completes write/sleep/getpid/exit syscalls and terminates cleanly; the
*trespasser* dereferences kernel memory from user mode and is killed by the
page-fault path while the kernel keeps running. `ush` is a second shell
running entirely in *user* mode — it reads a command and launches it with
`fork`/`exec`/`wait`, so `uname` above runs in a forked child that replaced
its image with the uname program.

## Building and running

Requirements: stable Rust (the pinned toolchain installs the
`riscv64gc-unknown-none-elf` target automatically) and `qemu-system-riscv64`
with its bundled OpenSBI firmware (`-bios default`; ships with QEMU, and on
Debian/Ubuntu also via the `opensbi` package).

```sh
make run        # build + boot with the console on stdio (Ctrl-A X quits)
make test       # headless boot; passes iff the kernel self-tests do
make gdb        # boot halted, waiting for a debugger on :1234
make objdump    # disassemble the kernel
```

## Architecture

### Boot flow

```
QEMU reset (M-mode, 0x80000000)
  └─ OpenSBI      firmware: delegate traps, program PMP, provide SBI services
      └─ entry.S  entered in S-mode at 0x80200000, a0=hartid a1=dtb;
         │        zero .bss, set boot stack
         └─ kmain()  drivers + allocators + VM + self-tests
             └─ scheduler()  runs threads; init becomes the shell
                 └─ sret     user processes at U-privilege
                      └─ ecall … up to the kernel (syscalls)
                           and from the kernel up to OpenSBI (timer, reset)
```

### Memory map (QEMU virt + kernel layout)

| Address        | What                                | Mapping    |
| -------------- | ----------------------------------- | ---------- |
| `0x0c00_0000`  | PLIC                                | RW-        |
| `0x1000_0000`  | UART0 (NS16550A)                    | RW-        |
| `0x4000_0000`  | user code / stack (per process)     | R-X/RW **+U** |
| `0x8000_0000`  | OpenSBI firmware (2 MiB, PMP-guarded) | unmapped |
| `0x8020_0000`  | kernel `.text`                      | R-X        |
| …              | `.user` (program images, source)    | R--        |
| …              | `.rodata`                           | R--        |
| …              | `.data`, `.bss`, boot stack         | RW-        |
| kernel end     | kernel heap (8 MiB)                 | RW-        |
| heap end       | page allocator pool (~117 MiB)      | RW-        |
| `0x87e0_0000`  | flattened device tree (reserved)    | RW-        |
| `0x8800_0000`  | top of DRAM (`-m 128M`)             |            |

The kernel is identity-mapped (VA = PA) at 4 KiB granularity, which keeps
the design honest — permissions are real, but enabling `satp` mid-boot
doesn't move the ground underneath the running code. Each user process
gets its own page table that shares the kernel mapping but adds private
code and stack pages in the `0x4000_0000` region; those pages are freed
when the process is reaped.

### Code tour

| File                 | Contents                                             |
| -------------------- | ---------------------------------------------------- |
| `src/entry.S`        | S-mode payload entry (OpenSBI handoff), boot stack   |
| `src/main.rs`        | `kmain`: init order, DTB reservation, boot self-tests |
| `src/sbi.rs`         | SBI client: Base/TIME/SRST extensions over ecall     |
| `src/riscv.rs`       | zero-cost CSR accessors (macro-generated, S-level)   |
| `src/memlayout.rs`   | physical map constants, linker-symbol accessors      |
| `src/uart.rs`        | NS16550A driver                                      |
| `src/console.rs`     | `println!` machinery + interrupt-fed input ring      |
| `src/spinlock.rs`    | interrupt-safe spinlock (push_off/pop_off nesting)   |
| `src/kernelvec.S`    | trap vector: build TrapFrame, dispatch, sret         |
| `src/trap.rs`        | exception/interrupt dispatch, user-fault isolation   |
| `src/timer.rs`       | SBI timer, ticks, `sleep_ticks`                      |
| `src/kalloc.rs`      | physical page allocator                              |
| `src/vm.rs`          | Sv39 page tables, kernel + per-process address spaces |
| `src/heap.rs`        | `GlobalAlloc` free-list heap                         |
| `src/sync.rs`        | counting semaphore, bounded blocking channel         |
| `src/plic.rs`        | PLIC claim/complete driver                           |
| `src/switch.S`       | `swtch`: callee-saved context switch                 |
| `src/userret.S`      | restore a full trap frame and sret (forked children) |
| `src/proc.rs`        | threads, scheduler, sleep/wakeup, join, fork, fds    |
| `src/syscall.rs`     | ecall dispatch + user-pointer validation             |
| `src/fs.rs`          | flat read-only in-memory filesystem                  |
| `src/user.rs`        | embedded U-mode programs (incl. the userland shell)  |
| `src/shell.rs`       | interactive kernel console shell                     |
| `src/power.rs`       | SBI SRST poweroff/reboot                             |

## Design notes

- **Why run on OpenSBI?** Because that's the real-world contract: on
  RISC-V, machine mode belongs to firmware and the OS is an S-mode payload
  that requests M-mode services (timer, reset, inter-processor interrupts)
  through the SBI — Linux boots exactly this way. Osmium's git history
  contains both worlds: first a hand-rolled M-mode firmware phase (which
  teaches what OpenSBI actually does), then this transition, which deleted
  every machine-level CSR from the kernel and replaced the privileged
  machinery with a ~140-line SBI client. The privilege stack is now
  honest: U-mode ecalls up to the kernel, and the kernel ecalls up to
  firmware.
- **Identity-mapped kernel, private user spaces.** The kernel is identity-
  mapped (virtual = physical), which sidesteps the classic trampoline
  problem: because every process page table *also* contains the full
  kernel mapping, a trap from user mode can run kernel code immediately,
  without first switching page tables. User processes still get real
  isolation — each has its own table, and its private code and stack live
  in a virtual-address slot the kernel never maps, so two processes at the
  same virtual address resolve to different physical pages.
- **Why no external crates?** Every line of the UART driver, the CSR
  accessors, the allocators, and the lock is in this repo. The point of the
  project is that there is no magic underneath.
- **The single-hart concurrency argument.** All scheduler state is touched
  only with interrupts disabled on one hart; spinlocks disable interrupts
  before spinning (so an interrupt handler can never deadlock against the
  code it interrupted), and the per-thread interrupt-enable intent is saved
  across context switches. The invariants are documented where they live.
- **Traps carry the whole design.** One vector serves kernel exceptions,
  device interrupts, timer preemption, and syscalls; the `sscratch`
  protocol (0 in S-mode, kernel trap stack while in U-mode) is what lets
  the same code trust `sp` from the kernel and distrust it from user space.

## References

- [The xv6-riscv teaching OS](https://github.com/mit-pdos/xv6-riscv) — the
  design ancestor of the trap/scheduler/sleep-wakeup discipline
- The RISC-V Privileged Architecture specification
- [The Adventures of OS](https://osblog.stephenmarz.com/) by Stephen Marz

## License

MIT
