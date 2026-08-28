//! Supervisor trap handling: one entry point (kernelvec.S) for exceptions,
//! interrupts, and — once user mode exists — syscalls.

use crate::riscv::*;

/// Saved register state of an interrupted context, built by kernelvec.S on
/// the kernel stack. Field order and `#[repr(C)]` are load-bearing: offsets
/// must match the assembly (slots 0..30 = x1..x31, then sepc, sstatus).
#[repr(C)]
pub struct TrapFrame {
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
    pub sepc: usize,
    pub sstatus: usize,
}

unsafe extern "C" {
    fn kernelvec();
}

/// Point stvec at the trap vector. sscratch = 0 marks "currently in
/// S-mode" for the vector's stack-selection protocol.
pub fn init() {
    unsafe {
        sscratch::write(0);
        stvec::write(kernelvec as *const () as usize);
    }
}

// Interrupt causes (scause with the interrupt bit set).
#[allow(dead_code)]
const IRQ_S_SOFT: usize = 1;
const IRQ_S_TIMER: usize = 5;
const IRQ_S_EXTERNAL: usize = 9;

// Exception causes.
const EXC_BREAKPOINT: usize = 3;
const EXC_ECALL_USER: usize = 8;

fn exception_name(code: usize) -> &'static str {
    match code {
        0 => "instruction address misaligned",
        1 => "instruction access fault",
        2 => "illegal instruction",
        3 => "breakpoint",
        4 => "load address misaligned",
        5 => "load access fault",
        6 => "store address misaligned",
        7 => "store access fault",
        8 => "environment call from U-mode",
        9 => "environment call from S-mode",
        12 => "instruction page fault",
        13 => "load page fault",
        15 => "store/AMO page fault",
        _ => "unknown exception",
    }
}

/// Length in bytes of the instruction at `addr`: RISC-V encodes it in the
/// low two bits (11 = 32-bit, anything else = 16-bit compressed).
fn instruction_len(addr: usize) -> usize {
    let low_half = unsafe { (addr as *const u16).read_volatile() };
    if low_half & 0b11 == 0b11 { 4 } else { 2 }
}

/// Central trap dispatcher, called from kernelvec.S with the saved frame.
#[unsafe(no_mangle)]
extern "C" fn kerneltrap(frame: &mut TrapFrame) {
    let cause = scause::read();
    let code = cause & !SCAUSE_INTERRUPT;
    let from_user = frame.sstatus & SSTATUS_SPP == 0;

    if cause & SCAUSE_INTERRUPT != 0 {
        match code {
            IRQ_S_TIMER => {
                crate::timer::on_tick();
            }
            IRQ_S_EXTERNAL => {
                if let Some(irq) = crate::plic::claim() {
                    match irq {
                        crate::memlayout::UART0_IRQ => crate::uart::handle_interrupt(),
                        _ => println!("trap: unexpected external irq {irq}"),
                    }
                    crate::plic::complete(irq);
                }
            }
            _ => {
                panic!("unexpected interrupt, scause={:#x}", cause);
            }
        }
    } else {
        match code {
            EXC_BREAKPOINT => {
                // Recoverable by design: report and step past the ebreak.
                println!(
                    "trap: breakpoint at {:#x} ({}-mode)",
                    frame.sepc,
                    if from_user { "U" } else { "S" }
                );
                frame.sepc += instruction_len(frame.sepc);
            }
            EXC_ECALL_USER => {
                // Syscall dispatch lands with the process module.
                panic!("ecall from U-mode before syscalls exist");
            }
            _ => {
                panic!(
                    "unhandled exception: {} (scause={:#x}) sepc={:#x} stval={:#x} from {}-mode",
                    exception_name(code),
                    cause,
                    frame.sepc,
                    stval::read(),
                    if from_user { "U" } else { "S" },
                );
            }
        }
    }
}
