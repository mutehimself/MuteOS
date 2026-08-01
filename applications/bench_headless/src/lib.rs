//! Runs `scheduler_eval`'s mixed CPU-bound/interactive workload benchmark
//! with fixed parameters, then exits QEMU via the `isa-debug-exit` device.
//!
//! This exists so the benchmark can be compared across scheduler policies
//! (`THESEUS_CONFIG=mlfq_scheduler` vs. the default round-robin) from a
//! script, the same way `qemu_test` runs Theseus's test suite headlessly:
//! boot straight into this instead of the interactive `shell`, and use the
//! debug-exit device's exit code to know when it's done, rather than
//! guessing at a timeout while trying to drive an interactive console with
//! no display attached.

#![no_std]

extern crate alloc;

use alloc::{string::String, vec::Vec};

use app_io::println;
use qemu_exit::{QEMUExit, X86};

static QEMU_EXIT_HANDLE: X86 = X86::new(0xf4, 0x11);

pub fn main(_: Vec<String>) -> isize {
    log::info!("bench_headless: running scheduler_eval mixed workload...");
    println!("bench_headless: running scheduler_eval mixed workload...");

    scheduler_eval::run_mixed(
        /* cpu_bound */ 4,
        /* interactive */ 16,
        /* cpu_iterations */ 20_000_000,
        /* bursts */ 200,
        /* work_per_burst */ 20_000,
        cpu::current_cpu(),
    );

    log::info!("bench_headless: done.");
    println!("bench_headless: done.");
    QEMU_EXIT_HANDLE.exit_success();
}
