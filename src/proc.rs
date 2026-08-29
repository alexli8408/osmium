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

/// Open files per process.
pub const MAX_FDS: usize = 8;

/// One open file: which ramfs file, and the read cursor into it.
#[derive(Clone, Copy)]
struct FdEntry {
    file_idx: usize,
    offset: usize,
}

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
        Context {
            ra: 0,
            sp: 0,
            s: [0; 12],
        }
    }
}

unsafe extern "C" {
    fn swtch(old: *mut Context, new: *const Context);
    /// Restore a full TrapFrame and sret to user (userret.S).
    fn userret(frame: *const crate::trap::TrapFrame, kstack_top: usize) -> !;
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
    pub name: &'static str,
    pub state: State,
    context: Context,
    entry: fn(),
    /// Top of the kernel stack; a user process's trap stack starts here.
    kstack_top: usize,
    /// U-mode entry point (virtual address), when this process runs user code.
    user_entry: usize,
    /// Initial U-mode stack pointer (virtual address).
    user_sp: usize,
    /// Root page table this process runs on. Kernel threads use the shared
    /// kernel table; user processes get their own (which also maps the
    /// kernel), stored here and freed at reap.
    page_table: usize,
    /// Some(root) if `page_table` is a private user table to free on reap.
    user_table: Option<usize>,
    /// Bottom of the user heap (fixed); `heap_brk` is its current top. Pages
    /// in [heap_base, heap_brk) are mapped lazily, on the first page fault
    /// that touches them.
    heap_base: usize,
    heap_brk: usize,
    /// Open-file table, indexed by file descriptor.
    fds: [Option<FdEntry>; MAX_FDS],
    /// For a forked child: the register state to restore on its first run,
    /// so it resumes exactly where the parent called fork.
    fork_frame: Option<Box<crate::trap::TrapFrame>>,
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
    debug_assert!(
        !riscv::intr_get(),
        "scheduler state touched with interrupts on"
    );
    unsafe { &mut *SCHED.0.get() }
}

/// Build a process, let `configure` fill in its user fields, and publish it
/// to the scheduler as Runnable. Everything is set *before* the process
/// becomes visible, so a preemption right after publication can never
/// observe a half-initialized thread.
fn spawn_with(name: &'static str, entry: fn(), configure: impl FnOnce(&mut Process)) -> usize {
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
        kstack_top: stack_top,
        user_entry: 0,
        user_sp: 0,
        page_table: crate::vm::kernel_root(),
        user_table: None,
        heap_base: 0,
        heap_brk: 0,
        fds: [None; MAX_FDS],
        fork_frame: None,
        _kstack: kstack,
    });
    configure(&mut proc);
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

/// Attach a freshly built user address space to a process: its private table,
/// entry/stack, and an empty heap at `USER_HEAP_BASE`.
fn attach_user_space(p: &mut Process, table: usize, entry: usize, sp: usize) {
    p.page_table = table;
    p.user_table = Some(table);
    p.user_entry = entry;
    p.user_sp = sp;
    p.heap_base = crate::vm::USER_HEAP_BASE;
    p.heap_brk = crate::vm::USER_HEAP_BASE;
}

/// Create a kernel thread that starts at `entry`; returns its pid.
pub fn spawn(name: &'static str, entry: fn()) -> usize {
    spawn_with(name, entry, |_| {})
}

/// Build a fresh user address space for `prog` = (link address, length):
/// copy the program image into private pages at `USER_TEXT` (R-X+U) and map
/// one stack page below `USER_STACK_TOP` (RW+U). Returns the new table.
fn load_user_image(prog: (usize, usize)) -> usize {
    use crate::memlayout::PAGE_SIZE;
    use crate::vm::{self, PTE_R, PTE_U, PTE_W, PTE_X, USER_STACK_TOP, USER_TEXT};

    let (src, len) = prog;
    assert!(len > 0, "empty user program");

    let table = vm::make_user_table();

    let npages = len.div_ceil(PAGE_SIZE);
    for i in 0..npages {
        let page = crate::kalloc::alloc().expect("no page for user text");
        let off = i * PAGE_SIZE;
        let n = (len - off).min(PAGE_SIZE);
        unsafe {
            core::ptr::copy_nonoverlapping((src + off) as *const u8, page as *mut u8, n);
            vm::map_pages(
                table,
                USER_TEXT + off,
                page,
                PAGE_SIZE,
                PTE_R | PTE_X | PTE_U,
            );
        }
    }

    let stack = crate::kalloc::alloc().expect("no page for user stack");
    unsafe {
        vm::map_pages(
            table,
            USER_STACK_TOP - PAGE_SIZE,
            stack,
            PAGE_SIZE,
            PTE_R | PTE_W | PTE_U,
        );
    }
    table
}

/// Load a user program into a fresh address space and start it. Returns the
/// new pid.
pub fn spawn_user(name: &'static str, prog: (usize, usize)) -> usize {
    let table = load_user_image(prog);
    spawn_with(name, user_thread_body, |p| {
        attach_user_space(p, table, crate::vm::USER_TEXT, crate::vm::USER_STACK_TOP);
    })
}

