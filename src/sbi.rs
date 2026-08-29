//! SBI client: the kernel's interface to M-mode firmware (OpenSBI).
//!
//! With the kernel running as a supervisor payload, machine mode belongs to
//! firmware. Anything only M-mode can do — arming the timer behind mtimecmp,
//! resetting the machine — is requested through the Supervisor Binary
//! Interface: an `ecall` from S-mode with an extension ID in a7 and a
//! function ID in a6, which traps *up* into the firmware rather than down
//! into us. Arguments travel in a0/a1; the result comes back as an error
//! code in a0 and a value in a1, with all other registers preserved.
//!
//! Only the extensions the kernel actually needs are wrapped: Base
//! (discovery/probing), TIME (timer), and SRST (system reset).

#![allow(dead_code)] // fully wired once the kernel boots on OpenSBI

use core::arch::asm;

/// Result of every SBI call: a standard error code and a call-specific value.
#[derive(Clone, Copy, Debug)]
pub struct SbiRet {
    pub error: isize,
    pub value: usize,
}

impl SbiRet {
    pub fn is_ok(&self) -> bool {
        self.error == 0
    }
}

/// Human-readable form of the standard SBI error codes.
pub fn error_name(error: isize) -> &'static str {
    match error {
        0 => "success",
        -1 => "failed",
        -2 => "not supported",
        -3 => "invalid parameter",
        -4 => "denied",
        -5 => "invalid address",
        -6 => "already available",
        -7 => "already started",
        -8 => "already stopped",
        -9 => "no shared memory",
        _ => "unknown error",
    }
}

/// The raw calling convention. The SBI spec guarantees the firmware
/// preserves every register except a0 (error) and a1 (value).
#[inline]
fn sbi_call(eid: usize, fid: usize, arg0: usize, arg1: usize) -> SbiRet {
    let error: isize;
    let value: usize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") arg0 => error,
            inlateout("a1") arg1 => value,
            in("a6") fid,
            in("a7") eid,
        );
    }
    SbiRet { error, value }
}

// Extension IDs. TIME and SRST spell themselves in ASCII.
const EID_BASE: usize = 0x10;
const EID_TIME: usize = 0x5449_4D45; // "TIME"
const EID_SRST: usize = 0x5352_5354; // "SRST"

// ---- Base extension: mandatory discovery interface ------------------------

/// SBI specification version implemented by the firmware, as (major, minor).
pub fn spec_version() -> (usize, usize) {
    let ret = sbi_call(EID_BASE, 0, 0, 0);
    ((ret.value >> 24) & 0x7f, ret.value & 0xff_ffff)
}

/// Which firmware implementation answered (1 = OpenSBI, ...).
pub fn impl_id() -> usize {
    sbi_call(EID_BASE, 1, 0, 0).value
}

/// Implementation-defined firmware version.
pub fn impl_version() -> usize {
    sbi_call(EID_BASE, 2, 0, 0).value
}

/// Name for a registered implementation ID.
pub fn impl_name(id: usize) -> &'static str {
    match id {
        0 => "BBL",
        1 => "OpenSBI",
        2 => "Xvisor",
        3 => "KVM",
        4 => "RustSBI",
        5 => "Diosix",
        6 => "Coffer",
        7 => "Xen",
        _ => "unknown SBI implementation",
    }
}

/// Does the firmware implement extension `eid`? (Base FID 3 returns nonzero
/// for implemented extensions.)
pub fn probe_extension(eid: usize) -> bool {
    let ret = sbi_call(EID_BASE, 3, eid, 0);
    ret.is_ok() && ret.value != 0
}

pub fn probe_time() -> bool {
    probe_extension(EID_TIME)
}

pub fn probe_srst() -> bool {
    probe_extension(EID_SRST)
}

// ---- TIME extension: the supervisor timer ---------------------------------

/// Arm the next timer interrupt for absolute time `when` (timebase ticks).
/// The firmware clears the pending supervisor timer interrupt as part of
/// arming, so calling this from the timer trap acknowledges the interrupt.
pub fn set_timer(when: usize) {
    sbi_call(EID_TIME, 0, when, 0);
}

// ---- SRST extension: system reset -----------------------------------------

pub const RESET_TYPE_SHUTDOWN: usize = 0;
pub const RESET_TYPE_COLD_REBOOT: usize = 1;
pub const RESET_TYPE_WARM_REBOOT: usize = 2;
pub const RESET_REASON_NONE: usize = 0;

/// Ask the firmware to reset the system. On success this never returns; a
/// return value means the request was refused (caller decides what next).
pub fn system_reset(reset_type: usize, reason: usize) -> SbiRet {
    sbi_call(EID_SRST, 0, reset_type, reason)
}
