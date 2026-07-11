# Snapshot Performance and Retention Gates

Status: measurement contract for the persistent-root migration, 2026-07-11.

## Purpose and workflow

Snapshot capture must remain bounded independently of live payload size. The
focused workloads cover resumable input, page-builder lists, execution mode
lists, partial stream lines, hyphenation patterns, diagnostic provenance, and
sparse Unicode code-table writes. They live in the standalone `tex-state`
benchmark crate so normal correctness tests stay fast.

Use Criterion to inspect latency distributions:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml \
  --bench snapshot_budgets
```

Run the deterministic allocation and latency gate with:

```bash
scripts/check-snapshot-budgets.sh
```

For an informational report that does not exit unsuccessfully on a budget
violation, omit `--enforce` and invoke the binary directly:

```bash
cargo run --release --manifest-path benchmarks/tex-state/Cargo.toml \
  --bin snapshot_gate
```

## Measurement semantics

Each row reports two deliberately different quantities:

- `logical_live_bytes` is the live semantic payload introduced by the workload:
  input/stream text bytes, decoded page/mode node bytes, pattern letters and
  values, provenance arena bytes, or assigned Unicode code-table words. It is
  not an allocator or RSS estimate.
- retained and peak bytes are requested heap-allocation deltas observed while
  capturing state. Retained bytes are still owned by the captured values at the
  observation. Peak bytes are the maximum aggregate requested-live-byte value
  reached at one allocator event during the same operation. The peak is one
  coherent total, not independently maximized component columns.

The allocator counter measures requested capacity, not platform allocator size
classes, metadata, resident pages, or shared immutable backing already present
before the observation. Workload construction and the first state-hash capture
occur outside the measured region. This isolates steady capture from initial
semantic hashing and makes results stable across machines.

## Budgets

The expected asymptotic capture cost is O(1) in every payload dimension. A large
row may take at most four times its small-row median plus 25 us of timing noise.
One large capture may retain at most 32 KiB of newly requested allocation, and
32 simultaneously retained captures may retain at most 32 times that bound.
These generous constants cover fixed snapshot tuples, root reference counts,
and allocator-independent bookkeeping while still rejecting payload clones.

Provenance and Unicode code-table roots already meet the bounded capture gate at
the migration baseline. Input, page, mode, stream, and hyphenation rows expose
known payload-linear captures owned by the persistent-root epic; the strict gate
is expected to remain red for those rows until their representation issues land.
Do not relax the budgets to make an owned-payload representation pass.

The gate intentionally keeps retained snapshots alive for one observation, so
capacity reuse or immediate drop cannot hide retained growth. It reports the
single-capture and 32-capture observations separately, which distinguishes a
large transient allocation from storage retained by incremental history.
