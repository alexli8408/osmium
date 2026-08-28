//! NS16550A UART driver (QEMU virt maps one at 0x1000_0000).
//!
//! Transmit is polled for now; interrupt-driven receive arrives with the
//! PLIC. Registers are byte-wide and accessed with volatile ops so the
//! compiler can neither reorder nor elide them.

use crate::memlayout::UART0;

// Register offsets (some share an offset and differ by read vs write).
const RBR: usize = 0; // receive buffer (read)
const THR: usize = 0; // transmit holding (write)
const IER: usize = 1; // interrupt enable
const FCR: usize = 2; // FIFO control (write)
const LCR: usize = 3; // line control
const LSR: usize = 5; // line status

const IER_RX_ENABLE: u8 = 1 << 0;
const FCR_FIFO_ENABLE: u8 = 1 << 0;
const FCR_FIFO_CLEAR: u8 = 3 << 1;
const LCR_EIGHT_BITS: u8 = 3;
const LCR_DLAB: u8 = 1 << 7;
const LSR_RX_READY: u8 = 1 << 0;
const LSR_TX_IDLE: u8 = 1 << 5;

#[inline]
fn reg_read(offset: usize) -> u8 {
    unsafe { ((UART0 + offset) as *const u8).read_volatile() }
}

#[inline]
fn reg_write(offset: usize, value: u8) {
    unsafe { ((UART0 + offset) as *mut u8).write_volatile(value) }
}

pub fn init() {
    // No interrupts while configuring.
    reg_write(IER, 0x00);
    // Set the baud divisor (latched behind DLAB). QEMU ignores the actual
    // rate, but real 16550s need it: 38.4Kbaud with the standard clock.
    reg_write(LCR, LCR_DLAB);
    reg_write(0, 0x03); // divisor low
    reg_write(1, 0x00); // divisor high
    // 8 data bits, no parity, 1 stop bit; leave DLAB.
    reg_write(LCR, LCR_EIGHT_BITS);
    // Enable and reset the FIFOs.
    reg_write(FCR, FCR_FIFO_ENABLE | FCR_FIFO_CLEAR);
}

/// Enable the receive-data interrupt. Called once the PLIC is routing UART
/// IRQs; before that the bit would just be ignored.
#[allow(dead_code)] // wired up when the PLIC lands
pub fn enable_rx_interrupt() {
    reg_write(IER, IER_RX_ENABLE);
}

/// Blocking transmit of one byte (spins on the FIFO having room).
pub fn put(byte: u8) {
    while reg_read(LSR) & LSR_TX_IDLE == 0 {}
    reg_write(THR, byte);
}

/// Non-blocking receive: `None` when the RX FIFO is empty.
#[allow(dead_code)] // used by the console input path once the PLIC lands
pub fn get() -> Option<u8> {
    if reg_read(LSR) & LSR_RX_READY != 0 {
        Some(reg_read(RBR))
    } else {
        None
    }
}
