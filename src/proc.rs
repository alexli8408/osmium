//! Kernel threads and the round-robin scheduler.
//!
//! Concurrency model: the kernel schedules on a single hart, and every
//! access to the scheduler state happens with supervisor interrupts
//! disabled (push_off held). That pair of facts is the entire safety
//! argument for the `SchedCell` below — there is exactly one hart, and it
//! cannot be re-entered by an interrupt while it holds the state.
//!
//! Each thread runs on its own 16 KiB heap-allocated kernel stack. The
//! scheduler itself runs on the boot stack; `swtch` (switch.S) moves the
//! hart between the scheduler context and thread contexts by swapping the
//! callee-saved register set.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::riscv;
use crate::spinlock::{self, pop_off, push_off};

pub const KSTACK_SIZE: usize = 16 * 1024;

/// Callee-saved register set; layout must match switch.S.
#[repr(C)]
#[derive(Default)]
pub struct Context {
    pub ra: usize,
    pub sp: usize,
    pub s: [usize; 12],
}

impl Context {
    pub const fn zeroed() -> Self {
        Context { ra: 0, sp: 0, s: [0; 12] }
    }
}

unsafe extern "C" {
    fn swtch(old: *mut Context, new: *const Context);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Runnable,
    Running,
    /// Waiting for a wakeup() on this channel value.
    Sleeping(usize),
    Zombie,
}

pub struct Process {
    pub pid: usize,
    #[allow(dead_code)] // read by process_list for the shell's ps
    pub name: &'static str,
    pub state: State,
    context: Context,
    entry: fn(),
    /// Owns the stack allocation; freed when the zombie is reaped.
    _kstack: Box<[u8]>,
}

struct Scheduler {
    /// Slot table: indices stay stable across exits (slots go back to None).
    procs: Vec<Option<Box<Process>>>,
    /// Slot index of the thread this hart is executing, if any.
    current: Option<usize>,
    /// Round-robin position.
    next_slot: usize,
    /// The scheduler loop's own context, living on the boot stack.
    scheduler_ctx: Context,
}

/// See the module comment: single hart + interrupts-off is the lock.
struct SchedCell(UnsafeCell<Scheduler>);
unsafe impl Sync for SchedCell {}

static SCHED: SchedCell = SchedCell(UnsafeCell::new(Scheduler {
    procs: Vec::new(),
    current: None,
    next_slot: 0,
    scheduler_ctx: Context::zeroed(),
}));

static NEXT_PID: AtomicUsize = AtomicUsize::new(1);

/// # Safety
/// Caller must hold push_off (interrupts disabled) for the borrow's whole
/// lifetime, and must not create overlapping borrows across swtch calls.
#[allow(clippy::mut_from_ref)]
unsafe fn sched_data() -> &'static mut Scheduler {
    debug_assert!(!riscv::intr_get(), "scheduler state touched with interrupts on");
    unsafe { &mut *SCHED.0.get() }
}

/// Create a kernel thread that starts at `entry`; returns its pid.
pub fn spawn(name: &'static str, entry: fn()) -> usize {
    let kstack = vec![0u8; KSTACK_SIZE].into_boxed_slice();
    // Stack grows down from the top, 16-byte aligned per the ABI.
    let stack_top = (kstack.as_ptr() as usize + KSTACK_SIZE) & !0xf;
    let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);

    let mut proc = Box::new(Process {
        pid,
        name,
        state: State::Runnable,
        context: Context::zeroed(),
        entry,
        _kstack: kstack,
    });
    // Forged context: first swtch into this thread "returns" to
    // thread_entry on the fresh stack.
    proc.context.ra = thread_entry as *const () as usize;
    proc.context.sp = stack_top;

    push_off();
    let sched = unsafe { sched_data() };
    match sched.procs.iter_mut().position(|slot| slot.is_none()) {
        Some(slot) => sched.procs[slot] = Some(proc),
        None => sched.procs.push(Some(proc)),
    }
    pop_off();
    pid
}

/// First code every thread runs, entered via the forged context's `ra`.
extern "C" fn thread_entry() {
    // We arrive holding the scheduler's push_off level; release it and run
    // the thread body with interrupts on.
    pop_off();
    riscv::intr_on();

    let entry = {
        push_off();
        let sched = unsafe { sched_data() };
        let cur = sched.current.expect("thread_entry with no current");
        let entry = sched.procs[cur].as_ref().expect("current slot empty").entry;
        pop_off();
        entry
    };
    entry();
    exit();
}

