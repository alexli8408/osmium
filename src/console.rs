//! Kernel console: `print!`/`println!` over the UART, behind a spinlock so
//! concurrent printers (kernel threads, interrupt handlers) don't interleave
//! mid-line.

use core::fmt::{self, Write};

use crate::spinlock::SpinLock;
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

static CONSOLE: SpinLock<Console> = SpinLock::new("console", Console);

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

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::console::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
