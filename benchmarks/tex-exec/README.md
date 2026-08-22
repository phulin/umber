# tex-exec benchmarks

This standalone crate contains the focused execution-layer shipout benchmark
and the canonical bounded-episode workload. It is excluded from the root
workspace correctness gate.

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
