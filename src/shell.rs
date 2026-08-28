//! An interactive kernel shell on the serial console.
//!
//! Runs as an ordinary kernel thread; blocks on console input (sleeping,
//! not spinning) and pokes at the rest of the kernel for diagnostics.

use alloc::string::String;

use crate::{console, heap, kalloc, memlayout, power, proc, timer, user};

const MAX_LINE: usize = 128;

pub fn shell_main() {
    println!();
    println!("osmium shell -- 'help' lists commands");
    let mut line = String::new();
    loop {
        print!("osmium> ");
        read_line(&mut line);
        run_command(line.trim());
    }
}

/// Read one line with echo and backspace editing.
fn read_line(line: &mut String) {
    line.clear();
    loop {
        let byte = console::getchar_blocking();
        match byte {
            b'\n' => {
                println!();
                return;
            }
            0x7f | 0x08 => {
                // Backspace: erase from buffer and terminal.
                if line.pop().is_some() {
                    print!("\x08 \x08");
                }
            }
            0x20..=0x7e if line.len() < MAX_LINE => {
                line.push(byte as char);
                print!("{}", byte as char);
            }
            _ => {}
        }
    }
}

fn run_command(cmd: &str) {
    match cmd {
        "" => {}
        "help" => {
            println!("  help      this text");
            println!("  ps        list processes");
            println!("  mem       memory statistics");
            println!("  uptime    time since boot");
            println!("  greet     spawn the U-mode greeter program");
            println!("  fault     spawn the U-mode trespasser (it will be killed)");
            println!("  echoline  run a U-mode program that reads a line and echoes it");
            println!("  uname     print the kernel version (via a U-mode program)");
            println!("  poweroff  exit QEMU");
            println!("  reboot    reset the machine");
        }
        "ps" => {
            println!("  {:<5} {:<12} {}", "PID", "NAME", "STATE");
            for (pid, name, state) in proc::process_list() {
                println!("  {pid:<5} {name:<12} {state:?}");
            }
        }
        "mem" => {
            println!(
                "  phys: {} free pages ({} MiB of {} MiB DRAM)",
                kalloc::free_pages(),
                kalloc::free_pages() * memlayout::PAGE_SIZE / (1024 * 1024),
                (memlayout::PHYS_TOP - memlayout::DRAM_BASE) / (1024 * 1024),
            );
            println!(
                "  heap: {} KiB free of {} KiB",
                heap::free_bytes() / 1024,
                memlayout::HEAP_SIZE / 1024,
            );
        }
        "uptime" => {
            let ms = timer::uptime_ms();
            println!(
                "  up {}.{:03} s ({} ticks at {} Hz)",
                ms / 1000,
                ms % 1000,
                timer::ticks(),
                timer::TICK_HZ,
            );
        }
        "greet" => {
            let pid = proc::spawn_user("u-greeter", user::greeter_addr());
            println!("  spawned pid {pid}");
        }
        "fault" => {
            let pid = proc::spawn_user("u-trespasser", user::trespasser_addr());
            println!("  spawned pid {pid}");
        }
        "echoline" => {
            // Join the child so the shell stops reading input while the
            // user program owns the console — otherwise both would race
            // for the same keystrokes.
            let pid = proc::spawn_user("u-echoline", user::echoline_addr());
            proc::join(pid);
        }
        "uname" => {
            let pid = proc::spawn_user("u-uname", user::uname_addr());
            proc::join(pid);
        }
        "poweroff" => power::poweroff(),
        "reboot" => power::reboot(),
        _ => println!("  unknown command '{cmd}' -- try 'help'"),
    }
}
