# tex-typeset benchmarks

This standalone crate owns the focused pure layout and compact-width
benchmarks. It is excluded from the root workspace correctness gate.

Run the Criterion workloads and deterministic allocation gate with:

```bash
cargo bench --manifest-path benchmarks/tex-typeset/Cargo.toml --bench widths
cargo bench --manifest-path benchmarks/tex-typeset/Cargo.toml --bench layout
cargo run --release --manifest-path benchmarks/tex-typeset/Cargo.toml \
  --bin layout_allocations
cargo run --release --manifest-path benchmarks/tex-typeset/Cargo.toml \
  --bin linebreak_direct
```

`widths` measures exact hpack width accumulation for 64- and 4,096-character
same-font runs and a 4,096-node mixed-font/interrupted list. It uses fixed
synthetic immutable TFM metrics, prepares arena state outside the timed loop,
and is the kernel budget for compact node-word width scans. The committed
means were remeasured after generation-tagged `NodeListId` expanded to two
words; the gate permits 10% cross-run noise above them. Absolute timing is
machine-specific. The gate compares only when the current Rust host triple and
exact compiler release match the committed baseline metadata. An unsupported
host is reported as a machine-readable, non-gating `unsupported` result and
exits `4`; it is neither a pass nor a regression. Baseline schema and row-name
drift remain hard failures on every host.

`layout` covers a 4,096-cell alignment with adversarial spans, a 1,024-node
paragraph, 20,000 nested math choices, 20,000 structural sub-mlists (both also
act as stack-safety gates), and repeated 1,024-noad conversion. The allocation
gate retains the larger 4,096-node paragraph workload. `layout_allocations`
measures the same pure kernels outside workload setup and enforces the existing
ceilings for allocation count and total allocated bytes; it remains outside
the ordinary unit-test tier.

`linebreak_direct` repeatedly analyzes one 4,096-record compact page-material
paragraph. It is the focused CPU and public-copy attribution workload for the
borrowed line-break scan; setup and compact publication occur before timing.
