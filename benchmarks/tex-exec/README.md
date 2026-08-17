# tex-exec benchmarks

This standalone crate contains the focused execution-layer shipout benchmark
and the native batch migration differential. It is excluded from the root
workspace correctness gate.

Run the shipout lowering cases with:

```bash
cargo bench --manifest-path benchmarks/tex-exec/Cargo.toml --bench shipout
```

`ordinary_hlist` measures the normal artifact-lowering fast path.
`deferred_math_lists` measures shipout-local Appendix G conversion for frozen
math lists that survived into a shipped tree. Both cases lower 1,024 child
nodes. Each Criterion iteration builds fresh state outside the timed region,
then times execution and artifact commit.

## Native batch ceiling

`native_batch` measures the production `MainControl::advance_episode` route
over one complete INITEX job. The job defines and calls parameterized macros,
performs local and global count assignments, branches with `\ifnum`, restores
an `\hbox` group, emits characters and explicit kerns, ships a page, and ends.
The `nested` shape adds a second macro that forwards its argument into two
inner calls.

Build once and measure each workload in an independent process:

```bash
CARGO_BUILD_JOBS=1 cargo build --release --manifest-path benchmarks/tex-exec/Cargo.toml --bin native_batch
python3 scripts/run-umber-guarded.py --timeout-seconds 600 --max-rss-mib 4096 -- benchmarks/tex-exec/target/release/native_batch 89551 10 direct
python3 scripts/run-umber-guarded.py --timeout-seconds 600 --max-rss-mib 4096 -- benchmarks/tex-exec/target/release/native_batch 179103 26 direct
python3 scripts/run-umber-guarded.py --timeout-seconds 600 --max-rss-mib 4096 -- benchmarks/tex-exec/target/release/native_batch 20000 0 nested
```

The timed allocation region begins after `Workload` constructs the immutable
source. The production run then creates fresh engine state and the synthetic
font, executes, retains all reported state and output, validates and serializes
the canonical page artifact, parses the serialized artifact, compiles and
serializes DVI, and retains terminal text, log text, and effects. The batch
episode mutates the same canonical count bank and group journal as scalar
`MainControl`; there is no standalone episode runner, benchmark-local semantic
executor, comparison adapter, or runtime engine choice. Correctness remains
owned by the repository's external fixtures and oracle workloads. The
episode's canonical tokenization/state seams, typed semantic barriers, counted
coverage fallback with mandatory admission/rollback proof, measurements, and
deletion-oriented migration plan are recorded in
`docs/native_batch_kernel.md`.
