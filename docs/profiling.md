# Profiling Umber with Gentle

Use the persistent in-process Gentle runner when investigating whole-engine
hotspots:

```bash
scripts/profile-gentle.sh
```

The script builds `gentle-profile` with release optimizations and full debug
information, records 50 measured executions with Samply, and writes the
profile to `target/profiles/gentle.json.gz`. Samply also writes its
presymbolication sidecar next to the profile when supported. Override the run
counts and output path with:

```bash
GENTLE_PROFILE_ITERATIONS=100 \
GENTLE_PROFILE_WARMUPS=2 \
GENTLE_PROFILE_OUTPUT=/tmp/gentle.json.gz \
scripts/profile-gentle.sh
```

Extra arguments are forwarded to the runner. Run the optimized workload
without Samply when checking timing or setup:

```bash
cargo run --profile profiling -p umber --bin gentle-profile -- \
  --iterations 10 --warmups 1
```

Pass `--checkpoints` to exercise the enabled named-boundary capture path. The
runner consumes every published checkpoint and folds its semantic hash into a
bounded observation instead of retaining snapshots across iterations:

```bash
GENTLE_PROFILE_ITERATIONS=200 scripts/profile-gentle.sh --checkpoints
```

The runner requires the same external inputs as Gentle conformance. Populate
them with `scripts/setup-conformance-tests.sh` if necessary.

At startup, the runner reads `gentle.tex`, `plain.tex`, `hyphen.tex`, and the
available Computer Modern TFM files into a memory-backed `World`. Seeded input
bytes are structurally shared by every fresh run. One warm-up and all measured
iterations execute in the same process without temporary directories, corpus
copies, or host file reads. Each iteration still includes normal engine input
opening and hashing through `World`, engine initialization, expansion,
execution, shipout, and final DVI generation. After sampling, the runner checks
the final DVI against the warm-up result.

The ordinary end-to-end Gentle conformance test remains the correctness and
host-workflow measurement. This profiling runner deliberately removes its
repeated staging, oracle reads, artifact writes, and temporary-directory
cleanup so those operations do not obscure engine hotspots.

## Analyze a capture

Use the repository analyzer for a repeatable text report instead of manually
expanding stacks in the Samply UI:

```bash
scripts/analyze-profile.sh
scripts/analyze-profile.sh --top 40 target/profiles/gentle.json.gz
```

The report ranks self time and recursion-deduplicated inclusive time. It also
separates runtime self samples by library and attributes them to the nearest
Umber frame, which makes allocator and memory-operation costs visible without
losing their application caller. Percentages use Samply sample weights rather
than assuming every sample has weight one.

For a focused question, restrict the report to stacks beneath a function. The
subtree report adds the function's immediate callees and shows both the share
within the subtree and the share of the complete capture:

```bash
scripts/analyze-profile.sh \
  --subtree drain_pending_output \
  target/profiles/gentle.json.gz
```

Samply normally writes `gentle.json.syms.json` beside `gentle.json.gz`; the
analyzer discovers that sidecar automatically and uses its exact address map,
including inline frames. Pass `--symbols PATH` for a sidecar elsewhere. If no
sidecar exists, already-symbolized profile names remain useful and unresolved
addresses are reported explicitly rather than guessed. Use `--thread TEXT` or
`--app TEXT` when automatic thread or application-library selection is
ambiguous, and `--json` for machine-readable output.

Compiler inlining can make broad entry-point frames such as `main` or `run`
dominate a whole-profile inclusive ranking. Self time is unaffected; use a
named subtree when comparing the internal costs of a subsystem.

## Checkpoint optimization evidence

Measure checkpoint changes with both a 200-iteration Samply capture and a
thermally conditioned, interleaved wall-clock comparison. Use one warm-up and
ten measured runs per invocation; condition with `BOOB`, then measure
`BOOBOBBO` five times. Compare checkpoint-enabled binaries first and run the
disabled control only for a candidate that improves enabled timing. This
pairing is necessary because sub-percent Samply movements on thermally
constrained machines have repeatedly failed to predict throughput.

After the page forest and mutable tail gained structural lazy projections, a
200-run Gentle capture placed the complete `EngineSession::publish` subtree at
about 3.4% of whole-run samples. Its remaining work was fragmented across the
journal (about 0.6%), current page (about 0.6%), input hashing (about 0.4%), and
input summary construction and validation (about 0.4%). Experiments that
removed a sampled clone/drop, recursively composed page-tail roots, fused input
validation with projection, or changed canonical byte decoding were flat or
worse under the paired evidence. Treat those residual entries as attribution,
not independent optimization budgets: moving work between their inline frames
does not by itself demonstrate a faster checkpoint path.
