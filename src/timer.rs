//! Timer interrupts via the SBI TIME extension.
//!
//! QEMU virt's timebase runs at 10 MHz. The timer compare register lives
//! behind machine mode, which belongs to OpenSBI — so the kernel reads the
//! clock directly (rdtime is an unprivileged CSR the firmware exposes) and
//! asks the firmware to arm each interrupt with sbi_set_timer. Arming also
//! acknowledges the pending supervisor timer interrupt, which is why
//! on_tick needs no separate clear.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::riscv;
use crate::{proc, sbi, spinlock};

/// Wait channel signaled by every timer tick (arbitrary unique token).
pub const TICK_CHAN: usize = 0x711c_c4a7;

/// QEMU virt timebase, in Hz.
pub const TIMEBASE_FREQ: usize = 10_000_000;
/// Scheduler tick rate.
pub const TICK_HZ: usize = 100;
const TICK_INTERVAL: usize = TIMEBASE_FREQ / TICK_HZ;

static TICKS: AtomicUsize = AtomicUsize::new(0);

/// Arm the first tick. Interrupts need not be enabled yet; the interrupt
/// pends until sstatus.SIE goes high.
pub fn init() {
    assert!(sbi::probe_time(), "firmware lacks the SBI TIME extension");
    sbi::set_timer(riscv::r_time() + TICK_INTERVAL);
}

/// Called from the trap handler on every S-timer interrupt. Re-arming via
/// the firmware is what clears the pending interrupt.
pub fn on_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    sbi::set_timer(riscv::r_time() + TICK_INTERVAL);
}

/// Ticks since boot (one every 1/TICK_HZ seconds).
pub fn ticks() -> usize {
    TICKS.load(Ordering::Relaxed)
}

/// Milliseconds since boot, straight from the timebase.
pub fn uptime_ms() -> usize {
    riscv::r_time() / (TIMEBASE_FREQ / 1000)
}

/// Put the calling thread to sleep for at least `n` ticks. The
/// check-then-sleep runs under push_off, so a tick (and its wakeup)
/// cannot slip between the check and the sleep.
pub fn sleep_ticks(n: usize) {
    let target = ticks() + n;
    while ticks() < target {
        spinlock::push_off();
        if ticks() < target {
            proc::sleep(TICK_CHAN);
        }
        spinlock::pop_off();
    }
}
