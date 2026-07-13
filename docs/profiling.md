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
