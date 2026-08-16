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

`native_batch` compares canonical stepped `MainControl` with the bounded shared
production batch episode over one complete INITEX job. The job defines and calls parameterized macros,
performs local and global count assignments, branches with `\ifnum`, restores
an `\hbox` group, emits characters and explicit kerns, ships a page, and ends.
The `nested` shape adds a second macro that forwards its argument into two
inner calls.

Build once, run the exact differential, and measure each implementation in an
independent process:

```bash
CARGO_BUILD_JOBS=1 cargo build --release --manifest-path benchmarks/tex-exec/Cargo.toml --bin native_batch
python3 scripts/run-umber-guarded.py --timeout-seconds 600 --max-rss-mib 4096 -- benchmarks/tex-exec/target/release/native_batch compare 89551 10 direct
python3 scripts/run-umber-guarded.py --timeout-seconds 600 --max-rss-mib 4096 -- benchmarks/tex-exec/target/release/native_batch production 89551 10 direct
python3 scripts/run-umber-guarded.py --timeout-seconds 600 --max-rss-mib 4096 -- benchmarks/tex-exec/target/release/native_batch shared 89551 10 direct
python3 scripts/run-umber-guarded.py --timeout-seconds 600 --max-rss-mib 4096 -- benchmarks/tex-exec/target/release/native_batch compare 20000 0 nested
```

The timed allocation region begins after `Workload` constructs the immutable
source. Both implementations then create fresh engine state and the same
synthetic font, execute, retain all reported state and output, validate and
serialize the canonical page artifact, parse the serialized artifact, compile
and serialize DVI, and retain terminal text, log text, and effects. The batch
episode is deliberately not a supported engine choice. Its canonical
tokenization seam, typed fallback boundary, measurements, and deletion-oriented
migration plan are recorded in `docs/native_batch_kernel.md`.
