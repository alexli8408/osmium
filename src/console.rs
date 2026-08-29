//! Kernel console: `print!`/`println!` over the UART, behind a spinlock so
//! concurrent printers (kernel threads, interrupt handlers) don't interleave
//! mid-line — plus the interrupt-fed input buffer readers pull from.

use core::fmt::{self, Write};

use crate::spinlock::{self, SpinLock};
use crate::uart;

struct Console;

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                uart::put(b'\r');
            }
            uart::put(byte);
        }
        Ok(())
    }
}

static CONSOLE: SpinLock<Console> = SpinLock::new(Console);

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    // fmt::Write on Console cannot fail.
    let _ = CONSOLE.lock().write_fmt(args);
}

/// Lock-free print for the panic path: if the panicking code held the
/// console lock, going through it again would deadlock and eat the message.
#[doc(hidden)]
pub fn _print_emergency(args: fmt::Arguments) {
    let _ = Console.write_fmt(args);
}

const INPUT_CAP: usize = 256;

/// Ring buffer between the UART RX interrupt (producer) and console
/// readers (consumers). Indices grow without bound; the distance `w - r`
/// is the fill level, and `% INPUT_CAP` picks the slot.
struct Input {
    buf: [u8; INPUT_CAP],
    r: usize,
    w: usize,
}

static INPUT: SpinLock<Input> = SpinLock::new(Input {
    buf: [0; INPUT_CAP],
    r: 0,
    w: 0,
});

/// Called from the UART interrupt for each received byte. Stores raw bytes
/// (terminal CR normalized to NL); echo and line editing are the reader's
/// job. Bytes arriving into a full buffer are dropped.
pub fn on_input(byte: u8) {
    let byte = if byte == b'\r' { b'\n' } else { byte };
    {
        let mut input = INPUT.lock();
        if input.w - input.r < INPUT_CAP {
            let slot = input.w % INPUT_CAP;
            input.buf[slot] = byte;
            input.w += 1;
        }
    }
    crate::proc::wakeup(input_chan());
}

/// Pop one byte of console input, or None when the buffer is empty.
pub fn getchar() -> Option<u8> {
    let mut input = INPUT.lock();
    if input.r == input.w {
        None
    } else {
        let byte = input.buf[input.r % INPUT_CAP];
        input.r += 1;
        Some(byte)
    }
}

/// Wait channel for console input: the buffer's own address.
fn input_chan() -> usize {
    &raw const INPUT as usize
}

/// Block the calling thread until a byte of input arrives.
pub fn getchar_blocking() -> u8 {
    loop {
        spinlock::push_off();
        if let Some(byte) = getchar() {
            spinlock::pop_off();
            return byte;
        }
        // Empty. Sleep atomically w.r.t. the interrupt that fills it.
        crate::proc::sleep(input_chan());
        spinlock::pop_off();
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::console::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
