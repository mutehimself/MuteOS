# Design doc: MLFQ scheduler

Implements [theseus-os/Theseus#1096](https://github.com/theseus-os/Theseus/issues/1096) — replacing round-robin scheduling with a multi-level feedback queue (MLFQ).

## Motivation

Theseus's existing schedulers are round-robin (no notion of priority or task
behavior), a fixed-priority scheduler (priority is static unless a caller
changes it), and an epoch scheduler. None of them adapt a task's scheduling
treatment to its *observed* behavior. MLFQ does: it starts every task at the
same priority and lets each task's own behavior — whether it tends to use
its full CPU quantum or block/yield early — determine how it's treated over
time, without requiring any task to be classified up front as "interactive"
or "batch."

## Design

Source: [`kernel/scheduler_mlfq/src/lib.rs`](../kernel/scheduler_mlfq/src/lib.rs).

- **8 priority levels** (`NUM_LEVELS`), level `0` highest priority.
- **New tasks start at level 0.** Short-lived and interactive tasks finish
  before ever being demoted; only tasks that turn out to be CPU-bound sink
  down.
- **Quanta grow linearly with level**: level `n` gets `(n + 1) * BASE_QUANTUM`
  (`BASE_QUANTUM` = 5ms). Tasks that have proven themselves CPU-bound run
  less often but for longer stretches once they get there, amortizing
  context-switch overhead for exactly the workload that doesn't need low
  latency.
- **Demotion is based on measured CPU time, not call count.** `next()` charges
  the previously-dispatched task for the wall-clock time it actually held the
  CPU (`Instant` recorded at dispatch, compared against `Instant::now()` the
  next time `next()` runs) and accumulates it per-level. This was a
  deliberate choice over "reset the count on any yield": accumulating means a
  task can't dodge demotion by chunking its CPU-bound work into pieces
  smaller than the quantum and yielding between them — it still gets charged
  for all of it.
- **Blocking before the quantum is exhausted is never penalized.** If a task
  is no longer runnable when it's charged (it blocked on I/O, a lock, sleep,
  etc.) it is not demoted, and its accumulated runtime is left as-is. This is
  the actual mechanism by which MLFQ favors interactive/I/O-bound tasks: they
  naturally block often, so they rarely accumulate enough runtime to be
  demoted, with no explicit classification needed.
- **Periodic priority boost** (every `BOOST_INTERVAL` = 300ms) resets every
  task to level 0. This bounds the worst-case wait time for any runnable
  task and prevents starvation of tasks that were previously demoted —
  without it, a task that used to be CPU-bound but has since become
  interactive would be stuck waiting behind level-0 traffic indefinitely.
- **Implements `PriorityScheduler`** (`priority()`/`set_priority()`, mapping
  MLFQ level to the `u8` priority convention the rest of Theseus already
  uses). This matters beyond the `ps`/`nice`-style use case: `sync_block`
  calls `scheduler::inherit_priority()` to temporarily boost a lock holder's
  priority when a higher-priority task blocks on that lock, to prevent
  unbounded priority inversion. Without a working `PriorityScheduler` impl,
  a lock holder could sit at a low MLFQ level while higher-priority waiters
  starved behind it whenever `mlfq_scheduler` is the active policy.

### Known simplifications

- Levels are a coarser granularity than the `u8` priority range other
  schedulers expose (0–255); MLFQ only distinguishes as many priorities as
  it has levels (8). `set_priority()` clamps into that range.
- `BASE_QUANTUM` and `BOOST_INTERVAL` are fixed constants rather than
  configurable at build or runtime. Tuning them per-workload is future work,
  not attempted here.

## Build integration

`kernel/spawn/src/lib.rs` selects the active scheduler at compile time via a
`cfg_if!` chain read from `THESEUS_CONFIG`. This adds a fourth arm:

```sh
make run THESEUS_CONFIG=mlfq_scheduler
```

`applications/ps` was updated to show the priority/level column under
`mlfq_scheduler`, matching its existing behavior for `epoch_scheduler` and
`priority_scheduler`.

## Benchmark methodology

`applications/scheduler_eval` already existed upstream as a scheduler
benchmark, but its only mode measures the aggregate time for N identical
tasks to each yield Y times — useful for raw context-switch overhead, but it
can't distinguish MLFQ's actual goal (favoring interactive tasks) from
round-robin, since every task in that benchmark behaves identically.

