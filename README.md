# Osmium

A RISC-V (RV64GC) operating system kernel written from scratch in Rust — no
external crates, no `libc`, no pre-built bootloader, no `unsafe` shortcuts
around real hardware protocols. It boots on QEMU's `virt` machine straight
out of reset in machine mode and ends up running preemptively-scheduled
processes in user mode behind hardware memory protection.

> Osmium: the densest naturally occurring metal. Chemical symbol **Os**.

## Features

- **M-mode boot** — starts at the reset vector with `-bios none`, does its
  own firmware work (trap delegation, PMP, Sstc enable), and `mret`s into
  supervisor mode
- **NS16550A UART driver** with polled transmit and interrupt-driven receive
- **Trap handling** — a single S-mode vector for exceptions, interrupts, and
  syscalls, with an `sscratch` protocol that safely takes traps arriving
  from either kernel or user code
- **Timer interrupts** at 100 Hz via the Sstc extension (`stimecmp`), no
  M-mode trampoline needed
- **Physical page allocator** — intrusive free list threaded through the
  free pages themselves; zero metadata overhead, poison-on-free
- **Sv39 virtual memory** — three-level page tables built by hand,
  identity-mapped with least privilege per section (text R-X, rodata R--,
  data RW-, user code R-X+U)
- **Kernel heap** — first-fit, address-ordered, coalescing free-list
  allocator implementing `GlobalAlloc`, so the kernel uses `Box`, `Vec`,
  and `String` from the `alloc` crate
- **PLIC driver** with the claim/complete protocol routing device IRQs
- **Kernel threads** — 16 KiB heap-allocated stacks, forged initial
  contexts, cooperative `yield` plus timer-driven **preemptive round-robin
  scheduling**, xv6-style `sleep`/`wakeup` channels with lost-wakeup-proof
  check-then-sleep
- **User mode** — processes drop to U-privilege with `sret`, call back into
  the kernel through `ecall` syscalls (`write`, `exit`, `yield`, `getpid`,
  `sleep_ms`), and get **killed on faults instead of taking the kernel
  down**; `sys_write` validates every user pointer against the page table's
  U bit before copying under `sstatus.SUM`
- **Interactive shell** on the serial console (`ps`, `mem`, `uptime`,
  spawn-a-user-program, `poweroff`)
- **Boot self-tests** for every subsystem, checked in CI by booting the
  kernel headless in QEMU

## Demo

```
osmium kernel booting on hart 0
  kernel image: 0x80000000..0x80028000 (160 KiB)
  traps: stvec -> kernelvec
trap: breakpoint at 0x800084dc (S-mode)
  traps: survived an ebreak round-trip
  kalloc: 30680 free pages (119 MiB)
  kalloc: alloc/free self-test ok
  vm: sv39 paging on (root table 0x87ffe000)
  vm: translation self-test ok
  heap: 8192 KiB free, Box/Vec/String self-test ok
  plic: routing uart irq 10 to hart 0
  timer: 100 Hz ticks running, uptime 169 ms
  proc: entering scheduler
  thread ping (pid 1): round 0
  thread pong (pid 2): round 0
user: trespasser about to touch kernel memory...
trap: killing pid 7: load page fault at sepc=0x8000f096 stval=0x80000000
user: hello from U-mode via ecall! my pid is 6
syscall: pid 6 exited with code 0
  proc: preemptive multitasking verified (busy0=995212, busy1=1051582)

ALL BOOT TESTS PASSED

osmium shell -- 'help' lists commands
osmium> ps
  PID   NAME         STATE
  8     init         Running
osmium> mem
  phys: 30609 free pages (119 MiB of 128 MiB DRAM)
  heap: 8175 KiB free of 8192 KiB
```

The two U-mode programs above are the system's own proof: the *greeter*
completes write/sleep/getpid/exit syscalls and terminates cleanly; the
*trespasser* dereferences kernel memory from user mode and is killed by the
page-fault path while the kernel keeps running.

## Building and running

Requirements: stable Rust (the pinned toolchain installs the
`riscv64gc-unknown-none-elf` target automatically) and `qemu-system-riscv64`
(≥ 7.0 for the Sstc extension).

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
  └─ entry.S      park secondary harts, zero .bss, set boot stack
      └─ start()  M-mode Rust: delegate traps, open PMP, enable Sstc, mret
          └─ kmain()  S-mode: drivers + allocators + VM + self-tests
              └─ scheduler()  runs threads; init becomes the shell
                  └─ sret     user processes at U-privilege
```

### Memory map (QEMU virt + kernel layout)

| Address        | What                                | Mapping    |
| -------------- | ----------------------------------- | ---------- |
| `0x0010_0000`  | sifive_test (poweroff/reboot)       | RW-        |
| `0x0c00_0000`  | PLIC                                | RW-        |
| `0x1000_0000`  | UART0 (NS16550A)                    | RW-        |
| `0x8000_0000`  | kernel `.text`                      | R-X        |
| …              | `.user` (embedded U-mode programs)  | R-X **+U** |
| …              | `.rodata`                           | R--        |
| …              | `.data`, `.bss`, boot stack         | RW-        |
| kernel end     | kernel heap (8 MiB)                 | RW-        |
| heap end       | page allocator pool (~119 MiB)      | RW-        |
| `0x8800_0000`  | top of DRAM (`-m 128M`)             |            |

The kernel is identity-mapped (VA = PA) at 4 KiB granularity, which keeps
the design honest — permissions are real, but enabling `satp` mid-boot
doesn't move the ground underneath the running code. User stacks are single
pages granted the U bit in place and stripped of it on reap.

### Code tour

| File                 | Contents                                             |
| -------------------- | ---------------------------------------------------- |
| `src/entry.S`        | reset-vector assembly, boot stack                    |
| `src/main.rs`        | M-mode `start`, S-mode `kmain`, boot self-tests      |
| `src/riscv.rs`       | zero-cost CSR accessors (macro-generated)            |
| `src/memlayout.rs`   | physical map constants, linker-symbol accessors      |
| `src/uart.rs`        | NS16550A driver                                      |
| `src/console.rs`     | `println!` machinery + interrupt-fed input ring      |
| `src/spinlock.rs`    | interrupt-safe spinlock (push_off/pop_off nesting)   |
| `src/kernelvec.S`    | trap vector: build TrapFrame, dispatch, sret         |
| `src/trap.rs`        | exception/interrupt dispatch, user-fault isolation   |
| `src/timer.rs`       | Sstc timer, ticks, `sleep_ticks`                     |
| `src/kalloc.rs`      | physical page allocator                              |
| `src/vm.rs`          | Sv39 page tables, kernel mappings, walks             |
| `src/heap.rs`        | `GlobalAlloc` free-list heap                         |
| `src/plic.rs`        | PLIC claim/complete driver                           |
| `src/switch.S`       | `swtch`: callee-saved context switch                 |
| `src/proc.rs`        | threads, scheduler, sleep/wakeup, user-mode entry    |
| `src/syscall.rs`     | ecall dispatch + user-pointer validation             |
| `src/user.rs`        | embedded U-mode assembly programs                    |
| `src/shell.rs`       | interactive console shell                            |
| `src/power.rs`       | sifive_test poweroff/reboot                          |

## Design notes

- **Why identity mapping?** It sidesteps the classic trampoline problem
  (trap handlers must be mapped at the same VA in every address space)
  while still exercising everything that matters pedagogically: real page
  tables, real permission faults, real TLB shootdowns. Privilege separation
  comes from the U bit rather than separate address spaces.
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
