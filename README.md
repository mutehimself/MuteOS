# MuteOS

A multi-level feedback queue scheduler, built and verified inside [Theseus OS](https://github.com/theseus-os/Theseus) — a Rust research operating system — and booted on real SMP hardware emulation with a clean 4-CPU bring-up.

**tl;dr**
- Designed and implemented an MLFQ scheduler (`kernel/scheduler_mlfq`) from scratch: 8 priority levels, CPU-time-based demotion (not yield-counting, so it can't be gamed), starvation-proof via periodic priority boosting, and fully wired into priority inheritance so lock-holder boosting still works correctly under it.
- Extended an existing benchmark tool with a mixed CPU-bound/interactive workload mode, ran it headlessly under both schedulers, and **measured a ~2x reduction in interactive-task latency** (406ms vs. 772ms avg) versus round-robin under identical load — the scheduler does what it was designed to do, with a number to prove it, not just an argument for why it should.
- Took it from source to a booted kernel: built a full OS image and watched it bring up 4 CPUs, initialize memory/ACPI/PCI/framebuffer, and reach a running shell — no panics, no faults.
- Did this on an immutable Linux host with no direct package manager access, which meant standing up a proper containerized build environment and debugging two separate silent build failures along the way (see below — this part is arguably the more instructive story).

## Measured results

| Scheduler | CPU-bound avg latency | Interactive avg latency | Makespan |
|---|---:|---:|---:|
| round-robin (default) | 1382.3ms | 772.0ms | 1446.2ms |
| **`mlfq_scheduler`** | **918.1ms** | **406.4ms** | **962.8ms** |

4 CPU-bound tasks + 16 interactive tasks, pinned to one CPU, run headlessly inside QEMU (methodology, exact commands, and honest caveats about single-run/emulated-timing variance are in [`docs/mlfq-scheduler.md`](docs/mlfq-scheduler.md#quantitative-results)).

## Proof it boots

This isn't a "trust me, it compiles" claim — it's a serial console log from an actual QEMU boot with the scheduler active:

```
[I] kernel/nano_core/src/lib.rs:144:
    ===================== Theseus build info: =====================
    CUSTOM CFGs: mlfq_scheduler overflow_checks relocation_model="static" target_thread_local
    ===============================================================
...
[I] kernel/multicore_bringup/src/x86_64.rs:511:  AP 1 is in Rust code. Ready!
[I] kernel/multicore_bringup/src/x86_64.rs:511:  AP 2 is in Rust code. Ready!
[I] kernel/multicore_bringup/src/x86_64.rs:511:  AP 3 is in Rust code. Ready!
[I] kernel/captain/src/lib.rs:151: Finished booting all 3 AP cores; 4 total CPUs are running.
...
[I] kernel/mod_mgmt/src/lib.rs:826: loaded new application crate: "shell-e786c6e1e4e3402a", num sections: 184, added 1 new symbols
[I] kernel/captain/src/lib.rs:220: captain::init(): initialization done! Spawning an idle task on BSP core 0 and enabling interrupts...
```

Full walkthrough and the complete build/boot log are in [`docs/mlfq-scheduler.md`](docs/mlfq-scheduler.md).

## The scheduler (`kernel/scheduler_mlfq`)

Theseus ships with round-robin, priority, and epoch schedulers, selectable at build time via a `THESEUS_CONFIG` cfg flag. This adds a fourth: a multi-level feedback queue scheduler (`make THESEUS_CONFIG=mlfq_scheduler`), implementing upstream issue [theseus-os/Theseus#1096](https://github.com/theseus-os/Theseus/issues/1096).

- **8 priority levels**; new tasks start at level 0 (highest priority).
- **Quanta grow linearly with level depth**, so tasks that prove themselves CPU-bound run less often but for longer stretches, amortizing context-switch overhead for exactly the workload that doesn't need low latency.
- **Demotion is based on measured CPU time actually consumed, not on counting voluntary yields** — a task can't dodge demotion by chunking CPU-bound work into pieces smaller than its quantum and yielding between them.
- **Blocking before the quantum is exhausted is never penalized.** This is the actual mechanism that makes MLFQ favor interactive/I/O-bound tasks over CPU-bound ones, with zero static classification of tasks required.
- **Periodic priority boost** bounds worst-case wait time and prevents starvation of long-demoted tasks.
- **Implements Theseus's `PriorityScheduler` trait**, which most from-scratch MLFQ implementations skip — without it, `sync_block`'s priority-inheritance mechanism (used to prevent unbounded priority inversion when a high-priority task blocks on a lock held by a low-priority one) silently stops working the moment this scheduler is selected. Catching this dependency meant reading the synchronization code, not just the scheduler API.

Full design rationale — why linear quanta, why CPU-time-based demotion instead of yield-counting, the priority-inheritance interaction — is in [`docs/mlfq-scheduler.md`](docs/mlfq-scheduler.md).

## The benchmark (`applications/scheduler_eval -m` + `applications/bench_headless`)

An existing upstream benchmark tool measured aggregate time for N *identical* tasks to yield — useful for raw context-switch overhead, but incapable of showing MLFQ's actual point, since every task in that test behaves the same way regardless of scheduler. Extended it (tracking [theseus-os/Theseus#758](https://github.com/theseus-os/Theseus/issues/758)) with a `-m`/`--mixed` mode that spawns a configurable mix of CPU-bound tasks (busy loop, never yields) and interactive tasks (short work bursts, yields between each), then reports each group's completion-latency distribution (avg/p50/min/max) separately — the metric that actually differentiates a scheduler that favors interactive workloads from one that doesn't.

Running it interactively turned out to need a real terminal (Theseus's serial-attached shell is spawned on demand by an interrupt-driven handshake a headless script can't reliably trigger). Rather than fight that, `applications/bench_headless` uses the same mechanism Theseus's own CI uses for automated testing: a `first_application` Cargo feature flag that boots straight into a fixed benchmark run instead of the interactive shell, signaling completion through QEMU's `isa-debug-exit` device — no console needed at all.

## Getting it to boot at all: the build-environment debugging

The host this was built on is an immutable/atomic Linux distro (Bazzite/Fedora Kinoite) — no direct `dnf install`. That meant standing up a [Fedora Toolbox](https://containertoolbx.org/) container as the real build environment, sharing the Rust toolchain in from the host via the mounted home directory, and bridging the project directory across the container boundary via its `/run/host` bind-mount.

Two failures showed up only at the very last step of the build, both silently:

1. **`grub-mkrescue` produced a non-bootable ISO with no error.** The Makefile redirects its stderr to `/dev/null`, so a missing dependency (the `grub2-pc-modules` package, which provides the actual i386-pc BIOS boot modules — not included in `grub2-tools-extra`) showed up only as QEMU refusing to boot the resulting disc ("Could not read from CDROM"), with nothing in the build log pointing at the cause. Diagnosed by checking `/usr/lib/grub/` for the missing module directory directly.
2. **GNU Make silently mis-resolves paths with spaces**, and — less obviously — resolves `pwd`/`CURDIR` through the *physical* filesystem path, bypassing any symlink you `cd` through to work around it. A symlinked path swap didn't fix it; only renaming the actual directory did.

Neither of these had an informative error message pointing at the root cause — both needed reading Makefile internals and reasoning about what a silent failure implied.

## Status

- **Boots successfully** end-to-end with `mlfq_scheduler` active (see above).
- **Benchmarked headlessly**, round-robin vs. `mlfq_scheduler`, with results above and full methodology/caveats in the design doc.
- **Upstream PR open**: [theseus-os/Theseus#1117](https://github.com/theseus-os/Theseus/pull/1117), addressing [#1096](https://github.com/theseus-os/Theseus/issues/1096) and [#758](https://github.com/theseus-os/Theseus/issues/758).

## What this is built on

This is not a from-scratch OS. The base — memory management, drivers, filesystem, windowing, a wasm runtime, roughly 160 kernel crates in total — is [Theseus OS](https://github.com/theseus-os/Theseus), created by Kevin Boos and the Theseus OS research project, MIT-licensed. [`THESEUS_README.md`](THESEUS_README.md) has Theseus's own documentation and acknowledgements; [`LICENSE-MIT`](LICENSE-MIT) is retained unmodified. Everything under "The scheduler," "The benchmark," and the build-environment work above is original to this repo.

## Building and running

Build instructions are otherwise unchanged from upstream Theseus — see [`THESEUS_README.md`](THESEUS_README.md) for the full setup guide.

```sh
git submodule update --init --recursive
make run                                # default (round-robin) scheduler
make run THESEUS_CONFIG=mlfq_scheduler  # MLFQ scheduler
```

## License

MIT, inherited from Theseus. See [`LICENSE-MIT`](LICENSE-MIT).
