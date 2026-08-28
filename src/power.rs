//! Power control via QEMU virt's sifive_test device: a magic 32-bit write
//! to 0x10_0000 makes QEMU exit (poweroff) or reset the machine.

use crate::memlayout::VIRT_TEST;
use crate::riscv;

const FINISHER_PASS: u32 = 0x5555; // exit code 0
const FINISHER_RESET: u32 = 0x7777;

pub fn poweroff() -> ! {
    println!("power: goodbye");
    unsafe { (VIRT_TEST as *mut u32).write_volatile(FINISHER_PASS) };
    // Not reached under QEMU; belt-and-suspenders for anything else.
    loop {
        riscv::wfi();
    }
}

pub fn reboot() -> ! {
    println!("power: rebooting");
    unsafe { (VIRT_TEST as *mut u32).write_volatile(FINISHER_RESET) };
    loop {
        riscv::wfi();
    }
}
