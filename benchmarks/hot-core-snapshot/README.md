# HotCore snapshot benchmark

This standalone crate owns the fixed-size HotCore runtime-mark controls added
by `umber2-awgc.2.3`. It does not run TeX command semantics.

Run the assertion-bearing warmed allocation and plateau gate with:

```bash
cargo run --manifest-path benchmarks/hot-core-snapshot/Cargo.toml
```

The gate warms every arena, stack, dense-bank, inverse-journal, and external
cursor path, then performs 10,000 aggregate accept/reject/retry cycles. It
requires zero allocation calls, zero requested bytes, exact retained-accounting
plateau, a 152-byte `HotSnapshot`, and zero snapshot-owned retained bytes.

Build or run the diagnostic Criterion rows with:

```bash
cargo bench --manifest-path benchmarks/hot-core-snapshot/Cargo.toml \
  --bench snapshots
```

The fixed-mark row compares empty, 1,024-word, and 65,536-word live states. The
rollback row mutates every storage family beneath one narrow mark.
