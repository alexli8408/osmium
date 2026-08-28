# Osmium

A RISC-V (RV64GC) operating system kernel written from scratch in Rust — no
external crates, no `libc`, no pre-built bootloader. It boots on QEMU's `virt`
machine straight out of reset in machine mode.

> Osmium: the densest naturally occurring metal. Chemical symbol **Os**.

## Status

Under active development. See the roadmap below.

- [x] Project scaffolding (bare-metal target, linker script, QEMU harness)
- [ ] M-mode boot → S-mode handoff, UART driver, `println!`
- [ ] Trap handling (exceptions + interrupts)
- [ ] Timer interrupts (sstc extension)
- [ ] Physical page allocator
- [ ] Sv39 virtual memory
- [ ] Kernel heap allocator (`Box`, `Vec`, `String`)
- [ ] PLIC + interrupt-driven console input
- [ ] Kernel threads with a preemptive round-robin scheduler
- [ ] User mode (U-privilege) processes with syscalls
- [ ] Interactive shell

## Requirements

- Rust (stable) with the `riscv64gc-unknown-none-elf` target
- `qemu-system-riscv64` (QEMU ≥ 7.0 for the sstc extension)

## Building and running

```sh
make run        # build + boot in QEMU (Ctrl-A X to quit)
make test       # headless boot, verifies the kernel self-tests pass
make gdb        # boot halted, waiting for a debugger on :1234
```

## License

MIT
