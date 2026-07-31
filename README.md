# MuteOS

MuteOS is a personal OS-internals project: a fork of [Theseus OS](https://github.com/theseus-os/Theseus), a research operating system written in Rust, with my own kernel-level additions layered on top. It exists to demonstrate hands-on systems programming — scheduling, concurrency, and kernel architecture — on a real, non-trivial codebase rather than a toy.

**This is not a from-scratch OS.** The base (memory management, drivers, filesystem, windowing, wasm runtime, etc. — roughly 160 kernel crates) is Theseus, created by Kevin Boos and the Theseus OS research project, and is MIT-licensed. See [`THESEUS_README.md`](THESEUS_README.md) for Theseus's own documentation, build instructions, and acknowledgements, and [`LICENSE-MIT`](LICENSE-MIT) for the license, which is retained unmodified.

## What's original here

### MLFQ scheduler (`kernel/scheduler_mlfq`)
Theseus ships with round-robin, priority, and epoch schedulers, selectable at build time via a `THESEUS_CONFIG` cfg flag. I added a fourth: a multi-level feedback queue scheduler (`make THESEUS_CONFIG=mlfq_scheduler`), implementing upstream issue [theseus-os/Theseus#1096](https://github.com/theseus-os/Theseus/issues/1096).

Design summary:
- 8 priority levels; new tasks start at level 0 (highest priority).
- Each level's quantum grows linearly with depth, so CPU-bound tasks that sink to lower levels are scheduled less often but for longer stretches, cutting context-switch overhead for exactly the workloads that don't need low latency.
- A task that blocks or yields before exhausting its quantum is **not** demoted and gets a fresh quantum next time it runs — this is what makes MLFQ favor interactive/I/O-bound tasks over CPU-bound ones without any static classification.
- A periodic priority boost resets every task to level 0, bounding worst-case wait time and preventing starvation of long-demoted tasks.
- Implements Theseus's `PriorityScheduler` trait, so it correctly participates in priority inheritance (`sync_block`'s lock-holder boosting) — without this, a task holding a lock could sit at a low MLFQ level while higher-priority waiters starved behind it.

Also wired the existing `ps` application to show the priority/level column under `mlfq_scheduler`, matching its existing behavior for the epoch/priority schedulers.

### In progress
- Benchmarking harness comparing turnaround time, interactive latency, and throughput of MLFQ against round-robin and priority scheduling under mixed CPU-bound/interactive workloads (tracking [theseus-os/Theseus#758](https://github.com/theseus-os/Theseus/issues/758)).
- Design doc with the benchmark results and the tradeoffs considered.
- An upstream PR against `theseus-os/Theseus` for the scheduler itself.

## Building and running

Build instructions are unchanged from upstream Theseus — see [`THESEUS_README.md`](THESEUS_README.md) for the full setup guide. Quick start:

```sh
git submodule update --init --recursive
make run                              # default (round-robin) scheduler
make run THESEUS_CONFIG=mlfq_scheduler  # MLFQ scheduler
```

## License

MIT, inherited from Theseus. See [`LICENSE-MIT`](LICENSE-MIT).
