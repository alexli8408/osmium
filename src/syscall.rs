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
const SYS_READ: usize = 6;
const SYS_UNAME: usize = 7;
const SYS_OPEN: usize = 8;
const SYS_FREAD: usize = 9;
const SYS_CLOSE: usize = 10;

/// Longest path SYS_OPEN accepts.
const PATH_MAX: usize = 64;

/// Longest single transfer we accept across the user boundary.
const WRITE_MAX: usize = 1024;

/// Reported by SYS_UNAME.
const UNAME: &[u8] = b"osmium 0.1.0 riscv64\n";

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

/// Copy `src` into user memory at `ptr`, first proving every touched page is
/// mapped U **and writable** (write-direction mirror of copy_from_user).
/// Returns the number of bytes written, or None if the range is rejected.
fn copy_to_user(ptr: usize, src: &[u8]) -> Option<usize> {
    let end = ptr.checked_add(src.len())?;
    if end > vm::VA_MAX {
        return None;
    }

    let root = vm::kernel_root();
    let mut page = ptr & !(PAGE_SIZE - 1);
    while page < end {
        let (_, pte) = vm::translate(root, page)?;
        if pte & PTE_U == 0 || pte & vm::PTE_W == 0 {
            return None;
        }
        page += PAGE_SIZE;
    }

    with_sum(|| {
        for (i, &byte) in src.iter().enumerate() {
            unsafe { ((ptr + i) as *mut u8).write_volatile(byte) };
        }
    });
    Some(src.len())
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
        SYS_READ => sys_read(frame.a0, frame.a1),
        SYS_UNAME => sys_uname(frame.a0, frame.a1),
        SYS_OPEN => sys_open(frame.a0, frame.a1),
        SYS_FREAD => sys_fread(frame.a0, frame.a1, frame.a2),
        SYS_CLOSE => {
            if proc::fd_close(frame.a0) {
                0
            } else {
                ERR
            }
        }
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

/// Blocking console read into a user buffer. Waits (sleeping, not spinning)
/// for at least one byte, then drains whatever is already buffered, up to
/// `len`. Returns the byte count, or ERR if the buffer is rejected.
fn sys_read(ptr: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let cap = len.min(WRITE_MAX);

    let mut buf = Vec::with_capacity(cap);
    buf.push(crate::console::getchar_blocking());
    while buf.len() < cap {
        match crate::console::getchar() {
            Some(byte) => buf.push(byte),
            None => break,
        }
    }

    match copy_to_user(ptr, &buf) {
        Some(n) => n,
        None => ERR,
    }
}

/// Copy the kernel version string into a user buffer; returns its length.
fn sys_uname(ptr: usize, len: usize) -> usize {
    let n = UNAME.len().min(len);
    match copy_to_user(ptr, &UNAME[..n]) {
        Some(written) => written,
        None => ERR,
    }
}

/// Open a ramfs file by name; returns a file descriptor or ERR.
fn sys_open(path_ptr: usize, path_len: usize) -> usize {
    if path_len == 0 || path_len > PATH_MAX {
        return ERR;
    }
    let Some(bytes) = copy_from_user(path_ptr, path_len) else {
        return ERR;
    };
    let Ok(name) = core::str::from_utf8(&bytes) else {
        return ERR;
    };
    let Some(file_idx) = crate::fs::lookup(name) else {
        return ERR;
    };
    proc::fd_install(file_idx).unwrap_or(ERR)
}

/// Read up to `len` bytes from an open fd into a user buffer, advancing the
/// fd's cursor. Returns the byte count (0 at EOF) or ERR.
fn sys_fread(fd: usize, ptr: usize, len: usize) -> usize {
    let Some((file_idx, offset)) = proc::fd_lookup(fd) else {
        return ERR;
    };
    let cap = len.min(WRITE_MAX);
    if cap == 0 {
        return 0;
    }
    let mut buf = alloc::vec![0u8; cap];
    let n = crate::fs::read_at(file_idx, offset, &mut buf);
    match copy_to_user(ptr, &buf[..n]) {
        Some(written) => {
            proc::fd_advance(fd, written);
            written
        }
        None => ERR,
    }
}
