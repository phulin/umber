# PNG import prototype

This standalone host-only benchmark compares the former custom PNG alpha path,
`png::Reader`, and the adopted `png::StreamingDecoder` path. It is outside the
root workspace and does not execute Umber, so the Umber process watchdog does
not apply.

Run it against representative RGBA inputs from recent arXiv sample row
`2605.26196`:

```bash
cargo run --release --manifest-path benchmarks/png-import/Cargo.toml -- \
  third_party/arxiv-recent-sample-100/sources/2605.26196/figs/aspects.png \
  third_party/arxiv-recent-sample-100/sources/2605.26196/figs/explanation_example_v3.png \
  third_party/arxiv-recent-sample-100/sources/2605.26196/figs/comp-summary-all.png
```

The inputs contain 3,050,420 pixels and have SHA-256 digests
`094e4bce698f96547843aa4f4011183031577488a56bf8a9f30a86925b2afc09`,
`0fc93a8942c6b836f981025656b77f1ff18047e8211c218533685ef2f343765d`,
and `10b8d422395ca9242649ce6b997aa4292a4c6379fc0780b033e4f8a35571f9ab`.
The benchmark reads inputs before timing and reports the median of nine complete
decode/split/level-1-encode iterations. Output hashes cover the concatenated
color and alpha streams. Reader output is intentionally unfiltered, while the
other paths preserve each source filter byte.
