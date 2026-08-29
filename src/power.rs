//! Power control via the SBI SRST (system reset) extension: shutdown and
//! reboot are firmware services now, requested with an ecall instead of a
//! magic write to the sifive_test device.

use crate::riscv;
use crate::sbi;

fn reset(reset_type: usize, verb: &str) -> ! {
    println!("power: {verb}");
    if sbi::probe_srst() {
        let ret = sbi::system_reset(reset_type, sbi::RESET_REASON_NONE);
        // Only reachable if the firmware refused the request.
        println!(
            "power: firmware refused system reset: {}",
            sbi::error_name(ret.error)
        );
    } else {
        println!("power: firmware lacks the SBI SRST extension");
    }
    // No way to power off without the firmware; park the hart.
    loop {
        riscv::wfi();
    }
}

pub fn poweroff() -> ! {
    reset(sbi::RESET_TYPE_SHUTDOWN, "goodbye")
}

pub fn reboot() -> ! {
    reset(sbi::RESET_TYPE_COLD_REBOOT, "rebooting")
}