/// Replace the calling process's image with `prog`: build a fresh address
/// space, discard the old one, and enter the new program at `USER_TEXT`.
/// Open files are preserved (Unix semantics). Never returns on success.
pub fn exec(prog: (usize, usize)) -> ! {
    use crate::vm::{self, USER_HEAP_BASE, USER_STACK_TOP, USER_TEXT};

    let new_table = load_user_image(prog);

    let (old_table, kstack_top) = with_current(|p| {
        let old = p.user_table;
        p.page_table = new_table;
        p.user_table = Some(new_table);
        p.user_entry = USER_TEXT;
        p.user_sp = USER_STACK_TOP;
        p.heap_base = USER_HEAP_BASE;
        p.heap_brk = USER_HEAP_BASE;
        p.fork_frame = None;
        (old, p.kstack_top)
    })
    .expect("exec outside a process");

    // Commit to the new address space with interrupts off, then discard the
    // old one (now unreachable) and drop to the new program. The kernel map
    // is in both tables, so kernel code/stack stay addressable throughout.
    riscv::intr_off();
    vm::switch_to(new_table);
    if let Some(old) = old_table {
        unsafe { vm::free_user_table(old) };
    }
    unsafe { enter_user(USER_TEXT, USER_STACK_TOP, kstack_top) }
}

/// Duplicate the calling process: clone its address space, heap bounds, and
/// open files into a child that returns 0 from fork, while the parent gets
/// the child's pid. `parent_frame` is the parent's saved register state at
/// the fork ecall.
pub fn fork(parent_frame: &crate::trap::TrapFrame) -> usize {
    let Some((ptable, pbase, pbrk, pfds)) =
        with_current(|p| (p.page_table, p.heap_base, p.heap_brk, p.fds))
    else {
        return usize::MAX; // fork from a kernel thread: unsupported
    };

    let child_table = crate::vm::clone_user_table(ptable);

    // The child resumes at the same pc with the same registers, except fork
    // returns 0 in the child.
    let mut child_frame = Box::new(parent_frame.clone());
    child_frame.a0 = 0;

    spawn_with("u-fork-child", forkret_body, |p| {
        attach_user_space(p, child_table, parent_frame.sepc, parent_frame.sp);
        p.heap_base = pbase;
        p.heap_brk = pbrk;
        p.fds = pfds;
        p.fork_frame = Some(child_frame);
    })
}

/// First code a forked child runs: restore the saved frame and drop to user.
fn forkret_body() {
    // Keep the frame owned by the process (freed at reap); take only a
    // pointer to its stable heap allocation, which outlives this borrow and
    // stays valid across the userret that never returns.
    let (frame_ptr, kstack_top) = with_current(|p| {
        let fp = p
            .fork_frame
            .as_deref()
            .expect("forked child without a frame")
            as *const crate::trap::TrapFrame;
        (fp, p.kstack_top)
    })
    .expect("no current process");
    riscv::intr_off();
    unsafe { userret(frame_ptr, kstack_top) }
}

/// Kernel-side body of a user process: drop to U-mode. The address space is
/// already built (spawn_user) and satp already points at it (the scheduler
/// switched to page_table before running this thread).
fn user_thread_body() {
    let (entry, user_sp, kstack_top) =
        with_current(|p| (p.user_entry, p.user_sp, p.kstack_top)).expect("no current process");
    assert!(entry != 0, "user process with no entry point");
    unsafe { enter_user(entry, user_sp, kstack_top) }
}

