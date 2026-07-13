# tex-out benchmarks

This standalone crate measures detached artifact and DVI work without engine,
page-builder, filesystem, or checkpoint costs.

Run the focused DVI cases with:

```bash
cargo bench --manifest-path benchmarks/tex-out/Cargo.toml --bench dvi
```

The fixtures cover a flat 4,096-character run, a 64-line nested page with 80
characters per line, and a 4,096-character run switching among 64 fonts.
Fixture construction, validation, and canonical v10 setup happen outside the
timed region.

- `fresh_commit/v10_only` measures canonical artifact encoding.
- `fresh_commit/v10_plus_dvi_plan` measures the production-style dual consumer
  that incrementally builds canonical v10 bytes and a precompiled DVI page.
- `plan_compile/owned_tree` measures DVI traversal from an owned detached page.
- `plan_compile/v10_stream` measures bounded streaming replay from canonical
  v10 bytes.
- `final_emit/owned_traversal` measures complete one-page DVI framing while
  traversing an owned artifact.
- `final_emit/precompiled_plan` measures the same complete DVI framing from a
  previously compiled page plan.

Absolute timings are machine-specific. Compare revisions on the same host,
toolchain, release profile, and fixture definitions.
