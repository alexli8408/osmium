//! Blocking synchronization primitives built on the scheduler's
//! sleep/wakeup: a counting semaphore and a bounded blocking channel.
//!
//! Correctness rests on the same single-hart, interrupts-off argument as
//! the rest of the kernel. A semaphore's `wait` checks the count and sleeps
//! **under one push_off level**, so on this hart nothing can run between the
//! check and the sleep — a concurrent `signal` therefore can't be lost.

use alloc::collections::VecDeque;

use crate::proc;
use crate::spinlock::{SpinLock, pop_off, push_off};

/// A counting semaphore. `wait` (P) blocks while the count is zero; `signal`
/// (V) raises it and wakes one or more waiters.
pub struct Semaphore {
    count: SpinLock<usize>,
}

impl Semaphore {
    pub const fn new(initial: usize) -> Self {
        Semaphore {
            count: SpinLock::new("sem", initial),
        }
    }

    /// The sleep/wakeup channel for this semaphore is its own address.
    fn chan(&self) -> usize {
        self as *const _ as usize
    }

    /// Decrement, blocking (sleeping) until the count is positive.
    pub fn wait(&self) {
        loop {
            push_off();
            {
                let mut count = self.count.lock();
                if *count > 0 {
                    *count -= 1;
                    drop(count);
                    pop_off();
                    return;
                }
            }
            // Count is zero: sleep atomically w.r.t. any signal, since
            // interrupts are off and we don't yield between check and sleep.
            proc::sleep(self.chan());
            pop_off();
        }
    }

    /// Increment and wake waiters.
    pub fn signal(&self) {
        push_off();
        *self.count.lock() += 1;
        pop_off();
        proc::wakeup(self.chan());
    }
}

/// A bounded, blocking, multi-producer/multi-consumer queue. `send` blocks
/// while full, `recv` blocks while empty. The two semaphores count free and
/// filled slots respectively, so the buffer never overruns its capacity and
/// no `recv` ever observes an empty buffer.
pub struct Channel<T> {
    buf: SpinLock<VecDeque<T>>,
    empty: Semaphore, // free slots
    full: Semaphore,  // filled slots
}

impl<T> Channel<T> {
    /// A channel that holds at most `capacity` items.
    pub const fn new(capacity: usize) -> Self {
        Channel {
            buf: SpinLock::new("chan", VecDeque::new()),
            empty: Semaphore::new(capacity),
            full: Semaphore::new(0),
        }
    }

    /// Enqueue `item`, blocking while the channel is full.
    pub fn send(&self, item: T) {
        self.empty.wait();
        self.buf.lock().push_back(item);
        self.full.signal();
    }

    /// Dequeue an item, blocking while the channel is empty.
    pub fn recv(&self) -> T {
        self.full.wait();
        let item = self
            .buf
            .lock()
            .pop_front()
            .expect("channel invariant: full semaphore without an item");
        self.empty.signal();
        item
    }
}
