//! Thin, zero-cost wrappers around RISC-V CSRs and privileged instructions.
//!
//! Each CSR becomes a module with `read`/`write`/`set`/`clear` functions so
//! call sites read like `sstatus::set(sstatus::SIE)`. Writes are `unsafe`:
//! most CSRs can break memory safety (satp, stvec, ...) when misused.

#![allow(dead_code)]

use core::arch::asm;

macro_rules! csr {
    ($name:ident, $csr:expr) => {
        pub mod $name {
            #[allow(unused_imports)]
            use super::*;

            #[inline]
            pub fn read() -> usize {
                let x: usize;
                unsafe { core::arch::asm!(concat!("csrr {0}, ", $csr), out(reg) x) };
                x
            }

            #[inline]
            pub unsafe fn write(x: usize) {
                unsafe { core::arch::asm!(concat!("csrw ", $csr, ", {0}"), in(reg) x) };
            }

            /// Set the bits in `mask` (csrs).
            #[inline]
            pub unsafe fn set(mask: usize) {
                unsafe { core::arch::asm!(concat!("csrs ", $csr, ", {0}"), in(reg) mask) };
            }

            /// Clear the bits in `mask` (csrc).
            #[inline]
            pub unsafe fn clear(mask: usize) {
                unsafe { core::arch::asm!(concat!("csrc ", $csr, ", {0}"), in(reg) mask) };
            }
        }
    };
}

// Machine-level CSRs (only touched once, in `start`, before entering S-mode).
csr!(mhartid, "mhartid");
csr!(mstatus, "mstatus");
csr!(mepc, "mepc");
csr!(medeleg, "medeleg");
csr!(mideleg, "mideleg");
csr!(mcounteren, "mcounteren");
csr!(pmpcfg0, "pmpcfg0");
csr!(pmpaddr0, "pmpaddr0");
// menvcfg is newer than LLVM's baseline CSR table; use its number.
csr!(menvcfg, "0x30a");

// Supervisor-level CSRs.
csr!(sstatus, "sstatus");
csr!(sie, "sie");
csr!(sip, "sip");
csr!(sepc, "sepc");
csr!(stvec, "stvec");
csr!(scause, "scause");
csr!(stval, "stval");
csr!(satp, "satp");
csr!(sscratch, "sscratch");
csr!(scounteren, "scounteren");
// stimecmp is part of the Sstc extension; use its number.
csr!(stimecmp, "0x14d");

// mstatus bits
pub const MSTATUS_MPP_MASK: usize = 3 << 11;
pub const MSTATUS_MPP_S: usize = 1 << 11;

// sstatus bits
pub const SSTATUS_SIE: usize = 1 << 1; // supervisor interrupt enable
pub const SSTATUS_SPIE: usize = 1 << 5; // SIE value restored by sret
pub const SSTATUS_SPP: usize = 1 << 8; // privilege sret returns to (0 = U)
pub const SSTATUS_SUM: usize = 1 << 18; // permit S-mode access to U pages

// sie / sip bits
pub const SIE_SSIE: usize = 1 << 1; // software
pub const SIE_STIE: usize = 1 << 5; // timer
pub const SIE_SEIE: usize = 1 << 9; // external

// menvcfg bits
pub const MENVCFG_STCE: usize = 1 << 63; // enable the Sstc extension

// scause: top bit = interrupt, low bits = code
pub const SCAUSE_INTERRUPT: usize = 1 << 63;

/// Current cycle-accurate wall clock: reads the `time` CSR, which QEMU's virt
/// machine drives at 10 MHz.
#[inline]
pub fn r_time() -> usize {
    let x: usize;
    unsafe { asm!("rdtime {0}", out(reg) x) };
    x
}

/// Hart ID, stashed in `tp` by `start` because mhartid is M-mode-only.
#[inline]
pub fn cpu_id() -> usize {
    let x: usize;
    unsafe { asm!("mv {0}, tp", out(reg) x) };
    x
}

/// Enable supervisor interrupts on this hart.
#[inline]
pub fn intr_on() {
    unsafe { sstatus::set(SSTATUS_SIE) };
}

/// Disable supervisor interrupts on this hart.
#[inline]
pub fn intr_off() {
    unsafe { sstatus::clear(SSTATUS_SIE) };
}

/// Are supervisor interrupts enabled on this hart?
#[inline]
pub fn intr_get() -> bool {
    sstatus::read() & SSTATUS_SIE != 0
}

/// Wait for an interrupt (low-power idle).
#[inline]
pub fn wfi() {
    unsafe { asm!("wfi") };
}
