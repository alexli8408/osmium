//! Syscall dispatch: the kernel side of `ecall` from U-mode.
//!
//! ABI: syscall number in a7, arguments in a0/a1, result returned in a0.
//! Numbers must match the user programs in user.rs.

use alloc::vec::Vec;

use crate::memlayout::PAGE_SIZE;
use crate::proc;
use crate::riscv::with_sum;
use crate::timer;
use crate::trap::TrapFrame;
use crate::vm::{self, PTE_U};

const SYS_WRITE: usize = 1;
const SYS_EXIT: usize = 2;
const SYS_YIELD: usize = 3;
const SYS_GETPID: usize = 4;
const SYS_SLEEP_MS: usize = 5;

/// Longest single write we accept from user space.
const WRITE_MAX: usize = 1024;

const ERR: usize = usize::MAX; // -1

/// Copy `len` bytes from user memory, first proving every touched page is
/// mapped with PTE_U. Walking the page table is the security boundary: a
/// bare pointer dereference would let user code read kernel memory through
/// the write syscall.
fn copy_from_user(ptr: usize, len: usize) -> Option<Vec<u8>> {
    // Reject a range that wraps the address space or leaves the Sv39 user
    // region *before* any of it reaches vm::walk (whose `va < VA_MAX`
    // assert would otherwise let a hostile pointer panic the kernel). A
    // rejected range returns None, which the caller turns into an error.
    let end = ptr.checked_add(len)?;
    if end > vm::VA_MAX {
        return None;
    }

    let root = vm::kernel_root();
    let mut page = ptr & !(PAGE_SIZE - 1);
    while page < end {
        let (_, pte) = vm::translate(root, page)?;
        if pte & PTE_U == 0 {
            return None;
        }
        page += PAGE_SIZE;
    }

    let mut buf = Vec::with_capacity(len);
    with_sum(|| {
        for i in 0..len {
            buf.push(unsafe { ((ptr + i) as *const u8).read_volatile() });
        }
    });
    Some(buf)
}

/// Handle an ecall from U-mode. The frame's a0 gets the return value, and
/// sepc advances past the (always 4-byte) ecall instruction.
pub fn dispatch(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let result = match frame.a7 {
        SYS_WRITE => sys_write(frame.a0, frame.a1),
        SYS_EXIT => {
            println!(
                "syscall: pid {} exited with code {}",
                proc::current_pid(),
                frame.a0
            );
            proc::exit();
        }
        SYS_YIELD => {
            proc::yield_now();
            0
        }
        SYS_GETPID => proc::current_pid(),
        SYS_SLEEP_MS => {
            // Saturate: a0 is user-controlled, so a plain multiply would
            // overflow-panic the kernel on a huge value in debug builds.
            let ticks = frame
                .a0
                .saturating_mul(timer::TICK_HZ)
                .div_ceil(1000)
                .max(1);
            timer::sleep_ticks(ticks);
            0
        }
        unknown => {
            println!(
                "syscall: pid {} made unknown syscall {unknown}",
                proc::current_pid()
            );
            ERR
        }
    };
    frame.a0 = result;
}

fn sys_write(ptr: usize, len: usize) -> usize {
    if len > WRITE_MAX {
        return ERR;
    }
    let Some(bytes) = copy_from_user(ptr, len) else {
        return ERR;
    };
    match core::str::from_utf8(&bytes) {
        Ok(s) => {
            print!("{s}");
            len
        }
        Err(_) => ERR,
    }
}
