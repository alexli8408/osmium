//! A spinlock that is safe to share with interrupt handlers.
//!
//! Taking the lock disables supervisor interrupts on this hart first
//! (push_off), which is what makes it sound: if an interrupt handler could
//! run while the lock is held and try to take the same lock, the hart would
//! deadlock against itself. push_off/pop_off nest, and the outermost pop_off
//! restores whatever interrupt state the outermost push_off saw.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::riscv;

/// Per-hart interrupt-disable nesting depth and the pre-nesting SIE state.
/// The kernel schedules on a single hart, so plain statics suffice; they are
/// only ever touched with interrupts disabled (or while disabling them).
static NOFF: AtomicUsize = AtomicUsize::new(0);
static INTENA: AtomicBool = AtomicBool::new(false);

pub fn push_off() {
    let old = riscv::intr_get();
    riscv::intr_off();
    if NOFF.load(Ordering::Relaxed) == 0 {
        INTENA.store(old, Ordering::Relaxed);
    }
    NOFF.fetch_add(1, Ordering::Relaxed);
}

pub fn pop_off() {
    debug_assert!(!riscv::intr_get(), "pop_off with interrupts on");
    let n = NOFF.fetch_sub(1, Ordering::Relaxed);
    debug_assert!(n > 0, "pop_off without matching push_off");
    if n == 1 && INTENA.load(Ordering::Relaxed) {
        riscv::intr_on();
    }
}

pub struct SpinLock<T> {
    locked: AtomicBool,
    #[allow(dead_code)] // diagnostics
    name: &'static str,
    value: UnsafeCell<T>,
}

// The lock provides the synchronization that makes sharing sound.
unsafe impl<T: Send> Sync for SpinLock<T> {}
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(name: &'static str, value: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            name,
            value: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        push_off();
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        SpinLockGuard { lock: self }
    }

    /// Name given at construction; used in diagnostics.
    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        self.name
    }
}

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        pop_off();
    }
}
