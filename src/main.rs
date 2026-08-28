//! Osmium: a RISC-V (RV64GC) kernel for QEMU's virt machine.
//!
//! Boot flow: QEMU resets into `_entry` (entry.S) in machine mode, which
//! calls `start` below. `start` does the minimum M-mode configuration —
//! delegate traps to S-mode, open the PMP, enable the Sstc timer extension —
//! then fakes a trap return into supervisor mode at `kmain`. Everything
//! after that runs in S-mode.

#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

core::arch::global_asm!(include_str!("entry.S"));

#[macro_use]
mod console;
mod memlayout;
mod riscv;
mod spinlock;
mod uart;

use riscv::*;

/// M-mode setup, called from entry.S on the boot stack. Never returns; it
/// `mret`s into `kmain` in S-mode.
#[unsafe(no_mangle)]
extern "C" fn start() -> ! {
    unsafe {
        // Tell mret where to go and in which privilege: S-mode, at kmain.
        let m = (mstatus::read() & !MSTATUS_MPP_MASK) | MSTATUS_MPP_S;
        mstatus::write(m);
        mepc::write(kmain as *const () as usize);

        // Paging off until the VM module builds a page table.
        satp::write(0);

        // Route all exceptions and interrupts to S-mode handlers.
        medeleg::write(0xffff);
        mideleg::write(0xffff);
        sie::set(SIE_SEIE | SIE_STIE | SIE_SSIE);

        // PMP: without at least one matching entry, S-mode gets an access
        // fault on every memory reference. One TOR entry covering the whole
        // 56-bit physical space with RWX opens it up.
        pmpaddr0::write(0x3f_ffff_ffff_ffff);
        pmpcfg0::write(0xf);

        // Sstc: let S-mode program its own timer through the stimecmp CSR
        // (no M-mode trampoline needed). Park it at "never" for now, and
        // let S-mode (and later U-mode) read cycle/time/instret.
        menvcfg::set(MENVCFG_STCE);
        mcounteren::set(0b111);
        stimecmp::write(usize::MAX);

        // S-mode cannot read mhartid; carry the hart ID in tp.
        asm!("csrr tp, mhartid");

        asm!("mret", options(noreturn));
    }
}

/// Supervisor-mode entry point.
#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    uart::init();
    println!();
    println!("osmium kernel booting on hart {}", cpu_id());
    println!(
        "  kernel image: {:#x}..{:#x} ({} KiB)",
        memlayout::kernel_start(),
        memlayout::kernel_end(),
        (memlayout::kernel_end() - memlayout::kernel_start()) / 1024
    );

    loop {
        wfi();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Interrupts off: nothing may run "under" a dying kernel.
    intr_off();
    // Bypass the console lock — the panicking context may hold it.
    console::_print_emergency(format_args!("\n!!! KERNEL PANIC: {}\n", info));
    loop {
        wfi();
    }
}