/// Hand the hart to the scheduler. Caller must hold exactly one push_off
/// level and have already set current's state to its non-Running value.
fn sched() {
    debug_assert!(spinlock::holding_one(), "sched: push_off depth != 1");
    let sched = unsafe { sched_data() };
    let cur = sched.current.expect("sched: no current thread");
    let proc = sched.procs[cur].as_mut().expect("sched: empty slot");
    debug_assert!(proc.state != State::Running, "sched: still Running");

    let intena = spinlock::saved_intena();
    let old = &mut proc.context as *mut Context;
    let new = &sched.scheduler_ctx as *const Context;
    unsafe { swtch(old, new) };
    // Back on this thread's stack; restore its interrupt intent.
    spinlock::set_intena(intena);
}

/// Cooperatively give up the CPU.
pub fn yield_now() {
    push_off();
    {
        let s = unsafe { sched_data() };
        if let Some(cur) = s.current {
            s.procs[cur].as_mut().unwrap().state = State::Runnable;
            sched();
        }
    }
    pop_off();
}

/// Called from the timer interrupt: preempt the running thread, if any.
pub fn on_tick_preempt() {
    // In an interrupt handler SIE is already clear, but push_off keeps the
    // nesting bookkeeping uniform.
    push_off();
    let should_yield = {
        let sched = unsafe { sched_data() };
        match sched.current {
            Some(cur) => sched.procs[cur].as_ref().unwrap().state == State::Running,
            None => false,
        }
    };
    pop_off();
    if should_yield {
        yield_now();
    }
}

/// Block until `wakeup(chan)`. Caller must hold exactly one push_off level;
/// the check-condition/sleep sequence is atomic w.r.t. interrupts, which is
/// what prevents lost wakeups on a single hart.
pub fn sleep(chan: usize) {
    let s = unsafe { sched_data() };
    let cur = s.current.expect("sleep outside a thread");
    s.procs[cur].as_mut().unwrap().state = State::Sleeping(chan);
    sched();
}

/// Make every thread sleeping on `chan` runnable. Safe from interrupts.
pub fn wakeup(chan: usize) {
    push_off();
    let sched = unsafe { sched_data() };
    for slot in sched.procs.iter_mut().flatten() {
        if slot.state == State::Sleeping(chan) {
            slot.state = State::Runnable;
        }
    }
    pop_off();
}

/// Terminate the calling thread. Its stack is freed later by the
/// scheduler (which by then runs on its own stack).
pub fn exit() -> ! {
    push_off();
    {
        let sched = unsafe { sched_data() };
        let cur = sched.current.expect("exit outside a thread");
        sched.procs[cur].as_mut().unwrap().state = State::Zombie;
    }
    sched();
    unreachable!("zombie thread rescheduled");
}

/// The pid of the calling thread (0 for the scheduler/boot context).
pub fn current_pid() -> usize {
    push_off();
    let sched = unsafe { sched_data() };
    let pid = sched
        .current
        .and_then(|cur| sched.procs[cur].as_ref())
        .map_or(0, |p| p.pid);
    pop_off();
    pid
}

/// Snapshot of (pid, name, state) for diagnostics (the shell's `ps`).
#[allow(dead_code)] // the shell arrives shortly
pub fn process_list() -> Vec<(usize, &'static str, State)> {
    push_off();
    let sched = unsafe { sched_data() };
    let list = sched
        .procs
        .iter()
        .flatten()
        .map(|p| (p.pid, p.name, p.state))
        .collect();
    pop_off();
    list
}

/// The scheduler loop. Runs forever on the boot stack; never returns.
pub fn scheduler() -> ! {
    loop {
        // Window for pending interrupts (and their wakeups) to land.
        riscv::intr_on();

        push_off();
        let sched = unsafe { sched_data() };

        // Reap zombies: safe here because this code runs on the boot
        // stack, never on the stack being freed.
        for slot in sched.procs.iter_mut() {
            if matches!(slot.as_deref(), Some(p) if p.state == State::Zombie) {
                *slot = None;
            }
        }

        // Round-robin scan for the next runnable thread.
        let n = sched.procs.len();
        let mut picked = None;
        for i in 0..n {
            let idx = (sched.next_slot + i) % n;
            if matches!(sched.procs[idx].as_deref(), Some(p) if p.state == State::Runnable) {
                picked = Some(idx);
                break;
            }
        }

        match picked {
            Some(idx) => {
                sched.next_slot = idx + 1;
                sched.current = Some(idx);
                sched.procs[idx].as_mut().unwrap().state = State::Running;
                let new = &sched.procs[idx].as_ref().unwrap().context as *const Context;
                let old = &mut sched.scheduler_ctx as *mut Context;
                // The thread runs holding this push_off level and gives it
                // back when it swtches to us again.
                unsafe { swtch(old, new) };
                let sched = unsafe { sched_data() };
                sched.current = None;
                pop_off();
            }
            None => {
                pop_off();
                // Nothing runnable: idle until an interrupt changes that.
                riscv::intr_on();
                riscv::wfi();
            }
        }
    }
}
