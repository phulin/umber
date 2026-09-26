# Explicit node-copy timing

This opt-in release timing package depends only on `tex-state/testing` to
access the synthetic copy seam. The `testing` axis widens the API without
profiling counters; do not enable `tex-state/profiling` here. Build with
`cargo build --release --manifest-path benchmarks/tex-state/timing/Cargo.toml`
and run the binary pinned to a dedicated CPU. Source construction and
destination rollback stay outside each reported copy interval.

Keep the workload shapes and copied-node denominator visible in the output.
No wall-clock budget belongs in the routine test suite.
