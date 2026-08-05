# tex-command benchmarks

This standalone crate contains focused command-core benchmarks and is excluded
from the root workspace correctness gate.

Run the allocation-count baseline with:

```bash
cargo run --release --manifest-path benchmarks/tex-command/Cargo.toml \
  --bin command_allocations
```

`command_allocations` directly exercises single-token backup, macro argument
matching, `scan_toks` absorption, keyword and dimension scanning, alignment
preamble scanning, command-text rendering, token-list iteration, and
control-sequence tokenization. Each row reports allocation count and requested
bytes per operation. The program builds fixed cases before measurement, runs a
discarded warmup, and then measures 64 operations with the same
`stats_alloc::Region` convention as the `tex-state` and `tex-exec` allocation
gates.

The `unobserved` configuration has neither an external observer nor paragraph
recording. `external_observer` attaches a non-allocating sink so the counts
include observation payload construction. `paragraph_recording` retains the
same observations in an active paragraph input transaction. Pure command-text
rendering supports only `unobserved`, because it has no processor observation
boundary. That workload clears and reuses a caller-owned render buffer around
the public append API, so it measures renderer-internal allocation rather than
the ownership allocation deliberately retained by the convenience wrapper.

To verify sensitivity, add `--perturb`. It deliberately requests one 64-byte
allocation per measured operation, so every reported row increases by exactly
one allocation and 64 requested bytes. The values are diagnostic baselines,
not correctness-test ceilings; optimization issues should record before and
after output from the same host, toolchain, profile, and revision.