This adds a `-m`/`--mixed` mode that spawns two kinds of tasks:

- **CPU-bound**: runs a busy loop for a configurable number of iterations
  with no voluntary yields, so it only gives up the CPU when preempted —
  exactly the behavior that should get it demoted under MLFQ.
- **Interactive**: performs a configurable number of short work/yield
  bursts, each ending in a voluntary `scheduler::schedule()` call — exactly
  the behavior MLFQ is meant to reward.

Every task records its own completion latency relative to a shared start
`Instant` captured before any task is spawned. The benchmark reports
avg/p50/min/max latency for each group separately, plus the overall
makespan:

```sh
scheduler_eval -m --cpu-bound 4 --interactive 16
```

The hypothesis this is designed to test: under load from CPU-bound tasks,
interactive-task latency under `mlfq_scheduler` should be meaningfully lower
than under `THESEUS_CONFIG=` (round-robin, the default), without a
proportional collapse in CPU-bound throughput. Comparing the same command's
output across `make run`, `make run THESEUS_CONFIG=priority_scheduler`, and
`make run THESEUS_CONFIG=mlfq_scheduler` gives a like-for-like comparison.

## Boot verification

**Confirmed working.** `make iso THESEUS_CONFIG=mlfq_scheduler` was built
end-to-end and booted in QEMU (BIOS boot, SeaBIOS → GRUB → nano_core). The
serial console log shows a full, clean boot: the build info banner reports
`CUSTOM CFGs: mlfq_scheduler ...` confirming the scheduler was actually
compiled in and selected; all 4 CPUs come up via the APIC/SIPI sequence;
memory, ACPI, PCI, PS/2, and the framebuffer initialize without errors; the
`shell` application crate is loaded and linked (pulling in its `window`,
`libterm`, `text_display`, and `color` dependencies on demand); and
`shell_loop` is spawned as the first application task, with no panics,
faults, or unhandled errors anywhere in the log. This is real evidence the
scheduler is correct enough to bring up a full SMP system, not just that it
type-checks.

Build environment note: `nasm` and several other Theseus build dependencies
aren't installable directly on the host here (an atomic/immutable Fedora
variant), so the build runs inside a Fedora Toolbox container (`toolbox
create`), with `nasm`, `gcc`, `make`, `mtools`, `xorriso`,
`grub2-pc`/`grub2-pc-modules` (the BIOS/i386-pc GRUB modules — installing
only `grub2-tools-extra` is *not* sufficient; `grub-mkrescue` silently
produces a non-bootable image without them, since the Makefile redirects its
stderr to `/dev/null`) installed there, while `cargo`/`rustc` are shared
in from the host via the mounted `$HOME`. QEMU itself runs on the host,
against the ISO the container built (the container's `/run/host` bind-mount
makes the project directory visible on both sides). One environment gotcha
worth noting: this Makefile's `ROOT_DIR` is derived from `abspath`/`CURDIR`,
which resolve through the kernel's physical path, not any shell-level
symlink — so a project directory with a space in its name (or reached via a
symlinked path) breaks path-splitting in Make. The directory was renamed to
remove the space rather than working around it.

## Quantitative results

**Not yet collected.** `applications/scheduler_eval -m` has been built into
this same image and is present in `/namespaces/_applications`, but running
it interactively requires driving Theseus's shell, which is bound to the
graphical console by default; a second shell can attach over a serial port,
but it's spawned on demand by an interrupt-driven connection-detector task,
and getting that handshake working reliably from a fully headless,
non-interactive script (rather than a real terminal attached to QEMU's
allocated pty) turned out to be a deeper rabbit hole than was worth
chasing further here.

To fill in this section, from a normal interactive terminal:
```sh
make run THESEUS_CONFIG=mlfq_scheduler FEATURES="--workspace --features theseus_features/scheduler_eval"
# in the Theseus shell:
scheduler_eval -m --cpu-bound 4 --interactive 16
```
Then repeat with `make run FEATURES="--workspace --features theseus_features/scheduler_eval"` (default round-robin) and compare the avg/p50 interactive-task latency and CPU-bound makespan between the two runs.
