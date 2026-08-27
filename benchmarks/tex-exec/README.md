# tex-exec benchmarks

This standalone crate contains the focused execution-layer shipout benchmark
and the canonical bounded-episode workload. It is excluded from the root
workspace correctness gate.

## Aggregate checkpoint baseline

Build and run the representative aggregate checkpoint baseline with:

```bash
cargo run --release --manifest-path benchmarks/tex-exec/Cargo.toml \
  --bin aggregate_checkpoint_baseline
```

The four rows cross minimal versus 64-unit accumulated state with one versus
32 simultaneously retained boundaries. Each row measures capture, retained
checkpoint clone, generation fork, and same-generation restore separately and
prints allocation calls, requested bytes, component payload retention, elapsed
nanoseconds, and a semantic checksum. Fixture construction is outside every
measured region. The accumulated fixture has deep registered-source and stored
token-list stacks at command quiescence, initialized patterns plus mutable
exceptions and saved hyphenation codes, contribution/current/discard page
lists, insertions and marks, nested mode lists, changed dependency cells, and
World effects, an open stream, and a committed artifact.

The component byte columns are exact fixture payload charges, excluding
allocator headers and spare capacity; shared token/source payload is charged
once to its sole fixture owner. Allocation `requested_bytes` is the allocator
authority for physical bytes requested by each measured operation.

The executable also enforces accumulated-state per-boundary allocation and
requested-byte ceilings for capture, checkpoint clone, fork, and restore. The
ceilings sit below the checked-in pre-mode/page-migration baseline, so restoring
an accumulated mode-list or page-builder clone makes the run fail rather than
merely changing a diagnostic row. The capture byte ceiling includes the fixed
exact core/node checkpoint bank introduced by the later core-family migration;
it does not scale with mode or page payload.

The first `MODE_PAGE_EARLY_SUFFIX_GATE` row captures a genuinely rooted
checkpoint containing mode, contribution, current-page, page-discard,
split-discard, insertion, and mark values, then compares one versus 4,096 later
writes in each growing lane. It isolates the mode/page owner fork, first
destructive mutation, and rejection seams from the command, core, and World
ownership families. The row reports each exercised destructive family:
accepted-node replacement, private and accepted mode-tail removal,
contribution and current-page removal, both discard clears, and insertion and
mark replacement. Allocation calls and requested bytes must be byte-for-byte
equal, and both replay-work counters must remain zero.

The elapsed nanoseconds are a single-shot diagnostic, not a statistical
threshold. Values such as 33 us versus 44 us for depths one and 4,096 represent
the same fixed lifecycle work plus scheduler, cache, and allocator noise; the
exact allocation equality, exact operation counters, and zero replay counters
are the deterministic CPU-work gate. A stable timing comparison belongs in a
repeated Criterion benchmark, not this correctness executable.

Run the shipout lowering cases with:

```bash
cargo bench --manifest-path benchmarks/tex-exec/Cargo.toml --bench shipout
```

`ordinary_hlist` measures the normal artifact-lowering fast path.
`deferred_math_lists` measures shipout-local Appendix G conversion for frozen
math lists that survived into a shipped tree. Both cases lower 1,024 child
nodes. Each Criterion iteration owns the complete generative Universe episode:
state construction, admitted page publication, execution, and artifact commit
all remain inside one HRTB callback so no live id or owner escapes into a
benchmark fixture.

## Canonical episode workload

`canonical_episode` measures the production `MainControl::advance_episode` route
over one complete INITEX job. The job defines and calls parameterized macros,
performs local and global count assignments, branches with `\ifnum`, restores
an `\hbox` group, emits characters and explicit kerns, ships a page, and ends.
The `nested` shape adds a second macro that forwards its argument into two
inner calls.

Build once and measure each workload in an independent process:

```bash
CARGO_BUILD_JOBS=1 cargo build --release --manifest-path benchmarks/tex-exec/Cargo.toml --bin canonical_episode
python3 scripts/run-umber-guarded.py --timeout-seconds 600 --max-rss-mib 4096 -- benchmarks/tex-exec/target/release/canonical_episode 89551 10 direct
python3 scripts/run-umber-guarded.py --timeout-seconds 600 --max-rss-mib 4096 -- benchmarks/tex-exec/target/release/canonical_episode 179103 26 direct
python3 scripts/run-umber-guarded.py --timeout-seconds 600 --max-rss-mib 4096 -- benchmarks/tex-exec/target/release/canonical_episode 20000 0 nested
```

The timed allocation region begins after `Workload` constructs the immutable
source. The production run then creates fresh engine state and the synthetic
font, executes, retains all reported state and output, validates and serializes
the canonical page artifact, parses the serialized artifact, compiles and
serializes DVI, and retains terminal text, log text, and effects. The episode
executes the complete profile vocabulary through the sole `CommandProcessor`,
`MainControl`, `Universe`, mode nest, node builder, and output transaction.
There is no admitted-root kernel, comparison adapter, coverage fallback, or
runtime engine choice. Correctness remains owned by the repository's external
fixtures and oracle workloads. Historical migration measurements and the final
canonical cutover are recorded in `docs/native_batch_kernel.md`.

The executable also enforces a fixed-plus-linear allocation-call ceiling. It
deliberately reports, but does not gate, requested bytes because its timed
region includes explicit cold page-artifact and DVI materialization; warmed
ownership and list-mutation allocation ceilings are enforced by their focused
profiling gates instead.
