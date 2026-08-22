# Final State and Generation Performance Gate

Status: final two-generation runtime contract, 2026-08-22.

## Purpose and workflow

The runtime no longer exposes persistent snapshot roots to benchmark callers.
The retained session owns exactly accepted prior and, during an edit, candidate
current. History is detached evidence. Rejection drops current, acceptance
drops prior and promotes current, and terminal retirement drops the last coarse
owner. There is no compactor, relocation graph, forwarding table, or retained
snapshot-owner chain.

Run the deterministic allocation and lifecycle gate with:

```bash
scripts/check-snapshot-budgets.sh
```

The script name is the established operator entry point. Its binary now prints
`FINAL_STATE_GATE` and `RETAINED_GENERATION_GATE`; it does not preserve or
emulate the deleted snapshot model.

Run the Criterion diagnostics separately:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml \
  --bench state_budgets
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml \
  --bench accepted_edit_scaling
```

## Measurement semantics

The gate uses the profiling-only allocator. Workload construction and initial
capacity growth occur before each warmed measurement. Allocation counts include
`alloc`, `alloc_zeroed`, and successful `realloc` requests in the named scope;
requested bytes are caller-visible requested capacity, not allocator metadata
or RSS.

The strict warmed rows are:

- one million admitted direct meaning and register reads;
- 16,384 same-cell assignment/rollback cycles after one priming cycle;
- one 16,384-node page contribution enqueue/dequeue round trip after capacity
  priming.

Every row must report zero allocation calls and zero requested bytes. The
assignment row restores an operation-local journal suffix on every cycle, so it
measures the runtime transaction shape rather than an artificial unbounded
journal append.

The lifecycle row creates prior and current under one session interning epoch,
drops current to model rejection, creates replacement current and drops prior
to model acceptance, then explicitly retires the terminal generation. It
requires exactly three creations and drops, no more than two simultaneous live
owners, one explicit retirement, and the baseline live-owner count at exit.

Cold generation construction, format materialization, detached output, and
source/history allocation are reported by their own diagnostics and are not
mislabelled as warmed hot-path work. The numerical full-document RSS and wall
authority is specified in [Phase 8 Runtime Performance Acceptance](phase8_performance_acceptance.md).
