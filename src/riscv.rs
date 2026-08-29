//! Thin, zero-cost wrappers around RISC-V CSRs and privileged instructions.
//!
//! Each CSR becomes a module with `read`/`write`/`set`/`clear` functions so
//! call sites read like `sstatus::set(sstatus::SIE)`. Writes are `unsafe`:
//! most CSRs can break memory safety (satp, stvec, ...) when misused.

use core::arch::asm;

// Every CSR gets the uniform read/write/set/clear quartet; not every
// variant has a caller, so the generated functions carry the allow. CSR
// modules with *no* callers at all should be deleted, and are not shielded.
macro_rules! csr {
    ($name:ident, $csr:expr) => {
        pub mod $name {
            #[allow(unused_imports)]
            use super::*;

            #[allow(dead_code)]
            #[inline]
            pub fn read() -> usize {
                let x: usize;
                unsafe { core::arch::asm!(concat!("csrr {0}, ", $csr), out(reg) x) };
                x
            }

            #[allow(dead_code)]
            #[inline]
            pub unsafe fn write(x: usize) {
                unsafe { core::arch::asm!(concat!("csrw ", $csr, ", {0}"), in(reg) x) };
            }

            /// Set the bits in `mask` (csrs).
            #[allow(dead_code)]
            #[inline]
            pub unsafe fn set(mask: usize) {
                unsafe { core::arch::asm!(concat!("csrs ", $csr, ", {0}"), in(reg) mask) };
            }

            /// Clear the bits in `mask` (csrc).
            #[allow(dead_code)]
            #[inline]
            pub unsafe fn clear(mask: usize) {
                unsafe { core::arch::asm!(concat!("csrc ", $csr, ", {0}"), in(reg) mask) };
            }
        }
    };
}

// Supervisor-level CSRs only: machine mode belongs to the OpenSBI
// firmware, and every M CSR would fault if touched from here.
csr!(sstatus, "sstatus");
csr!(sie, "sie");
csr!(sepc, "sepc");
csr!(stvec, "stvec");
csr!(scause, "scause");
csr!(stval, "stval");
csr!(satp, "satp");
csr!(sscratch, "sscratch");

// sstatus bits
pub const SSTATUS_SIE: usize = 1 << 1; // supervisor interrupt enable
pub const SSTATUS_SPIE: usize = 1 << 5; // SIE value restored by sret
pub const SSTATUS_SPP: usize = 1 << 8; // privilege sret returns to (0 = U)
pub const SSTATUS_SUM: usize = 1 << 18; // permit S-mode access to U pages

// sie bits
pub const SIE_SSIE: usize = 1 << 1; // software
pub const SIE_STIE: usize = 1 << 5; // timer
pub const SIE_SEIE: usize = 1 << 9; // external

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

/// Flush the whole TLB on this hart.
#[inline]
pub fn sfence_vma() {
    unsafe { asm!("sfence.vma zero, zero") };
}

/// Run `f` with sstatus.SUM set, letting S-mode touch U-marked pages.
/// Restores the previous SUM state (calls may nest).
pub fn with_sum<T>(f: impl FnOnce() -> T) -> T {
    let had_sum = sstatus::read() & SSTATUS_SUM != 0;
    unsafe { sstatus::set(SSTATUS_SUM) };
    let out = f();
    if !had_sum {
        unsafe { sstatus::clear(SSTATUS_SUM) };
    }
    out
}
