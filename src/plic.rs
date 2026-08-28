//! PLIC (platform-level interrupt controller) driver.
//!
//! The PLIC fans all external device IRQs into one supervisor external
//! interrupt per hart context. QEMU virt places it at 0x0c00_0000 and
//! numbers hart 0's contexts 0 (M-mode) and 1 (S-mode); we only program
//! the S-mode context.
//!
//! Flow: device raises IRQ -> PLIC latches it -> hart takes SEIE trap ->
//! handler *claims* the IRQ number, services the device, then *completes*
//! it so the PLIC can deliver the next one.

use crate::memlayout::{PLIC, UART0_IRQ};

/// Hart 0, S-mode context number on QEMU virt.
const CONTEXT: usize = 1;

const PRIORITY_BASE: usize = PLIC;
const ENABLE_BASE: usize = PLIC + 0x2000 + 0x80 * CONTEXT;
const THRESHOLD: usize = PLIC + 0x20_0000 + 0x1000 * CONTEXT;
const CLAIM: usize = PLIC + 0x20_0004 + 0x1000 * CONTEXT;

#[inline]
fn write32(addr: usize, value: u32) {
    unsafe { (addr as *mut u32).write_volatile(value) };
}

#[inline]
fn read32(addr: usize) -> u32 {
    unsafe { (addr as *const u32).read_volatile() }
}

/// Global PLIC setup: give each device IRQ we use a nonzero priority
/// (priority 0 means "never deliver").
pub fn init() {
    write32(PRIORITY_BASE + 4 * UART0_IRQ as usize, 1);
}

/// Per-hart setup: unmask our IRQs for this hart's S context and accept
/// any priority > 0.
pub fn init_hart() {
    let enable_word = ENABLE_BASE + 4 * (UART0_IRQ as usize / 32);
    write32(enable_word, read32(enable_word) | (1 << (UART0_IRQ % 32)));
    write32(THRESHOLD, 0);
}

/// Ask the PLIC which IRQ fired. Returns None on a spurious interrupt
/// (another hart or context already claimed it).
pub fn claim() -> Option<u32> {
    match read32(CLAIM) {
        0 => None,
        irq => Some(irq),
    }
}

/// Tell the PLIC we're done servicing `irq`.
pub fn complete(irq: u32) {
    write32(CLAIM, irq);
}
