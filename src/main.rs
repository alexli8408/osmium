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
core::arch::global_asm!(include_str!("kernelvec.S"));

#[macro_use]
mod console;
mod kalloc;
mod memlayout;
mod riscv;
mod spinlock;
mod timer;
mod trap;
mod uart;
mod vm;

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

    trap::init();
    println!("  traps: stvec -> kernelvec");

    // Prove the whole save/dispatch/restore path works: take a synchronous
    // breakpoint exception and come back from it.
    unsafe { asm!("ebreak") };
    println!("  traps: survived an ebreak round-trip");

    kalloc::init();
    println!(
        "  kalloc: {} free pages ({} MiB)",
        kalloc::free_pages(),
        kalloc::free_pages() * memlayout::PAGE_SIZE / (1024 * 1024)
    );

    // Exercise the allocator: pages must come back zeroed and be reusable.
    {
        let a = kalloc::alloc().expect("out of pages");
        let b = kalloc::alloc().expect("out of pages");
        assert_ne!(a, b);
        assert!(unsafe { core::slice::from_raw_parts(a as *const u8, memlayout::PAGE_SIZE) }
            .iter()
            .all(|&x| x == 0));
        unsafe {
            kalloc::free(a);
            kalloc::free(b);
        }
        println!("  kalloc: alloc/free self-test ok");
    }

    vm::init();
    vm::init_hart();
    {
        // Software-walk the live table and check least-privilege held.
        let root = vm::kernel_root();
        let (pa, pte) = vm::translate(root, memlayout::text_start()).unwrap();
        assert_eq!(pa, memlayout::text_start(), "text must be identity-mapped");
        assert!(pte & vm::PTE_X != 0 && pte & vm::PTE_W == 0, "text must be R-X");
        let (_, pte) = vm::translate(root, memlayout::rodata_start()).unwrap();
        assert!(pte & (vm::PTE_W | vm::PTE_X) == 0, "rodata must be R--");
        let (_, pte) = vm::translate(root, memlayout::data_start()).unwrap();
        assert!(pte & vm::PTE_W != 0 && pte & vm::PTE_X == 0, "data must be RW-");
        let (_, pte) = vm::translate(root, memlayout::UART0).unwrap();
        assert!(pte & vm::PTE_W != 0, "uart must be writable");
        assert!(vm::translate(root, 0x4000_0000).is_none(), "hole must be unmapped");
        println!("  vm: translation self-test ok");
    }

    timer::init();
    intr_on();
    let start_ticks = timer::ticks();
    while timer::ticks() < start_ticks + 3 {
        wfi();
    }
    println!(
        "  timer: {} Hz ticks running, uptime {} ms",
        timer::TICK_HZ,
        timer::uptime_ms()
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
