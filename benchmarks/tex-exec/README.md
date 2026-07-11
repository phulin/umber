# tex-exec benchmarks

This standalone crate contains focused execution-layer benchmarks that are
excluded from the root workspace correctness gate.

Run the shipout lowering cases with:

```bash
cargo bench --manifest-path benchmarks/tex-exec/Cargo.toml --bench shipout
cargo bench --manifest-path benchmarks/tex-exec/Cargo.toml --bench widths
```

`ordinary_hlist` measures the normal artifact-lowering fast path.
`deferred_math_lists` measures shipout-local Appendix G conversion for frozen
math lists that survived into a shipped tree. Both cases lower 1,024 child
nodes. Each Criterion iteration builds fresh state outside the timed region,
then times execution and artifact commit.

`widths` measures exact hpack width accumulation for 64- and 4,096-character
same-font runs and a 4,096-node mixed-font/interrupted list. It uses fixed
synthetic immutable TFM metrics, prepares arena state outside the timed loop,
and is the kernel budget for compact node-word width scans. The committed
means were remeasured after generation-tagged `NodeListId` expanded to two
words; the gate permits 10% cross-run noise above them. Absolute timing is
machine-specific, so comparisons require the same host, toolchain, profile,
and rebuilt revision.