/// Drop to U-mode: the sret twin of the M->S handoff in start().
///
/// # Safety
/// `entry` must be U-executable code and `user_sp` a U-writable stack.
unsafe fn enter_user(entry: usize, user_sp: usize, kstack_top: usize) -> ! {
    use crate::riscv::{SSTATUS_SPIE, SSTATUS_SPP, sepc, sscratch, sstatus};

    // No interrupts between arming sscratch and the sret: a trap in that
    // window would take the user path with a half-built state.
    riscv::intr_off();
    unsafe {
        sscratch::write(kstack_top); // kernelvec: "currently in U-mode"
        sepc::write(entry);
        sstatus::clear(SSTATUS_SPP); // sret goes to U
        sstatus::set(SSTATUS_SPIE); // ...with interrupts on
        core::arch::asm!("mv sp, {0}", "sret", in(reg) user_sp, options(noreturn));
    }
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

/// Wait channel woken whenever any process exits, so `join` can recheck.
const EXIT_CHAN: usize = 0xdead_0001;

/// Terminate the calling thread. Its stack is freed later by the
/// scheduler (which by then runs on its own stack).
pub fn exit() -> ! {
    push_off();
    {
        let sched = unsafe { sched_data() };
        let cur = sched.current.expect("exit outside a thread");
        sched.procs[cur].as_mut().unwrap().state = State::Zombie;
    }
    // Wake joiners before yielding for the last time (net push_off depth
    // stays 1, which is sched()'s precondition).
    wakeup(EXIT_CHAN);
    sched();
    unreachable!("zombie thread rescheduled");
}

/// Block until process `pid` has finished (become a zombie or been reaped).
/// Returns immediately if no such pid exists.
pub fn join(pid: usize) {
    loop {
        push_off();
        let s = unsafe { sched_data() };
        let still_running = s
            .procs
            .iter()
            .flatten()
            .any(|p| p.pid == pid && p.state != State::Zombie);
        if !still_running {
            pop_off();
            return;
        }
        // Atomic under push_off: the exit that would wake us can't slip
        // between the check above and this sleep.
        sleep(EXIT_CHAN);
        pop_off();
    }
}

/// Run `f` on the current process, with interrupts off. Returns None if
/// there is no current process (only true on the scheduler/boot context).
fn with_current<T>(f: impl FnOnce(&mut Process) -> T) -> Option<T> {
    push_off();
    let s = unsafe { sched_data() };
    let out = s.current.and_then(|cur| s.procs[cur].as_deref_mut()).map(f);
    pop_off();
    out
}

/// Install file `file_idx` in the calling process's fd table; returns the
/// new descriptor, or None if the table is full.
pub fn fd_install(file_idx: usize) -> Option<usize> {
    with_current(|p| {
        let fd = p.fds.iter().position(|slot| slot.is_none())?;
        p.fds[fd] = Some(FdEntry {
            file_idx,
            offset: 0,
        });
        Some(fd)
    })
    .flatten()
}

/// The (file index, current read offset) an fd refers to, or None if the fd
/// is not open in the calling process.
pub fn fd_lookup(fd: usize) -> Option<(usize, usize)> {
    with_current(|p| {
        p.fds
            .get(fd)
            .copied()
            .flatten()
            .map(|e| (e.file_idx, e.offset))
    })
    .flatten()
}

/// Advance an fd's read cursor by `delta`.
pub fn fd_advance(fd: usize, delta: usize) {
    with_current(|p| {
        if let Some(Some(e)) = p.fds.get_mut(fd) {
            e.offset += delta;
        }
    });
}

/// Close an fd; returns false if it was not open.
pub fn fd_close(fd: usize) -> bool {
    with_current(|p| match p.fds.get_mut(fd) {
        Some(slot @ Some(_)) => {
            *slot = None;
            true
        }
        _ => false,
    })
    .unwrap_or(false)
}

/// The page table of the calling process — the address space its user
/// pointers refer to. Falls back to the kernel table on a kernel thread.
pub fn current_root() -> usize {
    with_current(|p| p.page_table).unwrap_or_else(crate::vm::kernel_root)
}

/// Grow (or query, with delta 0) the calling process's heap. Returns the old
/// break — the base of the freshly reserved region — or `usize::MAX` if the
/// growth would exceed `USER_HEAP_MAX`. Only positive deltas are supported;
/// the new pages are mapped lazily by [`service_page_fault`], not here.
pub fn sbrk(delta: usize) -> usize {
    with_current(|p| {
        let old = p.heap_brk;
        let Some(new) = old.checked_add(delta) else {
            return usize::MAX;
        };
        if new > crate::vm::USER_HEAP_MAX {
            return usize::MAX;
        }
        p.heap_brk = new;
        old
    })
    .unwrap_or(usize::MAX)
}

/// Try to service a user page fault at `addr` by demand-mapping a heap page.
/// Returns true if a page was mapped (the faulting instruction should be
/// retried); false if the fault is not a growable-heap access and the
/// process should be killed.
pub fn service_page_fault(addr: usize) -> bool {
    use crate::memlayout::PAGE_SIZE;
    use crate::vm::{self, PTE_R, PTE_U, PTE_W};

    let Some((base, brk, table)) = with_current(|p| (p.heap_base, p.heap_brk, p.page_table)) else {
        return false;
    };
    if addr < base || addr >= brk {
        return false;
    }
    let page_va = addr & !(PAGE_SIZE - 1);
    // A mapped page faulting here would be a permission error, not a missing
    // page — don't paper over it.
    if vm::translate(table, page_va).is_some() {
        return false;
    }
    let Some(frame) = crate::kalloc::alloc() else {
        return false; // out of memory: let the process die
    };
    unsafe { vm::map_pages(table, page_va, frame, PAGE_SIZE, PTE_R | PTE_W | PTE_U) };
    crate::riscv::sfence_vma();
    true
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
                // Free the private address space (code + stack frames and the
                // page-table pages). Safe here: the scheduler runs on the
                // kernel table, so the table being freed is not active.
                if let Some(table) = slot.as_ref().unwrap().user_table {
                    unsafe { crate::vm::free_user_table(table) };
                }
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
                let table = sched.procs[idx].as_ref().unwrap().page_table;
                let new = &sched.procs[idx].as_ref().unwrap().context as *const Context;
                let old = &mut sched.scheduler_ctx as *mut Context;
                // Run the thread on its own address space (which also maps
                // the kernel), then take the kernel table back on return.
                crate::vm::switch_to(table);
                // The thread runs holding this push_off level and gives it
                // back when it swtches to us again.
                unsafe { swtch(old, new) };
                crate::vm::switch_to(crate::vm::kernel_root());
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
